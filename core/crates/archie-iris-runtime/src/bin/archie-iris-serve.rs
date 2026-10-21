//! `archie-iris-serve`: IRIS on 127.0.0.1, without Python.
//!
//! ```text
//! archie-iris-serve --weights iris-beta21-step132000.pt --text-tower <dir> [--port 8765]
//!                   [--sha256 <hex>] [--device auto|cpu|cuda] [--rem-spool <dir>]
//!                   [--skills-dir <dir>] [--text-device cpu|model] [--phrase-cache <dir>]
//!                   [--live-max-fps <n>] [--live-max-priority <class>]
//!                   [--record-expert-usage] [--slink-dir <dir>]
//! ```
//!
//! `--slink-dir` exports the `scene.state` S-LINK (every live focus's tracks) to other
//! processes through `<dir>/scene.state.sock`, owner-only.
//!
//! `--record-expert-usage` counts how often each MoE expert is chosen, per layer, and
//! `GET /experts` reports it: the routing histograms residency decisions are made from.
//!
//! `/live` answers every frame of a focus at once (damage, tracking) and reads it with the
//! model only as often as the focus's budget allows; `--live-max-fps` and
//! `--live-max-priority` are the envelope no request or advice can exceed.
//!
//! `--text-tower` is a directory with SigLIP2's `model.safetensors`, `tokenizer.json` and
//! `tokenizer_config.json`. `--phrase-table <dir>` instead serves only the phrases in a
//! pre-encoded table (`phrases.json` + `phrases.safetensors`).
//!
//! The text tower runs on the CPU in float32 by default (`--text-device cpu`): it only runs
//! for phrases it has not seen, and on the GPU it held ~0.6 GiB. `--phrase-cache` keeps
//! phrase features on disk, so a restart encodes nothing it encoded before. The everyday
//! vocabulary is encoded before the server says it is ready.

use archie_accel::advice::Envelope;
use archie_accel::plan::Priority;
use archie_iris_runtime::act::Actor;
use archie_iris_runtime::detect::{Detector, EVERYDAY_VOCABULARY};
use archie_iris_runtime::model::IrisModel;
use archie_iris_runtime::phrases::{PhraseSource, PhraseTable, TextTower};
use archie_iris_runtime::server::{router, Service};
use archie_iris_runtime::weights::Weights;
use archie_iris_runtime::{choose_device, Error};
use candle_core::{DType, Device};
use serde_json::json;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage: archie-iris-serve --weights <file.pt> (--text-tower <dir> | --phrase-table <dir>) \
         [--port 8765] [--sha256 <hex>] [--device auto|cpu|cuda] [--rem-spool <dir>] [--skills-dir <dir>] \
         [--text-device cpu|model] [--phrase-cache <dir>] [--live-max-fps <n>] \
         [--live-max-priority background|wanda_interactive|active_focus|voice_reflex] \
         [--record-expert-usage] [--slink-dir <dir>]"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut weights, mut tower, mut table, mut sha256) = (None, None, None, None);
    let (mut rem_spool, mut skills_dir, mut phrase_cache) = (None, None, None);
    let mut text_device = "cpu".to_owned();
    let mut record_usage = false;
    let mut slink_dir = None;
    let mut envelope = Envelope {
        max_priority: Priority::ActiveFocus,
        max_fps: 30.0,
        max_vram_bytes: 0,
        allow_partial_experts: false,
        expert_floor: 0.5,
    };
    let (mut port, mut device) = (8765u16, "auto".to_owned());
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--weights" => weights = Some(PathBuf::from(value())),
            "--text-tower" => tower = Some(PathBuf::from(value())),
            "--phrase-table" => table = Some(PathBuf::from(value())),
            "--sha256" => sha256 = Some(value()),
            "--port" => port = value().parse().unwrap_or_else(|_| usage()),
            "--device" => device = value(),
            "--rem-spool" => rem_spool = Some(PathBuf::from(value())),
            "--skills-dir" => skills_dir = Some(PathBuf::from(value())),
            "--text-device" => text_device = value(),
            "--phrase-cache" => phrase_cache = Some(PathBuf::from(value())),
            "--record-expert-usage" => record_usage = true,
            "--slink-dir" => slink_dir = Some(PathBuf::from(value())),
            "--live-max-fps" => envelope.max_fps = value().parse().unwrap_or_else(|_| usage()),
            "--live-max-priority" => {
                envelope.max_priority = match value().as_str() {
                    "background" => Priority::Background,
                    "wanda_interactive" => Priority::WandaInteractive,
                    "active_focus" => Priority::ActiveFocus,
                    "voice_reflex" => Priority::VoiceReflex,
                    _ => usage(),
                }
            }
            _ => usage(),
        }
    }
    let weights_path = weights.unwrap_or_else(|| usage());
    let (device, dtype) = choose_device(&device)?;
    let started = std::time::Instant::now();
    let loaded = Weights::load(&weights_path, sha256.as_deref(), &device, dtype)?;
    let config = loaded.release.config.clone();
    let (text_on, text_dtype) = match text_device.as_str() {
        "cpu" => (Device::Cpu, DType::F32),
        "model" => (device.clone(), dtype),
        _ => usage(),
    };
    let mut phrases: Box<dyn PhraseSource> = match (tower, table) {
        (Some(dir), None) => {
            let tower = TextTower::load(
                &dir,
                config.text_length,
                config.text_tokens_per_phrase,
                &text_on,
                text_dtype,
            )?;
            Box::new(match &phrase_cache {
                Some(cache) => tower.with_disk_cache(cache)?,
                None => tower,
            })
        }
        (None, Some(dir)) => Box::new(PhraseTable::load(
            &dir.join("phrases.json"),
            &dir.join("phrases.safetensors"),
        )?),
        _ => usage(),
    };
    if phrases.describe()["kind"] != "table" {
        let vocabulary: Vec<String> = EVERYDAY_VOCABULARY.lines().map(str::to_owned).collect();
        phrases.encode(&vocabulary)?;
    }
    let model = IrisModel::load(&loaded)?;
    model.record_expert_usage(record_usage);
    let actor = match Actor::load(&loaded) {
        Ok(actor) => Some(actor),
        Err(Error::Weights(_)) => None,
        Err(error) => return Err(error.into()),
    };
    let reader = archie_iris_runtime::read::Reader::load(&loaded)?;
    let has_skills = actor.as_ref().is_some_and(|a| a.bank.is_some());
    let info = json!({
        "weights": weights_path.display().to_string(),
        "sha256": loaded.release.sha256,
        "step": loaded.release.step,
        "score": loaded.release.score,
        "device": format!("{device:?}"),
        "dtype": format!("{dtype:?}"),
        "parameters": loaded.parameter_count(),
        "tensors_on_device": loaded.resident_count(),
        "tensors_in_release": loaded.names().count(),
        "text_device": text_device,
        "instance_embeddings": true,
        "action_head": actor.is_some(),
        "skill_adapters": has_skills,
        "rem_recording": rem_spool.is_some(),
        "reading": reader.is_some(),
    });
    drop(loaded);
    let mut service = Service::new(Detector::new(model, phrases), actor, info)
        .with_rem_spool(rem_spool)
        .with_live_envelope(envelope)
        .with_slink_dir(slink_dir)
        .with_reader(reader);
    if let Some(dir) = skills_dir {
        service = service.with_skills_dir(dir);
    }
    let health = service.health();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!(
        "{}",
        json!({"event": "iris_serve_ready", "url": format!("http://127.0.0.1:{port}"),
               "load_seconds": started.elapsed().as_secs_f64(), "health": health})
    );
    axum::serve(listener, router(service)).await?;
    Ok(())
}
