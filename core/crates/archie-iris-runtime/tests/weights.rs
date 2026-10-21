//! The release reader on `tests/reference/tiny-release.pt`, a release-shaped zip written by
//! `make_tiny_release.py` (no PyTorch needed): shared storages at offsets, a bfloat16
//! storage, a `[E, 1, H]` bias and a head the runtime never asks for.

use archie_iris_runtime::weights::{file_sha256, Weights};
use archie_iris_runtime::Error;
use candle_core::{DType, Device};
use std::collections::HashMap;
use std::path::PathBuf;

fn tiny() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference/tiny-release.pt")
}

fn values(weights: &Weights, name: &str) -> Vec<f32> {
    weights
        .get(name)
        .unwrap()
        .to_dtype(DType::F32)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap()
}

#[test]
fn tensors_leave_the_zip_only_when_asked_for() {
    let weights = Weights::load(&tiny(), None, &Device::Cpu, DType::F32).unwrap();
    assert_eq!(weights.release.step, Some(7));
    assert_eq!(weights.names().count(), 5);
    assert_eq!(weights.parameter_count(), 8 + 8 + 24 + 12 + 6);
    assert_eq!(weights.resident_count(), 0);

    let shared: Vec<f32> = (0..40).map(|i| 0.5 * i as f32 - 3.0).collect();
    assert_eq!(values(&weights, "fused_norm.weight"), shared[..8]);
    assert_eq!(values(&weights, "fused_norm.bias"), shared[8..16]);
    let bias = weights.get("fusion.0.moe.b_in").unwrap();
    assert_eq!(bias.dims(), &[2, 1, 12]);
    assert_eq!(values(&weights, "fusion.0.moe.b_in"), shared[16..40]);
    let position: Vec<f32> = (0..12).map(|i| 0.25 * i as f32 - 1.0).collect();
    assert_eq!(weights.get("position").unwrap().dims(), &[3, 4]);
    assert_eq!(values(&weights, "position"), position);

    // Asked again, it comes from the resident set, not from the zip.
    assert_eq!(weights.resident_count(), 4);
    assert_eq!(values(&weights, "fused_norm.weight"), shared[..8]);
    assert_eq!(weights.resident_count(), 4);
    assert!(weights.contains("read_decoder.weight"));
    assert!(matches!(
        weights.get("missing.weight"),
        Err(Error::MissingTensor(_))
    ));
}

#[test]
fn the_inference_dtype_applies_to_every_float_storage() {
    let weights = Weights::load(&tiny(), None, &Device::Cpu, DType::BF16).unwrap();
    assert_eq!(
        weights.get("fused_norm.weight").unwrap().dtype(),
        DType::BF16
    );
    assert_eq!(weights.get("position").unwrap().dtype(), DType::BF16);
}

#[test]
fn the_fingerprint_reads_no_tensor_into_the_resident_set() {
    let weights = Weights::load(&tiny(), None, &Device::Cpu, DType::F32).unwrap();
    let lazy = weights.base_fingerprint().unwrap();
    assert_eq!(weights.resident_count(), 0);
    // The same tensors held in memory give the same fingerprint.
    let names: Vec<String> = weights.names().cloned().collect();
    let tensors: HashMap<_, _> = names
        .iter()
        .map(|name| (name.clone(), weights.get(name).unwrap()))
        .collect();
    let eager = Weights::from_tensors(tensors, weights.release.clone());
    assert_eq!(eager.base_fingerprint().unwrap(), lazy);
}

#[test]
fn a_wrong_digest_is_refused_before_parsing() {
    let actual = file_sha256(&tiny()).unwrap();
    let loaded = Weights::load(
        &tiny(),
        Some(&actual.to_uppercase()),
        &Device::Cpu,
        DType::F32,
    );
    assert_eq!(loaded.unwrap().release.sha256, actual);
    let refused = Weights::load(&tiny(), Some(&"0".repeat(64)), &Device::Cpu, DType::F32);
    assert!(matches!(refused, Err(Error::Weights(message)) if message.contains("does not match")));
}
