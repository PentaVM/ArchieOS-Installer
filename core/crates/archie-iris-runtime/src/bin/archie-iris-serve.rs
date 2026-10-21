//! `archie-iris-serve`: IRIS on 127.0.0.1, without Python.
//!
//! ```text
//! archie-iris-serve --weights iris-beta21-step132000.pt --text-tower <dir> [--port 8765]
//!                   [--sha256 <hex>] [--device auto|cpu|cuda] [--rem-spool <dir>]
//!                   [--skills-dir <dir>]
//! ```
//!
//! `--text-tower` is a directory with SigLIP2's `model.safetensors`, `tokenizer.json` and
//! `tokenizer_config.json`. `--phrase-table <dir>` instead serves only the phrases in a
//! pre-encoded table (`phrases.json` + `phrases.safetensors`).

use archie_iris_runtime::act::Actor;
use archie_iris_runtime::detect::Detector;
use archie_iris_runtime::model::IrisModel;
use archie_iris_runtime::phrases::{PhraseSource, PhraseTable, TextTower};
use archie_iris_runtime::server::{router, Service};
use archie_iris_runtime::weights::Weights;
use archie_iris_runtime::{choose_device, Error};
use serde_json::json;
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage: archie-iris-serve --weights <file.pt> (--text-tower <dir> | --phrase-table <dir>) \
         [--port 8765] [--sha256 <hex>] [--device auto|cpu|cuda] [--rem-spool <dir>] [--skills-dir <dir>]"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut weights, mut tower, mut table, mut sha256) = (None, None, None, None);
    let (mut rem_spool, mut skills_dir) = (None, None);
    let (mut port, mut device) = (8765u16, "auto".to_owned());
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--weights" => weights = Some(PathBuf::from(value())),
            "--text-tower" => tower = Some(PathBuf::from(value())),
            "--phrase-table" => table = Some(PathBuf::from(value())),
            "--sha256" => sha256 = Some(value()),
            "--port" => port = value().parse().unwrap_or_else(|_| usage()),
            "--device" => device = value(),
            "--rem-spool" => rem_spool = Some(PathBuf::from(value())),
            "--skills-dir" => skills_dir = Some(PathBuf::from(value())),
            _ => usage(),
        }
    }
    let weights_path = weights.unwrap_or_else(|| usage());
    let (device, dtype) = choose_device(&device)?;
    let started = std::time::Instant::now();
    let loaded = Weights::load(&weights_path, sha256.as_deref(), &device, dtype)?;
    let config = loaded.release.config.clone();
    let phrases: Box<dyn PhraseSource> = match (tower, table) {
        (Some(dir), None) => Box::new(TextTower::load(
            &dir,
            config.text_length,
            config.text_tokens_per_phrase,
            &device,
            dtype,
        )?),
        (None, Some(dir)) => Box::new(PhraseTable::load(
            &dir.join("phrases.json"),
            &dir.join("phrases.safetensors"),
        )?),
        _ => usage(),
    };
    let model = IrisModel::load(&loaded)?;
    let actor = match Actor::load(&loaded) {
        Ok(actor) => Some(actor),
        Err(Error::Weights(_)) => None,
        Err(error) => return Err(error.into()),
    };
    let has_skills = actor.as_ref().is_some_and(|a| a.bank.is_some());
    let info = json!({
        "weights": weights_path.display().to_string(),
        "sha256": loaded.release.sha256,
        "step": loaded.release.step,
        "score": loaded.release.score,
        "device": format!("{device:?}"),
        "dtype": format!("{dtype:?}"),
        "parameters": loaded.parameter_count(),
        "instance_embeddings": true,
        "action_head": actor.is_some(),
        "skill_adapters": has_skills,
        "rem_recording": rem_spool.is_some(),
    });
    drop(loaded);
    let mut service =
        Service::new(Detector::new(model, phrases), actor, info).with_rem_spool(rem_spool);
    if let Some(dir) = skills_dir {
        service = service.with_skills_dir(dir);
    }
    let health = service.health();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!(
        "{}",
        json!({"event": "iris_serve_ready", "url": format!("http://127.0.0.1:{port}"),
               "load_seconds": started.elapsed().as_secs_f64(), "health": health})
    );
    axum::serve(listener, router(service)).await?;
    Ok(())
}
