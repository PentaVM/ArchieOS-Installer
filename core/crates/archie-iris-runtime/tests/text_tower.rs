//! The SigLIP2 text tower against Hugging Face's `SiglipTextModel`, on random weights.
//!
//! The release does not carry the tower and this check does not need the real one: the
//! same architecture on the same random weights and ids has to give the same numbers, with
//! and without an attention mask.
//!
//! ```text
//! IRIS_TEXT_REFERENCE=<dir with text_tower_random.safetensors and text_reference.safetensors> \
//!   cargo test --release -p archie-iris-runtime --test text_tower -- --ignored --nocapture
//! ```

use archie_iris_runtime::siglip::{load_text_weights, SiglipText};
use archie_iris_runtime::weights::{ModelConfig, Release, Weights};
use candle_core::{DType, Device, Tensor};

fn max_relative(ours: &Tensor, theirs: &Tensor) -> f32 {
    let a: Vec<f32> = ours.flatten_all().unwrap().to_vec1().unwrap();
    let b: Vec<f32> = theirs
        .to_dtype(DType::F32)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    let diff = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0f32, f32::max);
    diff / b.iter().map(|y| y.abs()).fold(0f32, f32::max)
}

#[test]
#[ignore = "needs reference outputs written by transformers"]
fn the_text_tower_matches_transformers() {
    let Some(dir) = std::env::var_os("IRIS_TEXT_REFERENCE").map(std::path::PathBuf::from) else {
        return;
    };
    let device = Device::Cpu;
    let tensors = load_text_weights(
        &dir.join("text_tower_random.safetensors"),
        "text_model",
        &device,
        DType::F32,
    )
    .unwrap();
    let release = Release {
        path: dir.clone(),
        sha256: String::new(),
        format: "test".into(),
        step: None,
        score: None,
        config: ModelConfig {
            image_size: 384,
            dimension: 768,
            heads: 12,
            feedforward: 3072,
            fusion_layers: 0,
            decoder_layers: 0,
            experts: 1,
            expert_top_k: 1,
            expert_capacity_factor: 1.0,
            queries: 0,
            max_phrases: 0,
            text_length: 64,
            text_tokens_per_phrase: 1,
            act_head: false,
            act_layers: 0,
            act_slots: 0,
            act_chunk: 1,
            siglip_repository: String::new(),
            siglip_revision: String::new(),
        },
    };
    let tower = SiglipText::load(&Weights::from_tensors(tensors, release), "text_model").unwrap();
    let reference =
        candle_core::safetensors::load(dir.join("text_reference.safetensors"), &device).unwrap();
    let ids = reference["ids"].to_dtype(DType::U32).unwrap();
    let mask = reference["mask"].to_dtype(DType::U8).unwrap();
    let plain = tower.forward(&ids, None).unwrap();
    let masked = tower.forward(&ids, Some(&mask)).unwrap();
    let checks = [
        (
            "pooled, no mask",
            max_relative(&plain.pooled, &reference["plain_pooled"]),
        ),
        (
            "states, no mask",
            max_relative(&plain.states, &reference["plain_states"]),
        ),
        (
            "pooled, masked",
            max_relative(&masked.pooled, &reference["masked_pooled"]),
        ),
        (
            "states, masked",
            max_relative(&masked.states, &reference["masked_states"]),
        ),
    ];
    for (name, relative) in checks {
        println!("{name:18} relative {relative:.2e}");
        assert!(relative < 1e-4, "{name}");
    }
}
