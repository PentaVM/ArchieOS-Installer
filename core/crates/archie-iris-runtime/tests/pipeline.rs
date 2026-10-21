//! The whole runtime on a release-shaped model with random weights: every tensor the
//! runtime loads, at the release's shapes where they are fixed (the two 768-wide, 12-layer
//! encoders, `[E, 1, H]` expert biases) and small where the configuration sets them (a
//! 64-pixel canvas, width 32). Numbers mean nothing; what is checked is that every path
//! runs end to end and that paths meant to agree do: several foci in one pass against each
//! alone, and the live pipeline with a real detector behind its worker.

use archie_accel::advice::{Envelope, ResourceAdvice};
use archie_accel::plan::{FocusMode, Priority};
use archie_iris_runtime::detect::{Detection, Detector};
use archie_iris_runtime::live::{spawn_worker, LiveConfig, LiveShared, Source};
use archie_iris_runtime::model::IrisModel;
use archie_iris_runtime::phrases::{PhraseFeatures, PhraseSource};
use archie_iris_runtime::preprocess::{Frame, Rgb};
use archie_iris_runtime::weights::{ModelConfig, Release, Weights};
use archie_iris_runtime::Error;
use candle_core::{DType, Device, Tensor};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const D: usize = 32;
const FF: usize = 64;
const EXPERTS: usize = 4;
const HIDDEN: usize = 48;
const ENCODER: usize = 768;

fn config() -> ModelConfig {
    ModelConfig {
        image_size: 64,
        dimension: D,
        heads: 4,
        feedforward: FF,
        fusion_layers: 2,
        decoder_layers: 2,
        experts: EXPERTS,
        expert_top_k: 2,
        expert_capacity_factor: 1.5,
        queries: 8,
        max_phrases: 4,
        text_length: 16,
        text_tokens_per_phrase: 2,
        act_head: false,
        act_layers: 0,
        act_slots: 0,
        act_chunk: 1,
        siglip_repository: String::new(),
        siglip_revision: String::new(),
    }
}

struct Builder {
    map: HashMap<String, Tensor>,
    /// One tensor per shape, shared by every name of that shape: the encoders' 24 layers
    /// take one layer's memory.
    shared: HashMap<Vec<usize>, Tensor>,
}

impl Builder {
    fn tensor(&mut self, name: &str, shape: &[usize], scale: f64) {
        let t = self
            .shared
            .entry(shape.to_vec())
            .or_insert_with(|| Tensor::randn(0f32, scale as f32, shape, &Device::Cpu).unwrap())
            .clone();
        self.map.insert(name.to_owned(), t);
    }

    fn fresh(&mut self, name: &str, shape: &[usize], scale: f32) {
        let t = Tensor::randn(0f32, scale, shape, &Device::Cpu).unwrap();
        self.map.insert(name.to_owned(), t);
    }

    fn norm(&mut self, prefix: &str, width: usize) {
        let ones = Tensor::ones(width, DType::F32, &Device::Cpu).unwrap();
        self.map.insert(format!("{prefix}.weight"), ones);
        self.fresh(&format!("{prefix}.bias"), &[width], 0.02);
    }

    fn linear(&mut self, prefix: &str, out: usize, input: usize) {
        let scale = 1.0 / (input as f64).sqrt();
        self.tensor(&format!("{prefix}.weight"), &[out, input], scale);
        self.fresh(&format!("{prefix}.bias"), &[out], 0.02);
    }

    fn attention(&mut self, prefix: &str) {
        for part in ["query", "key", "value", "output"] {
            self.linear(&format!("{prefix}.{part}"), D, D);
        }
    }

    fn scorer(&mut self, prefix: &str) {
        self.linear(&format!("{prefix}.region"), D, D);
        self.linear(&format!("{prefix}.phrase"), D, D);
        let log_scale = Tensor::new(2.3f32, &Device::Cpu).unwrap();
        let bias = Tensor::new(-1.0f32, &Device::Cpu).unwrap();
        self.map.insert(format!("{prefix}.log_scale"), log_scale);
        self.map.insert(format!("{prefix}.bias"), bias);
    }

    fn box_head(&mut self, prefix: &str) {
        self.linear(&format!("{prefix}.mlp.0"), D, D);
        self.linear(&format!("{prefix}.mlp.2"), D, D);
        self.linear(&format!("{prefix}.mlp.4"), 4, D);
    }
}

fn weights() -> Weights {
    let mut b = Builder {
        map: HashMap::new(),
        shared: HashMap::new(),
    };
    let c = config();
    let grid = c.image_size / 16;
    // V-JEPA 2.1 ViT-B.
    b.tensor(
        "vjepa.encoder.patch_embed_img.proj.weight",
        &[ENCODER, 3, 1, 16, 16],
        0.02,
    );
    b.fresh("vjepa.encoder.patch_embed_img.proj.bias", &[ENCODER], 0.02);
    b.fresh("vjepa.encoder.img_mod_embed", &[1, 1, ENCODER], 0.02);
    for i in 0..12 {
        let p = format!("vjepa.encoder.blocks.{i}");
        b.norm(&format!("{p}.norm1"), ENCODER);
        b.linear(&format!("{p}.attn.qkv"), 3 * ENCODER, ENCODER);
        b.linear(&format!("{p}.attn.proj"), ENCODER, ENCODER);
        b.norm(&format!("{p}.norm2"), ENCODER);
        b.linear(&format!("{p}.mlp.fc1"), 4 * ENCODER, ENCODER);
        b.linear(&format!("{p}.mlp.fc2"), ENCODER, 4 * ENCODER);
    }
    b.norm("vjepa.encoder.norms_block.3", ENCODER);
    // SigLIP2 vision, with a smaller position table than the grid (interpolated, as 24 -> 32).
    b.tensor(
        "siglip.model.embeddings.patch_embedding.weight",
        &[ENCODER, 3, 16, 16],
        0.02,
    );
    b.fresh(
        "siglip.model.embeddings.patch_embedding.bias",
        &[ENCODER],
        0.02,
    );
    b.fresh(
        "siglip.model.embeddings.position_embedding.weight",
        &[9, ENCODER],
        0.02,
    );
    for i in 0..12 {
        let p = format!("siglip.model.encoder.layers.{i}");
        b.norm(&format!("{p}.layer_norm1"), ENCODER);
        for part in ["q_proj", "k_proj", "v_proj", "out_proj"] {
            b.linear(&format!("{p}.self_attn.{part}"), ENCODER, ENCODER);
        }
        b.norm(&format!("{p}.layer_norm2"), ENCODER);
        b.linear(&format!("{p}.mlp.fc1"), 4 * ENCODER, ENCODER);
        b.linear(&format!("{p}.mlp.fc2"), ENCODER, 4 * ENCODER);
    }
    b.norm("siglip.model.post_layernorm", ENCODER);
    for projection in [
        "vjepa_projection",
        "siglip_projection",
        "text_projection",
        "text_token_projection",
    ] {
        b.norm(&format!("{projection}.0"), ENCODER);
        b.linear(&format!("{projection}.1"), D, ENCODER);
    }
    b.fresh("phrase_slot.weight", &[c.max_phrases, D], 0.1);
    b.fresh("position", &[1, grid * grid, D], 0.1);
    b.fresh("null_phrase", &[D], 0.1);
    for i in 0..c.fusion_layers {
        let p = format!("fusion.{i}");
        for norm in [
            "image_norm1",
            "image_norm2",
            "image_norm3",
            "text_norm1",
            "text_norm2",
        ] {
            b.norm(&format!("{p}.{norm}"), D);
        }
        for attention in ["image_self", "image_text", "text_image"] {
            b.attention(&format!("{p}.{attention}"));
        }
        b.linear(&format!("{p}.moe.router"), EXPERTS, D);
        b.linear(&format!("{p}.moe.shared.0"), FF, D);
        b.linear(&format!("{p}.moe.shared.2"), D, FF);
        b.fresh(&format!("{p}.moe.w_in"), &[EXPERTS, D, HIDDEN], 0.2);
        b.fresh(&format!("{p}.moe.b_in"), &[EXPERTS, 1, HIDDEN], 0.02);
        b.fresh(&format!("{p}.moe.w_out"), &[EXPERTS, HIDDEN, D], 0.15);
        b.fresh(&format!("{p}.moe.b_out"), &[EXPERTS, 1, D], 0.02);
        b.linear(&format!("{p}.text_ffn.0"), FF, D);
        b.linear(&format!("{p}.text_ffn.2"), D, FF);
    }
    b.norm("fused_norm", D);
    b.scorer("token_scorer");
    b.box_head("token_box");
    b.linear("query_projection.0", D, D);
    b.norm("query_projection.1", D);
    for i in 0..c.decoder_layers {
        let p = format!("decoder.{i}");
        for norm in ["norm1", "norm2", "norm3", "norm4"] {
            b.norm(&format!("{p}.{norm}"), D);
        }
        for attention in ["self_attention", "image_attention", "text_attention"] {
            b.attention(&format!("{p}.{attention}"));
        }
        b.linear(&format!("{p}.ffn.0"), FF, D);
        b.linear(&format!("{p}.ffn.2"), D, FF);
        b.linear(&format!("{p}.box_position.0"), D, 4);
        b.linear(&format!("{p}.box_position.2"), D, D);
        b.scorer(&format!("decoder_scorers.{i}"));
        b.box_head(&format!("decoder_boxes.{i}"));
    }
    b.norm("instance_head.0", D * 9);
    b.linear("instance_head.1", 64, D * 9);
    b.linear("instance_head.3", 16, 64);
    Weights::from_tensors(
        b.map,
        Release {
            path: PathBuf::from("random.pt"),
            sha256: "0".repeat(64),
            format: "archie-iris-beta1-weights-v1".into(),
            step: None,
            score: None,
            config: config(),
        },
    )
}

/// Text features from the phrase itself, so a phrase always reads the same.
struct Phrases;

impl PhraseSource for Phrases {
    fn encode(&mut self, phrases: &[String]) -> Result<Vec<Arc<PhraseFeatures>>, Error> {
        phrases
            .iter()
            .map(|phrase| {
                let seed = phrase
                    .bytes()
                    .fold(17u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
                let value = |i: u32| {
                    ((seed.wrapping_mul(2654435761).wrapping_add(i * 40503)) % 2000) as f32 / 1000.0
                        - 1.0
                };
                let pooled = (0..ENCODER as u32).map(value).collect();
                let rows = 1 + (seed % 2) as u32;
                let states = (0..rows)
                    .map(|r| {
                        (0..ENCODER as u32)
                            .map(|i| value(i + 1000 * (r + 1)))
                            .collect()
                    })
                    .collect();
                Ok(Arc::new(PhraseFeatures::new(pooled, states)?))
            })
            .collect()
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({"kind": "test"})
    }
}

fn detector() -> Detector {
    Detector::new(IrisModel::load(&weights()).unwrap(), Box::new(Phrases))
}

fn picture(width: usize, height: usize, shift: usize) -> Rgb {
    let mut pixels = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let (u, v) = ((x + shift) as u32, y as u32);
            pixels.extend_from_slice(&[
                ((u * 7 + v * 3) % 256) as u8,
                ((u * v) % 251) as u8,
                ((u * 13 + v * 29) % 256) as u8,
            ]);
        }
    }
    Rgb::new(width, height, pixels).unwrap()
}

fn phrases(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

fn close(a: &[Detection], b: &[Detection]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.phrase == y.phrase
                && (x.score - y.score).abs() <= 1e-3
                && x.bbox
                    .iter()
                    .zip(&y.bbox)
                    .all(|(p, q)| (p - q).abs() <= 0.11)
        })
}

#[test]
fn every_detection_path_runs_on_a_release_shaped_model() {
    let mut detector = detector();
    let frame = Frame::new(picture(96, 72, 0), 64);
    let found = detector
        .detect_frame(
            &frame,
            &phrases(&["cup", "phone", "person"]),
            0.0,
            3,
            0.5,
            true,
        )
        .unwrap();
    assert!(!found.is_empty());
    for d in &found {
        assert!(d.bbox[0] >= 0.0 && d.bbox[2] <= 96.0 && d.bbox[1] >= 0.0 && d.bbox[3] <= 72.0);
        assert_eq!(d.embedding.as_ref().map(Vec::len), Some(16));
    }
    // The same frame again comes from the frame cache and reads the same.
    let again = detector
        .detect_frame(
            &frame,
            &phrases(&["cup", "phone", "person"]),
            0.0,
            3,
            0.5,
            true,
        )
        .unwrap();
    assert_eq!(found, again);
    // Open mode over the everyday vocabulary, 4 phrases a row.
    let open = detector
        .detect_everything_frame(&frame, 0.0, &phrases(&["red button"]), 10, 0.5, true)
        .unwrap();
    assert!(!open.is_empty() && open.len() <= 10);
    assert!(open.iter().all(|d| d.embedding.is_some()));
    detector.model.record_expert_usage(true);
    detector
        .detect_frame(&frame, &phrases(&["cup"]), 0.5, 3, 0.5, false)
        .unwrap();
    let usage = detector.model.expert_usage().unwrap();
    assert_eq!(usage.len(), 2);
    // 16 image tokens, top-2: 32 choices per layer.
    assert!(usage.iter().all(|layer| layer.iter().sum::<u64>() == 32));
}

#[test]
fn several_foci_in_one_pass_read_as_each_alone() {
    let mut detector = detector();
    let frames: Vec<Frame> = (0..2)
        .map(|i| Frame::new(picture(80, 64, 11 * i), 64))
        .collect();
    let asked = [
        phrases(&["cup", "phone", "keys"]),
        phrases(&["person", "cup", "door", "screen"]),
    ];
    let requests: Vec<(&Frame, &[String])> = frames
        .iter()
        .zip(&asked)
        .map(|(f, p)| (f, p.as_slice()))
        .collect();
    let together = detector.detect_many(&requests, 0.0, 4, 0.5, false).unwrap();
    for (index, (frame, asked)) in requests.iter().enumerate() {
        let alone = detector
            .detect_frame(frame, asked, 0.0, 4, 0.5, false)
            .unwrap();
        assert!(
            close(&together[index], &alone),
            "focus {index}: {:?} vs {:?}",
            together[index],
            alone
        );
    }
}

#[test]
fn the_live_pipeline_runs_with_a_real_detector() {
    let detector = Arc::new(Mutex::new(detector()));
    let envelope = Envelope {
        max_priority: Priority::ActiveFocus,
        max_fps: 30.0,
        max_vram_bytes: 0,
        allow_partial_experts: false,
        expert_floor: 0.5,
    };
    let live = LiveShared::new(LiveConfig::default(), envelope);
    live.advise(
        &ResourceAdvice {
            focus: "camera:test".into(),
            share: 1.0,
            mode: FocusMode::Focused,
            phrases: phrases(&["cup", "phone"]),
            read_text: false,
            priority: Priority::ActiveFocus,
            max_fps: 30.0,
        },
        0.0,
    )
    .unwrap();
    let worker_detector = detector.clone();
    let worker = spawn_worker(live.clone(), move |batch| {
        let mut detector = worker_detector.lock().unwrap();
        batch
            .iter()
            .map(|r| {
                detector
                    .detect_frame(&r.frame, &r.phrases, r.threshold, 3, 0.5, false)
                    .map_err(|e| e.to_string())
            })
            .collect()
    });
    // The first frame asks for a read; wait for the worker to finish it.
    let first = Frame::new(picture(160, 120, 0), 64);
    let (_, request) = live
        .hub
        .lock()
        .unwrap()
        .observe("camera:test", &first, live.now_ms())
        .unwrap();
    live.submit(request.expect("a first frame is read"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while live.hub.lock().unwrap().foci()[0]
        .2
        .last_inference_ms
        .is_none()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut sources = Vec::new();
    let mut inferred = false;
    for i in 1..40 {
        let frame = Frame::new(picture(160, 120, i), 64);
        let (reply, request) = live
            .hub
            .lock()
            .unwrap()
            .observe("camera:test", &frame, live.now_ms())
            .unwrap();
        if let Some(request) = request {
            live.submit(request);
        }
        live.publish_scene(&reply, live.now_ms()).unwrap();
        inferred |= reply.inference_age_ms.is_some();
        sources.extend(reply.detections.iter().map(|d| d.source));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    live.shutdown();
    worker.join().unwrap();
    assert!(inferred, "the worker read at least one frame");
    assert!(sources
        .iter()
        .any(|s| matches!(s, Source::Tracked | Source::Inferred | Source::Lost)));
    assert!(live.scene.revision() >= 39);
}
