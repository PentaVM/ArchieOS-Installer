//! `archie-iris-text-pack <siglip2 repository dir> <out dir>`
//!
//! IRIS reads phrases with SigLIP2's text tower only. This keeps the `text_model.*` tensors
//! of the repository's `model.safetensors` (1.13 of its 1.5 GB), unchanged in name and
//! dtype, and copies the tokenizer files beside them, so an image can carry the tower and
//! IRIS starts without downloading it. `archie-iris-serve --text-tower <out dir>` reads the
//! result exactly as it reads the whole repository.
//!
//! `SOURCE` records where it came from: the SHA-256 of the file it was cut from and of the
//! file it wrote.

use archie_iris_runtime::weights::file_sha256;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const COPIED: [&str; 4] = [
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "config.json",
];

fn pack(from: &Path, to: &Path) -> Result<(), String> {
    let model = from.join("model.safetensors");
    let all = candle_core::safetensors::load(&model, &candle_core::Device::Cpu)
        .map_err(|error| format!("{}: {error}", model.display()))?;
    let text: HashMap<String, candle_core::Tensor> = all
        .into_iter()
        .filter(|(name, _)| name.starts_with("text_model."))
        .collect();
    if text.is_empty() {
        return Err(format!("{} holds no text_model.* tensors", model.display()));
    }
    std::fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
    let out = to.join("model.safetensors");
    candle_core::safetensors::save(&text, &out)
        .map_err(|error| format!("{}: {error}", out.display()))?;
    for name in COPIED {
        let source = from.join(name);
        if source.is_file() {
            std::fs::copy(&source, to.join(name))
                .map_err(|error| format!("{}: {error}", source.display()))?;
        } else if name.starts_with("tokenizer.") {
            return Err(format!("{} is missing", source.display()));
        }
    }
    let source = format!(
        "cut from {}\nsource sha256 {}\ntext_model tensors {}\npacked sha256 {}\n",
        model.display(),
        file_sha256(&model).map_err(|error| error.to_string())?,
        text.len(),
        file_sha256(&out).map_err(|error| error.to_string())?,
    );
    std::fs::write(to.join("SOURCE"), source).map_err(|error| error.to_string())?;
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [from, to] = args.as_slice() else {
        eprintln!("usage: archie-iris-text-pack <siglip2 repository dir> <out dir>");
        return ExitCode::from(64);
    };
    match pack(from, to) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("archie-iris-text-pack: {error}");
            ExitCode::FAILURE
        }
    }
}
