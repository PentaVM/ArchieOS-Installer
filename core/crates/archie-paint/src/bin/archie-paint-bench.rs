//! `archie-paint-bench` — DRAW THE MONA LISA.
//!
//! ```text
//! archie-paint-bench paint --reference <png|jpg> --out <events.jsonl> [--focus <focus.json>]
//!                          [--width 384] [--see-seconds S]
//! archie-paint-bench judge --reference <png|jpg> --events <events.jsonl> --out-dir <dir>
//!                          [--see-seconds S] [--plan-seconds S] [--painter NAME] [--critic MODEL,…]
//! archie-paint-bench run   --reference <png|jpg> --out-dir <dir> [--focus <focus.json>]
//!                          [--see-seconds S] [--painter-cmd "<command …>"]
//! ```
//!
//! `paint` is the deterministic painter ([`archie_paint::bench::realist`]). `judge` replays
//! a painter's keyboard and mouse on white paper within what is left of the fifteen seconds,
//! measures the drawing against the picture, and asks a critic model on Nebius
//! (`NEBIUS_TOKEN_FACTORY_KEY`) how alike and how real it is. `run` times a painter — this
//! one, or any command that writes events to stdout — and judges it. The result goes to
//! `<dir>/result.json`, the drawing to `<dir>/drawing.png`, a side-by-side to
//! `<dir>/compare.png`.

use archie_paint::bench::{self, Focus, Input, Sheet, EVENTS_PER_SECOND, SECONDS};
use archie_paint::Picture;
use base64::Engine;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.remove(at);
    (at < args.len()).then(|| args.remove(at))
}

fn number<T: std::str::FromStr>(args: &mut Vec<String>, flag: &str) -> Result<Option<T>, String> {
    take(args, flag)
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|_| format!("{flag} is a number"))
        })
        .transpose()
}

fn reference(path: &str, width: u32) -> Result<Sheet, String> {
    let picture = Picture::read(Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
    if picture.colour(0, 0).is_none() {
        return Err(format!("{path} has no colour"));
    }
    let height = (picture.height() as f32 * width as f32 / picture.width() as f32).round() as u32;
    Ok(bench::fitted(&picture, width, height))
}

fn focus_of(path: Option<String>) -> Result<Vec<Focus>, String> {
    match path {
        Some(path) => serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?,
        )
        .map_err(|e| format!("{path}: {e}"))
        .and_then(|value| {
            serde_json::from_value::<Vec<Focus>>(value.get("focus").cloned().unwrap_or(value))
                .map_err(|e| format!("{path}: {e}"))
        }),
        None => Ok(Vec::new()),
    }
}

/// The realist's settings from `--palette N --layers 12:1,6:0.45,… --focus-strength F`.
fn settings(args: &mut Vec<String>) -> Result<bench::Realist, String> {
    let mut settings = bench::Realist::default();
    if let Some(n) = number(args, "--palette")? {
        settings.palette = n;
    }
    if let Some(text) = take(args, "--layers") {
        settings.layers = text
            .split(',')
            .map(|layer| {
                let (d, share) = layer
                    .split_once(':')
                    .ok_or("--layers is diameter:share,…")?;
                Ok((
                    d.parse().map_err(|_| "a diameter is a whole number")?,
                    share.parse().map_err(|_| "a share is a number")?,
                ))
            })
            .collect::<Result<_, String>>()?;
    }
    if let Some(f) = number(args, "--focus-strength")? {
        settings.focus_strength = f;
    }
    Ok(settings)
}

/// Events left for drawing once the painter has looked and planned.
fn budget(see: f32, plan: f32) -> usize {
    ((SECONDS - see - plan).max(0.0) * EVENTS_PER_SECOND) as usize
}

fn paint(mut args: Vec<String>) -> Result<ExitCode, String> {
    let started = Instant::now();
    let path = take(&mut args, "--reference").ok_or("--reference is required")?;
    let out = take(&mut args, "--out");
    let width: u32 = number(&mut args, "--width")?.unwrap_or(384);
    let see: f32 = number(&mut args, "--see-seconds")?.unwrap_or(0.0);
    let focus = focus_of(take(&mut args, "--focus"))?;
    let settings = settings(&mut args)?;
    let target = reference(&path, width)?;
    // Planned for a little less than all the time left, then cut to what truly is left.
    let planned = bench::realist_with(&target, &focus, budget(see, 1.0), &settings);
    let mut events = planned;
    events.truncate(budget(see, started.elapsed().as_secs_f32()));
    let text: String = events
        .iter()
        .map(|event| serde_json::to_string(event).expect("an event serialises") + "\n")
        .collect();
    match out {
        Some(path) => std::fs::write(&path, text).map_err(|e| format!("{path}: {e}"))?,
        None => print!("{text}"),
    }
    eprintln!(
        "{} events planned in {:.2} s",
        events.len(),
        started.elapsed().as_secs_f32()
    );
    Ok(ExitCode::SUCCESS)
}

fn read_events(text: &str) -> Vec<Input> {
    // A line that is not an event is what a real device would do with noise: nothing.
    text.lines()
        .filter_map(|line| serde_json::from_str::<Input>(line.trim()).ok())
        .collect()
}

fn side_by_side(a: &Sheet, b: &Sheet) -> Sheet {
    let gap = 8;
    let mut out = Sheet::white(a.width * 2 + gap, a.height);
    for y in 0..a.height {
        for x in 0..a.width {
            out.pixels[(y * out.width + x) as usize] = a.pixels[(y * a.width + x) as usize];
            out.pixels[(y * out.width + a.width + gap + x) as usize] =
                b.pixels[(y * b.width + x) as usize];
        }
    }
    out
}

async fn critic(reference: &Sheet, drawing: &Sheet, models: &[String]) -> serde_json::Value {
    let Ok(key) = std::env::var("NEBIUS_TOKEN_FACTORY_KEY") else {
        return json!({"error": "NEBIUS_TOKEN_FACTORY_KEY is not set"});
    };
    let image = |sheet: &Sheet| {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(sheet.png())
        )
    };
    let question = "You are an art critic judging a drawing benchmark. The FIRST image is the \
        reference: Leonardo da Vinci's Mona Lisa. The SECOND image is a drawing an AI made of \
        it with only a mouse and keyboard in 15 seconds. Judge ONLY the second image against \
        the first. Reply with one line of JSON and nothing else: {\"likeness\": 0-10 (how \
        exactly it reproduces the reference: composition, face, hands, colours), \"realism\": \
        0-10 (how photorealistic and painterly it looks rather than blocky or scribbled), \
        \"critique\": \"at most two sentences on what most keeps it from the reference\"}. \
        Be strict and consistent: 10 is indistinguishable from the reference.";
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
    {
        Ok(http) => http,
        Err(error) => return json!({"error": error.to_string()}),
    };
    let mut last_error = String::new();
    for model in models {
        let body = json!({
            "model": model, "temperature": 0.0, "max_tokens": 2048,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": question},
                {"type": "image_url", "image_url": {"url": image(reference)}},
                {"type": "image_url", "image_url": {"url": image(drawing)}},
            ]}],
        });
        let answer = http
            .post("https://api.tokenfactory.nebius.com/v1/chat/completions")
            .bearer_auth(&key)
            .json(&body)
            .send()
            .await;
        let text = match answer {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.json::<serde_json::Value>().await {
                    Ok(value) => value["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    Err(error) => {
                        last_error = format!("{model}: {error}");
                        continue;
                    }
                },
                Err(error) => {
                    last_error = format!(
                        "{model}: {}",
                        error.status().map_or("failed".into(), |s| s.to_string())
                    );
                    continue;
                }
            },
            Err(error) => {
                last_error = format!("{model}: {error}");
                continue;
            }
        };
        let parsed = text
            .find('{')
            .zip(text.rfind('}'))
            .and_then(|(a, b)| serde_json::from_str::<serde_json::Value>(&text[a..=b]).ok());
        if let Some(mut verdict) = parsed {
            if verdict["likeness"].is_number() && verdict["realism"].is_number() {
                verdict["model"] = json!(model);
                return verdict;
            }
        }
        last_error = format!(
            "{model}: no verdict in {:?}",
            text.chars().take(160).collect::<String>()
        );
    }
    json!({"error": last_error})
}

async fn judge_into(
    target: &Sheet,
    events: &[Input],
    see: f32,
    plan: f32,
    painter: &str,
    out_dir: &Path,
    critics: &[String],
) -> Result<serde_json::Value, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let allowed = budget(see, plan);
    let (drawing, replayed) = bench::replay(events, target.width, target.height, allowed);
    let ssim = bench::ssim(target, &drawing);
    let colour_error = bench::colour_error(target, &drawing);
    let verdict = critic(target, &drawing, critics).await;
    let critic_quality = match (verdict["likeness"].as_f64(), verdict["realism"].as_f64()) {
        (Some(l), Some(r)) => Some(((l + r) / 20.0).clamp(0.0, 1.0) as f32),
        _ => None,
    };
    let objective = 0.5 * ssim.max(0.0) + 0.5 * (1.0 - (colour_error / 50.0).min(1.0));
    let quality = match critic_quality {
        Some(critic) => 0.6 * critic + 0.4 * objective,
        None => objective,
    };
    let seconds = see + plan + replayed.seconds_of_input;
    let on_time = if see + plan <= SECONDS {
        1.0
    } else {
        SECONDS / (see + plan)
    };
    let score = 100.0 * quality * on_time;
    std::fs::write(out_dir.join("drawing.png"), drawing.png()).map_err(|e| e.to_string())?;
    std::fs::write(
        out_dir.join("compare.png"),
        side_by_side(target, &drawing).png(),
    )
    .map_err(|e| e.to_string())?;
    let result = json!({
        "benchmark": "draw-the-mona-lisa", "version": 1, "painter": painter,
        "unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "score": (score * 10.0).round() / 10.0,
        "quality": quality, "objective": objective, "critic_quality": critic_quality,
        "ssim": ssim, "colour_error_delta_e": colour_error,
        "seconds": {"see": see, "plan": plan, "input": replayed.seconds_of_input, "total": seconds, "limit": SECONDS},
        "events": replayed, "critic": verdict,
    });
    std::fs::write(
        out_dir.join("result.json"),
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(result)
}

fn critics(args: &mut Vec<String>) -> Vec<String> {
    take(args, "--critic")
        .unwrap_or_else(|| "moonshotai/Kimi-K3,openbmb/MiniCPM-V-4_5".into())
        .split(',')
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect()
}

async fn judge(mut args: Vec<String>) -> Result<ExitCode, String> {
    let path = take(&mut args, "--reference").ok_or("--reference is required")?;
    let events_path = take(&mut args, "--events").ok_or("--events is required")?;
    let out_dir = PathBuf::from(take(&mut args, "--out-dir").ok_or("--out-dir is required")?);
    let see: f32 = number(&mut args, "--see-seconds")?.unwrap_or(0.0);
    let plan: f32 = number(&mut args, "--plan-seconds")?.unwrap_or(0.0);
    let painter = take(&mut args, "--painter").unwrap_or_else(|| "unknown".into());
    let width: u32 = number(&mut args, "--width")?.unwrap_or(384);
    let critics = critics(&mut args);
    let target = reference(&path, width)?;
    let text = std::fs::read_to_string(&events_path).map_err(|e| format!("{events_path}: {e}"))?;
    let result = judge_into(
        &target,
        &read_events(&text),
        see,
        plan,
        &painter,
        &out_dir,
        &critics,
    )
    .await?;
    println!("{}", serde_json::to_string(&result).unwrap_or_default());
    Ok(ExitCode::SUCCESS)
}

async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let path = take(&mut args, "--reference").ok_or("--reference is required")?;
    let out_dir = PathBuf::from(take(&mut args, "--out-dir").ok_or("--out-dir is required")?);
    let see: f32 = number(&mut args, "--see-seconds")?.unwrap_or(0.0);
    let focus = take(&mut args, "--focus");
    let width: u32 = number(&mut args, "--width")?.unwrap_or(384);
    let painter_cmd = take(&mut args, "--painter-cmd");
    let critics = critics(&mut args);
    let settings = settings(&mut args)?;
    let painter_name = take(&mut args, "--painter").unwrap_or_else(|| {
        if painter_cmd.is_some() {
            "external".into()
        } else if focus.is_some() {
            "realist+iris-focus".into()
        } else {
            "realist".into()
        }
    });
    let target = reference(&path, width)?;
    let started = Instant::now();
    let text = match painter_cmd {
        Some(command) => {
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .env("BENCH_REFERENCE", &path)
                .env("BENCH_WIDTH", width.to_string())
                .env("BENCH_SEE_SECONDS", see.to_string())
                .output()
                .map_err(|e| format!("painter: {e}"))?;
            String::from_utf8_lossy(&output.stdout).into_owned()
        }
        None => {
            let focus = focus_of(focus)?;
            let mut events = bench::realist_with(&target, &focus, budget(see, 1.0), &settings);
            events.truncate(budget(see, started.elapsed().as_secs_f32()));
            events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap_or_default() + "\n")
                .collect()
        }
    };
    let plan = started.elapsed().as_secs_f32();
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("events.jsonl"), &text).map_err(|e| e.to_string())?;
    let result = judge_into(
        &target,
        &read_events(&text),
        see,
        plan,
        &painter_name,
        &out_dir,
        &critics,
    )
    .await?;
    println!("{}", serde_json::to_string(&result).unwrap_or_default());
    Ok(ExitCode::SUCCESS)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: archie-paint-bench paint|judge|run …");
        return ExitCode::from(2);
    }
    let command = args.remove(0);
    let outcome = match command.as_str() {
        "paint" => paint(args),
        "judge" => judge(args).await,
        "run" => run(args).await,
        other => Err(format!("unknown command {other:?}: paint, judge or run")),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            eprintln!("archie-paint-bench: {error}");
            ExitCode::from(1)
        }
    }
}
