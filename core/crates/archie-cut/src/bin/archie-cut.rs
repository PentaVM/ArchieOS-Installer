//! `archie-cut` — a video from a timeline, from the command line.
//!
//! ```text
//! archie-cut probe  <media folder> <file>
//! archie-cut render <media folder> <timeline.json> <name.mp4> [--melt melt] [--limit-s 1800]
//! ```
//!
//! Every file is resolved inside the media folder; the video and its MLT project are
//! written to `renders/` there. Results are printed as JSON.

use archie_cut::{probe, render, Timeline};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    if at + 1 >= args.len() {
        return None;
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Some(value)
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("archie-cut: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: Vec<String>) -> Result<String, String> {
    const USAGE: &str =
        "usage: archie-cut probe <media> <file> | render <media> <timeline.json> <name.mp4>";
    let melt = take(&mut args, "--melt").unwrap_or_else(|| "melt".into());
    let ffprobe = take(&mut args, "--ffprobe").unwrap_or_else(|| "ffprobe".into());
    let limit = take(&mut args, "--limit-s")
        .map(|s| s.parse::<u64>().map_err(|_| "--limit-s is seconds"))
        .transpose()?
        .unwrap_or(1800);
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["probe", media, file] => {
            let found =
                probe(Path::new(&ffprobe), Path::new(media), file).map_err(|e| e.to_string())?;
            serde_json::to_string_pretty(&found).map_err(|e| e.to_string())
        }
        ["render", media, plan, name] => {
            let text = std::fs::read_to_string(plan).map_err(|e| format!("{plan}: {e}"))?;
            let timeline: Timeline =
                serde_json::from_str(&text).map_err(|e| format!("{plan}: {e}"))?;
            let made = render(
                &timeline,
                Path::new(media),
                name,
                Path::new(&melt),
                Duration::from_secs(limit),
            )
            .map_err(|e| e.to_string())?;
            serde_json::to_string_pretty(&made).map_err(|e| e.to_string())
        }
        _ => Err(USAGE.into()),
    }
}
