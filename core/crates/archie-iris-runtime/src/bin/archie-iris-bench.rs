//! `archie-iris-bench`: where IRIS's time goes on this machine (F0 of the plan).
//!
//! ```text
//! archie-iris-bench --weights iris-beta21-step132000.pt (--text-tower <dir> | --phrase-table <dir>)
//!                   --image frame.jpg [--device auto|cpu|cuda] [--sha256 <hex>]
//!                   [--phrases a,b,c] [--rounds 10] [--live-seconds 10] [--camera-fps 60]
//!                   [--skip-open] [--record-expert-usage] [--out result.json]
//! ```
//!
//! Each scenario runs once to warm up, then `--rounds` times; medians and 90th percentiles
//! are reported in milliseconds:
//!
//! - `encode`: the two encoders on a new canvas (the frame cache is bypassed);
//! - `focused`: `detect` with the phrases (16 at most), a frame not seen before;
//! - `focused_cached`: the same frame again (encoders skipped by the frame cache);
//! - `open`: the everyday vocabulary, 17 rows;
//! - `multi4`: four foci with their own phrases in one pass (`detect_many`).
//!
//! `live` then plays a camera at `--camera-fps`: the image panned by two pixels a frame
//! through the live pipeline, with a worker running the model as the planner asks. It
//! reports how long each frame took to answer (damage and tracking, no model) and how many
//! model reads per second the worker completed, so "frames answered per second" and
//! "inferences per second" are measured separately. Nothing here is a claim until it has
//! run on the machine in question.

use archie_accel::advice::{Envelope, ResourceAdvice};
use archie_accel::plan::{FocusMode, Priority};
use archie_iris_runtime::choose_device;
use archie_iris_runtime::detect::{Detector, EVERYDAY_VOCABULARY};
use archie_iris_runtime::live::{spawn_worker, LiveConfig, LiveShared};
use archie_iris_runtime::model::IrisModel;
use archie_iris_runtime::phrases::{PhraseSource, PhraseTable, TextTower};
use archie_iris_runtime::preprocess::{decode, Frame, Rgb};
use archie_iris_runtime::weights::Weights;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn usage() -> ! {
    eprintln!(
        "usage: archie-iris-bench --weights <file.pt> (--text-tower <dir> | --phrase-table <dir>) \
         --image <jpg|png> [--device auto|cpu|cuda] [--sha256 <hex>] [--phrases a,b] [--rounds 10] \
         [--live-seconds 10] [--camera-fps 60] [--skip-open] [--record-expert-usage] [--out <json>]"
    );
    std::process::exit(2);
}

fn summary(mut samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    let round = |v: f64| (v * 10.0).round() / 10.0;
    json!({"median_ms": round(at(0.5)), "p90_ms": round(at(0.9)), "min_ms": round(samples[0]),
           "runs": samples.len(), "per_second_at_median": round(1000.0 / at(0.5))})
}

fn time(rounds: usize, mut run: impl FnMut(usize) -> Result<(), String>) -> Result<Value, String> {
    run(0)?;
    let mut samples = Vec::with_capacity(rounds);
    for round in 1..=rounds {
        let started = Instant::now();
        run(round)?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(summary(samples))
}

/// The image panned `shift` pixels right (wrapping), so every frame differs.
fn panned(image: &Rgb, shift: usize) -> Rgb {
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..image.height {
        for x in 0..image.width {
            let from = (y * image.width + (x + shift) % image.width) * 3;
            pixels.extend_from_slice(&image.pixels[from..from + 3]);
        }
    }
    Rgb::new(image.width, image.height, pixels).expect("same size")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut weights, mut tower, mut table, mut sha256, mut image_path, mut out) =
        (None, None, None, None, None, None);
    let mut device = "auto".to_owned();
    let mut phrases: Vec<String> = ["person", "cup", "phone", "screen", "keyboard", "hand"]
        .map(str::to_owned)
        .to_vec();
    let (mut rounds, mut live_seconds, mut camera_fps) = (10usize, 10f64, 60f64);
    let (mut skip_open, mut record_usage) = (false, false);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--weights" => weights = Some(PathBuf::from(value())),
            "--text-tower" => tower = Some(PathBuf::from(value())),
            "--phrase-table" => table = Some(PathBuf::from(value())),
            "--sha256" => sha256 = Some(value()),
            "--image" => image_path = Some(PathBuf::from(value())),
            "--device" => device = value(),
            "--phrases" => {
                phrases = value()
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            "--rounds" => rounds = value().parse().unwrap_or_else(|_| usage()),
            "--live-seconds" => live_seconds = value().parse().unwrap_or_else(|_| usage()),
            "--camera-fps" => camera_fps = value().parse().unwrap_or_else(|_| usage()),
            "--skip-open" => skip_open = true,
            "--record-expert-usage" => record_usage = true,
            "--out" => out = Some(PathBuf::from(value())),
            _ => usage(),
        }
    }
    let weights_path = weights.unwrap_or_else(|| usage());
    let image = decode(&std::fs::read(image_path.unwrap_or_else(|| usage()))?)?;
    let (device, dtype) = choose_device(&device)?;
    let started = Instant::now();
    let loaded = Weights::load(&weights_path, sha256.as_deref(), &device, dtype)?;
    let config = loaded.release.config.clone();
    let mut source: Box<dyn PhraseSource> = match (tower, table) {
        (Some(dir), None) => Box::new(TextTower::load(
            &dir,
            config.text_length,
            config.text_tokens_per_phrase,
            &candle_core::Device::Cpu,
            candle_core::DType::F32,
        )?),
        (None, Some(dir)) => Box::new(PhraseTable::load(
            &dir.join("phrases.json"),
            &dir.join("phrases.safetensors"),
        )?),
        _ => usage(),
    };
    if !skip_open {
        let vocabulary: Vec<String> = EVERYDAY_VOCABULARY.lines().map(str::to_owned).collect();
        source.encode(&vocabulary)?;
    }
    source.encode(&phrases)?;
    let model = IrisModel::load(&loaded)?;
    model.record_expert_usage(record_usage);
    let tensors = (loaded.resident_count(), loaded.names().count());
    drop(loaded);
    let load_seconds = started.elapsed().as_secs_f64();
    let size = config.image_size;
    let mut detector = Detector::new(model, source);
    let fresh = |round: usize| Frame::new(panned(&image, 3 * round + 1), size);

    let encode = time(rounds, |round| {
        let frame = fresh(round);
        detector
            .model
            .encode_image(
                &archie_iris_runtime::preprocess::canvas_tensor(
                    &frame.canvas,
                    &detector.model.device,
                )
                .map_err(|e| e.to_string())?,
            )
            .and_then(|t| {
                Ok(t.to_dtype(candle_core::DType::F32)?
                    .sum_all()?
                    .to_scalar::<f32>()?)
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    // With IRIS_PROFILE set, each stage waits for the GPU and the focused frames' time is
    // reported by stage (the totals are then slower than without it).
    let stages = |runs: usize| -> Value {
        let taken = archie_iris_runtime::profile::take();
        if taken.is_empty() {
            return Value::Null;
        }
        Value::Object(
            taken
                .into_iter()
                .map(|(name, total, _)| {
                    (
                        name.to_owned(),
                        json!((total / runs.max(1) as f64 * 10.0).round() / 10.0),
                    )
                })
                .collect(),
        )
    };
    let _ = stages(1);
    let focused = time(rounds, |round| {
        detector
            .detect_frame(&fresh(1000 + round), &phrases, 0.35, 20, 0.5, false)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    let focused_stages = stages(rounds + 1);
    let same = fresh(5000);
    let focused_cached = time(rounds, |_| {
        detector
            .detect_frame(&same, &phrases, 0.35, 20, 0.5, false)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    let focused_cached_stages = stages(rounds + 1);
    let open = if skip_open {
        Value::Null
    } else {
        time(rounds.min(5), |round| {
            detector
                .detect_everything_frame(&fresh(2000 + round), 0.35, &[], 60, 0.5, false)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })?
    };
    let quarters: Vec<Vec<String>> = (0..4)
        .map(|i| {
            phrases
                .iter()
                .skip(i % phrases.len().max(1))
                .chain(phrases.iter())
                .take(phrases.len().clamp(1, 4))
                .cloned()
                .collect()
        })
        .collect();
    let multi4 = time(rounds, |round| {
        let frames: Vec<Frame> = (0..4).map(|i| fresh(3000 + round * 4 + i)).collect();
        let requests: Vec<(&Frame, &[String])> = frames
            .iter()
            .zip(&quarters)
            .map(|(f, p)| (f, p.as_slice()))
            .collect();
        detector
            .detect_many(&requests, 0.35, 20, 0.5, false)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    let usage_after_static = detector.model.expert_usage()?;

    // A camera through the live pipeline, with the model on a worker.
    let detector = Arc::new(Mutex::new(detector));
    let envelope = Envelope {
        max_priority: Priority::ActiveFocus,
        max_fps: 60.0,
        max_vram_bytes: 0,
        allow_partial_experts: false,
        expert_floor: 0.5,
    };
    let live = LiveShared::new(LiveConfig::default(), envelope);
    live.advise(
        &ResourceAdvice {
            focus: "camera:bench".into(),
            share: 1.0,
            mode: FocusMode::Focused,
            phrases: phrases.clone(),
            read_text: false,
            priority: Priority::ActiveFocus,
            max_fps: 60.0,
        },
        0.35,
    )?;
    let reads = Arc::new(AtomicUsize::new(0));
    let worker_detector = detector.clone();
    let worker_reads = reads.clone();
    let worker = spawn_worker(live.clone(), move |batch| {
        let mut detector = worker_detector.lock().expect("detector");
        batch
            .iter()
            .map(|request| {
                let found = detector
                    .detect_frame(
                        &request.frame(size),
                        &request.phrases,
                        request.threshold,
                        20,
                        0.5,
                        false,
                    )
                    .map_err(|e| e.to_string());
                worker_reads.fetch_add(1, Ordering::Relaxed);
                found
            })
            .collect()
    });
    let period = Duration::from_secs_f64(1.0 / camera_fps.max(1.0));
    let frames_total = (live_seconds * camera_fps).round() as usize;
    let camera: Vec<Rgb> = (0..frames_total.min(240))
        .map(|i| panned(&image, 2 * i))
        .collect();
    let mut answer_ms = Vec::with_capacity(frames_total);
    let (mut tracked, mut empty) = (0usize, 0usize);
    let live_started = Instant::now();
    for index in 0..frames_total {
        let due = live_started + period * index as u32;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let frame = &camera[index % camera.len()];
        let answered = Instant::now();
        let (reply, request) =
            live.hub
                .lock()
                .expect("hub")
                .observe("camera:bench", frame, live.now_ms())?;
        if let Some(request) = request {
            live.submit(request);
        }
        live.publish_scene(&reply, live.now_ms())?;
        answer_ms.push(answered.elapsed().as_secs_f64() * 1000.0);
        if reply.detections.is_empty() {
            empty += 1;
        } else if reply
            .detections
            .iter()
            .any(|d| d.source == archie_iris_runtime::live::Source::Tracked)
        {
            tracked += 1;
        }
    }
    let live_elapsed = live_started.elapsed().as_secs_f64();
    let completed = reads.load(Ordering::Relaxed);
    live.shutdown();
    worker.join().ok();
    let answered_fps = frames_total as f64 / live_elapsed;
    let live_report = json!({
        "camera_fps": camera_fps,
        "frames": frames_total,
        "seconds": (live_elapsed * 100.0).round() / 100.0,
        "frames_answered_per_second": (answered_fps * 10.0).round() / 10.0,
        "answer": summary(answer_ms),
        "model_reads": completed,
        "model_reads_per_second": ((completed as f64 / live_elapsed) * 10.0).round() / 10.0,
        "frames_with_tracked_boxes": tracked,
        "frames_without_boxes": empty,
    });
    let report = json!({
        "event": "iris_bench",
        "weights": weights_path.display().to_string(),
        "device": format!("{device:?}"),
        "dtype": format!("{dtype:?}"),
        "flash_attention": cfg!(feature = "flash-attn"),
        "image": [image.width, image.height],
        "phrases": phrases,
        "load_seconds": (load_seconds * 100.0).round() / 100.0,
        "tensors_on_device": tensors.0,
        "tensors_in_release": tensors.1,
        "encode": encode,
        "focused": focused,
        "focused_cached": focused_cached,
        "focused_stages_ms": focused_stages,
        "focused_cached_stages_ms": focused_cached_stages,
        "open": open,
        "multi4": multi4,
        "live": live_report,
        "expert_usage": usage_after_static,
    });
    let text = serde_json::to_string_pretty(&report)?;
    println!("{text}");
    if let Some(path) = out {
        std::fs::write(path, text + "\n")?;
    }
    Ok(())
}
