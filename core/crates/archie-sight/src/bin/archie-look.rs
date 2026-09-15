//! `archie-look` — Archie finds something in a window on the stage and activates it.
//!
//! ```text
//! archie-look --socket <path> --key-file <path> --policy <apps.toml> --app <app_id>
//!             --find <description> [--node button:save] [--kind button]
//!             --model <provider model> [--endpoint <url>] [--screen public|internal]
//! ```
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
    use archie_sight::{Sight, Target};
    use archie_stage::{learn, CommandKey, Operator, StageClient, StagePolicy, StageWorld};
    use archie_types::{
        AgentId, Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId,
        PrivacyGrade, ProviderKind, TokenCost, TraceId,
    };
    use std::collections::BTreeMap;
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
            return Err("raw_frame_retention_forbidden".into());
        }
        let socket = take(&mut args, "--socket").ok_or("--socket is required")?;
        let key = load_key(&take(&mut args, "--key-file").ok_or("--key-file is required")?)?;
        let policy_path = take(&mut args, "--policy").ok_or("--policy is required")?;
        let app = take(&mut args, "--app").ok_or("--app is required")?;
        let description = take(&mut args, "--find").ok_or("--find is required")?;
        let node = NodeId::new(take(&mut args, "--node").unwrap_or_else(|| "stage:target".into()))
            .map_err(|_| "--node is not a scene node id")?;
        const KINDS: &str = "button, input, link, checkbox, tab, menu_item";
        let kind = match take(&mut args, "--kind").as_deref().unwrap_or("button") {
            "button" => NodeKind::Button,
            "input" => NodeKind::Input,
            "link" => NodeKind::Link,
            "checkbox" => NodeKind::Checkbox,
            "tab" => NodeKind::Tab,
            "menu_item" => NodeKind::MenuItem,
            other => return Err(format!("--kind {other:?} is not one of {KINDS}")),
        };
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

        let cloud = endpoint_url.is_none();
        let endpoint = match &endpoint_url {
            None => ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1"),
            Some(url) => ProviderEndpoint::nvidia_nim(url, Locality::Local),
        }
        .map_err(|e| e.to_string())?;
        let descriptor = descriptor(&provider_model, cloud, screen)?;
        let locality = descriptor.locality;
        let grounding =
            LocateAnythingClient::new(endpoint, descriptor).map_err(|e| e.to_string())?;

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
        let mut operator = Operator::new(key, policy, PolicyEngine::default());

        let target = Target {
            app_id: app.clone(),
            node,
            kind,
            description,
            description_grade: PrivacyGrade::Public,
            screen_grade: screen,
            task_id: format!("look-{}", TraceId::new()),
        };
        let principal = Principal::Agent {
            agent: AgentId::new(),
        };
        let mut sight = Sight {
            operator: &mut operator,
            client: &mut client,
            world: &mut world,
            snapshot_dir,
            principal,
            engine: PolicyEngine::default(),
            timeout: Duration::from_secs(30),
        };
        let operation_id = (archie_types::Timestamp::now().as_millis() as u64).max(1);
        println!("looking at {app} and asking {provider_model} where the target is");
        let sighting = match sight
            .find_and_activate(&target, &grounding, locality, operation_id)
            .await
        {
            Ok(sighting) => sighting,
            Err(error) => {
                eprintln!("archie-look: {}", explain(&error));
                return Ok(ExitCode::from(4));
            }
        };
        println!(
            "captured {}x{}; grounded at x={} y={} {}x{}; clicked ({}, {}); {}",
            sighting.before.width,
            sighting.before.height,
            sighting.bbox.x,
            sighting.bbox.y,
            sighting.bbox.width,
            sighting.bbox.height,
            sighting.point.x,
            sighting.point.y,
            sighting.outcome.outcome_code
        );
        Ok(ExitCode::SUCCESS)
    }

    #[cfg(test)]
    mod tests {
        use super::run;

        #[tokio::test]
        async fn raw_frame_retention_is_rejected_before_io() {
            let error = run(vec!["--keep".into(), "/tmp/captures".into()])
                .await
                .expect_err("raw frame retention must fail");
            assert_eq!(error, "raw_frame_retention_forbidden");
        }
    }
}
