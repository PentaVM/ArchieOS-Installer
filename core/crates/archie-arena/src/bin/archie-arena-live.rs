//! `archie-arena-live` — one run of a task in which IRIS acts frame by frame.
//!
//! ```text
//! archie-arena-live --task <task.toml> --tier <seconds> --socket <control.sock> --key-file <key>
//!                   --policy <apps.toml> --out <runs dir> --iris <http://127.0.0.1:8765>
//!                   [--settle-s 20] [--planners <model,...>] [--skill <text>]
//! ```
//!
//! Before the clock (untimed): the task's reset, the window's settling, and the planner's
//! steps of what to do. On the clock: capture, `POST /act` to IRIS with the step in force,
//! and the changes to what is held sent through the Stage gate — as fast as a capture and an
//! answer allow. The run writes `<out>/<task>/<run>/` (result.json, trajectory.jsonl, a frame
//! every second in frames/) and appends to `<out>/results.jsonl`.

use archie_arena::live::{step_at, teal_pixels, Control, Decision, Held, Plan, ResetCounter, Step};
use archie_arena::model::Nebius;
use archie_arena::seat::{load_key, take, Seat};
use archie_arena::task::Task;
use archie_policy::PolicyEngine;
use archie_stage::{learn, Operator, PointerButton, StageAction, StageClient, StagePolicy, StageWorld};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;

/// How wide the compositor draws the frames IRIS reads (IRIS letterboxes to 512).
const FRAME_WIDTH: u32 = 512;

const PLANNER: &str = "You are the planner of Archie, an AI that operates a computer. A fast visual model, IRIS, \
does the doing: it sees the screen many times a second and turns ONE short instruction at a time into mouse and \
keyboard input. You decide WHAT it should do, never HOW (no coordinates, no key names unless the task names them). \
Reply with ONE JSON object only: {\"skill\": \"the tool or game being used, a few words\", \"steps\": [{\"do\": \
\"one short imperative for IRIS, e.g. drive toward the ball / jump and roll left / click the Export button\", \
\"seconds\": N}, ...]}. Order the steps; their seconds should add up to about the time given.";

fn log_line(path: &Path, value: serde_json::Value) {
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{value}");
    }
}

async fn send(seat: &mut Seat, action: StageAction, why: &str) -> String {
    match seat.act(action, why, Duration::from_secs(5)).await {
        Ok(outcome) if outcome.executed() => "executed".into(),
        Ok(outcome) => format!("{outcome:?}"),
        Err(error) => error,
    }
}

async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let task_path = PathBuf::from(take(&mut args, "--task").ok_or("--task is required")?);
    let tier: u32 = take(&mut args, "--tier").ok_or("--tier is required")?.parse().map_err(|_| "--tier is seconds")?;
    let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
    let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
    let policy = StagePolicy::load(&take(&mut args, "--policy").ok_or("--policy is required")?).map_err(|e| e.to_string())?;
    let out_root = PathBuf::from(take(&mut args, "--out").ok_or("--out is required")?);
    let iris = take(&mut args, "--iris").ok_or("--iris is required")?.trim_end_matches('/').to_string();
    let settle: u64 = take(&mut args, "--settle-s").map(|v| v.parse().unwrap_or(20)).unwrap_or(20);
    let planners = take(&mut args, "--planners");
    let skill_flag = take(&mut args, "--skill");
    let mut task = Task::load(&task_path)?;
    if let Some(list) = planners {
        task.planners = list.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
    }
    let stamp = format!("{}-live-t{tier}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let out = out_root.join(&task.id).join(&stamp);
    let frames = out.join("frames");
    std::fs::create_dir_all(&frames).map_err(|e| e.to_string())?;
    let trajectory = out.join("trajectory.jsonl");

    if let Some(command) = task.reset.as_ref() {
        let status = tokio::process::Command::new("sh").arg("-c").arg(command).status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("the reset failed: {status}"));
        }
    }
    let stream = UnixStream::connect(&socket).await.map_err(|e| format!("{socket}: {e}"))?;
    let (read, write) = stream.into_split();
    let mut seat = Seat {
        operator: Operator::new(key, policy, PolicyEngine::default()),
        client: StageClient::new(read, write),
        world: StageWorld::default(),
        snapshots: Path::new(&socket).parent().ok_or("--socket has no directory")?.join("snapshots"),
        app_id: task.app_id.clone(),
        principal_id: "archie-arena-live".into(),
    };
    let ready_by = Instant::now() + Duration::from_secs(240);
    while seat.world.surfaces_of(&task.app_id).next().is_none() {
        learn(&mut seat.client, &mut seat.world, Duration::from_millis(500)).await.map_err(|e| e.to_string())?;
        if Instant::now() > ready_by {
            return Err(format!("{} never opened a window", task.app_id));
        }
    }
    // The application loads (a game's assets, a page) before anything is timed.
    tokio::time::sleep(Duration::from_secs(settle)).await;
    let first = seat.look().await?;
    std::fs::write(out.join("first.png"), first.png()).map_err(|e| e.to_string())?;

    // The plan, untimed: what to do, not how.
    let nebius = Nebius::from_environment()?;
    let brief = format!("TASK: {}\n\nTIME: {tier} s once you answer. The screenshot is the screen now ({}x{}).",
                        task.prompt, first.width, first.height);
    let answer = nebius
        .ask(&task.planners, PLANNER, &brief, &[archie_arena::image::data_url(&first)], 4000, Duration::from_secs(300))
        .await?;
    let plan: Plan = answer
        .text
        .find('{')
        .zip(answer.text.rfind('}'))
        .and_then(|(a, b)| serde_json::from_str(&answer.text[a..=b]).ok())
        .unwrap_or(Plan { skill: String::new(), steps: vec![Step { what: task.prompt.clone(), seconds: tier as f32 }] });
    let skill = skill_flag.unwrap_or_else(|| plan.skill.clone());
    log_line(&trajectory, json!({"t": 0.0, "plan": plan.steps.iter().map(|s| json!({"do": s.what, "seconds": s.seconds})).collect::<Vec<_>>(),
                                 "skill": skill, "planner": answer.model, "planner_seconds": answer.seconds}));

    // The clock.
    let http = reqwest::Client::builder().timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
    let _ = http.post(format!("{iris}/act/reset?session={stamp}")).send().await;
    let focus = send(&mut seat, StageAction::Focus, "give the application the keyboard").await;
    log_line(&trajectory, json!({"t": 0.0, "focus": focus}));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(tier as u64);
    let mut held = Held::default();
    let mut resets = ResetCounter::default();
    let (mut ticks, mut sent, mut refused, mut failed) = (0u64, 0u64, 0u64, 0u64);
    let mut act_ms: Vec<f32> = Vec::new();
    let mut last_saved = -1i64;
    // The frame IRIS reads is drawn FRAME_WIDTH wide by the compositor and sent raw: a
    // full-size PNG cost 240 ms of a 400 ms decision. While IRIS decides on a frame, the
    // decision before it keeps playing: its chunk (the controls planned for the next frames)
    // is carried out on a 60 Hz clock, so the game gets input every frame even when a decision
    // takes several. One connection does both, in turn: the Stage's sequence check is global.
    let surface_width = seat.world.surfaces_of(&task.app_id).next().map(|s| s.width).unwrap_or(0).max(1) as f32;
    let mut clock = tokio::time::interval(Duration::from_millis(16));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut decision: Option<Decision> = None;
    let mut decisions = 0u64;
    while Instant::now() < deadline {
        let tick = Instant::now();
        let elapsed = started.elapsed().as_secs_f32();
        let Some(step) = step_at(&plan.steps, elapsed).cloned() else { break };
        let window = match seat.frame(FRAME_WIDTH).await {
            Ok(frame) => frame,
            Err(error) => {
                failed += 1;
                log_line(&trajectory, json!({"t": elapsed, "look": error}));
                continue;
            }
        };
        let frame_ms = started.elapsed().as_millis();
        let look_ms = tick.elapsed().as_millis();
        // Window pixels per frame pixel: where IRIS points goes back to the window with it.
        let scale = (surface_width / window.width.max(1) as f32).max(1.0);
        // The popup's teal was counted on full-size captures; a smaller frame has fewer pixels.
        if resets.observe(teal_pixels(&window.pixels), (150.0 / (scale * scale)).ceil() as usize) {
            log_line(&trajectory, json!({"t": elapsed, "event": "flip_reset_popup"}));
        }
        if elapsed as i64 > last_saved {
            last_saved = elapsed as i64;
            let _ = std::fs::write(frames.join(format!("{:04}.png", ticks)), window.png());
        }
        let url = format!("{iris}/act?session={stamp}&instruction={}&skill={}", urlencode(&step.what), urlencode(&skill));
        let ask = http.post(&url).body(archie_arena::image::ppm(&window)).send();
        tokio::pin!(ask);
        let mut played = Vec::new();
        let answer = loop {
            tokio::select! {
                answer = &mut ask => break answer,
                _ = clock.tick() => {
                    if let Some(current) = &decision {
                        let now_ms = started.elapsed().as_millis();
                        let (actions, _) = held.actions_towards(&current.at(now_ms), now_ms);
                        for action in actions {
                            let kind = action.kind().as_str();
                            let said = send(&mut seat, action, &step.what).await;
                            if said == "executed" { sent += 1 } else { refused += 1 }
                            played.push(format!("{kind}:{said}"));
                        }
                    }
                }
            }
        };
        let control: Control = match answer {
            Ok(response) => match response.json().await {
                Ok(control) => control,
                Err(error) => {
                    failed += 1;
                    log_line(&trajectory, json!({"t": elapsed, "iris": error.to_string()}));
                    continue;
                }
            },
            Err(error) => {
                failed += 1;
                log_line(&trajectory, json!({"t": elapsed, "iris": error.to_string()}));
                continue;
            }
        };
        if let Some(error) = &control.error {
            failed += 1;
            log_line(&trajectory, json!({"t": elapsed, "iris_error": error}));
            continue;
        }
        act_ms.push(control.latency_ms);
        decisions += 1;
        let current = Decision { number: decisions, control, frame_ms };
        let now_ms = started.elapsed().as_millis();
        let (mut actions, unsupported) = held.actions_towards(&current.at(now_ms), now_ms);
        // A GUI step IRIS says is a click lands where it points, once per decision.
        let click = match current.control.step_type.as_str() {
            "click" => Some(PointerButton::Left),
            "right_click" => Some(PointerButton::Right),
            _ => None,
        };
        if let (Some(button), true) = (click, task.judge == "gui") {
            if let [x, y] = current.control.pointer[..] {
                actions.push(StageAction::Click { x: (x * scale).round() as i32, y: (y * scale).round() as i32, button });
            }
        }
        let mut outcomes = Vec::new();
        for action in actions {
            let kind = action.kind().as_str();
            let said = send(&mut seat, action, &step.what).await;
            if said == "executed" {
                sent += 1;
            } else {
                refused += 1;
            }
            outcomes.push(format!("{kind}:{said}"));
        }
        ticks += 1;
        log_line(&trajectory, json!({"t": elapsed, "step": step.what, "keys": current.control.keys,
                                     "buttons": current.control.buttons, "step_type": current.control.step_type,
                                     "chunk": current.control.chunk.len(), "act_ms": current.control.latency_ms,
                                     "look_ms": look_ms, "tick_ms": tick.elapsed().as_millis(),
                                     "sent": outcomes, "played_from_chunk": played, "unsupported": unsupported}));
        decision = Some(current);
    }
    for action in held.release_all() {
        let _ = send(&mut seat, action, "the run is over").await;
    }
    let spent = started.elapsed().as_secs_f32();
    act_ms.sort_by(|a, b| a.total_cmp(b));
    let result = json!({
        "task": task.id, "tier": tier, "run": stamp, "mode": "iris-live", "seconds_spent": spent,
        "ticks": ticks, "ticks_per_second": ticks as f32 / spent.max(0.001), "actions_executed": sent,
        "actions_refused": refused, "failed": failed, "plan_steps": plan.steps.len(), "skill": skill,
        "iris_act_ms_p50": act_ms.get(act_ms.len() / 2), "flip_resets": resets.resets,
        "score": if task.judge == "flip_reset" { json!(resets.resets) } else { json!(null) },
    });
    std::fs::write(out.join("result.json"), serde_json::to_string_pretty(&result).unwrap_or_default()).map_err(|e| e.to_string())?;
    log_line(&out_root.join("results.jsonl"), result.clone());
    println!("{result}");
    Ok(ExitCode::SUCCESS)
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("archie-arena-live: {error}");
            ExitCode::FAILURE
        }
    }
}
