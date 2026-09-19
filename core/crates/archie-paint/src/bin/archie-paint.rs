//! `archie-paint` — Archie draws a picture on the stage, one stroke at a time.
//!
//! ```text
//! archie-paint --socket <path> --key-file <path> --policy <apps.toml> --app <app_id>
//!              --image <file.png> [--surface N] [--inset 40] [--spacing 5]
//!              [--canvas x,y,w,h] [--limit N] [--levels 70:14,110:11,...]
//!              [--pen 2] [--dry-run [--preview <file.png>]]
//! ```
//!
//! Every stroke goes the same way as every other action: the stage policy has to allow
//! `stroke` for that application, a lease has to authorise the one operation, the command
//! is sealed, and the compositor re-checks each point against the window as it is now. A
//! picture is nothing special here; it is a few hundred ordinary commands in a row.
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A picture can be planned anywhere. Drawing one needs a stage, and a stage is a Unix
    // socket on the machine it runs on.
    if args.iter().any(|arg| arg == "--dry-run") {
        return match planning::main(args) {
            Ok(code) => code,
            Err(message) => {
                eprintln!("archie-paint: {message}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    #[cfg(unix)]
    {
        unix::main(args).await
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!(
            "archie-paint talks to the compositor over a Unix socket; run it on the stage's host, \
             or pass --dry-run to plan a picture here"
        );
        std::process::ExitCode::FAILURE
    }
}

/// Everything about a picture that does not need a stage: what to draw, and where.
mod planning {
    use archie_paint::{Canvas, Hatching, Picture, Plan};
    use std::path::Path;
    use std::process::ExitCode;

    pub fn main(mut args: Vec<String>) -> Result<ExitCode, String> {
        let image = super::take(&mut args, "--image").ok_or("--image is required")?;
        let inset: u32 = super::number(&mut args, "--inset")?.unwrap_or(40);
        let spacing: u32 = super::number(&mut args, "--spacing")?.unwrap_or(10);
        let min_run: u32 = super::number(&mut args, "--min-run")?.unwrap_or(4);
        let preview = super::take(&mut args, "--preview");
        let levels = super::levels_of(&mut args)?;
        let shares = super::shares_of(&mut args)?;
        // The canvas draws with a pen this wide, so the plan is made for one and the
        // preview is drawn with one.
        let pen: u32 = super::number(&mut args, "--pen")?.unwrap_or(2);
        let canvas = super::canvas_of(super::take(&mut args, "--canvas").as_deref(), None, inset)?;
        let soften: Option<u32> = super::number(&mut args, "--soften")?;
        let picture = super::prepared(Path::new(&image), soften)?;
        let plan = super::plan_with(&picture, canvas, spacing, min_run, levels, &shares, pen);
        report(&plan, canvas);
        println!(
            "tone levels from the picture itself: {:?}",
            levels_of(&picture, &shares, pen)
        );
        if let Some(out) = preview {
            let paper = archie_paint::preview(&plan, canvas, pen);
            write_grey(Path::new(&out), &paper, canvas.width, canvas.height)?;
            println!("drew the plan on paper: {out}");
        }
        Ok(ExitCode::SUCCESS)
    }

    /// The preview, as a plain grey PNG.
    fn write_grey(path: &Path, grey: &[u8], width: u32, height: u32) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(grey))
            .map_err(|e| e.to_string())
    }

    pub fn report(plan: &Plan, canvas: Canvas) {
        println!(
            "{} strokes, {:.0} pixels of line, in a canvas of {}x{} at {},{}",
            plan.strokes.len(),
            plan.drawn_length(),
            canvas.width,
            canvas.height,
            canvas.x,
            canvas.y
        );
    }

    /// How much of the picture each pass of the pen covers, and how far apart its lines
    /// are: the darkest half faintly, then less and less of it more and more closely.
    pub const SHARES: [f32; 6] = [0.95, 0.82, 0.62, 0.42, 0.24, 0.09];
    /// How far apart the lightest pass's lines are; a pass that covers less of the picture
    /// is drawn closer together, so that darker really does come out darker.
    pub const SPACING: u32 = 17;

    /// The tone levels this picture's own darkness puts them at.
    ///
    /// The pen has a width, and lines closer together than about three times it leave no
    /// paper between them: what was meant as a dark grey comes out solid black. That floor
    /// is why the planned picture and the drawn one now look like each other.
    pub fn levels_of(picture: &Picture, shares: &[f32], pen: u32) -> Vec<(u8, u32)> {
        let closest = (pen.max(1) * 3).max(3);
        picture
            .darkness_at(shares)
            .into_iter()
            .zip(shares.iter())
            .map(|(darkness, share)| {
                (
                    darkness,
                    ((SPACING as f32 * share.clamp(0.0, 1.0)).round() as u32).max(closest),
                )
            })
            .collect()
    }

    pub fn hatching(
        picture: &Picture,
        spacing: u32,
        min_run: u32,
        shares: &[f32],
        pen: u32,
    ) -> Hatching {
        Hatching {
            min_run,
            ..Hatching::from_levels(&levels_of(picture, shares, pen))
        }
        .spread(spacing)
    }
}

use archie_paint::{Canvas, Picture, Plan};

/// The picture as the pen sees it: its tones pulled apart, and its grain taken out so that
/// what the pen follows is tone rather than noise.
fn prepared(path: &std::path::Path, soften: Option<u32>) -> Result<Picture, String> {
    let picture = Picture::read(path).map_err(|e| e.to_string())?;
    let radius = soften.unwrap_or_else(|| (picture.width() / 200).max(1));
    Ok(picture.softened(radius).stretched())
}

fn plan_with(
    picture: &Picture,
    canvas: Canvas,
    spacing: u32,
    min_run: u32,
    levels: Option<Vec<(u8, u32)>>,
    shares: &[f32],
    pen: u32,
) -> Plan {
    let hatching = match levels {
        Some(levels) => archie_paint::Hatching {
            min_run,
            ..archie_paint::Hatching::from_levels(&levels)
        },
        None => planning::hatching(picture, spacing, min_run, shares, pen),
    };
    archie_paint::plan(picture, canvas, &hatching)
}

/// `--shares 0.85,0.65,0.45`: how much of the picture each pass of the pen covers, darkest
/// first. Without it, what `planning::SHARES` says.
fn shares_of(args: &mut Vec<String>) -> Result<Vec<f32>, String> {
    let Some(text) = take(args, "--shares") else {
        return Ok(planning::SHARES.to_vec());
    };
    let mut shares = Vec::new();
    for part in text.split(',') {
        shares.push(
            part.trim()
                .parse::<f32>()
                .map_err(|_| "--shares is numbers from 0 to 1".to_string())?,
        );
    }
    if shares.is_empty() {
        return Err("--shares needs at least one pass".into());
    }
    Ok(shares)
}

/// `--levels 70:14,110:11,150:8`: each pass of the pen, as how dark the picture has to be
/// before it touches, and how far apart its lines are.
fn levels_of(args: &mut Vec<String>) -> Result<Option<Vec<(u8, u32)>>, String> {
    let Some(text) = take(args, "--levels") else {
        return Ok(None);
    };
    let mut levels = Vec::new();
    for part in text.split(',') {
        let (darkness, spacing) = part
            .trim()
            .split_once(':')
            .ok_or("--levels is pairs like 70:14,110:11")?;
        levels.push((
            darkness
                .trim()
                .parse()
                .map_err(|_| "a level's darkness is 0 to 255".to_string())?,
            spacing
                .trim()
                .parse()
                .map_err(|_| "a level's spacing is a number of pixels".to_string())?,
        ));
    }
    if levels.is_empty() {
        return Err("--levels needs at least one pass".into());
    }
    Ok(Some(levels))
}

fn take(args: &mut Vec<String>, name: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == name)?;
    if at + 1 >= args.len() {
        return None;
    }
    args.remove(at);
    Some(args.remove(at))
}

fn number<T: std::str::FromStr>(args: &mut Vec<String>, name: &str) -> Result<Option<T>, String> {
    match take(args, name) {
        Some(value) => Ok(Some(
            value.parse().map_err(|_| format!("{name} is a number"))?,
        )),
        None => Ok(None),
    }
}

/// Where to draw: what was asked for, or the window with a margin taken off it.
fn canvas_of(asked: Option<&str>, surface: Option<(i32, i32)>, inset: u32) -> Result<Canvas, String> {
    if let Some(text) = asked {
        let parts: Vec<&str> = text.split(',').collect();
        if parts.len() != 4 {
            return Err("--canvas is x,y,width,height".into());
        }
        let number = |at: usize| -> Result<i64, String> {
            parts[at]
                .trim()
                .parse()
                .map_err(|_| "--canvas is x,y,width,height".to_string())
        };
        let (width, height) = (number(2)?, number(3)?);
        if width <= 0 || height <= 0 {
            return Err("a canvas with no room in it is not a canvas".into());
        }
        return Ok(Canvas::new(
            number(0)? as i32,
            number(1)? as i32,
            width as u32,
            height as u32,
        ));
    }
    let (width, height) = surface.ok_or("without a stage, --canvas says where to draw")?;
    if width <= 0 || height <= 0 {
        return Err("that window has no room to draw in".into());
    }
    Ok(Canvas::new(0, 0, width as u32, height as u32).inset(inset))
}

#[cfg(unix)]
mod unix {

    use archie_policy::{PolicyEngine, Principal};
    use archie_stage::{
        learn, ActOutcome, ActRequest, CommandKey, Operator, PointerButton, StageAction,
        StageClient, StagePolicy, StageWorld,
    };
    use archie_types::PrivacyGrade;

    /// How long a stroke waits for a person who has taken the stage before giving up on it.
    const PATIENCE_MS: u64 = 120_000;
    use std::path::Path;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};
    use tokio::net::UnixStream;

    pub async fn main(args: Vec<String>) -> ExitCode {
        match run(args).await {
            Ok(code) => code,
            Err(message) => {
                eprintln!("archie-paint: {message}");
                ExitCode::FAILURE
            }
        }
    }

    use super::{canvas_of, levels_of, number, plan_with, prepared, shares_of, take};

    async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
        let socket = take(&mut args, "--socket");
        let key_file = take(&mut args, "--key-file");
        let policy_path = take(&mut args, "--policy");
        let app = take(&mut args, "--app");
        let image = take(&mut args, "--image").ok_or("--image is required")?;
        let surface: Option<u32> = number(&mut args, "--surface")?;
        let inset: u32 = number(&mut args, "--inset")?.unwrap_or(40);
        let spacing: u32 = number(&mut args, "--spacing")?.unwrap_or(10);
        let min_run: u32 = number(&mut args, "--min-run")?.unwrap_or(4);
        let limit: Option<usize> = number(&mut args, "--limit")?;
        let canvas_arg = take(&mut args, "--canvas");
        let levels = levels_of(&mut args)?;
        let shares = shares_of(&mut args)?;
        let pen: u32 = number(&mut args, "--pen")?.unwrap_or(2);
        if !args.is_empty() {
            return Err(format!("unexpected arguments: {args:?}"));
        }

        let soften: Option<u32> = number(&mut args, "--soften")?;
        let picture = prepared(Path::new(&image), soften)?;
        let socket = socket.ok_or("--socket is required")?;
        let app = app.ok_or("--app is required")?;
        let key = load_key(&key_file.ok_or("--key-file is required")?)?;
        let policy =
            StagePolicy::load(&policy_path.ok_or("--policy is required")?).map_err(|e| e.to_string())?;
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        let mut world = StageWorld::default();
        learn(&mut client, &mut world, Duration::from_millis(400))
            .await
            .map_err(|e| e.to_string())?;
        let found = match surface {
            Some(id) => world.surface(id).filter(|s| s.app_id == app).cloned(),
            None => world.surfaces_of(&app).next().cloned(),
        };
        let found = found.ok_or_else(|| format!("{app} has no surface on this stage"))?;
        let canvas = canvas_of(
            canvas_arg.as_deref(),
            Some((found.width, found.height)),
            inset,
        )?;
        let plan = plan_with(&picture, canvas, spacing, min_run, levels, &shares, pen);
        super::planning::report(&plan, canvas);

        let mut operator = Operator::new(key, policy, PolicyEngine::default());
        let started = Instant::now();
        let (mut drawn, mut refused) = (0_usize, 0_usize);
        let mut paused_ms = 0_u64;
        let total = plan.strokes.len().min(limit.unwrap_or(usize::MAX));
        for (index, stroke) in plan.strokes.iter().take(total).enumerate() {
            let request = ActRequest {
                app_id: app.clone(),
                surface: Some(found.surface),
                action: StageAction::Stroke {
                    points: stroke.clone(),
                    button: PointerButton::Left,
                },
                text_grade: PrivacyGrade::Public,
                principal: Principal::Human {
                    id: "archie-paint".into(),
                    authenticated: true,
                },
                task_id: None,
                user_present: true,
                approved: false,
                justification: "draw one stroke of a picture".into(),
            };
            // A person reaching for the mouse takes the stage, and a picture is not a
            // reason to argue with that: the stroke waits until they are done rather than
            // being spent against a stage that is refusing it. The compositor gives the
            // agent the stage back after its own quiet period.
            let mut waited_ms = 0_u64;
            let outcome = loop {
                let outcome = operator
                    .act(&mut client, &mut world, &request, Duration::from_secs(30))
                    .await
                    .map_err(|e| e.to_string())?;
                let held = matches!(
                    &outcome,
                    ActOutcome::Receipt { outcome, .. }
                        if outcome == "operator_has_control" || outcome == "preempted_by_operator"
                );
                if !held || waited_ms >= PATIENCE_MS {
                    break outcome;
                }
                if waited_ms == 0 {
                    println!("a person has the stage; waiting for them");
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                waited_ms += 500;
                paused_ms += 500;
            };
            match &outcome {
                ActOutcome::Receipt { outcome, .. } if outcome == "executed" => {
                    drawn += 1;
                    if waited_ms > 0 {
                        println!("back to it after {:.1}s", waited_ms as f32 / 1000.0);
                    }
                }
                other => {
                    refused += 1;
                    if refused <= 3 {
                        eprintln!("archie-paint: stroke {index} was not drawn: {other:?}");
                    }
                }
            }
            if index % 25 == 24 {
                println!(
                    "{} of {total} strokes, {:.1}s",
                    index + 1,
                    started.elapsed().as_secs_f32()
                );
            }
        }
        let seconds = started.elapsed().as_secs_f32();
        let drawing = seconds - paused_ms as f32 / 1000.0;
        println!(
            "drew {drawn} strokes and had {refused} refused in {seconds:.1}s, of which \
             {:.1}s was waiting for a person ({:.1} strokes a second while drawing)",
            paused_ms as f32 / 1000.0,
            drawn as f32 / drawing.max(0.001)
        );
        Ok(if refused == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(4)
        })
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
}
