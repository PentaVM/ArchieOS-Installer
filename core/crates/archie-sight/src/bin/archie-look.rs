//! `archie-look` — Archie finds something in a window on the stage and activates it.
//!
//! ```text
//! archie-look --socket <path> --key-file <path> --policy <apps.toml> --app <app_id>
//!             --model <provider model> [--endpoint <url>] [--screen public|internal]
//!             --find <description> [--node button:save] [--kind button] [--no-a11y]
//!             [--reflex <dir with reflex.checkpoint>]
//!
//! archie-look … --session      one target per line on stdin: <node> <kind> <description>
//! ```
//!
//! A session keeps the stage connection, what it has learned about each target and the
//! connection to the grounding model, and prints one JSON line per activation saying how
//! the position was known and where the time went. That is the loop IRIS needs: the model
//! is asked once per target, not once per click.
//!
//! The grounding model is reached through Nebius Token Factory unless `--endpoint` names a
//! local OpenAI-compatible server such as a NIM; the Nebius credential is read from
//! `NEBIUS_TOKEN_FACTORY_KEY` and never printed. `--screen` states what the window shows
//! (default: public); a cloud model whose retention nobody verified is only ever given
//! public windows. Captures remain ephemeral and are never exposed as a persistence option.

#[cfg(unix)]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    unix::main().await
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "archie-look talks to the Archie Stage over a Unix socket; run it on the stage's host"
    );
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
mod unix {
    use archie_iris::{LocateAnythingClient, NodeId, NodeKind};
    use archie_model_runtime::ProviderEndpoint;
    use archie_policy::{PolicyEngine, Principal};
    use archie_sight::{Sight, Sighting, Target};
    use archie_stage::{learn, CommandKey, Operator, StageClient, StagePolicy, StageWorld};
    use archie_types::{
        AgentId, Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId,
        PrivacyGrade, ProviderKind, TokenCost, TraceId,
    };
    use std::collections::BTreeMap;
    use std::io::{BufRead, Write};
    use std::path::Path;
    use std::process::ExitCode;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
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

    fn flag(args: &mut Vec<String>, flag: &str) -> bool {
        match args.iter().position(|a| a == flag) {
            Some(at) => {
                args.remove(at);
                true
            }
            None => false,
        }
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

    /// An error with its causes, each said once: "transport failed" alone does not tell
    /// anyone that the runtime refused to send an internal window to that model.
    fn explain(error: &dyn std::error::Error) -> String {
        let mut message = error.to_string();
        let mut cause = error.source();
        while let Some(inner) = cause {
            let text = inner.to_string();
            if !message.contains(&text) {
                message.push_str(": ");
                message.push_str(&text);
            }
            cause = inner.source();
        }
        message
    }

    fn kind_of(name: &str) -> Result<NodeKind, String> {
        const KINDS: &str = "button, input, link, checkbox, tab, menu_item";
        match name {
            "button" => Ok(NodeKind::Button),
            "input" => Ok(NodeKind::Input),
            "link" => Ok(NodeKind::Link),
            "checkbox" => Ok(NodeKind::Checkbox),
            "tab" => Ok(NodeKind::Tab),
            "menu_item" => Ok(NodeKind::MenuItem),
            other => Err(format!("--kind {other:?} is not one of {KINDS}")),
        }
    }

    fn descriptor(
        provider_model: &str,
        cloud: bool,
        screen: PrivacyGrade,
    ) -> Result<ModelDescriptor, String> {
        let mut labels = BTreeMap::new();
        labels.insert("provider-model".to_string(), provider_model.to_string());
        // Archie model ids are dotted lowercase words; the provider's name stays in a label.
        let sanitized: String = provider_model
            .chars()
            .take(96)
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        Ok(ModelDescriptor {
            id: ModelId::parse(format!("look.model-{sanitized}"))
                .map_err(|_| "the model name cannot become an Archie model id".to_string())?,
            display_name: provider_model.to_string(),
            provider: if cloud {
                ProviderKind::NebiusTokenFactory
            } else {
                ProviderKind::NvidiaNim
            },
            locality: if cloud {
                Locality::Cloud
            } else {
                Locality::Local
            },
            node: None,
            context_window: 32_000,
            // The grounding client's own ceiling. A model that reasons before it answers,
            // as Kimi-K3 does, needs most of it for one box.
            max_output_tokens: 512,
            capabilities: vec![Capability::known("compute.inference")],
            // Unverified retention in the cloud: public only, whatever was asked.
            max_privacy_grade: if cloud { PrivacyGrade::Public } else { screen },
            retention: if cloud {
                DataRetention::Unknown
            } else {
                DataRetention::ZeroVerified
            },
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: None,
            quality_tier: 50,
            required_accelerator_bytes: None,
            labels,
        })
    }

    pub async fn main() -> ExitCode {
        match run(std::env::args().skip(1).collect()).await {
            Ok(code) => code,
            Err(message) => {
                eprintln!("archie-look: {message}");
                ExitCode::FAILURE
            }
        }
    }

    async fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
        if args.iter().any(|argument| argument == "--keep") {
            // Window pixels stay inside the bounded pass: there is no option that writes
            // them somewhere for later.
            return Err("raw_frame_retention_forbidden".into());
        }
        let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
        let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
        let policy_path = take(&mut args, "--policy").ok_or("--policy is required")?;
        let app = take(&mut args, "--app").ok_or("--app is required")?;
        let session = flag(&mut args, "--session");
        // Applications describe their own controls for screen readers; that is the fastest
        // and most exact way to find one, and `--no-a11y` is for measuring without it.
        let ask_applications = !flag(&mut args, "--no-a11y");
        // The local model, when one has been trained. It never names anything; it says
        // where the controls are, which is enough to put a grounding answer on a real one
        // and to find a control again after the window moved it.
        let reflex = take(&mut args, "--reflex");
        let description = take(&mut args, "--find");
        let node = take(&mut args, "--node").unwrap_or_else(|| "stage:target".into());
        let kind = take(&mut args, "--kind").unwrap_or_else(|| "button".into());
        let provider_model = take(&mut args, "--model").ok_or("--model is required")?;
        let endpoint_url = take(&mut args, "--endpoint");
        let screen = match take(&mut args, "--screen").as_deref().unwrap_or("public") {
            "public" => PrivacyGrade::Public,
            "internal" => PrivacyGrade::Internal,
            _ => return Err("--screen is public or internal".into()),
        };

        if !args.is_empty() {
            return Err(format!("unexpected arguments: {args:?}"));
        }
        if session == description.is_some() {
            return Err("give either --find <description> or --session".into());
        }

        let cloud = endpoint_url.is_none();
        let endpoint = match &endpoint_url {
            None => ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1"),
            Some(url) => ProviderEndpoint::nvidia_nim(url, Locality::Local),
        }
        .map_err(|e| e.to_string())?;
        let descriptor = descriptor(&provider_model, cloud, screen)?;
        let locality = descriptor.locality;
        // One client for the whole session: a background ask holds it while the loop works.
        let grounding =
            Arc::new(LocateAnythingClient::new(endpoint, descriptor).map_err(|e| e.to_string())?);

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
        let principal = Principal::Agent {
            agent: AgentId::new(),
        };
        let mut sight = Sight::new(operator, client, world, snapshot_dir, principal);
        if ask_applications {
            match archie_a11y::Accessibility::connect().await {
                Ok(bus) => sight = sight.asking(Box::new(bus)),
                Err(error) => eprintln!(
                    "archie-look: nothing to ask about controls ({error}); looking instead"
                ),
            }
        }

        if let Some(dir) = &reflex {
            match archie_reflex::Deployed::load(Path::new(dir)) {
                Ok(model) => sight = sight.recognising(Arc::new(model)),
                Err(error) => eprintln!(
                    "archie-look: no local model in {dir} ({error}); every answer stands as it came"
                ),
            }
        }

        if session {
            return session_loop(&mut sight, &app, screen, &grounding, locality).await;
        }
        let target = target_for(&app, &node, &kind, &description.unwrap_or_default(), screen)?;
        println!("looking at {app} and asking {provider_model} where the target is");
        let sighting = match sight
            .find_and_activate(&target, &grounding, locality, operation_id())
            .await
        {
            Ok(sighting) => sighting,
            Err(error) => {
                eprintln!("archie-look: {}", explain(&error));
                return Ok(ExitCode::from(4));
            }
        };
        report(&sighting);
        Ok(ExitCode::SUCCESS)
    }

    fn operation_id() -> u64 {
        (archie_types::Timestamp::now().as_millis() as u64).max(1)
    }

    fn target_for(
        app: &str,
        node: &str,
        kind: &str,
        description: &str,
        screen: PrivacyGrade,
    ) -> Result<Target, String> {
        Ok(Target {
            app_id: app.to_string(),
            node: NodeId::new(node.to_string()).map_err(|_| "--node is not a scene node id")?,
            kind: kind_of(kind)?,
            description: description.to_string(),
            description_grade: PrivacyGrade::Public,
            screen_grade: screen,
            task_id: format!("look-{}", TraceId::new()),
        })
    }

    fn report(sighting: &Sighting) {
        let capture = match &sighting.before {
            Some(before) => format!(
                "captured {}x{} (sha256 {}); ",
                before.width,
                before.height,
                &before.sha256[..16]
            ),
            None => String::new(),
        };
        println!(
            "{capture}{} at x={} y={} {}x{}; clicked ({}, {}) in {} ms; {}",
            sighting.knowledge.as_str(),
            sighting.bbox.x,
            sighting.bbox.y,
            sighting.bbox.width,
            sighting.bbox.height,
            sighting.point.x,
            sighting.point.y,
            sighting.timings.total_ms,
            sighting.outcome.outcome_code
        );
    }

    /// One target per line: `<node> <kind> <description…>`. Blank lines and `#` are
    /// ignored.
    ///
    /// The whole list is read before anything is done with it, so that a grounding model
    /// can be asked about the targets nothing else can answer for while the ones that are
    /// already known are being worked. Every line answers with one JSON object saying how
    /// its position was known and where each millisecond went, and the last one sums up.
    async fn session_loop<R, W>(
        sight: &mut Sight<R, W>,
        app: &str,
        screen: PrivacyGrade,
        grounding: &Arc<LocateAnythingClient>,
        locality: Locality,
    ) -> Result<ExitCode, String>
    where
        R: tokio::io::AsyncRead + Unpin,
        W: tokio::io::AsyncWrite + Unpin,
    {
        let mut targets = Vec::new();
        for line in std::io::stdin().lock().lines() {
            let line = line.map_err(|e| e.to_string())?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.splitn(3, char::is_whitespace);
            let (Some(node), Some(kind), Some(description)) =
                (words.next(), words.next(), words.next())
            else {
                return Err(format!("{line:?} is not <node> <kind> <description>"));
            };
            targets.push(target_for(app, node, kind, description.trim(), screen)?);
        }

        let started = Instant::now();
        let asking = sight
            .start_finding(&targets, grounding, locality)
            .await
            .map_err(|error| explain(&error))?;
        let (mut done, mut failed) = (0_u64, 0_u64);
        for target in &targets {
            let at = Instant::now();
            match sight
                .find_and_activate(target, grounding, locality, operation_id())
                .await
            {
                Ok(sighting) => {
                    done += 1;
                    let timings = sighting.timings;
                    println!(
                        "{}",
                        serde_json::json!({
                            "node": target.node.as_str(),
                            "known": sighting.knowledge.as_str(),
                            "x": sighting.point.x,
                            "y": sighting.point.y,
                            "declare_ms": timings.declare_ms,
                            "look_ms": timings.look_ms,
                            "ground_ms": timings.ground_ms,
                            "recognise_ms": timings.recognise_ms,
                            "act_ms": timings.act_ms,
                            "verify_ms": timings.verify_ms,
                            "total_ms": timings.total_ms,
                            "outcome": sighting.outcome.outcome_code,
                        })
                    );
                }
                Err(error) => {
                    failed += 1;
                    println!(
                        "{}",
                        serde_json::json!({
                            "node": target.node.as_str(),
                            "error": explain(&error),
                            "total_ms": at.elapsed().as_millis() as u64,
                        })
                    );
                }
            }
            std::io::stdout().flush().map_err(|e| e.to_string())?;
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        println!(
            "{}",
            serde_json::json!({
                "session": app,
                "asked_in_background": asking,
                "activated": done,
                "failed": failed,
                "elapsed_ms": elapsed_ms,
                "per_second": if elapsed_ms == 0 { 0.0 } else { done as f64 * 1_000.0 / elapsed_ms as f64 },
            })
        );
        Ok(if failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(4)
        })
    }
}
