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

/// The real tower (`google/siglip2-base-patch16-384` at `f775b65`) and its Gemma tokenizer
/// against `transformers` on the same phrases (lowercased, padded to 64, no attention mask:
/// the repository asks for none), including Spanish with accents and an empty phrase.
///
/// Two checks: the tokenizer gives the same 64 ids and the tower on them the same numbers
/// (float32); and what `TextTower::encode` hands fusion is those numbers rounded through
/// float16, as IRIS caches phrase features.
///
/// ```text
/// IRIS_TEXT_TOWER=<the repository's files> IRIS_TEXT_TOWER_REFERENCE=<real_tower.*> \
///   cargo test --release -p archie-iris-runtime --test text_tower -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs the real SigLIP2 text tower and reference outputs written by transformers"]
fn the_real_tower_and_tokenizer_match_transformers() {
    use archie_iris_runtime::phrases::{PhraseSource, TextTower};
    let (Some(tower), Some(reference)) = (
        std::env::var_os("IRIS_TEXT_TOWER").map(std::path::PathBuf::from),
        std::env::var_os("IRIS_TEXT_TOWER_REFERENCE").map(std::path::PathBuf::from),
    ) else {
        return;
    };
    let device = Device::Cpu;
    let mut text = TextTower::load(&tower, 64, 32, &device, DType::F32).unwrap();
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reference.join("real_tower.json")).unwrap()).unwrap();
    let phrases: Vec<String> = meta["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    let stored =
        candle_core::safetensors::load(reference.join("real_tower.safetensors"), &device).unwrap();
    let ids: Vec<Vec<u32>> = stored["ids"]
        .to_vec2::<i64>()
        .unwrap()
        .into_iter()
        .map(|row| row.into_iter().map(|id| id as u32).collect())
        .collect();

    // 1. The tokenizer, configured as the runtime configures it.
    let mut tokenizer = tokenizers::Tokenizer::from_file(tower.join("tokenizer.json")).unwrap();
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: 64,
            ..Default::default()
        }))
        .unwrap();
    tokenizer.with_padding(Some(tokenizers::PaddingParams {
        strategy: tokenizers::PaddingStrategy::Fixed(64),
        pad_id: 0,
        pad_token: "<pad>".into(),
        ..Default::default()
    }));
    let lowered: Vec<String> = phrases.iter().map(|p| p.to_lowercase()).collect();
    for (row, got) in tokenizer
        .encode_batch(lowered, true)
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(
            got.get_ids(),
            ids[row].as_slice(),
            "{:?}: token ids",
            phrases[row]
        );
    }

    // 2. The tower on those ids, in float32.
    let (pooled, states) = text.run_ids(&ids).unwrap();
    let pooled_error = max_relative(&pooled, &stored["pooled"]);
    let states_error = max_relative(&states, &stored["states"]);
    println!("real tower, float32: pooled {pooled_error:e}, states {states_error:e}");
    assert!(pooled_error < 1e-5 && states_error < 1e-5);

    // 3. What fusion receives: the real tokens only, rounded through float16.
    let f16 = |v: f32| half::f16::from_f32(v).to_f32();
    let features = text.encode(&phrases).unwrap();
    for (row, feature) in features.iter().enumerate() {
        let count = ids[row].iter().filter(|&&id| id != 0).count().clamp(1, 32);
        assert_eq!(
            feature.states.len(),
            count,
            "{:?}: token count",
            phrases[row]
        );
        let theirs: Vec<f32> = stored["pooled"].get(row).unwrap().to_vec1().unwrap();
        let worst = feature
            .pooled
            .iter()
            .zip(&theirs)
            .map(|(ours, theirs)| (ours - f16(*theirs)).abs() / theirs.abs().max(1e-3))
            .fold(0f32, f32::max);
        // One float16 step either way, where float32 noise lands next to a rounding edge.
        assert!(worst <= 1e-3, "{:?}: pooled off by {worst:e}", phrases[row]);
    }
}
