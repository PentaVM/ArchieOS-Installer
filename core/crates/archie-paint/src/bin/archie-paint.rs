//! `archie-paint` — Archie draws a picture on the stage, one stroke at a time.
//!
//! ```text
//! archie-paint --socket <path> --key-file <path> --policy <apps.toml> --app <app_id>
//!              --image <file.png> [--surface N] [--inset 40] [--spacing 5]
//!              [--canvas x,y,w,h] [--limit N] [--levels 70:14,110:11,...]
//!              [--pen 2] [--points 2] [--find-paper]
//!              [--flow [--seeding 7] [--step 4] [--steps 120] [--darker-than 96] [--least-form 0.4] [--most N]
//!                      [--swatches "#141210,…"] [--palette x,y,step]]
//!              [--dry-run [--preview <file.png>] [--as-seen <px/s>] [--frames 60]]
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
        // The speed the stage will draw at, so the preview can show what will land rather
        // than what was asked for.
        let seen_at: Option<f32> = super::number(&mut args, "--as-seen")?;
        let frames: f32 = super::number(&mut args, "--frames")?.unwrap_or(60.0);
        let flowing = super::flag(&mut args, "--flow");
        let flow = super::flow_of(&mut args)?;
        let swatches = super::swatches_of(&mut args)?;
        let levels = super::levels_of(&mut args)?;
        let shares = super::shares_of(&mut args)?;
        // The canvas draws with a pen this wide, so the plan is made for one and the
        // preview is drawn with one.
        let pen: u32 = super::number(&mut args, "--pen")?.unwrap_or(2);
        let max_points: usize = super::number(&mut args, "--points")?.unwrap_or(2);
        let canvas = super::canvas_of(super::take(&mut args, "--canvas").as_deref(), None, inset)?;
        let soften: Option<u32> = super::number(&mut args, "--soften")?;
        let picture = super::prepared(Path::new(&image), soften)?;
        let asked = super::AsAsked {
            spacing,
            min_run,
            levels,
            shares: shares.clone(),
            pen,
            max_points,
        };
        if flowing {
            let mut groups = archie_paint::flow_plan(&picture, canvas, &flow, &swatches, max_points);
            // What the application will see of it, not what was planned: the pointer is a
            // place reported at a frame rate, and the drawing is made of those reports.
            if let Some(speed) = seen_at {
                let mut kept = 0_usize;
                for (_, plan) in &mut groups {
                    for stroke in &mut plan.strokes {
                        *stroke = archie_paint::seen_by(stroke, speed, frames);
                        kept += stroke.len();
                    }
                }
                println!(
                    "at {speed:.0} pixels a second the application sees {kept} positions of it, \n                     {frames:.0} a second"
                );
            }
            let strokes: usize = groups.iter().map(|(_, plan)| plan.strokes.len()).sum();
            let length: f32 = groups.iter().map(|(_, plan)| plan.drawn_length()).sum();
            println!(
                "{strokes} lines in {} colours, {length:.0} pixels of line, in a canvas of {}x{} at {},{}",
                groups.len(),
                canvas.width,
                canvas.height,
                canvas.x,
                canvas.y
            );
            if let Some(out) = preview {
                let paper = archie_paint::preview_in_colour(&groups, canvas, &swatches, pen);
                write_colour(Path::new(&out), &paper, canvas.width, canvas.height)?;
                println!("drew the plan on paper: {out}");
            }
            return Ok(ExitCode::SUCCESS);
        }
        let plan = super::plan_with(&picture, canvas, &asked);
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

    /// The preview of a picture drawn in several colours.
    fn write_colour(path: &Path, rgb: &[u8], width: u32, height: u32) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(rgb))
            .map_err(|e| e.to_string())
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
        max_points: usize,
    ) -> Hatching {
        Hatching {
            min_run,
            max_points,
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

/// The swatches a drawing application is asked to draw with, and where to click for each.
/// Finding them by looking is the accessibility path's job; here they are given.
#[cfg_attr(not(unix), allow(dead_code))]
struct Palette {
    colours: Vec<[u8; 3]>,
    at: Option<(i32, i32, i32)>,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl Palette {
    /// Where to click for swatch `at`, when the application's palette was described.
    fn swatch(&self, at: usize) -> Option<(i32, i32)> {
        let (x, y, step) = self.at?;
        Some((x + step * at as i32, y))
    }
}

/// `--swatches "#1b1a17,#6b4a2f,…"`, and the default the demo canvas has.
fn swatches_of(args: &mut Vec<String>) -> Result<Vec<[u8; 3]>, String> {
    let text = take(args, "--swatches").unwrap_or_else(|| {
        "#141210,#4a352a,#8a6a46,#c39c6d,#e6d3b3,#5d6b63".to_string()
    });
    let mut colours = Vec::new();
    for part in text.split(',') {
        let hex = part.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return Err("--swatches is colours like #1b1a17,#6b4a2f".into());
        }
        let mut colour = [0_u8; 3];
        for (at, pair) in hex.as_bytes().chunks(2).enumerate() {
            let pair = std::str::from_utf8(pair).map_err(|_| "a colour is hex")?;
            colour[at] = u8::from_str_radix(pair, 16).map_err(|_| "a colour is hex")?;
        }
        colours.push(colour);
    }
    if colours.is_empty() {
        return Err("--swatches needs at least one colour".into());
    }
    Ok(colours)
}

/// `--palette 40,1130,44`: the first swatch's middle, and how far apart they are.
#[cfg_attr(not(unix), allow(dead_code))]
fn palette_at(args: &mut Vec<String>) -> Result<Option<(i32, i32, i32)>, String> {
    let Some(text) = take(args, "--palette") else {
        return Ok(None);
    };
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 3 {
        return Err("--palette is x,y,step".into());
    }
    let mut numbers = [0_i32; 3];
    for (at, part) in parts.iter().enumerate() {
        numbers[at] = part
            .trim()
            .parse()
            .map_err(|_| "--palette is three numbers".to_string())?;
    }
    Ok(Some((numbers[0], numbers[1], numbers[2])))
}

/// How the lines that follow the form are to be drawn, as the command line says it.
fn flow_of(args: &mut Vec<String>) -> Result<archie_paint::Flow, String> {
    let mut flow = archie_paint::Flow::default();
    if let Some(seeding) = number(args, "--seeding")? {
        flow.seeding = seeding;
    }
    if let Some(step) = number::<f32>(args, "--step")? {
        flow.step = step;
    }
    if let Some(steps) = number(args, "--steps")? {
        flow.steps = steps;
    }
    if let Some(darker) = number(args, "--darker-than")? {
        flow.darker_than = darker;
    }
    if let Some(coherence) = number(args, "--coherence")? {
        flow.coherence = coherence;
    }
    if let Some(least) = number::<f32>(args, "--least-form")? {
        flow.least_form = least;
    }
    if let Some(most) = number(args, "--most")? {
        flow.most = most;
    }
    Ok(flow)
}

/// How a picture is to be hatched, as the command line says it.
struct AsAsked {
    spacing: u32,
    min_run: u32,
    levels: Option<Vec<(u8, u32)>>,
    shares: Vec<f32>,
    pen: u32,
    max_points: usize,
}

fn plan_with(picture: &Picture, canvas: Canvas, asked: &AsAsked) -> Plan {
    let hatching = match &asked.levels {
        Some(levels) => archie_paint::Hatching {
            min_run: asked.min_run,
            max_points: asked.max_points,
            ..archie_paint::Hatching::from_levels(levels)
        },
        None => planning::hatching(
            picture,
            asked.spacing,
            asked.min_run,
            &asked.shares,
            asked.pen,
            asked.max_points,
        ),
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

#[cfg_attr(not(unix), allow(dead_code))]
fn flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|arg| arg == name) {
        Some(at) => {
            args.remove(at);
            true
        }
        None => false,
    }
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
    use archie_paint::{Canvas, Picture};
    use archie_stage::{
        learn, ActOutcome, ActRequest, CommandKey, LookOutcome, Operator, PointerButton,
        StageAction, StageClient, StagePolicy, StageWorld,
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

    use super::{
        canvas_of, flag, flow_of, levels_of, number, palette_at, plan_with, prepared, shares_of,
        swatches_of, take, AsAsked, Palette,
    };

    /// One thing to do on the way to a picture.
    enum Deed {
        /// Pick up a colour, by clicking the swatch the application offers for it.
        Swatch(i32, i32),
        Line(Vec<archie_stage::StrokePoint>),
    }

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
        // Where to draw can be looked for rather than given: an application that can be
        // drawn in has paper in it, and paper is the large light rectangle in its window.
        let find_paper = flag(&mut args, "--find-paper");
        let flowing = flag(&mut args, "--flow");
        let flow = flow_of(&mut args)?;
        let palette = Palette {
            colours: swatches_of(&mut args)?,
            at: palette_at(&mut args)?,
        };
        let levels = levels_of(&mut args)?;
        let shares = shares_of(&mut args)?;
        let pen: u32 = number(&mut args, "--pen")?.unwrap_or(2);
        // How many points one command may carry. A line of many points is one command, and
        // a command is what costs; this is the whole difference between a slow picture and
        // a fast one.
        let max_points: usize = number(&mut args, "--points")?.unwrap_or(2);
        let soften: Option<u32> = number(&mut args, "--soften")?;
        if !args.is_empty() {
            return Err(format!("unexpected arguments: {args:?}"));
        }

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
        let mut operator = Operator::new(key, policy, PolicyEngine::default());
        let canvas = if find_paper {
            let paper = look_for_paper(
                &mut operator,
                &mut client,
                &mut world,
                &app,
                found.surface,
                &socket,
            )
            .await?;
            println!(
                "the paper is {}x{} at {},{} of that window",
                paper.width, paper.height, paper.x, paper.y
            );
            paper.inset(inset)
        } else {
            canvas_of(
                canvas_arg.as_deref(),
                Some((found.width, found.height)),
                inset,
            )?
        };
        // What to do, in order: a colour to pick up, or a line to draw with it.
        let mut deeds: Vec<Deed> = Vec::new();
        if flowing {
            let groups = archie_paint::flow_plan(&picture, canvas, &flow, &palette.colours, max_points);
            let lines: usize = groups.iter().map(|(_, plan)| plan.strokes.len()).sum();
            let length: f32 = groups.iter().map(|(_, plan)| plan.drawn_length()).sum();
            println!(
                "{lines} lines in {} colours, {length:.0} pixels of line, in a canvas of {}x{} at {},{}",
                groups.len(),
                canvas.width,
                canvas.height,
                canvas.x,
                canvas.y
            );
            for (colour, plan) in groups {
                if let Some((x, y)) = palette.swatch(colour) {
                    deeds.push(Deed::Swatch(x, y));
                }
                deeds.extend(plan.strokes.into_iter().map(Deed::Line));
            }
        } else {
            let plan = plan_with(
                &picture,
                canvas,
                &AsAsked {
                    spacing,
                    min_run,
                    levels,
                    shares,
                    pen,
                    max_points,
                },
            );
            super::planning::report(&plan, canvas);
            deeds.extend(plan.strokes.into_iter().map(Deed::Line));
        }
        let started = Instant::now();
        let (mut drawn, mut refused) = (0_usize, 0_usize);
        let mut paused_ms = 0_u64;
        let total = deeds.len().min(limit.unwrap_or(usize::MAX));
        for (index, deed) in deeds.iter().take(total).enumerate() {
            let action = match deed {
                Deed::Swatch(x, y) => StageAction::Click {
                    x: *x,
                    y: *y,
                    button: PointerButton::Left,
                },
                Deed::Line(points) => StageAction::Stroke {
                    points: points.clone(),
                    button: PointerButton::Left,
                },
            };
            let request = ActRequest {
                app_id: app.clone(),
                surface: Some(found.surface),
                action,
                text_grade: PrivacyGrade::Public,
                principal: Principal::Human {
                    id: "archie-paint".into(),
                    authenticated: true,
                },
                task_id: None,
                user_present: true,
                approved: false,
                justification: match deed {
                    Deed::Swatch(..) => "pick up a colour the application offers".into(),
                    Deed::Line(_) => "draw one stroke of a picture".to_string(),
                },
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



    /// Looks at the window once and takes the largest light rectangle in it for paper.
    ///
    /// The capture goes the way every capture goes: a read lease, the compositor's checks,
    /// a digest, and the file removed as it is read. Nothing of it is kept but a rectangle.
    async fn look_for_paper(
        operator: &mut Operator,
        client: &mut StageClient<
            tokio::net::unix::OwnedReadHalf,
            tokio::net::unix::OwnedWriteHalf,
        >,
        world: &mut StageWorld,
        app: &str,
        surface: u32,
        socket: &str,
    ) -> Result<Canvas, String> {
        let snapshots = Path::new(socket)
            .parent()
            .ok_or("--socket has no directory")?
            .join("snapshots");
        let request = ActRequest {
            app_id: app.to_string(),
            surface: Some(surface),
            action: StageAction::Snapshot { region: None },
            text_grade: PrivacyGrade::Public,
            principal: Principal::Human {
                id: "archie-paint".into(),
                authenticated: true,
            },
            task_id: None,
            user_present: true,
            approved: false,
            justification: "look for the paper to draw on".into(),
        };
        let outcome = operator
            .look(client, world, &request, &snapshots, Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;
        let snapshot = match outcome {
            LookOutcome::Captured(snapshot) => snapshot,
            LookOutcome::NotCaptured(outcome) => {
                return Err(format!("the window was not captured: {outcome:?}"))
            }
        };
        let picture = Picture::from_png(&snapshot.png).map_err(|e| e.to_string())?;
        let paper = archie_paint::paper_in(&picture, archie_paint::PAPER)
            .ok_or("there is no paper in that window to draw on")?;
        // The capture may be of one part of the window; the paper is in the window.
        Ok(Canvas::new(
            paper.x + snapshot.x,
            paper.y + snapshot.y,
            paper.width,
            paper.height,
        ))
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
