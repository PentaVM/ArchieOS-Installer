//! `archie-reflex` — trains the local detector on what applications declared, and judges it.
//!
//! ```text
//! archie-reflex train --corpus <dir> --out <dir> [--epochs 120] [--seed 20260917]
//!                     [--learning-rate 0.02] [--momentum 0.9] [--presence-weight 12]
//! archie-reflex check --model <dir> --corpus <dir> [--least-confidence 600] [--seed …]
//! archie-reflex look  --model <dir> --capture <file.png>
//! ```
//!
//! `check` says the only thing worth knowing about a detector that exists to be clicked
//! through: of the controls an application declared, how many the model found well enough
//! that a click in the middle of what it drew would land inside the real one.

use archie_reflex::{Box2, Corpus, Deployed, Detector, Sample, Tile, Training};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("archie-reflex: {message}");
            ExitCode::FAILURE
        }
    }
}

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|a| a == flag)?;
    if at + 1 >= args.len() {
        return None;
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Some(value)
}

fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let command = if args.is_empty() {
        String::new()
    } else {
        args.remove(0)
    };
    match command.as_str() {
        "train" => train(args),
        "check" => check(args),
        "look" => look(args),
        _ => Err("usage: archie-reflex train|check|look ...".into()),
    }
}

fn corpus_of(args: &mut Vec<String>) -> Result<Vec<Sample>, String> {
    let dir = take(args, "--corpus").ok_or("--corpus is required")?;
    let samples = Corpus::at(&dir).read().map_err(|e| e.to_string())?;
    if samples.is_empty() {
        return Err(format!("{dir} has taught nothing yet"));
    }
    Ok(samples)
}

fn train(mut args: Vec<String>) -> Result<ExitCode, String> {
    let samples = corpus_of(&mut args)?;
    let out = PathBuf::from(take(&mut args, "--out").ok_or("--out is required")?);
    let mut training = Training::default();
    if let Some(epochs) = take(&mut args, "--epochs") {
        training.epochs = epochs.parse().map_err(|_| "--epochs is a number")?;
    }
    if let Some(seed) = take(&mut args, "--seed") {
        training.seed = seed.parse().map_err(|_| "--seed is a number")?;
    }
    if let Some(rate) = take(&mut args, "--learning-rate") {
        training.learning_rate = rate.parse().map_err(|_| "--learning-rate is a number")?;
    }
    if let Some(momentum) = take(&mut args, "--momentum") {
        training.momentum = momentum.parse().map_err(|_| "--momentum is a number")?;
    }
    if let Some(weight) = take(&mut args, "--presence-weight") {
        training.presence_weight = weight
            .parse()
            .map_err(|_| "--presence-weight is a number")?;
    }
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {args:?}"));
    }

    let controls: usize = samples.iter().map(|sample| sample.controls.len()).sum();
    println!(
        "{} windows, {controls} controls declared in them, {} epochs",
        samples.len(),
        training.epochs
    );
    let detector = Detector::train(&samples, training, |epoch| {
        // Every epoch of a short run, because a short run is someone watching.
        if training.epochs <= 20 || epoch.epoch % 10 == 0 || epoch.epoch + 1 == training.epochs {
            println!(
                "epoch {:3}  loss {:.4}  held back {:.4}  found {}/1000  spread {}/1000  alive {}/1000",
                epoch.epoch,
                epoch.loss,
                epoch.held_back_loss,
                epoch.found_permille,
                epoch.spread_permille,
                epoch.alive_permille
            );
        }
    })
    .map_err(|e| e.to_string())?;
    println!("{} numbers in the model", detector.parameters());

    let deployed =
        Deployed::of(&detector, "archie.reflex-controls", "1").map_err(|e| e.to_string())?;
    deployed.write(&out).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} bytes, sha256 {})",
        out.join("reflex.checkpoint").display(),
        deployed.checkpoint().len(),
        &deployed.manifest().checkpoint_sha256[..16]
    );
    // Scored on the windows it was never taught, which is the only score worth printing.
    let (order, teaching) = training.split(samples.len());
    let judged: Vec<Sample> = order[teaching..]
        .iter()
        .map(|index| samples[*index].clone())
        .collect();
    println!("scored on {} windows it never saw", judged.len());
    report(
        "as trained",
        &scored(&judged, |sample| {
            detector
                .detect(sample.tile.pixels(), 500)
                .into_iter()
                .map(|prediction| (prediction.control, prediction.confidence_permille))
                .collect()
        }),
    );
    report(
        "as deployed",
        &scored(&judged, |sample| {
            deployed
                .detect(&sample.tile, 500)
                .into_iter()
                .map(|prediction| (prediction.control, prediction.confidence_permille))
                .collect()
        }),
    );
    Ok(ExitCode::SUCCESS)
}

fn check(mut args: Vec<String>) -> Result<ExitCode, String> {
    let samples = corpus_of(&mut args)?;
    let model = PathBuf::from(take(&mut args, "--model").ok_or("--model is required")?);
    // How sure it has to be before it says anything. Sweeping this is the honest way to
    // choose it: recall and invention move against each other.
    // The seed decides which windows were kept back, so the same one scores the same
    // model on the same windows it never saw.
    let mut training = Training::default();
    if let Some(seed) = take(&mut args, "--seed") {
        training.seed = seed.parse().map_err(|_| "--seed is a number")?;
    }
    let least: Vec<u16> = match take(&mut args, "--least-confidence") {
        Some(value) => vec![value
            .parse()
            .map_err(|_| "--least-confidence is a number")?],
        None => vec![500, 600, 700, 800, 900],
    };
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {args:?}"));
    }
    let deployed = Deployed::load(&model).map_err(|e| e.to_string())?;
    let (order, teaching) = training.split(samples.len());
    let judged: Vec<Sample> = order[teaching..]
        .iter()
        .map(|index| samples[*index].clone())
        .collect();
    println!(
        "{} on {} windows it was never taught",
        deployed.manifest().model_id,
        judged.len()
    );
    for least in least {
        report(
            &format!("sure {least}/1000 or more"),
            &scored(&judged, |sample| {
                deployed
                    .detect(&sample.tile, least)
                    .into_iter()
                    .map(|prediction| (prediction.control, prediction.confidence_permille))
                    .collect()
            }),
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn look(mut args: Vec<String>) -> Result<ExitCode, String> {
    let model = PathBuf::from(take(&mut args, "--model").ok_or("--model is required")?);
    let capture = take(&mut args, "--capture").ok_or("--capture is required")?;
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {args:?}"));
    }
    let deployed = Deployed::load(&model).map_err(|e| e.to_string())?;
    let tile = Tile::from_png(&std::fs::read(Path::new(&capture)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let at = std::time::Instant::now();
    let found = deployed.detect(&tile, 500);
    let took = at.elapsed();
    for prediction in &found {
        let (x, y, width, height) = prediction
            .control
            .in_window(tile.source_width, tile.source_height);
        println!(
            "{:?} at {x},{y} {width}x{height}  sure {}/1000  unsure of its kind {}/1000",
            prediction.control.kind,
            prediction.confidence_permille,
            prediction.uncertainty_permille
        );
    }
    println!("{} controls in {:?}", found.len(), took);
    Ok(ExitCode::SUCCESS)
}

/// What a detector got right, counted the way a click cares about.
#[derive(Default)]
struct Score {
    declared: usize,
    found: usize,
    clickable: usize,
    said: usize,
    invented: usize,
    kind_right: usize,
}

fn report(what: &str, score: &Score) {
    let per_thousand =
        |part: usize, whole: usize| (part * 1_000).checked_div(whole).unwrap_or_default();
    println!(
        "{what}: found {}/1000 of {} declared controls, {}/1000 of them near enough to click, \
         {}/1000 of what it said was not there, kind right {}/1000",
        per_thousand(score.found, score.declared),
        score.declared,
        per_thousand(score.clickable, score.declared),
        per_thousand(score.invented, score.said.max(1)),
        per_thousand(score.kind_right, score.found.max(1))
    );
}

fn scored(samples: &[Sample], mut detect: impl FnMut(&Sample) -> Vec<(Box2, u16)>) -> Score {
    let mut score = Score::default();
    for sample in samples {
        let found = detect(sample);
        score.declared += sample.controls.len();
        score.said += found.len();
        let mut used = vec![false; found.len()];
        for control in &sample.controls {
            let (centre_x, centre_y) = control.centre();
            // The nearest thing it said that is not already spoken for.
            let mut best: Option<(usize, f32)> = None;
            for (index, (box2, _)) in found.iter().enumerate() {
                if used[index] {
                    continue;
                }
                let (fx, fy) = box2.centre();
                let distance = ((fx - centre_x).powi(2) + (fy - centre_y).powi(2)).sqrt();
                if best.is_none() || best.is_some_and(|(_, so_far)| distance < so_far) {
                    best = Some((index, distance));
                }
            }
            let Some((index, _)) = best else {
                continue;
            };
            let (box2, _) = found[index];
            // Found at all: what it drew overlaps the real control enough to be about it.
            if overlap(&box2, control) < 0.3 {
                continue;
            }
            used[index] = true;
            score.found += 1;
            if box2.kind == control.kind {
                score.kind_right += 1;
            }
            // Near enough to click: the middle of what it drew is inside the real control.
            let (fx, fy) = box2.centre();
            if fx >= control.x
                && fx <= control.x + control.width
                && fy >= control.y
                && fy <= control.y + control.height
            {
                score.clickable += 1;
            }
        }
        score.invented += used.iter().filter(|used| !**used).count();
    }
    score
}

/// The share of the smaller box that two boxes share.
fn overlap(a: &Box2, b: &Box2) -> f32 {
    let x = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let y = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if x <= 0.0 || y <= 0.0 {
        return 0.0;
    }
    (x * y) / (a.width * a.height).min(b.width * b.height).max(1.0)
}
