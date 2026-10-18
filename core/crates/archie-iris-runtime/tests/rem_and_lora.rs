use archie_iris_runtime::lora::{
    default_targets, with_adapter, LoraPair, SkillAdapter, SkillBank, DEFAULT_ALPHA, DEFAULT_RANK,
};
use archie_iris_runtime::nn::Linear;
use archie_iris_runtime::preprocess::Rgb;
use archie_iris_runtime::rem::{inspect_window_meta, recorded_samples, spool_window, Recording};
use archie_iris_runtime::weights::{ModelConfig, Release, Weights};
use candle_core::{DType, Device, Tensor};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn default_lora_targets_match_python_bank_order() {
    let targets = default_targets(6, 4);
    assert_eq!(targets.len(), 35);
    assert_eq!(targets[0], "fusion.5.image_self.query");
    assert_eq!(targets[11], "fusion.5.text_image.output");
    assert_eq!(targets[12], "act.memory.layers.0.self_attn.out_proj");
    assert_eq!(targets[13], "act.memory.layers.0.linear1");
    assert_eq!(targets[14], "act.memory.layers.0.linear2");
    assert_eq!(targets[24], "act.step_type");
    assert_eq!(targets[34], "act.pointer_refine");
}

#[test]
fn linear_applies_active_lora_delta_and_restores_base_on_exit() {
    let device = Device::Cpu;
    let mut map = HashMap::new();
    let w = Tensor::from_vec(vec![1.0f32, 0.0, 0.0, 1.0], (2, 2), &device).unwrap();
    let b = Tensor::from_vec(vec![0.5f32, -0.5], 2, &device).unwrap();
    map.insert("act.pointer_refine.weight".to_owned(), w);
    map.insert("act.pointer_refine.bias".to_owned(), b);
    let release = Release {
        path: PathBuf::from("test.pt"),
        sha256: "a".repeat(64),
        format: "archie-iris-beta1-weights-v1".to_owned(),
        step: Some(1),
        score: None,
        config: ModelConfig {
            image_size: 64,
            dimension: 32,
            heads: 4,
            feedforward: 64,
            fusion_layers: 1,
            decoder_layers: 1,
            experts: 4,
            expert_top_k: 2,
            expert_capacity_factor: 1.5,
            queries: 10,
            max_phrases: 4,
            text_length: 16,
            text_tokens_per_phrase: 1,
            act_head: true,
            act_layers: 1,
            act_slots: 4,
            act_chunk: 1,
            siglip_repository: String::new(),
            siglip_revision: String::new(),
        },
    };
    let weights = Weights::from_tensors(map, release);
    let linear = Linear::load(&weights, "act.pointer_refine").unwrap();
    let x = Tensor::from_vec(vec![2.0f32, 3.0], (1, 2), &device).unwrap();

    let base_out: Vec<f32> = linear
        .forward(&x)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    assert_eq!(base_out, vec![2.5, 2.5]);

    let a = Tensor::from_vec(vec![1.0f32, 1.0], (1, 2), &device).unwrap();
    let b_lora = Tensor::from_vec(vec![0.5f32, -1.0], (2, 1), &device).unwrap();
    let mut pairs = HashMap::new();
    pairs.insert("act.pointer_refine".to_owned(), LoraPair { a, b: b_lora });
    let adapter = Arc::new(SkillAdapter {
        skill: "browser".to_owned(),
        scale: DEFAULT_ALPHA / DEFAULT_RANK as f64, // 2.0
        pairs,
    });

    let adapted: Vec<f32> = with_adapter(Some(adapter), || linear.forward(&x))
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    // x @ a.T = [5.0]; [5.0] @ b.T = [2.5, -5.0]; scaled by 2.0 = [5.0, -10.0]
    assert_eq!(adapted, vec![7.5, -7.5]);

    let after: Vec<f32> = linear
        .forward(&x)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    assert_eq!(after, base_out);
}

#[test]
fn base_fingerprint_matches_across_f32_and_bf16_and_ignores_text_tower() {
    let device = Device::Cpu;
    let release = Release {
        path: PathBuf::from("test.pt"),
        sha256: "b".repeat(64),
        format: "archie-iris-beta1-weights-v1".to_owned(),
        step: Some(1),
        score: None,
        config: ModelConfig {
            image_size: 64,
            dimension: 32,
            heads: 4,
            feedforward: 64,
            fusion_layers: 1,
            decoder_layers: 1,
            experts: 4,
            expert_top_k: 2,
            expert_capacity_factor: 1.5,
            queries: 10,
            max_phrases: 4,
            text_length: 16,
            text_tokens_per_phrase: 1,
            act_head: true,
            act_layers: 1,
            act_slots: 4,
            act_chunk: 1,
            siglip_repository: String::new(),
            siglip_revision: String::new(),
        },
    };
    let t_bf16 = Tensor::from_vec(vec![0.125f32, -0.5, 1.25, 3.0], (2, 2), &device)
        .unwrap()
        .to_dtype(DType::BF16)
        .unwrap();
    let t_f32 = t_bf16.to_dtype(DType::F32).unwrap();

    let mut m1 = HashMap::new();
    m1.insert("act.keys.weight".to_owned(), t_f32);
    m1.insert(
        "text.model.encoder.weight".to_owned(),
        Tensor::zeros((4, 4), DType::F32, &device).unwrap(),
    );
    let w1 = Weights::from_tensors(m1, release.clone());

    let mut m2 = HashMap::new();
    m2.insert("act.keys.weight".to_owned(), t_bf16);
    let w2 = Weights::from_tensors(m2, release);

    assert_eq!(
        w1.base_fingerprint().unwrap(),
        w2.base_fingerprint().unwrap()
    );
}

#[test]
fn rem_spool_writes_npz_window_and_sidecar_and_verifies_digests() {
    let dir = std::env::temp_dir().join(format!(
        "archie-iris-rem-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let base_sha = "c6cadb02f5a5bffc4ea3c3348d32b475ef7531f643675c06c65451037e4f3b97";

    assert!(Recording::validate_new(dir.clone(), "../bad", "run-1", base_sha, "public").is_err());
    assert!(Recording::validate_new(dir.clone(), "browser", "-bad", base_sha, "public").is_err());
    assert!(Recording::validate_new(dir.clone(), "browser", "run-1", base_sha, "secret").is_err());

    let rec = Recording::validate_new(dir.clone(), "browser", "run-20261018", base_sha, "public")
        .unwrap();
    let frame = Rgb {
        width: 8,
        height: 8,
        pixels: vec![128u8; 8 * 8 * 3],
    };
    let recent = vec![(frame.clone(), 9.0), (frame.clone(), 9.5)];
    let compacted = vec![
        (frame.clone(), 7.0),
        (frame.clone(), 7.5),
        (frame.clone(), 8.0),
        (frame.clone(), 8.5),
    ];
    let events = vec![
        (vec![0.0f32; 138], 8.0),
        (vec![0.0f32; 138], 8.5),
        (vec![0.0f32; 138], 9.0),
        (vec![0.0f32; 138], 9.5),
    ];
    let control = json!({
        "step_type": "click",
        "keys": ["enter"],
        "buttons": ["left"],
        "mouse": [0, 0],
        "pointer_canvas": [0.25, 0.75],
        "scroll": "none",
        "pad_axes": {
            "left_x": 0.0,
            "left_y": 0.0,
            "right_x": 0.0,
            "right_y": 0.0,
            "left_trigger": 0.0,
            "right_trigger": 0.0
        },
        "pad_buttons": []
    });

    let npz_path = spool_window(
        &rec,
        "000000",
        "click search button",
        "browser",
        &frame,
        &recent,
        &compacted,
        &events,
        &control,
        10.0,
    )
    .unwrap();
    assert!(npz_path.is_file());

    let meta = inspect_window_meta(&npz_path).unwrap();
    assert_eq!(meta["sample_id"], "000000");
    assert_eq!(meta["source"], "browser");
    assert_eq!(meta["durable"], json!(["browser"]));
    assert_eq!(meta["target"]["step_type"], 1);
    assert_eq!(meta["target"]["pointer"], json!([0.25, 0.75]));

    let samples = recorded_samples(&dir, "browser", "run-20261018").unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["sample_id"], "000000");
    assert_eq!(samples[0]["receipt_id"], "run-20261018.000000");
    assert_eq!(samples[0]["base_sha256"], base_sha);
    assert_eq!(samples[0]["sensitivity"], "public");
    assert_eq!(samples[0]["window_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(samples[0]["target_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(samples[0]["future_sha256"].as_str().unwrap().len(), 64);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skill_bank_rejects_invalid_names_and_missing_files() {
    let device = Device::Cpu;
    let targets = default_targets(1, 1);
    let mut map = HashMap::new();
    for name in &targets {
        map.insert(
            format!("{name}.weight"),
            Tensor::zeros((4, 4), DType::F32, &device).unwrap(),
        );
    }
    let release = Release {
        path: PathBuf::from("test.pt"),
        sha256: "d".repeat(64),
        format: "archie-iris-beta1-weights-v1".to_owned(),
        step: Some(1),
        score: None,
        config: ModelConfig {
            image_size: 64,
            dimension: 4,
            heads: 2,
            feedforward: 8,
            fusion_layers: 1,
            decoder_layers: 1,
            experts: 2,
            expert_top_k: 1,
            expert_capacity_factor: 1.5,
            queries: 4,
            max_phrases: 2,
            text_length: 8,
            text_tokens_per_phrase: 1,
            act_head: true,
            act_layers: 1,
            act_slots: 2,
            act_chunk: 1,
            siglip_repository: String::new(),
            siglip_revision: String::new(),
        },
    };
    let weights = Weights::from_tensors(map, release);
    let mut bank = SkillBank::from_weights(&weights, device, DType::F32).unwrap();
    assert!(bank
        .use_skill("../escape", std::path::Path::new("/tmp"))
        .is_err());
    assert!(bank
        .use_skill("nonexistent_skill", std::path::Path::new("/tmp"))
        .is_err());
}
