//! Scripted drawing drills whose outcomes are checked against Stage captures.

use crate::image::{page_in_editor, paper};
use crate::seat::{load_key, take, Seat};
use archie_paint::bench::Sheet;
use archie_policy::PolicyEngine;
use archie_stage::{
    learn, KeyChord, Operator, PointerButton, StageAction, StageClient, StagePolicy, StageWorld,
    StrokePoint,
};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;

/// A small deterministic generator so the same seed reproduces the same drill positions.
struct Generator(u64);

impl Generator {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
}

fn luma(pixel: [u8; 3]) -> f32 {
    0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32
}

/// Mean luminance reduction in a padded region around a horizontal or diagonal stroke.
fn darkening(before: &Sheet, after: &Sheet, line: [[i32; 2]; 2], pad: i32) -> f32 {
    if before.width != after.width || before.height != after.height {
        return 0.0;
    }
    let min_x = line[0][0].min(line[1][0]).saturating_sub(pad).max(0) as u32;
    let max_x =
        (line[0][0].max(line[1][0]).saturating_add(pad) as u32).min(before.width.saturating_sub(1));
    let min_y = line[0][1].min(line[1][1]).saturating_sub(pad).max(0) as u32;
    let max_y = (line[0][1].max(line[1][1]).saturating_add(pad) as u32)
        .min(before.height.saturating_sub(1));
    let mut delta = 0.0;
    let mut samples = 0usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let at = (y * before.width + x) as usize;
            delta += (luma(before.pixels[at]) - luma(after.pixels[at])).max(0.0);
            samples += 1;
        }
    }
    if samples == 0 {
        0.0
    } else {
        delta / samples as f32
    }
}

/// Width of the dark mark where a vertical probe crosses a stroke.
fn stroke_width(sheet: &Sheet, x: u32, y: u32, page_height: u32) -> u32 {
    if sheet.width == 0 || sheet.height == 0 {
        return 0;
    }
    let x = x.min(sheet.width - 1);
    let radius = (page_height / 12).max(8);
    let lo = y.saturating_sub(radius);
    let hi = y.saturating_add(radius).min(sheet.height - 1);
    let mut best = 0;
    let mut run = 0;
    for row in lo..=hi {
        if luma(sheet.pixels[(row * sheet.width + x) as usize]) < 175.0 {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn mean_page_delta(before: &Sheet, after: &Sheet, page: (u32, u32, u32, u32)) -> f32 {
    if before.width != after.width || before.height != after.height {
        return f32::INFINITY;
    }
    let (x, y, w, h) = page;
    let mut total = 0.0;
    let mut count = 0usize;
    for row in y..y.saturating_add(h).min(before.height) {
        for col in x..x.saturating_add(w).min(before.width) {
            let at = (row * before.width + col) as usize;
            for channel in 0..3 {
                total +=
                    (before.pixels[at][channel] as f32 - after.pixels[at][channel] as f32).abs();
                count += 1;
            }
        }
    }
    if count == 0 {
        f32::INFINITY
    } else {
        total / count as f32
    }
}

fn save_frame(dir: &Path, name: &str, sheet: &Sheet) -> Result<String, String> {
    let file = format!("{name}.png");
    std::fs::write(dir.join(&file), sheet.png()).map_err(|e| e.to_string())?;
    Ok(file)
}

fn line_action(line: [[i32; 2]; 2]) -> StageAction {
    StageAction::Strokes {
        paths: vec![line
            .iter()
            .map(|p| StrokePoint { x: p[0], y: p[1] })
            .collect()],
        button: PointerButton::Left,
    }
}

fn key_action(key: &str) -> Result<StageAction, String> {
    let chord = KeyChord::parse(key).ok_or_else(|| format!("unsupported practice key: {key}"))?;
    Ok(StageAction::Key { chord })
}

fn page_of(sheet: &Sheet) -> Result<(u32, u32, u32, u32), String> {
    page_in_editor(sheet)
        .map(|(page, _)| page)
        .or_else(|| paper(sheet))
        .ok_or("no white drawing page found; open a blank Photopea document first".into())
}

fn write_record(file: &mut std::fs::File, record: Value) -> Result<(), String> {
    writeln!(file, "{record}").map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn capture_step(
    seat: &mut Seat,
    out: &Path,
    log: &mut std::fs::File,
    id: usize,
    goal: &str,
    action: StageAction,
    move_description: Value,
    line: [[i32; 2]; 2],
) -> Result<(Sheet, u32), String> {
    let before = seat.look().await?;
    let page = page_of(&before)?;
    let prefix = format!("step-{id:03}");
    let before_file = save_frame(out, &format!("{prefix}-before"), &before)?;
    let outcome = seat.act(action, goal, Duration::from_secs(5)).await?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let after = seat.look().await?;
    let after_file = save_frame(out, &format!("{prefix}-after"), &after)?;
    let measured = darkening(&before, &after, line, 12);
    let verified = measured >= 1.0;
    // The same line as fractions of the page: what IRIS is to find on screen by itself.
    let fraction = |p: [i32; 2]| {
        [
            (p[0] - page.0 as i32) as f32 / page.2.max(1) as f32,
            (p[1] - page.1 as i32) as f32 / page.3.max(1) as f32,
        ]
    };
    write_record(
        log,
        json!({
            "step": id, "goal": goal, "move": move_description,
            "page_fraction": {"from": fraction(line[0]), "to": fraction(line[1])},
            "page": {"x":page.0,"y":page.1,"width":page.2,"height":page.3},
            "before": before_file, "after": after_file, "verified": verified,
            "measure": {"mean_luminance_drop": measured}, "stage_outcome": format!("{outcome:?}")
        }),
    )?;
    Ok((after, page.2))
}

async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
    let key_path = take(&mut args, "--key-file").ok_or("--key-file is required")?;
    let policy_path = take(&mut args, "--policy").ok_or("--policy is required")?;
    let out_root = PathBuf::from(take(&mut args, "--out").ok_or("--out is required")?);
    // Photopea runs in the session's Firefox, which is the window the Stage knows.
    let app_id = take(&mut args, "--app").unwrap_or_else(|| "firefox".into());
    let count: usize = take(&mut args, "--count")
        .map(|v| v.parse().unwrap_or(3))
        .unwrap_or(3)
        .clamp(1, 12);
    let seed: u64 = take(&mut args, "--seed")
        .map(|v| v.parse().unwrap_or(1))
        .unwrap_or(1);
    if !args.is_empty() {
        return Err(format!("unknown arguments: {}", args.join(" ")));
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let out = out_root.join(format!("practice-{stamp}"));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut log = std::fs::File::create(out.join("practice.jsonl")).map_err(|e| e.to_string())?;
    let policy = StagePolicy::load(&policy_path).map_err(|e| e.to_string())?;
    let key = load_key(&key_path)?;
    let stream = UnixStream::connect(&socket)
        .await
        .map_err(|e| format!("{socket}: {e}"))?;
    let (read, write) = stream.into_split();
    let mut seat = Seat {
        operator: Operator::new(key, policy, PolicyEngine::default()),
        client: StageClient::new(read, write),
        world: StageWorld::default(),
        snapshots: Path::new(&socket)
            .parent()
            .ok_or("--socket has no directory")?
            .join("snapshots"),
        app_id: app_id.clone(),
        principal_id: "archie-practice".into(),
    };

    let ready_by = Instant::now() + Duration::from_secs(180);
    let first = loop {
        learn(
            &mut seat.client,
            &mut seat.world,
            Duration::from_millis(500),
        )
        .await
        .map_err(|e| e.to_string())?;
        if seat.world.surfaces_of(&app_id).next().is_some() {
            if let Ok(sheet) = seat.look().await {
                if page_of(&sheet).is_ok() {
                    break sheet;
                }
            }
        }
        if Instant::now() >= ready_by {
            return Err(format!("{app_id} never showed a blank page"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let page = page_of(&first)?;
    write_record(
        &mut log,
        json!({
            "kind":"practice_session", "app":app_id, "seed":seed,
            "page":{"x":page.0,"y":page.1,"width":page.2,"height":page.3},
            "frames":"PNG before and after each checked move"
        }),
    )?;

    // Establish the real Photopea defaults; these setup keys are recorded but not graded.
    for setup_key in ["d", "b"] {
        let outcome = seat
            .act(
                key_action(setup_key)?,
                "select the default colour and brush",
                Duration::from_secs(5),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        write_record(
            &mut log,
            json!({"kind":"setup","key":setup_key,"stage_outcome":format!("{outcome:?}")}),
        )?;
    }

    let mut rng = Generator(seed.max(1));
    let (x, y, w, h) = page;
    let margin_x = (w / 5).max(2) as i32;
    let x0 = x as i32 + margin_x;
    let x1 = (x + w).saturating_sub(margin_x as u32) as i32;
    let usable_h = (h * 3 / 5).max(3);
    for index in 0..count {
        let fraction = 2 + rng.next() % usable_h.saturating_sub(1).max(1);
        let center_y = y as i32 + h as i32 / 5 + fraction as i32;
        let line = match index % 3 {
            0 => [[x0, center_y], [x1, center_y]],
            1 => [[x0, center_y + 5], [x1, center_y - 5]],
            _ => [[x0, center_y - 5], [x1, center_y + 5]],
        };
        let _ = capture_step(
            &mut seat,
            &out,
            &mut log,
            index + 1,
            "draw a black brush stroke on the page",
            line_action(line),
            json!({"type":"stroke","from":line[0],"to":line[1]}),
            line,
        )
        .await?;
    }

    // Compare a normal probe with one after four real brush-size key presses.
    let baseline_y = y as i32 + h as i32 * 3 / 4;
    let baseline = [[x0, baseline_y], [x1, baseline_y]];
    let (baseline_sheet, _) = capture_step(
        &mut seat,
        &out,
        &mut log,
        count + 1,
        "draw a baseline stroke before changing brush size",
        line_action(baseline),
        json!({"type":"brush_width_baseline","from":baseline[0],"to":baseline[1]}),
        baseline,
    )
    .await?;
    let baseline_width = stroke_width(
        &baseline_sheet,
        ((x0 + x1) / 2) as u32,
        baseline_y as u32,
        h,
    );
    for _ in 0..4 {
        seat.act(
            key_action("]")?,
            "increase brush size",
            Duration::from_secs(5),
        )
        .await?;
    }
    let wider_y = y as i32 + h as i32 * 9 / 10;
    let wider = [[x0, wider_y], [x1, wider_y]];
    let (wider_sheet, _) = capture_step(
        &mut seat,
        &out,
        &mut log,
        count + 2,
        "draw a second stroke after enlarging the brush",
        line_action(wider),
        json!({"type":"brush_width_after","key_presses":4,"from":wider[0],"to":wider[1]}),
        wider,
    )
    .await?;
    let wider_width = stroke_width(&wider_sheet, ((x0 + x1) / 2) as u32, wider_y as u32, h);
    write_record(
        &mut log,
        json!({"kind":"brush_width_check","baseline_px":baseline_width,"after_px":wider_width,"verified":wider_width > baseline_width,"measure":{"delta_px":wider_width as i32 - baseline_width as i32}}),
    )?;

    // Undo is graded only if the page returns close to its own pre-stroke capture.
    let undo_before = seat.look().await?;
    let undo_page = page_of(&undo_before)?;
    let undo_y = undo_page.1 as i32 + undo_page.3 as i32 / 2;
    let undo_line = [
        [undo_page.0 as i32 + undo_page.2 as i32 / 4, undo_y],
        [undo_page.0 as i32 + undo_page.2 as i32 * 3 / 4, undo_y],
    ];
    let before_file = save_frame(&out, "undo-before", &undo_before)?;
    seat.act(
        line_action(undo_line),
        "draw a reversible test stroke",
        Duration::from_secs(5),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let painted = seat.look().await?;
    let painted_file = save_frame(&out, "undo-painted", &painted)?;
    seat.act(
        key_action("ctrl+z")?,
        "undo the test stroke",
        Duration::from_secs(5),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let undone = seat.look().await?;
    let undone_file = save_frame(&out, "undo-after", &undone)?;
    let delta = mean_page_delta(&undo_before, &undone, undo_page);
    write_record(
        &mut log,
        json!({"step":count+3,"goal":"undo restores the page","move":[{"type":"stroke","from":undo_line[0],"to":undo_line[1]},{"type":"key","keys":"ctrl+z"}],"page":{"x":undo_page.0,"y":undo_page.1,"width":undo_page.2,"height":undo_page.3},"before":before_file,"after_stroke":painted_file,"after":undone_file,"verified":delta <= 2.0,"measure":{"mean_absolute_rgb_delta":delta}}),
    )?;

    println!("practice records: {}", out.display());
    Ok(ExitCode::SUCCESS)
}

pub fn main_entry() -> ExitCode {
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => match runtime.block_on(run(std::env::args().skip(1).collect())) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("archie-practice: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("archie-practice: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{darkening, mean_page_delta, stroke_width};
    use archie_paint::bench::Sheet;

    #[test]
    fn sees_a_dark_stroke_and_measures_its_width() {
        let before = Sheet::white(40, 30);
        let mut after = before.clone();
        for y in 14..=16 {
            for x in 5..=34 {
                after.pixels[(y * 40 + x) as usize] = [0, 0, 0];
            }
        }
        assert!(darkening(&before, &after, [[5, 15], [34, 15]], 3) > 20.0);
        assert_eq!(stroke_width(&after, 20, 15, 30), 3);
    }

    #[test]
    fn undo_comparison_is_zero_for_the_restored_page() {
        let page = Sheet::white(20, 20);
        assert_eq!(mean_page_delta(&page, &page, (2, 2, 16, 16)), 0.0);
    }
}
