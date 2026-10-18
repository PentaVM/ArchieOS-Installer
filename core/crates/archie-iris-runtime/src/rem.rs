//! REM experience spooling (`archie-iris-rem-experience-v1`) and NumPy `.npz` window archives
//! (`archie-iris-act-window-v1`), compatible with `archie_iris_foundry.rem_sleep`.

use crate::act::{
    GAMEPAD_AXES, GAMEPAD_BUTTONS, KEYS, MOUSE_BINS, MOUSE_BUTTONS, SCROLL, STEP_TYPES,
};
use crate::lora::valid_skill_name;
use crate::preprocess::Rgb;
use crate::weights::file_sha256;
use crate::Error;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const RECORD_FORMAT: &str = "archie-iris-rem-experience-v1";
pub const WINDOW_FORMAT: &str = "archie-iris-act-window-v1";
pub const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Recording {
    pub root: PathBuf,
    pub app: String,
    pub run_id: String,
    pub base_sha256: String,
    pub sensitivity: String,
}

impl Recording {
    pub fn validate_new(
        root: PathBuf,
        app: &str,
        run_id: &str,
        base_sha256: &str,
        sensitivity: &str,
    ) -> Result<Self, Error> {
        if !valid_skill_name(app) || !valid_identifier(run_id) {
            return Err(Error::Refused("record_identifier_invalid".into()));
        }
        if sensitivity != "public" && sensitivity != "local_personal" {
            return Err(Error::Refused("record_sensitivity_invalid".into()));
        }
        if !valid_sha256(base_sha256) {
            return Err(Error::Refused("experience_schema_invalid".into()));
        }
        Ok(Self {
            root,
            app: app.to_owned(),
            run_id: run_id.to_owned(),
            base_sha256: base_sha256.to_owned(),
            sensitivity: sensitivity.to_owned(),
        })
    }
}

pub fn valid_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b'_' || *b == b'-')
}

pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn canonical_digest(value: &Value) -> String {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    hex::encode(Sha256::digest(serialized.as_bytes()))
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Converts a decoded `control` JSON into the `ControlTarget` dict stored in `meta.target`.
pub fn control_target_json(control: &Value) -> Value {
    let index_of =
        |names: &[&str], target: &str| names.iter().position(|n| *n == target).unwrap_or(0);
    let step_type = index_of(&STEP_TYPES, control["step_type"].as_str().unwrap_or("none"));
    let keys: Vec<usize> = control["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k.as_str().and_then(|s| KEYS.iter().position(|n| *n == s)))
        .collect();
    let buttons: Vec<usize> = control["buttons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            b.as_str()
                .and_then(|s| MOUSE_BUTTONS.iter().position(|n| *n == s))
        })
        .collect();
    let mouse_x = {
        let bin = control["mouse"][0].as_i64().unwrap_or(0) as i32;
        MOUSE_BINS.iter().position(|&b| b == bin).unwrap_or(11)
    };
    let mouse_y = {
        let bin = control["mouse"][1].as_i64().unwrap_or(0) as i32;
        MOUSE_BINS.iter().position(|&b| b == bin).unwrap_or(11)
    };
    let scroll = index_of(&SCROLL, control["scroll"].as_str().unwrap_or("none"));
    let pad_axes: Vec<f64> = GAMEPAD_AXES
        .iter()
        .map(|name| control["pad_axes"][*name].as_f64().unwrap_or(0.0))
        .collect();
    let pad_buttons: Vec<usize> = control["pad_buttons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            b.as_str()
                .and_then(|s| GAMEPAD_BUTTONS.iter().position(|n| *n == s))
        })
        .collect();
    let pointer = control
        .get("pointer_canvas")
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "step_type": step_type,
        "keys": keys,
        "buttons": buttons,
        "mouse_x": mouse_x,
        "mouse_y": mouse_y,
        "pointer": pointer,
        "scroll": scroll,
        "pad_axes": pad_axes,
        "pad_buttons": pad_buttons,
    })
}

fn format_npy_header(descr: &str, shape: &[usize]) -> Vec<u8> {
    let shape_str = match shape.len() {
        0 => "()".to_owned(),
        1 => format!("({},)", shape[0]),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let dict = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape_str}, }}");
    let prefix_len = 10usize;
    let unpadded = prefix_len + dict.len() + 1;
    let total = unpadded.div_ceil(64) * 64;
    let header_len = total - prefix_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"\x93NUMPY\x01\x00");
    out.extend_from_slice(&(header_len as u16).to_le_bytes());
    out.extend_from_slice(dict.as_bytes());
    out.resize(total - 1, b' ');
    out.push(b'\n');
    out
}

fn encode_npy_u8(shape: &[usize], payload: &[u8]) -> Vec<u8> {
    let mut out = format_npy_header("|u1", shape);
    out.extend_from_slice(payload);
    out
}

fn encode_npy_unicode_scalar(text: &str) -> Vec<u8> {
    let chars: Vec<u32> = text.chars().map(u32::from).collect();
    let descr = format!("<U{}", chars.len().max(1));
    let mut out = format_npy_header(&descr, &[]);
    if chars.is_empty() {
        out.extend_from_slice(&0u32.to_le_bytes());
    } else {
        for ch in chars {
            out.extend_from_slice(&ch.to_le_bytes());
        }
    }
    out
}

fn decode_npy_unicode_scalar(bytes: &[u8]) -> Result<String, Error> {
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(Error::Refused("window_archive_invalid".into()));
    }
    let major = bytes[6];
    let (header_offset, header_len) = if major == 1 {
        let len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        (10usize, len)
    } else if major == 2 && bytes.len() >= 12 {
        let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        (12usize, len)
    } else {
        return Err(Error::Refused("window_archive_invalid".into()));
    };
    let data_start = header_offset + header_len;
    if bytes.len() < data_start {
        return Err(Error::Refused("window_archive_invalid".into()));
    }
    let header = std::str::from_utf8(&bytes[header_offset..data_start])
        .map_err(|_| Error::Refused("window_archive_invalid".into()))?;
    if !header.contains("'<U") {
        return Err(Error::Refused("act_window_format_unknown".into()));
    }
    let body = &bytes[data_start..];
    if !body.len().is_multiple_of(4) {
        return Err(Error::Refused("window_archive_invalid".into()));
    }
    let mut out = String::with_capacity(body.len() / 4);
    for chunk in body.chunks_exact(4) {
        let code = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if code == 0 {
            continue;
        }
        let ch =
            char::from_u32(code).ok_or_else(|| Error::Refused("window_archive_invalid".into()))?;
        out.push(ch);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub fn spool_window(
    record: &Recording,
    sample_id: &str,
    instruction: &str,
    skill: &str,
    now_canvas: &Rgb,
    recent_frames: &[(Rgb, f64)],
    compacted_frames: &[(Rgb, f64)],
    events: &[(Vec<f32>, f64)],
    control: &Value,
    now: f64,
) -> Result<PathBuf, Error> {
    if !valid_identifier(sample_id) {
        return Err(Error::Refused("experience_identifier_invalid".into()));
    }
    let h = now_canvas.height as usize;
    let w = now_canvas.width as usize;
    let recent_ages: Vec<f64> = recent_frames.iter().map(|(_, t)| round4(now - t)).collect();
    let compacted_ages: Vec<f64> = compacted_frames
        .iter()
        .map(|(_, t)| round4(now - t))
        .collect();
    let event_vectors: Vec<&Vec<f32>> = events.iter().map(|(v, _)| v).collect();
    let event_ages: Vec<f64> = events.iter().map(|(_, t)| round4(now - t)).collect();
    let durable_text = if skill.is_empty() { instruction } else { skill };
    let meta = json!({
        "format": WINDOW_FORMAT,
        "sample_id": sample_id,
        "source": record.app,
        "recent_ages": recent_ages,
        "compacted_ages": compacted_ages,
        "events": event_vectors,
        "event_ages": event_ages,
        "durable": [durable_text],
        "target": control_target_json(control),
    });
    let meta_str = serde_json::to_string(&meta).map_err(|e| Error::Refused(e.to_string()))?;

    let folder = record
        .root
        .join("windows")
        .join(&record.app)
        .join(&record.run_id);
    std::fs::create_dir_all(&folder)?;
    let target = folder.join(format!("{sample_id}.npz"));
    let sidecar = record
        .root
        .join("records")
        .join(&record.app)
        .join(&record.run_id)
        .join(format!("{sample_id}.json"));
    if sidecar.exists() {
        return Err(Error::Refused("experience_record_already_exists".into()));
    }
    if target.exists() {
        return Err(Error::Refused("experience_window_already_exists".into()));
    }

    let tmp_path = folder.join(format!(".{sample_id}.{}.npz.tmp", std::process::id()));
    {
        let file = File::create(&tmp_path)?;
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("now.npy", opts)
            .map_err(|e| Error::Refused(e.to_string()))?;
        zip.write_all(&encode_npy_u8(&[h, w, 3], &now_canvas.pixels))?;

        let mut recent_bytes = Vec::with_capacity(recent_frames.len() * now_canvas.pixels.len());
        for (frame, _) in recent_frames {
            recent_bytes.extend_from_slice(&frame.pixels);
        }
        zip.start_file("recent.npy", opts)
            .map_err(|e| Error::Refused(e.to_string()))?;
        zip.write_all(&encode_npy_u8(
            &[recent_frames.len(), h, w, 3],
            &recent_bytes,
        ))?;

        let mut compacted_bytes =
            Vec::with_capacity(compacted_frames.len() * now_canvas.pixels.len());
        for (frame, _) in compacted_frames {
            compacted_bytes.extend_from_slice(&frame.pixels);
        }
        zip.start_file("compacted.npy", opts)
            .map_err(|e| Error::Refused(e.to_string()))?;
        zip.write_all(&encode_npy_u8(
            &[compacted_frames.len(), h, w, 3],
            &compacted_bytes,
        ))?;

        zip.start_file("meta.npy", opts)
            .map_err(|e| Error::Refused(e.to_string()))?;
        zip.write_all(&encode_npy_unicode_scalar(&meta_str))?;

        let finished = zip.finish().map_err(|e| Error::Refused(e.to_string()))?;
        finished.sync_all()?;
    }
    let size = std::fs::metadata(&tmp_path)?.len();
    if size > MAX_WINDOW_BYTES {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Error::Refused("window_too_large".into()));
    }
    std::fs::rename(&tmp_path, &target)?;

    let window_sha256 = file_sha256(&target)?;
    if let Some(parent) = sidecar.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let semantic_key: String = if instruction.is_empty() {
        record.app.clone()
    } else {
        instruction.chars().take(256).collect()
    };
    let receipt = format!("{}.{sample_id}", record.run_id);
    let rel_window = format!("{}/{}/{sample_id}.npz", record.app, record.run_id);
    let sidecar_json = json!({
        "format": RECORD_FORMAT,
        "app": record.app,
        "run_id": record.run_id,
        "sample_id": sample_id,
        "window": rel_window,
        "window_sha256": window_sha256,
        "base_sha256": record.base_sha256,
        "semantic_key": semantic_key,
        "supervision_receipt": receipt,
        "machine_analysis_allowed": true,
        "training_allowed": true,
        "sensitivity": record.sensitivity,
        "embedding_model_sha256": Value::Null,
        "embedding": Value::Null,
    });
    let mut handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sidecar)?;
    let mut line = serde_json::to_vec(&sidecar_json).map_err(|e| Error::Refused(e.to_string()))?;
    line.push(b'\n');
    handle.write_all(&line)?;
    handle.sync_all()?;
    Ok(target)
}

pub fn inspect_window_meta(path: &Path) -> Result<Value, Error> {
    let meta_size = std::fs::metadata(path)?.len();
    if meta_size > MAX_WINDOW_BYTES {
        return Err(Error::Refused("window_too_large".into()));
    }
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(BufReader::new(file))
        .map_err(|_| Error::Refused("window_archive_invalid".into()))?;
    if zip.len() != 4 {
        return Err(Error::Refused("window_fields_invalid".into()));
    }
    let mut names = HashSet::with_capacity(4);
    let mut total_uncompressed = 0u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|_| Error::Refused("window_archive_invalid".into()))?;
        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        names.insert(entry.name().to_owned());
    }
    let expected: HashSet<String> = ["now.npy", "recent.npy", "compacted.npy", "meta.npy"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if names != expected || total_uncompressed > 128 * 1024 * 1024 {
        return Err(Error::Refused("window_fields_invalid".into()));
    }
    let mut meta_entry = zip
        .by_name("meta.npy")
        .map_err(|_| Error::Refused("window_fields_invalid".into()))?;
    let mut raw = Vec::new();
    meta_entry.read_to_end(&mut raw)?;
    let text = decode_npy_unicode_scalar(&raw)?;
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| Error::Refused("act_window_format_unknown".into()))?;
    if parsed.get("format").and_then(Value::as_str) != Some(WINDOW_FORMAT) {
        return Err(Error::Refused("act_window_format_unknown".into()));
    }
    Ok(parsed)
}

pub fn recorded_samples(root: &Path, app: &str, run_id: &str) -> Result<Vec<Value>, Error> {
    if !valid_skill_name(app) || !valid_identifier(run_id) {
        return Err(Error::Refused("record_identifier_invalid".into()));
    }
    let dir = root.join("records").join(app).join(run_id);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut samples = Vec::with_capacity(paths.len());
    for sidecar in paths {
        let entry: Value = serde_json::from_slice(&std::fs::read(&sidecar)?)
            .map_err(|e| Error::Refused(e.to_string()))?;
        let rel = entry["window"]
            .as_str()
            .ok_or_else(|| Error::Refused("window_path_invalid".into()))?;
        let window_path = root.join("windows").join(rel);
        let meta = inspect_window_meta(&window_path)?;
        let target = meta.get("target").unwrap_or(&Value::Null);
        let empty_future = json!([]);
        let future = meta.get("future").unwrap_or(&empty_future);
        samples.push(json!({
            "sample_id": entry["sample_id"],
            "receipt_id": entry["supervision_receipt"],
            "window_sha256": entry["window_sha256"],
            "base_sha256": entry["base_sha256"],
            "sensitivity": entry["sensitivity"],
            "target_sha256": canonical_digest(target),
            "future_sha256": canonical_digest(future),
        }));
    }
    Ok(samples)
}
