//! `archie-arena` — one run of a task, on the Archie Stage, at one time limit.
//!
//! ```text
//! archie-arena run --task <task.toml> --tier <seconds> --socket <control.sock> --key-file <key>
//!                  --policy <apps.toml> --out <runs dir> [--memory <memory.json>]
//!                  [--reset "<command>"] [--max-width 1280]
//! ```
//!
//! `--reset` is run before the clock starts, to give the application back its first state
//! (a new white document, a game's first frame): it is the arena's, not Archie's, and it is
//! not timed. From then on only Archie acts, only with the mouse and keyboard, only through
//! the Stage gate. The run writes `<out>/<run>/` (result.json, trajectory.jsonl, final.png,
//! compare.png) and appends a line to `<out>/results.jsonl`; the lessons go to `--memory`.

use archie_arena::image::{crop, data_url, decode, document_now, fit_width, page_in_editor};
use archie_arena::memory::{Memory, Remembered};
use archie_arena::model::Nebius;
use archie_arena::moves::{decision_of, stage_actions, Decision, Frame, Move};
use archie_arena::task::Task;
use archie_paint::bench::{self, Sheet};
use archie_paint::Picture;
use archie_policy::{PolicyEngine, Principal};
use archie_stage::{
    drain, learn, ActOutcome, ActRequest, CommandKey, LookOutcome, Operator, StageAction, StageClient, StagePolicy,
    StageWorld,
};
use archie_types::PrivacyGrade;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

type Client = StageClient<OwnedReadHalf, OwnedWriteHalf>;

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.remove(at);
    (at < args.len()).then(|| args.remove(at))
}

fn load_key(path: &str) -> Result<CommandKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let hex = text.trim();
    if hex.len() != 64 {
        return Err(format!("{path}: a stage key is 64 hex characters"));
    }
    let mut bytes = [0_u8; 32];
    for (at, pair) in hex.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(pair).map_err(|_| "a key is hex")?;
        bytes[at] = u8::from_str_radix(pair, 16).map_err(|_| "a key is hex")?;
    }
    Ok(CommandKey::from_bytes(&bytes))
}

/// The Stage, as the arena uses it: one connection, one operator, one application.
struct Seat {
    operator: Operator,
    client: Client,
    world: StageWorld,
    snapshots: PathBuf,
    app_id: String,
}

impl Seat {
    fn request(&self, action: StageAction, why: &str) -> Result<ActRequest, String> {
        let surface = self
            .world
            .surfaces_of(&self.app_id)
            .next()
            .ok_or_else(|| format!("{} has no window on the stage", self.app_id))?
            .surface;
        Ok(ActRequest {
            app_id: self.app_id.clone(),
            surface: Some(surface),
            action,
            text_grade: PrivacyGrade::Public,
            principal: Principal::Human { id: "archie-arena".into(), authenticated: true },
            task_id: None,
            user_present: true,
            approved: false,
            justification: why.chars().take(200).collect(),
        })
    }

    async fn look(&mut self) -> Result<Sheet, String> {
        drain(&mut self.client, &mut self.world).await.map_err(|e| e.to_string())?;
        let request = self.request(StageAction::Snapshot { region: None }, "look at the window")?;
        match self
            .operator
            .look(&mut self.client, &mut self.world, &request, &self.snapshots, Duration::from_secs(10))
            .await
            .map_err(|e| e.to_string())?
        {
            LookOutcome::Captured(snapshot) => decode(&snapshot.png),
            LookOutcome::NotCaptured(outcome) => Err(format!("no capture: {outcome:?}")),
        }
    }

    async fn act(&mut self, action: StageAction, why: &str, timeout: Duration) -> Result<ActOutcome, String> {
        // What the compositor said since the last action first: a dialog that opened moved the
        // window's geometry on, and a command prepared for the old one is refused as
        // surface_changed.
        drain(&mut self.client, &mut self.world).await.map_err(|e| e.to_string())?;
        let request = self.request(action, why)?;
        self.operator
            .act(&mut self.client, &mut self.world, &request, timeout)
            .await
            .map_err(|e| e.to_string())
    }
}

const SYSTEM: &str = "You are Archie, the AI of ArchieOS, operating a real computer. You see a screenshot \
of one application window and act ONLY with the mouse and the keyboard, through actions you \
return as JSON. Coordinates are pixels of the screenshot you are given (x to the right, y down). \
Reply with ONE JSON object and nothing else: {\"thought\": \"one sentence: what you see and what you do next\", \
\"actions\": [ ... ]}. Actions: {\"do\":\"click\",\"x\":X,\"y\":Y} | {\"do\":\"double_click\",\"x\":X,\"y\":Y} | \
{\"do\":\"drag\",\"from\":[X,Y],\"to\":[X,Y]} | {\"do\":\"key\",\"keys\":\"b\" or \"ctrl+z\"} | \
{\"do\":\"type\",\"text\":\"...\"} | {\"do\":\"strokes\",\"paths\":[[[X,Y],[X,Y],...], ...]} (press, draw \
along the points, release; many lines per action) | {\"do\":\"scroll\",\"x\":X,\"y\":Y,\"dy\":N} | \
{\"do\":\"wait\",\"ms\":N} | {\"do\":\"note\",\"text\":\"something to remember for next time, such as where a tool is\"} | \
{\"do\":\"done\"}. Time is short and counts while you think: keep the thought to one sentence and \
ALWAYS return actions — several at once, never an empty list. You will see their result in the next \
screenshot.";

fn log_line(path: &Path, value: serde_json::Value) {
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{value}");
    }
}

async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let task_path = PathBuf::from(take(&mut args, "--task").ok_or("--task is required")?);
    let tier: u32 = take(&mut args, "--tier").ok_or("--tier is required")?.parse().map_err(|_| "--tier is seconds")?;
    let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
    let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
    let policy = StagePolicy::load(&take(&mut args, "--policy").ok_or("--policy is required")?).map_err(|e| e.to_string())?;
    let out_root = PathBuf::from(take(&mut args, "--out").ok_or("--out is required")?);
    let memory_path = take(&mut args, "--memory").map(PathBuf::from);
    let reset = take(&mut args, "--reset");
    let max_width: u32 = take(&mut args, "--max-width").map(|v| v.parse().unwrap_or(1280)).unwrap_or(1280);
    let actors_flag = take(&mut args, "--actors");
    let planners_flag = take(&mut args, "--planners");
    let mut task = Task::load(&task_path)?;
    let models = |list: String| -> Vec<String> {
        list.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect()
    };
    if let Some(list) = actors_flag {
        task.actors = models(list);
    }
    if let Some(list) = planners_flag {
        task.planners = models(list);
    }
    let nebius = Nebius::from_environment()?;
    let stamp = format!("{}-t{tier}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let out = out_root.join(&task.id).join(&stamp);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let trajectory = out.join("trajectory.jsonl");

    // The application's first state: the arena's doing, not timed.
    if let Some(command) = reset.as_ref().or(task.reset.as_ref()) {
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
    };
    // Ready when the window is there and, for a picture, its white page sits inside the
    // editor's workspace in two captures a second apart: a page still loading is all white,
    // and a run timed from it would be timed from before the editor existed.
    // The first run after the session starts waits for a cold browser and a first download of
    // the editor, drawn in software: 120 s was not always enough.
    let ready_by = Instant::now() + Duration::from_secs(240);
    let mut seen_before: Option<(u32, u32, u32, u32)> = None;
    let (first, page) = loop {
        learn(&mut seat.client, &mut seat.world, Duration::from_millis(500)).await.map_err(|e| e.to_string())?;
        if seat.world.surfaces_of(&task.app_id).next().is_some() {
            if let Ok(sheet) = seat.look().await {
                if task.judge != "picture" {
                    break (sheet, None);
                }
                match page_in_editor(&sheet) {
                    Some((found, workspace)) if seen_before.is_some_and(|b| b == found) => {
                        break (sheet, Some((found, workspace)));
                    }
                    Some((found, _)) => seen_before = Some(found),
                    None => seen_before = None,
                }
            }
        }
        if Instant::now() > ready_by {
            return Err(format!("{} never showed a white page to draw on", task.app_id));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    std::fs::write(out.join("first.png"), first.png()).map_err(|e| e.to_string())?;
    let reference = match &task.reference {
        Some(path) => Some(Picture::read(path).map_err(|e| format!("{}: {e}", path.display()))?),
        None => None,
    };
    let reference_url = match (&reference, &task.reference) {
        (Some(_), Some(path)) => Some(data_url(&decode(&std::fs::read(path).map_err(|e| e.to_string())?)?)),
        _ => None,
    };
    let docs = match &task.docs {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
        None => String::new(),
    };
    let mut memory = memory_path.as_ref().map(|p| Memory::load(p)).unwrap_or_default();
    let lessons = memory.lessons(&task.id, tier);

    // What Archie is told at every decision, the brief's and the timed ones alike.
    let context = |clock: &str, seen: &Sheet| -> String {
        let mut text = format!("TASK: {}\n\n{clock} The screenshot is {}x{} pixels.", task.prompt, seen.width, seen.height);
        if reference_url.is_some() {
            text.push_str(" The FIRST image is the reference picture; the SECOND is the screenshot.");
        }
        if !task.ends_early {
            text.push_str(" The run lasts the whole time and \"done\" does not end it: use every second left to make the result better.");
        }
        if !docs.is_empty() {
            text.push_str(&format!("\n\nDOCUMENTATION:\n{}", docs.chars().take(6000).collect::<String>()));
        }
        if !lessons.is_empty() {
            text.push_str("\n\nLESSONS FROM YOUR EARLIER RUNS OF THIS TASK:\n- ");
            text.push_str(&lessons.join("\n- "));
        }
        text
    };
    let images_with = |seen: &Sheet| -> Vec<String> {
        let mut images: Vec<String> = reference_url.iter().cloned().collect();
        images.push(data_url(seen));
        images
    };
    // Every screen Archie decided on is kept beside the moves it chose (frames/NNNN.png, in
    // the coordinates the moves use; the moves are in trajectory.jsonl under the same frame
    // number): the pairs IRIS learns to act from.
    let frames = out.join("frames");
    std::fs::create_dir_all(&frames).map_err(|e| e.to_string())?;
    let mut frame_count = 0usize;
    let mut keep_frame = |seen: &Sheet| -> usize {
        let index = frame_count;
        let _ = std::fs::write(frames.join(format!("{index:04}.png")), seen.png());
        frame_count += 1;
        index
    };

    // The brief, before the clock. Archie reads the task, the reference and the first screen
    // and plans with no time counted: the thinking is the planner's (Kimi), what is timed is
    // the doing, which IRIS is to take over. The plan comes back at every later decision, and
    // its opening moves run the moment the clock starts.
    let (brief_seen, brief_scale) = fit_width(&first, max_width);
    let brief_frame = Frame { scale: brief_scale, width: first.width, height: first.height };
    let brief_index = keep_frame(&brief_seen);
    let brief_text = format!(
        "{}\n\nBEFORE THE CLOCK: the {tier} s have not started yet. Plan now, without hurry: the whole task and the \
         fastest way to do it well in {tier} s. Reply with ONE JSON object: {{\"thought\": \"one sentence\", \
         \"plan\": [\"short ordered steps, each concrete: which tool, which colour (hex), where on screen in pixels\"], \
         \"actions\": [the opening actions, run the moment the clock starts: as many as you are sure of]}}. The plan \
         is shown to you again at every later step.",
        context(&format!("Time: the whole {tier} s, from the moment you answer."), &brief_seen)
    );
    let mut plan: Vec<String> = Vec::new();
    let mut briefing_seconds = 0.0f32;
    let mut pending: Option<(Decision, Frame)> = None;
    match nebius.ask(&task.planners, SYSTEM, &brief_text, &images_with(&brief_seen), 8000, Duration::from_secs(300)).await {
        Ok(answer) => {
            briefing_seconds = answer.seconds;
            match decision_of(&answer.text) {
                Ok(decision) => {
                    plan = decision.plan.clone();
                    log_line(&trajectory, json!({
                        "t": 0.0, "brief": true, "model": answer.model, "model_seconds": answer.seconds, "frame": brief_index,
                        "thought": decision.thought, "plan": plan, "actions": decision.actions.len(), "moves": decision.actions,
                    }));
                    pending = Some((decision, brief_frame));
                }
                Err(error) => log_line(&trajectory, json!({"t": 0.0, "brief": true, "unreadable": error})),
            }
        }
        Err(error) => log_line(&trajectory, json!({"t": 0.0, "brief": true, "error": error})),
    }
    let plan_text = if plan.is_empty() {
        String::new()
    } else {
        format!("\n\nYOUR PLAN (made before the clock):\n- {}", plan.join("\n- "))
    };

    // The clock: from here on, looking and deciding count as much as acting.
    let started = Instant::now();
    let deadline = started + Duration::from_secs(tier as u64);
    let mut reports: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let (mut decisions, mut executed, mut refused, mut failed_calls) = (0usize, 0usize, 0usize, 0usize);
    let mut last_actions = String::new();
    'run: while Instant::now() < deadline {
        let (decision, frame) = match pending.take() {
            Some(briefed) => briefed,
            None => {
                let window = match seat.look().await {
                    Ok(sheet) => sheet,
                    Err(error) => {
                        reports.push(format!("could not look: {error}"));
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                };
                let (seen, scale) = fit_width(&window, max_width);
                let frame = Frame { scale, width: window.width, height: window.height };
                let index = keep_frame(&seen);
                let left = deadline.saturating_duration_since(Instant::now());
                let mut text = context(&format!("Time left: {:.1} s of {tier} s.", left.as_secs_f32()), &seen);
                text.push_str(&plan_text);
                if !last_actions.is_empty() {
                    text.push_str(&format!("\n\nYOUR LAST ACTIONS AND WHAT HAPPENED:\n{last_actions}"));
                }
                // One decision may not take the whole run: a model that hangs is left for the next.
                let answer = match nebius.ask(&task.actors, SYSTEM, &text, &images_with(&seen), 3000, left.min(Duration::from_secs(25))).await {
                    Ok(answer) => answer,
                    Err(error) => {
                        failed_calls += 1;
                        reports.push(format!("no decision: {error}"));
                        log_line(&trajectory, json!({"t": started.elapsed().as_secs_f32(), "error": error}));
                        continue;
                    }
                };
                decisions += 1;
                let decision = match decision_of(&answer.text) {
                    Ok(decision) => decision,
                    Err(error) => {
                        last_actions = format!("(your answer could not be read: {error})");
                        log_line(&trajectory, json!({"t": started.elapsed().as_secs_f32(), "model": answer.model, "unreadable": answer.text.chars().take(400).collect::<String>()}));
                        continue;
                    }
                };
                log_line(&trajectory, json!({
                    "t": started.elapsed().as_secs_f32(), "model": answer.model, "model_seconds": answer.seconds,
                    "tokens": answer.completion_tokens, "thought": decision.thought, "actions": decision.actions.len(),
                    "frame": index, "moves": decision.actions,
                }));
                (decision, frame)
            }
        };
        last_actions.clear();
        for step in &decision.actions {
            if Instant::now() >= deadline {
                break 'run;
            }
            match step {
                Move::Done if task.ends_early => break 'run,
                Move::Done => {
                    last_actions.push_str("- done: noted, but the run goes on until the time is up
");
                }
                Move::Note { text } => notes.push(text.clone()),
                Move::Wait { ms } => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    tokio::time::sleep(Duration::from_millis(*ms).min(left)).await;
                }
                _ => {}
            }
            let actions = match stage_actions(step, frame) {
                Ok(actions) => actions,
                Err(error) => {
                    last_actions.push_str(&format!("- {step:?}: not done ({error})\n"));
                    continue;
                }
            };
            for action in actions {
                if Instant::now() >= deadline {
                    break 'run;
                }
                let kind = action.kind().as_str();
                let outcome = seat.act(action, &decision.thought, Duration::from_secs(20)).await;
                let said = match &outcome {
                    Ok(o) if o.executed() => {
                        executed += 1;
                        "executed".to_string()
                    }
                    Ok(o) => {
                        refused += 1;
                        format!("{o:?}")
                    }
                    Err(e) => {
                        refused += 1;
                        e.clone()
                    }
                };
                log_line(&trajectory, json!({"t": started.elapsed().as_secs_f32(), "act": kind, "outcome": said}));
                let short: String = format!("{step:?}").chars().take(80).collect();
                last_actions.push_str(&format!("- {short}: {said}\n"));
            }
        }
    }
    let spent = started.elapsed().as_secs_f32();

    // Judged on what the window shows when the time is up.
    let last = seat.look().await?;
    let mut result = json!({
        "task": task.id, "tier": tier, "run": stamp, "seconds_spent": spent, "decisions": decisions,
        "actions_executed": executed, "actions_refused": refused, "failed_model_calls": failed_calls,
        "notes": notes, "lessons_given": lessons, "plan": plan, "briefing_seconds": briefing_seconds,
    });
    let mut score = 0.0f32;
    if let (Some(reference), Some((first_page, workspace))) = (&reference, page) {
        let (x, y, w, h) = document_now(&last, first_page, workspace);
        result["document"] = json!([x, y, w, h]);
        let drawing = crop(&last, x, y, w, h);
        let target = bench::fitted(reference, drawing.width, drawing.height);
        let ssim = bench::ssim(&target, &drawing);
        let colour_error = bench::colour_error(&target, &drawing);
        // Measured as what was gained over the untouched page: a white page shares the light
        // parts of most pictures, and unnormalised it outscored a run that painted.
        let measure = |ssim: f32, colour_error: f32| 0.5 * ssim.max(0.0) + 0.5 * (1.0 - (colour_error / 50.0).min(1.0));
        let blank = Sheet::white(drawing.width, drawing.height);
        let untouched = measure(bench::ssim(&target, &blank), bench::colour_error(&target, &blank));
        let objective = ((measure(ssim, colour_error) - untouched) / (1.0 - untouched).max(1e-3)).clamp(0.0, 1.0);
        let critic_question = "You are an art critic. The FIRST image is a reference picture; the SECOND is a copy an \
            AI made of it by operating a Photoshop-like program with only a mouse and keyboard under a time limit. \
            Judge ONLY the second against the first. Reply with one line of JSON: {\"likeness\": 0-10, \"realism\": \
            0-10, \"critique\": \"at most two sentences on what most keeps it from the reference\"}. An empty or \
            untouched canvas is 0 and 0. Be strict and consistent.";
        let verdict = match nebius
            .ask(&task.critics, "You judge drawings.", critic_question, &[data_url(&target), data_url(&drawing)], 2048, Duration::from_secs(120))
            .await
        {
            Ok(answer) => {
                let text = answer.text;
                text.find('{')
                    .zip(text.rfind('}'))
                    .and_then(|(a, b)| serde_json::from_str::<serde_json::Value>(&text[a..=b]).ok())
                    .unwrap_or_else(|| json!({"error": "no verdict"}))
            }
            Err(error) => json!({"error": error}),
        };
        let critic = match (verdict["likeness"].as_f64(), verdict["realism"].as_f64()) {
            (Some(l), Some(r)) => Some(((l + r) / 20.0).clamp(0.0, 1.0) as f32),
            _ => None,
        };
        let quality = critic.map_or(objective, |c| 0.6 * c + 0.4 * objective);
        score = 100.0 * quality;
        std::fs::write(out.join("final.png"), drawing.png()).map_err(|e| e.to_string())?;
        let mut compare = Sheet::white(target.width * 2 + 8, target.height);
        for row in 0..target.height {
            for col in 0..target.width {
                compare.pixels[(row * compare.width + col) as usize] = target.pixels[(row * target.width + col) as usize];
                compare.pixels[(row * compare.width + target.width + 8 + col) as usize] = drawing.pixels[(row * drawing.width + col) as usize];
            }
        }
        std::fs::write(out.join("compare.png"), compare.png()).map_err(|e| e.to_string())?;
        result["ssim"] = json!(ssim);
        result["colour_error_delta_e"] = json!(colour_error);
        result["objective"] = json!(objective);
        result["critic"] = verdict;
    }
    result["score"] = json!((score * 10.0).round() / 10.0);
    result["best_before"] = json!(memory.best(&task.id, tier));

    // Reflection: the run, turned into lessons for the next.
    let log = std::fs::read_to_string(&trajectory).unwrap_or_default();
    let reflect = format!(
        "You are Archie reviewing your own run of a benchmark, to do better next time.\nTASK: {}\nTIME LIMIT: {tier} s\n\
         SCORE: {:.1}/100 (best before at this limit: {})\nCRITIC: {}\nYOUR NOTES: {}\nLESSONS YOU WERE GIVEN: {}\n\
         WHAT HAPPENED (one JSON line per decision or action):\n{}\n\nReply with one line of JSON: {{\"lessons\": \
         [\"at most 6 short, concrete lessons for the next run: which keys or clicks did what, what wasted time, what \
         to do first. Say where a control is by what it looks like and where it sits in the layout (\\\"the black \
         square at the bottom of the left toolbar\\\"), never by pixels alone: the window and the document change \
         size between runs, and a remembered coordinate once sent a click to empty space and the colour's hex \
         digits into Photopea as shortcuts (e = eraser). Say too what to check on screen before typing.\"]}}. Keep \
         the given lessons that held, correct the ones that were wrong.",
        task.prompt,
        score,
        memory.best(&task.id, tier).map_or("none".into(), |b| format!("{b:.1}")),
        result["critic"],
        notes.join(" | "),
        lessons.join(" | "),
        log.lines().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"),
    );
    let learned: Vec<String> = match nebius.ask(&task.critics, "You write lessons.", &reflect, &[data_url(&fit_width(&last, max_width).0)], 3000, Duration::from_secs(120)).await {
        Ok(answer) => answer
            .text
            .find('{')
            .zip(answer.text.rfind('}'))
            .and_then(|(a, b)| serde_json::from_str::<serde_json::Value>(&answer.text[a..=b]).ok())
            .and_then(|v| v["lessons"].as_array().cloned())
            .map(|items| items.iter().filter_map(|i| i.as_str().map(str::to_string)).take(6).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let mut kept = learned.clone();
    kept.extend(notes.iter().take(4).cloned());
    result["lessons_learned"] = json!(learned);
    memory.runs.push(Remembered { run: stamp.clone(), task: task.id.clone(), tier, score, lessons: kept });
    if let Some(path) = &memory_path {
        memory.save(path)?;
    }
    std::fs::write(out.join("result.json"), serde_json::to_string_pretty(&result).unwrap_or_default()).map_err(|e| e.to_string())?;
    log_line(&out_root.join("results.jsonl"), result.clone());
    println!("{result}");
    Ok(ExitCode::SUCCESS)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("run") {
        eprintln!("usage: archie-arena run --task <task.toml> --tier <seconds> --socket <control.sock> --key-file <key> --policy <apps.toml> --out <dir> [--memory <file>] [--reset <command>]");
        return ExitCode::from(2);
    }
    args.remove(0);
    match run(args).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("archie-arena: {error}");
            ExitCode::FAILURE
        }
    }
}
