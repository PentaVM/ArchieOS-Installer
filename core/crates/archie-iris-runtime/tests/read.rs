//! IRIS's reading head against `beta_model.ReadDecoder` on the real Beta 2.1 weights.
//!
//! IRIS_WEIGHTS=iris-beta21-step132000.pt IRIS_READ_REFERENCE=<read-reference.safetensors> \
//!     cargo test --release -p archie-iris-runtime --test read -- --ignored --nocapture
//!
//! `tests/reference/read_reference.py` writes the reference: teacher-forced logits and the
//! greedy tokens over a fixed random scene state, on the CPU in float32. The Rust decoder
//! keeps keys and values where Python reruns the prefix, so this is the check that the cache
//! changes nothing. With `IRIS_DEVICE=cuda` (and `--features cuda`) it runs on the GPU in f32.

use archie_iris_runtime::read::Reader;
use archie_iris_runtime::weights::Weights;
use candle_core::{DType, Device};
use std::path::PathBuf;

#[test]
#[ignore = "needs the release and a reference written by read_reference.py"]
fn the_reading_head_matches_python() {
    let (Some(weights), Some(reference)) = (
        std::env::var_os("IRIS_WEIGHTS").map(PathBuf::from),
        std::env::var_os("IRIS_READ_REFERENCE").map(PathBuf::from),
    ) else {
        eprintln!("IRIS_WEIGHTS and IRIS_READ_REFERENCE are not both set; nothing compared");
        return;
    };
    let device = match std::env::var("IRIS_DEVICE").as_deref() {
        Ok("cuda") => Device::new_cuda(0).expect("a CUDA device"),
        _ => Device::Cpu,
    };
    let loaded = Weights::load(&weights, None, &device, DType::F32).expect("weights");
    let reader = Reader::load(&loaded)
        .expect("reader")
        .expect("a reading head");
    let tensors = candle_core::safetensors::load(&reference, &device).expect("reference");
    let memory = tensors["memory"].clone();
    let tokens = tensors["tokens"].to_dtype(DType::U32).unwrap();

    let got = reader.logits(&tokens, &memory).unwrap();
    let want = tensors["logits"].to_dtype(DType::F32).unwrap();
    let worst = (&got - &want)
        .unwrap()
        .abs()
        .unwrap()
        .max_all()
        .unwrap()
        .to_scalar::<f32>()
        .unwrap();
    let scale = want
        .abs()
        .unwrap()
        .max_all()
        .unwrap()
        .to_scalar::<f32>()
        .unwrap();
    eprintln!(
        "teacher-forced logits: max |difference| {worst:e}, relative {:e}",
        worst / scale
    );
    assert!(worst / scale < 1e-4, "logits differ: {worst} of {scale}");

    let generated = reader.generate(&memory).unwrap();
    let want: Vec<Vec<i64>> = tensors["generated"].to_vec2().unwrap();
    for (row, ((tokens, text), expected)) in generated.iter().zip(&want).enumerate() {
        let expected: Vec<u32> = expected.iter().map(|&t| t as u32).collect();
        eprintln!("row {row}: {text:?}");
        assert_eq!(tokens, &expected, "row {row} generated differently");
    }
}
