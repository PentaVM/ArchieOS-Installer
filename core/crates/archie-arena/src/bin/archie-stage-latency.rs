//! `archie-stage-latency` — how long one action takes through the Stage's gate, per kind.
//!
//! ```text
//! archie-stage-latency --socket <control.sock> --key-file <key> --policy <apps.toml>
//!                      --app <app_id> [--count 40] [--out <sent.jsonl>] [--snapshot <end.png>]
//! ```
//!
//! Each round holds a key down and lets it go (what a game's "drive forward" is), moves the
//! pointer and clicks. Every action is signed, leased and checked like any other; the time
//! measured is from asking to the compositor's receipt. `--out` writes when each action was
//! sent (wall clock, ms), so the probe window's own log can say when the input arrived.

use archie_arena::seat::{load_key, take, Seat};
use archie_policy::PolicyEngine;
use archie_stage::{learn, Key, Operator, PointerButton, StageAction, StageClient, StagePolicy, StageWorld};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;

fn wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn percentile(sorted: &[f64], share: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * share).round() as usize]
}

async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
    let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
    let policy = StagePolicy::load(&take(&mut args, "--policy").ok_or("--policy is required")?).map_err(|e| e.to_string())?;
    let app_id = take(&mut args, "--app").ok_or("--app is required")?;
    let count: usize = take(&mut args, "--count").map(|v| v.parse().unwrap_or(40)).unwrap_or(40).clamp(1, 1000);
    let mut sent = match take(&mut args, "--out") {
        Some(path) => Some(std::fs::File::create(&path).map_err(|e| format!("{path}: {e}"))?),
        None => None,
    };
    let snapshot = take(&mut args, "--snapshot");
    let stream = UnixStream::connect(&socket).await.map_err(|e| format!("{socket}: {e}"))?;
    let (read, write) = stream.into_split();
    let mut seat = Seat {
        operator: Operator::new(key, policy, PolicyEngine::default()),
        client: StageClient::new(read, write),
        world: StageWorld::default(),
        snapshots: Path::new(&socket).parent().ok_or("--socket has no directory")?.join("snapshots"),
        app_id: app_id.clone(),
        principal_id: "archie-latency".into(),
    };
    let ready_by = Instant::now() + Duration::from_secs(30);
    while seat.world.surfaces_of(&app_id).next().is_none() {
        learn(&mut seat.client, &mut seat.world, Duration::from_millis(300)).await.map_err(|e| e.to_string())?;
        if Instant::now() > ready_by {
            return Err(format!("{app_id} has no window on the stage"));
        }
    }
    let focus = seat.act(StageAction::Focus, "focus the probe window", Duration::from_secs(5)).await?;
    if !focus.executed() {
        return Err(format!("focus refused: {focus:?}"));
    }
    let w = Key::parse("w").ok_or("w is a key")?;
    let mut times: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    let mut refused: BTreeMap<&'static str, usize> = BTreeMap::new();
    for round in 0..count {
        let x = 200 + (round as i32 % 5) * 80;
        let y = 150 + (round as i32 % 3) * 60;
        let plan: [(&'static str, StageAction); 4] = [
            ("key_down", StageAction::KeyDown { key: w }),
            ("key_up", StageAction::KeyUp { key: w }),
            ("move", StageAction::Move { x, y }),
            ("click", StageAction::Click { x, y, button: PointerButton::Left }),
        ];
        for (kind, action) in plan {
            let at = wall_ms();
            let started = Instant::now();
            let outcome = seat.act(action, "latency probe", Duration::from_secs(5)).await;
            let took = started.elapsed().as_secs_f64() * 1000.0;
            let executed = matches!(&outcome, Ok(o) if o.executed());
            if executed {
                times.entry(kind).or_default().push(took);
            } else {
                *refused.entry(kind).or_default() += 1;
            }
            if let Some(file) = sent.as_mut() {
                let _ = writeln!(file, "{}", json!({"t": at, "kind": kind, "round": round, "gate_ms": took,
                                                   "executed": executed, "x": x, "y": y}));
            }
        }
    }
    // What the window shows at the end, for a check no log can give (what was typed where).
    if let Some(path) = snapshot {
        let window = seat.look().await?;
        std::fs::write(&path, window.png()).map_err(|e| format!("{path}: {e}"))?;
    }
    let mut summary = serde_json::Map::new();
    for (kind, mut values) in times {
        values.sort_by(|a, b| a.total_cmp(b));
        summary.insert(kind.into(), json!({"n": values.len(), "p50_ms": percentile(&values, 0.5),
                                           "p95_ms": percentile(&values, 0.95), "max_ms": percentile(&values, 1.0)}));
    }
    summary.insert("refused".into(), json!(refused));
    println!("{}", serde_json::Value::Object(summary));
    Ok(ExitCode::SUCCESS)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("archie-stage-latency: {error}");
            ExitCode::FAILURE
        }
    }
}
