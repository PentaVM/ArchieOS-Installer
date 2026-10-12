//! The Rust runtime against the Python IRIS, on the released weights.
//!
//! Needs a weights file and the reference outputs written by the Python IRIS
//! (`archie_iris_foundry`, CPU, float32) for one image:
//!
//! ```text
//! IRIS_WEIGHTS=iris-beta21-step132000.pt IRIS_REFERENCE=<dir> IRIS_IMAGE=<png> \
//!   cargo test --release -p archie-iris-runtime --test parity -- --ignored --nocapture
//! ```
//!
//! `<dir>` holds `reference.safetensors`, `reference.json` and `phrases.json`. The phrase
//! features in it stand in for the SigLIP2 text tower, which the release does not carry; the
//! tower itself is compared separately (`text_tower`), on random weights.

use archie_iris_runtime::act::Actor;
use archie_iris_runtime::detect::{Detection, Detector};
use archie_iris_runtime::model::IrisModel;
use archie_iris_runtime::phrases::{PhraseSource, PhraseTable};
use archie_iris_runtime::preprocess::{decode, letterbox, to_tensor};
use archie_iris_runtime::weights::Weights;
use candle_core::{DType, Device, Tensor};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Largest absolute difference, and the reference's largest magnitude.
fn compare(name: &str, ours: &Tensor, theirs: &Tensor) -> f32 {
    let a: Vec<f32> = ours
        .to_dtype(DType::F32)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    let b: Vec<f32> = theirs
        .to_dtype(DType::F32)
        .unwrap()
        .flatten_all()
        .unwrap()
        .to_vec1()
        .unwrap();
    assert_eq!(
        a.len(),
        b.len(),
        "{name}: shapes differ, {:?} vs {:?}",
        ours.dims(),
        theirs.dims()
    );
    let max_diff = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0f32, f32::max);
    let scale = b.iter().map(|y| y.abs()).fold(0f32, f32::max);
    println!(
        "{name:24} max |diff| {max_diff:.3e}   max |ref| {scale:.3e}   rel {:.3e}",
        max_diff / scale.max(1e-12)
    );
    max_diff / scale.max(1e-12)
}

fn same_detections(name: &str, ours: &[Detection], theirs: &Value) -> (usize, usize) {
    let theirs = theirs.as_array().unwrap();
    let key = |phrase: &str, b: &[f64]| {
        format!("{phrase}|{:.0},{:.0},{:.0},{:.0}", b[0], b[1], b[2], b[3])
    };
    let mine: Vec<String> = ours.iter().map(|d| key(&d.phrase, &d.bbox)).collect();
    let reference: Vec<String> = theirs
        .iter()
        .map(|d| {
            key(
                d["phrase"].as_str().unwrap(),
                &d["box"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let matched = mine.iter().filter(|k| reference.contains(k)).count();
    let mut worst_score = 0f64;
    for d in ours {
        if let Some(r) = theirs.iter().find(|r| {
            r["phrase"] == d.phrase.as_str()
                && key(&d.phrase, &d.bbox)
                    == key(
                        r["phrase"].as_str().unwrap(),
                        &r["box"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect::<Vec<_>>(),
                    )
        }) {
            worst_score = worst_score.max((r["score"].as_f64().unwrap() - d.score).abs());
        }
    }
    println!("{name:24} {} ours, {} reference, {matched} identical (phrase and box to the pixel), worst score gap {worst_score:.4}", ours.len(), reference.len());
    (matched, reference.len())
}

#[test]
#[ignore = "needs the released weights and the Python reference outputs"]
fn rust_matches_the_python_iris() {
    let (Some(weights_path), Some(reference), Some(image_path)) = (
        env("IRIS_WEIGHTS"),
        env("IRIS_REFERENCE"),
        env("IRIS_IMAGE"),
    ) else {
        eprintln!("IRIS_WEIGHTS, IRIS_REFERENCE and IRIS_IMAGE are not all set; nothing compared");
        return;
    };
    let device = Device::Cpu;
    let started = std::time::Instant::now();
    let weights = Weights::load(&weights_path, None, &device, DType::F32).expect("weights");
    println!(
        "loaded {} tensors, {} parameters, step {:?}, in {:.1}s",
        weights.names().count(),
        weights.parameter_count(),
        weights.release.step,
        started.elapsed().as_secs_f64()
    );
    let model = IrisModel::load(&weights).expect("model");
    let mut actor = Actor::load(&weights).expect("action head");
    drop(weights);
    let stored: HashMap<String, Tensor> =
        candle_core::safetensors::load(reference.join("reference.safetensors"), &device).unwrap();
    let report: Value =
        serde_json::from_slice(&std::fs::read(reference.join("reference.json")).unwrap()).unwrap();
    let mut table = PhraseTable::load(
        &reference.join("phrases.json"),
        &reference.join("reference.safetensors"),
    )
    .expect("phrase table");

    // 1. The canvas, to the byte.
    let image = decode(&std::fs::read(&image_path).unwrap()).unwrap();
    let (canvas, _) = letterbox(&image, model.config.image_size);
    let expected: Vec<u8> = stored["canvas"].flatten_all().unwrap().to_vec1().unwrap();
    let differing = canvas
        .pixels
        .iter()
        .zip(&expected)
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "canvas                   {differing} of {} bytes differ",
        expected.len()
    );
    assert_eq!(
        differing, 0,
        "the letterboxed canvas must match Pillow exactly"
    );

    // 2. Encoders and fusion, tensor by tensor.
    let images = to_tensor(&canvas, &device).unwrap();
    let t = std::time::Instant::now();
    let (vjepa, siglip) = model.encode_parts(&images).unwrap();
    println!("encoders                 {:.2}s", t.elapsed().as_secs_f64());
    assert!(compare("vjepa", &vjepa.squeeze(0).unwrap(), &stored["vjepa"]) < 1e-3);
    assert!(compare("siglip", &siglip.squeeze(0).unwrap(), &stored["siglip"]) < 1e-3);
    let tokens = model.encode_image(&images).unwrap();
    assert!(compare("tokens", &tokens.squeeze(0).unwrap(), &stored["tokens"]) < 1e-3);

    let phrases: Vec<String> = report["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    let features = table.encode(&phrases).unwrap();
    let encoded = model.encode_phrases(&[features]).unwrap();
    let t = std::time::Instant::now();
    // From the reference tokens, so fusion is judged on its own.
    let (fused, text) = model
        .fuse(
            &stored["tokens"].unsqueeze(0).unwrap(),
            &encoded.text,
            &encoded.text_padding,
        )
        .unwrap();
    let decoded = model.decode(&fused, &text, &encoded).unwrap();
    println!("fuse + decode            {:.2}s", t.elapsed().as_secs_f64());
    assert!(compare("fused", &fused.squeeze(0).unwrap(), &stored["fused"]) < 1e-3);
    compare(
        "logits",
        &decoded.logits.squeeze(0).unwrap(),
        &stored["logits"],
    );
    compare(
        "boxes",
        &decoded.boxes.squeeze(0).unwrap(),
        &stored["boxes"],
    );

    // 3. What callers see.
    let mut detector = Detector {
        model,
        phrases: Box::new(table),
    };
    let t = std::time::Instant::now();
    let hits = detector
        .detect(&image, &phrases, 0.05, 20, 0.5, true)
        .unwrap();
    println!("detect                   {:.2}s", t.elapsed().as_secs_f64());
    let (matched, total) = same_detections("detect", &hits, &report["detect"]);
    assert!(matched * 100 >= total * 95, "detect: {matched} of {total}");
    let t = std::time::Instant::now();
    let everything = detector
        .detect_everything(&image, 0.05, &["red button".to_owned()], 60, 0.5, true)
        .unwrap();
    println!("detect_everything        {:.2}s", t.elapsed().as_secs_f64());
    let (matched, total) = same_detections("everything", &everything, &report["everything"]);
    assert!(
        matched * 100 >= total * 90,
        "everything: {matched} of {total}"
    );
    // Embeddings of identical detections.
    let mut worst = 0f64;
    for d in &hits {
        if let Some(r) = report["detect"].as_array().unwrap().iter().find(|r| {
            r["phrase"] == d.phrase.as_str()
                && r["box"][0].as_f64() == Some(d.bbox[0])
                && r["box"][1].as_f64() == Some(d.bbox[1])
        }) {
            let theirs: Vec<f64> = r["embedding"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let ours = d.embedding.as_ref().unwrap();
            let dot: f64 = ours.iter().zip(&theirs).map(|(a, b)| a * b).sum();
            worst = worst.max(1.0 - dot);
        }
    }
    println!("instance embeddings      worst 1 - cosine {worst:.2e}");
    assert!(worst < 1e-3);

    // 4. Acting, three frames of one session.
    let detector_model = &detector.model;
    let mut agreed = 0;
    for (step, (instruction, now)) in [
        ("click the search field", 10.0),
        ("click the search field", 10.6),
        ("type hello", 11.3),
    ]
    .iter()
    .enumerate()
    {
        let mut control = actor
            .act(
                detector_model,
                detector.phrases.as_mut(),
                "parity",
                &image,
                instruction,
                "browser",
                *now,
            )
            .unwrap();
        let reference = &report["act"][step];
        let mut differences = Vec::new();
        for key in [
            "step_type",
            "keys",
            "buttons",
            "mouse",
            "scroll",
            "pad_buttons",
            "pointer_source",
            "memory",
        ] {
            if control[key] != reference[key] {
                differences.push(format!("{key}: {} vs {}", control[key], reference[key]));
            }
        }
        let pointer_gap = (0..2)
            .map(|i| {
                (control["pointer_canvas"][i].as_f64().unwrap()
                    - reference["pointer_canvas"][i].as_f64().unwrap())
                .abs()
            })
            .fold(0f64, f64::max);
        let axes_gap = reference["pad_axes"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (control["pad_axes"][k].as_f64().unwrap() - v.as_f64().unwrap()).abs())
            .fold(0f64, f64::max);
        let chunk_same = control["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .zip(reference["chunk"].as_array().unwrap())
            .filter(|(a, b)| {
                a["keys"] == b["keys"]
                    && a["step_type"] == b["step_type"]
                    && a["mouse"] == b["mouse"]
            })
            .count();
        control.as_object_mut().unwrap().remove("chunk");
        println!("act {step}                    pointer gap {pointer_gap:.4}, pad axes gap {axes_gap:.3}, chunk {chunk_same}/7 alike, differences {differences:?}");
        if differences.is_empty() && pointer_gap <= 0.002 {
            agreed += 1;
        }
    }
    assert_eq!(
        agreed, 3,
        "every frame of the session should decide as Python did"
    );
}
