//! `archie-stage-cli` — drive the Archie Stage by hand, through the same gate Cerebra uses.
//!
//! A smoke path, like `archie-infer` and `archie-search`. Every action is checked against
//! the stage policy and a one-operation lease for the operator running the command, sealed
//! with the compositor's key, and sent over its control socket; the compositor re-checks it
//! and answers with a receipt.
//!
//! ```text
//! archie-stage-cli keygen <key-file>
//! archie-stage-cli watch  --socket <path>
//! archie-stage-cli act    --socket <path> --key-file <path> --app <app_id> <action...>
//!     click <x> <y> | double-click <x> <y> | move <x> <y> | drag <x> <y> <to_x> <to_y>
//!     scroll <x> <y> <steps_y> | type <text> | key [ctrl+][shift+][alt+]<key> | focus
//! archie-stage-cli look   --socket <path> --key-file <path> --app <app_id> --out <file.png>
//! ```
//!
//! Coordinates are surface-local. `--surface` picks a surface when an application has more
//! than one; otherwise the first surface the compositor reports for `--app` is used.
//! `--approve` stands for the person at the keyboard approving an action the policy holds.

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    unix::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "archie-stage-cli talks to the compositor over a Unix socket; run it on the stage's host"
    );
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use archie_policy::{PolicyEngine, Principal};
    use archie_stage::{
        learn, ActOutcome, ActRequest, CommandKey, KeyChord, Operator, PointerButton, StageAction,
        StageClient, StagePolicy, StageText, StageWorld,
    };
    use archie_types::PrivacyGrade;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::ExitCode;
    use std::time::Duration;
    use tokio::net::UnixStream;

    const DEFAULT_POLICY: &str = "/etc/archie/stage/apps.toml";

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(e) => {
                eprintln!("archie-stage-cli: {e}");
                return ExitCode::FAILURE;
            }
        };
        match runtime.block_on(run(args)) {
            Ok(code) => code,
            Err(message) => {
                eprintln!("archie-stage-cli: {message}");
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

    fn flag(args: &mut Vec<String>, flag: &str) -> bool {
        match args.iter().position(|a| a == flag) {
            Some(at) => {
                args.remove(at);
                true
            }
            None => false,
        }
    }

    async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
        let command = if args.is_empty() {
            String::new()
        } else {
            args.remove(0)
        };
        match command.as_str() {
            "keygen" => keygen(args),
            "watch" => watch(args).await,
            "act" => act(args).await,
            "look" => look(args).await,
            _ => Err("usage: archie-stage-cli keygen|watch|act|look …".into()),
        }
    }

    /// Captures a surface to a file only its owner can read.
    async fn look(mut args: Vec<String>) -> Result<ExitCode, String> {
        let socket = take(&mut args, "--socket").ok_or("look needs --socket")?;
        let key = load_key(&take(&mut args, "--key-file").ok_or("look needs --key-file")?)?;
        let app = take(&mut args, "--app").ok_or("look needs --app")?;
        let out = take(&mut args, "--out").ok_or("look needs --out")?;
        let surface = take(&mut args, "--surface")
            .map(|s| s.parse::<u32>())
            .transpose()
            .map_err(|_| "--surface is a number")?;
        let policy_path = take(&mut args, "--policy").unwrap_or_else(|| DEFAULT_POLICY.into());
        let policy = StagePolicy::load(&policy_path).map_err(|e| e.to_string())?;
        let snapshot_dir = std::path::Path::new(&socket)
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
        let mut operator = Operator::new(key, policy, PolicyEngine::default());
        let request = ActRequest {
            app_id: app.clone(),
            surface,
            action: StageAction::Snapshot,
            text_grade: PrivacyGrade::Public,
            principal: Principal::Human {
                id: "stage-cli".into(),
                authenticated: true,
            },
            task_id: None,
            user_present: true,
            approved: false,
            justification: "operator capture from archie-stage-cli".into(),
        };
        let outcome = operator
            .look(
                &mut client,
                &mut world,
                &request,
                &snapshot_dir,
                Duration::from_secs(30),
            )
            .await
            .map_err(|e| e.to_string())?;
        match outcome {
            archie_stage::LookOutcome::Captured(snapshot) => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&out)
                    .map_err(|e| format!("{out}: {e}"))?;
                file.write_all(&snapshot.png).map_err(|e| e.to_string())?;
                println!(
                    "captured surface {} of {app}: {}x{}, sha256 {}",
                    snapshot.surface, snapshot.width, snapshot.height, snapshot.sha256
                );
                Ok(ExitCode::SUCCESS)
            }
            archie_stage::LookOutcome::NotCaptured(outcome) => {
                eprintln!("not captured: {outcome:?}");
                Ok(ExitCode::from(4))
            }
        }
    }

    fn keygen(args: Vec<String>) -> Result<ExitCode, String> {
        let path = args.first().ok_or("keygen needs a path")?;
        let (_key, bytes) = CommandKey::generate().map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("{path}: {e}"))?;
        file.write_all(hex::encode(bytes.as_ref()).as_bytes())
            .map_err(|e| e.to_string())?;
        println!("wrote a new stage key to {path} (mode 0600)");
        Ok(ExitCode::SUCCESS)
    }

    fn load_key(path: &str) -> Result<CommandKey, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let bytes: [u8; 32] = hex::decode(text.trim())
            .map_err(|_| format!("{path}: not a hex key"))?
            .try_into()
            .map_err(|_| format!("{path}: a stage key is 32 bytes"))?;
        Ok(CommandKey::from_bytes(&bytes))
    }

    async fn watch(mut args: Vec<String>) -> Result<ExitCode, String> {
        let socket = take(&mut args, "--socket").ok_or("watch needs --socket")?;
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        while let Some(event) = client.next_event().await.map_err(|e| e.to_string())? {
            println!(
                "{}",
                serde_json::to_string(&event).map_err(|e| e.to_string())?
            );
        }
        Ok(ExitCode::SUCCESS)
    }

    fn parse_action(words: &[String]) -> Result<(StageAction, PrivacyGrade), String> {
        let int = |i: usize| -> Result<i32, String> {
            words
                .get(i)
                .ok_or("missing a number")?
                .parse::<i32>()
                .map_err(|_| format!("{:?} is not a number", words[i]))
        };
        let verb = words.first().ok_or("act needs an action")?;
        let action = match verb.as_str() {
            "click" => StageAction::Click {
                x: int(1)?,
                y: int(2)?,
                button: PointerButton::Left,
            },
            "right-click" => StageAction::Click {
                x: int(1)?,
                y: int(2)?,
                button: PointerButton::Right,
            },
            "double-click" => StageAction::DoubleClick {
                x: int(1)?,
                y: int(2)?,
                button: PointerButton::Left,
            },
            "move" => StageAction::Move {
                x: int(1)?,
                y: int(2)?,
            },
            "drag" => StageAction::Drag {
                x: int(1)?,
                y: int(2)?,
                to_x: int(3)?,
                to_y: int(4)?,
                button: PointerButton::Left,
            },
            "scroll" => StageAction::Scroll {
                x: int(1)?,
                y: int(2)?,
                steps_x: 0,
                steps_y: int(3)?,
            },
            "type" => StageAction::Text {
                text: StageText::new(words[1..].join(" ")),
            },
            "key" => {
                let spec = words.get(1).ok_or("key needs a key")?;
                let chord = KeyChord::parse(spec)
                    .ok_or("not a key this stage can press, like enter or ctrl+s")?;
                StageAction::Key { chord }
            }
            "focus" => StageAction::Focus,
            other => return Err(format!("{other:?} is not a stage action")),
        };
        Ok((action, PrivacyGrade::Internal))
    }

    async fn act(mut args: Vec<String>) -> Result<ExitCode, String> {
        let socket = take(&mut args, "--socket").ok_or("act needs --socket")?;
        let key = load_key(&take(&mut args, "--key-file").ok_or("act needs --key-file")?)?;
        let app = take(&mut args, "--app").ok_or("act needs --app")?;
        let surface = take(&mut args, "--surface")
            .map(|s| s.parse::<u32>())
            .transpose()
            .map_err(|_| "--surface is a number")?;
        let policy_path = take(&mut args, "--policy").unwrap_or_else(|| DEFAULT_POLICY.into());
        let sequence = take(&mut args, "--sequence")
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| "--sequence is a number")?;
        let approved = flag(&mut args, "--approve");
        let (action, text_grade) = parse_action(&args)?;
        let policy = StagePolicy::load(&policy_path).map_err(|e| e.to_string())?;

        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        let mut world = StageWorld::default();
        learn(&mut client, &mut world, Duration::from_millis(400))
            .await
            .map_err(|e| e.to_string())?;

        let mut operator = Operator::new(key, policy, PolicyEngine::default());
        if let Some(sequence) = sequence {
            operator.advance_sequence(sequence);
        }
        // The operator running this command is the principal, at the keyboard.
        let request = ActRequest {
            app_id: app.clone(),
            surface,
            action,
            text_grade,
            principal: Principal::Human {
                id: "stage-cli".into(),
                authenticated: true,
            },
            task_id: None,
            user_present: true,
            approved,
            justification: "operator action from archie-stage-cli".into(),
        };
        println!("sending {} to {app}", request.action.kind().as_str());
        let outcome = operator
            .act(&mut client, &mut world, &request, Duration::from_secs(30))
            .await
            .map_err(|e| e.to_string())?;
        Ok(match outcome {
            ActOutcome::Receipt {
                sequence: 0,
                outcome,
            } => {
                println!("receipt: {outcome} (the compositor could not verify the command)");
                ExitCode::from(4)
            }
            ActOutcome::Receipt { outcome, .. } => {
                println!("receipt: {outcome}");
                if outcome == "executed" {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(4)
                }
            }
            ActOutcome::NeedsApproval { reason } => {
                eprintln!("needs approval ({reason}); rerun with --approve if you are the person approving it");
                ExitCode::from(3)
            }
            ActOutcome::Refused { code, message } => {
                eprintln!("refused ({code}): {message}");
                ExitCode::from(2)
            }
            ActOutcome::NoSurface => {
                eprintln!("no surface of {app} is on the stage");
                ExitCode::from(2)
            }
        })
    }
}
