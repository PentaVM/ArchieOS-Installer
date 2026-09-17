//! `archie-teach` — applications that declare their controls teach a local model.
//!
//! ```text
//! archie-teach --socket <path> --key-file <path> --policy <apps.toml> --app <app_id>
//!              --corpus <dir> --rounds <n> [--every-ms 1700]
//! ```
//!
//! Each round captures one window through the stage gate, asks the application where its
//! controls are, and keeps the pair: the window reduced to a 160x96 grey tile, and the
//! boxes, in the tile's own coordinates. A round in which the application declares nothing
//! teaches nothing and is dropped.
//!
//! What is kept is deliberately poor: no screenshot, no words, no application id, no time.
//! A tile is too small to read and large enough to see that something is a button, which
//! is the whole of what the model is being taught.

#[cfg(unix)]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    unix::main().await
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!("archie-teach needs a stage and an accessibility bus; run it on their host");
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use archie_a11y::Accessibility;
    use archie_policy::{PolicyEngine, Principal};
    use archie_reflex::{Box2, ControlKind, Corpus, Sample, Tile};
    use archie_sight::Sight;
    use archie_stage::{learn, CommandKey, Operator, StageClient, StagePolicy, StageWorld};
    use archie_types::AgentId;
    use std::path::Path;
    use std::process::ExitCode;
    use std::time::Duration;
    use tokio::net::UnixStream;

    fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
        let at = args.iter().position(|a| a == flag)?;
        if at + 1 >= args.len() {
            return None;
        }
        let value = args.remove(at + 1);
        args.remove(at);
        Some(value)
    }

    fn load_key(path: &str) -> Result<CommandKey, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let text = text.trim();
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{path}: a stage key is 64 hex characters"));
        }
        let mut bytes = [0_u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
                .map_err(|_| "not hex".to_string())?;
        }
        Ok(CommandKey::from_bytes(&bytes))
    }

    pub async fn main() -> ExitCode {
        match run(std::env::args().skip(1).collect()).await {
            Ok(code) => code,
            Err(message) => {
                eprintln!("archie-teach: {message}");
                ExitCode::FAILURE
            }
        }
    }

    async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
        let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
        let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
        let policy_path = take(&mut args, "--policy").ok_or("--policy is required")?;
        let app = take(&mut args, "--app").ok_or("--app is required")?;
        let corpus = Corpus::at(take(&mut args, "--corpus").ok_or("--corpus is required")?);
        let rounds: usize = take(&mut args, "--rounds")
            .ok_or("--rounds is required")?
            .parse()
            .map_err(|_| "--rounds is a number")?;
        let every = Duration::from_millis(
            take(&mut args, "--every-ms")
                .map(|value| value.parse::<u64>().unwrap_or(1_700))
                .unwrap_or(1_700),
        );
        if !args.is_empty() {
            return Err(format!("unexpected arguments: {args:?}"));
        }

        let bus = Accessibility::connect()
            .await
            .map_err(|error| format!("nothing to learn from: {error}"))?;
        let policy = StagePolicy::load(&policy_path).map_err(|e| e.to_string())?;
        let snapshot_dir = Path::new(&socket)
            .parent()
            .ok_or("--socket has no directory")?
            .join("snapshots");
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        let mut world = StageWorld::default();
        learn(&mut client, &mut world, Duration::from_millis(400))
            .await
            .map_err(|e| e.to_string())?;
        let operator = Operator::new(key, policy, PolicyEngine::default());
        let mut sight = Sight::new(
            operator,
            client,
            world,
            snapshot_dir,
            Principal::Agent {
                agent: AgentId::new(),
            },
        );

        let (mut kept, mut dropped) = (0_usize, 0_usize);
        for round in 0..rounds {
            if round > 0 {
                tokio::time::sleep(every).await;
            }
            let Some(surface) = sight
                .world()
                .surfaces_of(&app)
                .next()
                .map(|surface| (surface.surface, surface.pid, surface.width, surface.height))
            else {
                return Err(format!("no window of {app} is on the stage"));
            };
            let snapshot = match sight.look(&app, Some(surface.0)).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("archie-teach: round {round} was not captured: {error}");
                    dropped += 1;
                    continue;
                }
            };
            let controls = match bus
                .controls_of(surface.1, Some((surface.2, surface.3)))
                .await
            {
                Ok(controls) => controls,
                Err(error) => {
                    eprintln!("archie-teach: round {round} declared nothing: {error}");
                    dropped += 1;
                    continue;
                }
            };
            let tile = Tile::from_png(&snapshot.png).map_err(|e| e.to_string())?;
            let boxes: Vec<Box2> = controls
                .iter()
                .filter_map(|control| {
                    Box2::from_window(
                        ControlKind::of(control.kind),
                        control.bounds,
                        snapshot.width,
                        snapshot.height,
                    )
                })
                .collect();
            if boxes.is_empty() {
                dropped += 1;
                continue;
            }
            corpus
                .append(&Sample {
                    tile,
                    controls: boxes,
                })
                .map_err(|e| e.to_string())?;
            kept += 1;
            if kept % 25 == 0 {
                println!("{kept} windows taught");
            }
        }
        println!(
            "{kept} windows taught, {dropped} dropped; the corpus at {} now holds {}",
            corpus.path().display(),
            corpus.count()
        );
        Ok(ExitCode::SUCCESS)
    }
}
