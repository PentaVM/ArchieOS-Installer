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
    eprintln!("archie-stage-cli talks to the compositor over a Unix socket; run it on the stage's host");
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
    use archie_stage::{
        stage_resource, CommandKey, Key, KeyChord, Modifier, PointerButton, Sealer, StageAction, StageAdmission,
        StageClient, StageEvent, StageGate, StagePolicy, StageProposal, StageText, StageWorld,
    };
    use archie_types::{PrivacyGrade, Timestamp, TraceId};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::ExitCode;
    use std::time::Duration;
    use tokio::net::UnixStream;

    const DEFAULT_POLICY: &str = "/etc/archie/stage/apps.toml";

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
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
        let command = if args.is_empty() { String::new() } else { args.remove(0) };
        match command.as_str() {
            "keygen" => keygen(args),
            "watch" => watch(args).await,
            "act" => act(args).await,
            _ => Err("usage: archie-stage-cli keygen|watch|act …".into()),
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
        let stream = UnixStream::connect(&socket).await.map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        while let Some(event) = client.next_event().await.map_err(|e| e.to_string())? {
            println!("{}", serde_json::to_string(&event).map_err(|e| e.to_string())?);
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
            "click" => StageAction::Click { x: int(1)?, y: int(2)?, button: PointerButton::Left },
            "right-click" => StageAction::Click { x: int(1)?, y: int(2)?, button: PointerButton::Right },
            "double-click" => StageAction::DoubleClick { x: int(1)?, y: int(2)?, button: PointerButton::Left },
            "move" => StageAction::Move { x: int(1)?, y: int(2)? },
            "drag" => StageAction::Drag {
                x: int(1)?,
                y: int(2)?,
                to_x: int(3)?,
                to_y: int(4)?,
                button: PointerButton::Left,
            },
            "scroll" => StageAction::Scroll { x: int(1)?, y: int(2)?, steps_x: 0, steps_y: int(3)? },
            "type" => StageAction::Text {
                text: StageText::new(words[1..].join(" ")),
            },
            "key" => {
                let spec = words.get(1).ok_or("key needs a key")?;
                let mut parts: Vec<&str> = spec.split('+').collect();
                let key = Key::parse(parts.pop().unwrap_or_default()).ok_or("not a key this stage can press")?;
                let mut chord = KeyChord::key(key);
                for part in parts {
                    chord = chord.with(match part {
                        "ctrl" => Modifier::Ctrl,
                        "shift" => Modifier::Shift,
                        "alt" => Modifier::Alt,
                        "super" => Modifier::Super,
                        other => return Err(format!("{other:?} is not a modifier")),
                    });
                }
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
        let wanted_surface = take(&mut args, "--surface").map(|s| s.parse::<u32>()).transpose().map_err(|_| "--surface is a number")?;
        let policy_path = take(&mut args, "--policy").unwrap_or_else(|| DEFAULT_POLICY.into());
        let sequence = take(&mut args, "--sequence")
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| "--sequence is a number")?;
        let approve = flag(&mut args, "--approve");
        let (action, grade) = parse_action(&args)?;
        let policy = StagePolicy::load(&policy_path).map_err(|e| e.to_string())?;

        let stream = UnixStream::connect(&socket).await.map_err(|e| format!("{socket}: {e}"))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);

        // Learn the stage as it is: the compositor greets every connection with its surfaces
        // and the current authority.
        let mut world = StageWorld::default();
        let settle = tokio::time::sleep(Duration::from_millis(400));
        tokio::pin!(settle);
        loop {
            tokio::select! {
                _ = &mut settle => break,
                event = client.next_event() => match event.map_err(|e| e.to_string())? {
                    Some(event) => world.apply(&event),
                    None => return Err("the compositor closed the connection".into()),
                },
            }
        }
        let surface = match wanted_surface {
            Some(id) => world.surface(id).cloned(),
            None => world.surfaces_of(&app).next().cloned(),
        }
        .ok_or_else(|| format!("no surface of {app} is on the stage"))?;

        // The operator running this command is the principal, and asks for exactly one
        // operation on this application.
        let principal = Principal::Human {
            id: "stage-cli".into(),
            authenticated: true,
        };
        let operation = u64::from(surface.surface) << 32 | (Timestamp::now().as_millis() as u64 & 0xffff_ffff);
        let lease = PolicyEngine::default()
            .decide(&CapabilityRequest {
                trace_id: TraceId::new(),
                task_id: None,
                principal: principal.clone(),
                resource: stage_resource(&app),
                scope: Scope::OneOperation { operation },
                constraints: Constraints::with_actions([Action::Execute]),
                justification: "operator action from archie-stage-cli".into(),
            })
            .lease
            .ok_or("policy did not grant a lease")?;
        let proposal = StageProposal {
            app_id: app.clone(),
            surface: surface.surface,
            geometry_revision: surface.geometry_revision,
            action,
            text_grade: grade,
            operation_id: operation,
            task_id: None,
            trace_id: "stage-cli".into(),
        };
        let command = match StageGate::authorize(&policy, &lease, &principal, true, &proposal, Timestamp::now()) {
            StageAdmission::Allowed(command) => command,
            StageAdmission::NeedsApproval { command, reason } if approve => {
                eprintln!("approved by the operator: {reason}");
                command
            }
            StageAdmission::NeedsApproval { reason, .. } => {
                eprintln!("needs approval ({reason}); rerun with --approve if you are the person approving it");
                return Ok(ExitCode::from(3));
            }
            StageAdmission::Refused(refusal) => {
                eprintln!("refused ({}): {refusal}", refusal.code());
                return Ok(ExitCode::from(2));
            }
        };

        // Sequences must rise above anything the compositor accepted; the clock is a
        // simple source of that for a hand-driven tool.
        let first = sequence.unwrap_or_else(|| Timestamp::now().as_millis() as u64);
        let mut sealer = Sealer::new(key, first);
        let sealed = sealer.seal(&command, Timestamp::now(), 5_000).map_err(|e| e.to_string())?;
        client.send(&sealed).await.map_err(|e| e.to_string())?;
        println!("sent seq {} ({:?})", sealed.sequence(), command.receipt().action);

        let deadline = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return Err("no receipt within 30 s".into()),
                event = client.next_event() => match event.map_err(|e| e.to_string())? {
                    Some(StageEvent::Receipt { seq, outcome, .. }) if seq == sealed.sequence() => {
                        println!("receipt: {outcome}");
                        return Ok(if outcome == "executed" { ExitCode::SUCCESS } else { ExitCode::from(4) });
                    }
                    // The compositor could not verify a line from this connection, and so
                    // could not read its sequence either; it answers only the sender.
                    Some(StageEvent::Receipt { seq: 0, outcome, .. }) => {
                        println!("receipt: {outcome} (the compositor could not verify the command)");
                        return Ok(ExitCode::from(4));
                    }
                    Some(event) => world.apply(&event),
                    None => return Err("the compositor closed the connection".into()),
                },
            }
        }
    }
}
