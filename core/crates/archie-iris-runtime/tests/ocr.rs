//! IRIS's OCR expert against `transformers`' GOT-OCR 2.0 on the released weights.
//!
//! GOT_DIR=<GOT-OCR-2.0-hf dir> GOT_REFERENCE=<dir written by got_reference.py> \
//!     cargo test --release -p archie-iris-runtime --test ocr -- --ignored --nocapture
//!
//! `tests/reference/got_reference.py` renders two lines of Spanish, runs the Python model on
//! the CPU in float32 and keeps the processor's pixels and prompt ids, the projector's image
//! tokens, the logits after the prompt and the greedy reading. The same pixels go through the
//! Rust expert: every number is compared, then the reading. The Rust canvas (its own bicubic,
//! not torchvision's) is compared too, and its reading of the PNG must be the same text.

use archie_iris_runtime::ocr::{canvas, OcrExpert};
use archie_iris_runtime::preprocess::decode;
use candle_core::{DType, Device, Tensor};
use std::path::PathBuf;

fn relative(got: &Tensor, want: &Tensor) -> f32 {
    let worst = (got - want)
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
    worst / scale
}

#[test]
#[ignore = "needs GOT-OCR 2.0 and a reference written by got_reference.py"]
fn the_ocr_expert_matches_transformers() {
    let (Some(model), Some(reference)) = (
        std::env::var_os("GOT_DIR").map(PathBuf::from),
        std::env::var_os("GOT_REFERENCE").map(PathBuf::from),
    ) else {
        eprintln!("GOT_DIR and GOT_REFERENCE are not both set; nothing compared");
        return;
    };
    let device = Device::Cpu;
    let started = std::time::Instant::now();
    let expert = OcrExpert::load(&model, &device, DType::F32).expect("the expert");
    eprintln!("loaded in {:.1} s", started.elapsed().as_secs_f32());
    let t = candle_core::safetensors::load(reference.join("got-reference.safetensors"), &device)
        .expect("reference");

    let want_ids: Vec<u32> = t["input_ids"]
        .get(0)
        .unwrap()
        .to_vec1::<i64>()
        .unwrap()
        .into_iter()
        .map(|v| v as u32)
        .collect();
    assert_eq!(expert.prompt_ids(None).unwrap(), want_ids, "prompt ids");

    let started = std::time::Instant::now();
    let features = expert.encode(&t["pixel_values"]).unwrap();
    eprintln!("encoded in {:.1} s", started.elapsed().as_secs_f32());
    let error = relative(&features, &t["image_features"]);
    eprintln!("image tokens: relative max error {error:e}");
    assert!(error < 1e-3, "image tokens differ: {error}");

    let logits = expert.prompt_logits(&want_ids, &features).unwrap();
    let error = relative(&logits, &t["prompt_logits"]);
    eprintln!("prompt logits: relative max error {error:e}");
    assert!(error < 1e-3, "prompt logits differ: {error}");

    let want: Vec<u32> = t["generated"]
        .to_vec1::<i64>()
        .unwrap()
        .into_iter()
        .map(|v| v as u32)
        .filter(|&v| {
            expert
                .decode(&[v])
                .map(|s| s != "<|im_end|>")
                .unwrap_or(true)
        })
        .collect();
    let started = std::time::Instant::now();
    let got = expert.generate(&want_ids, &features, 32).unwrap();
    let text = expert.decode(&got).unwrap();
    eprintln!(
        "read {:?} in {:.1} s",
        text,
        started.elapsed().as_secs_f32()
    );
    assert_eq!(got, want[..got.len().min(want.len())], "greedy reading");
    let reference_text = std::fs::read_to_string(reference.join("got-reference.txt")).unwrap();
    assert_eq!(text.trim(), reference_text.trim());

    // The Rust canvas from the PNG itself: close to the processor's, and read the same.
    let png = decode(&std::fs::read(reference.join("text.png")).unwrap()).unwrap();
    let pixels = canvas(&png, &device).unwrap();
    let difference = (&pixels - &t["pixel_values"])
        .unwrap()
        .abs()
        .unwrap()
        .mean_all()
        .unwrap()
        .to_scalar::<f32>()
        .unwrap();
    eprintln!("canvas: mean |difference| against the processor {difference:e}");
    let readings = expert.read(&png, &[], 32).unwrap();
    eprintln!("from the PNG: {:?}", readings[0].text);
    assert_eq!(readings[0].text, reference_text.trim());
}
