//! `archie` — the ArchieOS command-line interface.
//!
//! A thin client. Every subcommand is one control request and one rendering; the CLI
//! holds no state and makes no decisions, so what it shows is always what the daemon
//! actually believes rather than a second, drifting model of the system.
//!
//! Every command accepts `--json` for machine consumption.

use archie_core::control::{
    request_over, ControlRequest, ControlResponse, CortexTrajectoryView, DeviceOperationView,
    DeviceView, ModelView, ModuleView, PeerView, ProviderView, StatusView, TotalsView,
};
use archie_core::endpoint;
use archie_fabric::{plan_enrolment, Enrolment, FabricChoice, NodeIdentity};
use archie_state::StatePaths;
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

#[derive(Debug, Parser)]
#[command(
    name = "archie",
    version,
    about = "Query and control the local ArchieOS node."
)]
struct Cli {
    /// Relocate the ArchieOS tree, matching `archied --root`.
    #[arg(long, value_name = "DIR", global = true)]
    root: Option<String>,

    /// Emit raw JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate the local workstation boundary and optional runtimes.
    Doctor,
    /// Create the local runtime directories and show first-run state.
    Setup,
    /// Start the ArchieOS workstation target.
    Start,
    /// Stop the ArchieOS workstation target.
    Stop,
    /// Node identity, hardware, capabilities, peers, modules and health.
    Status,
    /// The capability and resource graph across the fabric.
    Topology,
    /// Installed modules, and why each did or did not load.
    Modules,
    /// Installed data-provider manifests, and why each does or does not run.
    Providers,
    /// Fabric peers.
    Peers,
    /// Locally recorded device pairing inventory (read-only; no adapter connection).
    Devices,
    /// Recent prepared/cancelled physical-operation records (read-only; never dispatches).
    DeviceOperations {
        /// How many records to show.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Recent correction/replay metadata (read-only; no private payloads or graph writes).
    CortexTrajectories {
        /// How many records to show.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Registered model providers.
    Models,
    /// The boot report from the running daemon.
    Boot,
    /// Metrics snapshot.
    Metrics,
    /// Recent audit records.
    Audit {
        /// How many records to show.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Check that the daemon is alive.
    Ping,

    /// Put this node into a fabric.
    ///
    /// The only subcommand that writes rather than asks, and the only one that works
    /// with the daemon down — which is the state it exists for. A node that belongs to
    /// no fabric boots, runs, and reports a degraded stage saying so; this is what that
    /// message tells the operator to run.
    ///
    /// With no arguments it starts a new fabric owned by this machine. There is no
    /// guest flag: isolation is what happens when nobody does anything special, and
    /// joining is the step that takes an action.
    Enrol {
        /// Join the fabric this owner key belongs to, instead of starting a new one.
        ///
        /// The private key, because admitting a node means issuing a certificate and
        /// only the owner key can do that. A design where a fingerprint sufficed would
        /// be one where knowing a public value let you join.
        #[arg(long, value_name = "HEX")]
        owner_key: Option<String>,
    },
    /// Inspect or configure inference providers.
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// Inspect, add, authenticate or probe external harnesses.
    Harness {
        #[command(subcommand)]
        command: HarnessCommand,
    },
    /// Inspect the local sealed Vault.
    Vault {
        #[command(subcommand)]
        command: VaultCommand,
    },
    /// Inspect the local ASR model.
    Speech {
        #[command(subcommand)]
        command: SpeechCommand,
    },
    /// Test the local audio bridge.
    Voice {
        #[command(subcommand)]
        command: VoiceCommand,
    },
    /// Inspect professional tool adapters.
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Manage controlled workspaces.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Inspect the locally deployed Iris visual-reflex model.
    Iris {
        #[command(subcommand)]
        command: IrisCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProviderCommand {
    List,
    Add { id: String },
    Test { id: String },
}

#[derive(Debug, Subcommand)]
enum HarnessCommand {
    List,
    Add { id: String },
    Login { id: String },
    Test { id: String },
}

#[derive(Debug, Subcommand)]
enum VaultCommand {
    Status,
}
#[derive(Debug, Subcommand)]
enum SpeechCommand {
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
}
#[derive(Debug, Subcommand)]
enum ModelCommand {
    Status,
}
#[derive(Debug, Subcommand)]
enum VoiceCommand {
    Test,
}
#[derive(Debug, Subcommand)]
enum ToolsCommand {
    List,
    Test { id: String },
}
#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    Create { id: String },
    List,
}
#[derive(Debug, Subcommand)]
enum IrisCommand {
    /// Verify the manifest, checkpoint digest and fixed-point tensor bounds.
    VerifyModel {
        /// Deployment manifest; defaults to /archie/models/iris/champion/model-manifest.json.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// INT8 checkpoint; defaults to /archie/models/iris/champion/model.int8.json.
        #[arg(long)]
        checkpoint: Option<PathBuf>,
    },
}

impl Command {
    /// The daemon request this command makes, or `None` when it makes none.
    ///
    /// `Option` rather than a panic on the local arm: a command that never reaches the
    /// daemon is a real kind of command, not a mistake, and encoding it in the type
    /// means adding another cannot produce an `unreachable!` that is one day reached.
    fn to_request(&self) -> Option<ControlRequest> {
        Some(match self {
            Self::Doctor
            | Self::Setup
            | Self::Start
            | Self::Stop
            | Self::Provider { .. }
            | Self::Harness { .. }
            | Self::Vault { .. }
            | Self::Speech { .. }
            | Self::Voice { .. }
            | Self::Tools { .. }
            | Self::Workspace { .. }
            | Self::Iris { .. } => return None,
            Self::Status => ControlRequest::Status,
            Self::Topology => ControlRequest::Topology,
            Self::Modules => ControlRequest::Modules,
            Self::Providers => ControlRequest::Providers,
            Self::Peers => ControlRequest::Peers,
            Self::Devices => ControlRequest::Devices,
            Self::DeviceOperations { limit } => ControlRequest::DeviceOperations { limit: *limit },
            Self::CortexTrajectories { limit } => {
                ControlRequest::CortexTrajectories { limit: *limit }
            }
            Self::Models => ControlRequest::Models,
            Self::Boot => ControlRequest::Boot,
            Self::Metrics => ControlRequest::Metrics,
            Self::Audit { limit } => ControlRequest::Audit { limit: *limit },
            Self::Ping => ControlRequest::Ping,
            Self::Enrol { .. } => return None,
        })
    }
}

fn workstation_root(cli_root: Option<&str>) -> PathBuf {
    cli_root
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/archie"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn run_process(command: &str, args: &[&str]) -> std::process::ExitCode {
    match ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) if status.success() => std::process::ExitCode::SUCCESS,
        Ok(_) | Err(_) => std::process::ExitCode::FAILURE,
    }
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(format!("{} is not a bounded model file", path.display()));
    }
    std::fs::read(path).map_err(|error| error.to_string())
}

async fn local_command(command: &Command, root: &Path, json: bool) -> std::process::ExitCode {
    use archie_workstation::harness::{HarnessKind, HarnessSpec};
    match command {
        Command::Doctor => {
            let checks = archie_workstation::doctor();
            if json {
                println!("{}", serde_json::to_string_pretty(&checks).unwrap());
            } else {
                for c in &checks {
                    println!("{:<5} {:<20} {}", c.status, c.name, c.detail);
                }
            }
            if checks.iter().all(|c| c.status.is_healthy()) {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            }
        }
        Command::Setup => {
            for directory in ["runtime", "workspaces", "cache", "models", "state", "logs"] {
                if let Err(error) = std::fs::create_dir_all(root.join(directory)) {
                    eprintln!("archie: {error}");
                    return std::process::ExitCode::FAILURE;
                }
            }
            println!("workstation directories ready at {}", root.display());
            std::process::ExitCode::SUCCESS
        }
        Command::Start => run_process("sudo", &["systemctl", "start", "archie-workstation.target"]),
        Command::Stop => run_process("sudo", &["systemctl", "stop", "archie-workstation.target"]),
        Command::Provider { command } => {
            let path = root.join("state/providers.toml");
            match command {
                ProviderCommand::List => {
                    match archie_workstation::provider::load_providers(&path) {
                        Ok(items) => {
                            for (id, p) in items {
                                println!("{id}\t{:?}\t{}", p.locality, p.base_url);
                            }
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("archie: {e}");
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
                ProviderCommand::Add { id } => {
                    match archie_workstation::provider::builtin_provider(id)
                        .ok_or("unknown provider")
                        .and_then(|p| {
                            archie_workstation::provider::save_provider(&path, p)
                                .map_err(|_| "invalid provider")
                        }) {
                        Ok(()) => {
                            println!(
                                "provider {id} configured; credentials remain environment-backed"
                            );
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("archie: {e}");
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
                ProviderCommand::Test { id } => {
                    let providers =
                        archie_workstation::provider::load_providers(&path).unwrap_or_default();
                    let Some(p) = providers.get(id) else {
                        eprintln!("archie: provider not configured");
                        return std::process::ExitCode::FAILURE;
                    };
                    let endpoint = match p.openai_endpoint() {
                        Ok(value) => value,
                        Err(e) => {
                            eprintln!("archie: {e}");
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    let client = match archie_model_runtime::InferenceClient::new(endpoint) {
                        Ok(value) => value,
                        Err(e) => {
                            eprintln!("archie: {e}");
                            return std::process::ExitCode::FAILURE;
                        }
                    };
                    match client.models().await {
                        Ok(models) => {
                            println!("provider {id}: {} model(s) discovered", models.data.len());
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("archie: {e}");
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
            }
        }
        Command::Harness { command } => {
            let config_path = root.join("state/harnesses.toml");
            let kind = |id: &str| match id {
                "codex" => Some(HarnessKind::Codex),
                "claude" => Some(HarnessKind::Claude),
                "hermes" => Some(HarnessKind::Hermes),
                "uhp" => Some(HarnessKind::Uhp),
                _ => None,
            };
            match command {
                HarnessCommand::List => {
                    for harness in archie_workstation::harness::load_harnesses(&config_path)
                        .unwrap_or_default()
                    {
                        let available = harness.probe().unwrap_or_else(|_| "not installed".into());
                        println!("{}\t{available}", harness.id);
                    }
                    std::process::ExitCode::SUCCESS
                }
                HarnessCommand::Add { id } => {
                    let Some(harness) = kind(id).map(HarnessSpec::official) else {
                        eprintln!("archie: unknown harness");
                        return std::process::ExitCode::FAILURE;
                    };
                    match archie_workstation::harness::save_harness(&config_path, harness) {
                        Ok(()) => {
                            println!("harness {id} registered; install remains first-run and license-controlled");
                            std::process::ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("archie: {e}");
                            std::process::ExitCode::FAILURE
                        }
                    }
                }
                HarnessCommand::Login { id } => match kind(id) {
                    Some(HarnessKind::Codex) => run_process("codex", &["login"]),
                    Some(HarnessKind::Claude) => run_process("claude", &[]),
                    Some(HarnessKind::Hermes) => run_process("hermes", &["setup"]),
                    _ => std::process::ExitCode::FAILURE,
                },
                HarnessCommand::Test { id } => match kind(id)
                    .map(HarnessSpec::official)
                    .and_then(|h| h.probe().ok())
                {
                    Some(version) => {
                        println!("{id}: {version}");
                        std::process::ExitCode::SUCCESS
                    }
                    None => std::process::ExitCode::FAILURE,
                },
            }
        }
        Command::Vault {
            command: VaultCommand::Status,
        } => {
            let vault = Path::new("/vault");
            println!(
                "vault={} mode=sealed references=opaque",
                if vault.exists() {
                    "ready"
                } else {
                    "unavailable"
                }
            );
            if vault.exists() || cfg!(windows) {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            }
        }
        Command::Speech {
            command:
                SpeechCommand::Model {
                    command: ModelCommand::Status,
                },
        } => {
            let model = root.join("models/nemotron-speech-streaming-en-0.6b.q8_0.gguf");
            println!(
                "model={} path={}",
                if model.is_file() { "ready" } else { "missing" },
                model.display()
            );
            if model.is_file() {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            }
        }
        Command::Voice {
            command: VoiceCommand::Test,
        } => run_process("nemo-speech", &["--help"]),
        Command::Tools {
            command: ToolsCommand::List,
        } => {
            println!("freecad\tworkspace-scoped\tnetwork=false");
            std::process::ExitCode::SUCCESS
        }
        Command::Tools {
            command: ToolsCommand::Test { id },
        } if id == "freecad" => run_process("/usr/lib/archie/freecad-smoke-test", &[]),
        Command::Tools { .. } => std::process::ExitCode::FAILURE,
        Command::Workspace {
            command: WorkspaceCommand::Create { id },
        } => {
            if !valid_id(id) {
                return std::process::ExitCode::FAILURE;
            }
            match std::fs::create_dir(root.join("workspaces").join(id)) {
                Ok(()) => {
                    println!("workspace {id} created");
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("archie: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Command::Workspace {
            command: WorkspaceCommand::List,
        } => {
            if let Ok(entries) = std::fs::read_dir(root.join("workspaces")) {
                for entry in entries.flatten().filter(|e| e.path().is_dir()) {
                    println!("{}", entry.file_name().to_string_lossy());
                }
            }
            std::process::ExitCode::SUCCESS
        }
        Command::Iris {
            command:
                IrisCommand::VerifyModel {
                    manifest,
                    checkpoint,
                },
        } => {
            use archie_iris::{FoundryReflexModel, VisualReflexModel};
            let directory = root.join("models/iris/champion");
            let manifest_path = manifest
                .clone()
                .unwrap_or_else(|| directory.join("model-manifest.json"));
            let checkpoint_path = checkpoint
                .clone()
                .unwrap_or_else(|| directory.join("model.int8.json"));
            let loaded = read_bounded(&manifest_path, 64 * 1024)
                .and_then(|manifest_json| {
                    read_bounded(&checkpoint_path, 1024 * 1024)
                        .map(|checkpoint_json| (manifest_json, checkpoint_json))
                })
                .and_then(|(manifest_json, checkpoint_json)| {
                    FoundryReflexModel::load(&manifest_json, &checkpoint_json)
                        .map_err(|error| error.to_string())
                });
            match loaded {
                Ok(model) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(model.manifest()).unwrap()
                        );
                    } else {
                        println!(
                            "iris model={} revision={} quantization={} checkpoint={}",
                            model.manifest().model_id,
                            model.manifest().revision,
                            model.manifest().quantization.as_deref().unwrap_or("none"),
                            model.manifest().checkpoint_sha256
                        );
                    }
                    std::process::ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("archie: {error}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        _ => std::process::ExitCode::FAILURE,
    }
}

/// Put this node into a fabric.
///
/// The same decision the installer makes, through the same code, because there should be
/// one of it: a machine enrolled after the fact must end up in exactly the state a
/// machine enrolled during installation is in, or the two paths drift and only one of
/// them is the one anybody tests.
///
/// Refuses to re-enrol. Moving a machine between fabrics silently would orphan whatever
/// it holds and give no sign it had happened; a person who means it can remove the
/// membership file, which is an action rather than a flag.
fn enrol(paths: &StatePaths, owner_key: Option<&str>, json: bool) -> std::process::ExitCode {
    let node = match NodeIdentity::load_or_create(paths.identity_key()) {
        Ok(node) => node,
        Err(e) => {
            eprintln!("archie: cannot read this node's key: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match Enrolment::load(&node.public_key_hex(), paths.membership()) {
        Ok(existing) if existing.is_enrolled() => {
            let fingerprint = existing.fabric_fingerprint().unwrap_or_default();
            eprintln!("archie: this node already belongs to fabric {fingerprint}.");
            eprintln!("Enrolling again would move it, orphaning anything it holds there.");
            eprintln!(
                "If you mean it, remove {} first.",
                paths.membership().display()
            );
            return std::process::ExitCode::FAILURE;
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("archie: the existing membership file is unusable: {e}");
            return std::process::ExitCode::FAILURE;
        }
    }

    let choice = match owner_key {
        Some(hex) => FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex: hex.to_string(),
        },
        None => FabricChoice::StartNew,
    };

    let now_ms = archie_types::Timestamp::now().as_millis();
    let plan = match plan_enrolment(&node, &choice, now_ms) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("archie: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = plan.enrolment.save(paths.membership()) {
        eprintln!("archie: cannot write the membership: {e}");
        return std::process::ExitCode::FAILURE;
    }
    // Only when this machine made the fabric. A machine joining one was lent the key long
    // enough to issue a certificate for itself and is not given a copy.
    if let Some(owner) = &plan.owner {
        if let Err(e) = owner.save(paths.owner_key()) {
            eprintln!("archie: cannot write the owner key: {e}");
            return std::process::ExitCode::FAILURE;
        }
    }

    if json {
        let record = serde_json::json!({
            "fabric": plan.fabric_fingerprint(),
            "started_new_fabric": plan.started_new_fabric,
            "membership": paths.membership().display().to_string(),
            "owner_key": plan.owner.as_ref().map(|_| paths.owner_key().display().to_string()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&record).expect("serialisable")
        );
    } else {
        println!("{}", plan.summary());
        if plan.owner.is_some() {
            println!();
            println!(
                "The key that admits other machines to it is at {}.",
                paths.owner_key().display()
            );
            println!("Copy it somewhere safe: the fabric keeps working without it, but nothing");
            println!("new can join until it comes back.");
        }
        // The daemon reads its enrolment once, at boot. Saying so is cheaper than
        // somebody watching an empty peer list and concluding the fabric is broken.
        println!();
        println!("Restart archied for this to take effect.");
    }
    std::process::ExitCode::SUCCESS
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let paths = match &cli.root {
        Some(root) => StatePaths::rooted(root),
        None => StatePaths::from_env(),
    };

    // Local commands never reach the daemon, and `enrol` in particular must work with
    // it down: a node that belongs to no fabric is one whose daemon is reporting a
    // degraded boot stage, and telling somebody to start the daemon first would be
    // telling them to fix the thing they are trying to fix.
    let Some(request) = cli.command.to_request() else {
        return match &cli.command {
            Command::Enrol { owner_key } => enrol(&paths, owner_key.as_deref(), cli.json),
            other => local_command(other, &workstation_root(cli.root.as_deref()), cli.json).await,
        };
    };

    let mut stream = match endpoint::connect(&paths).await {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("archie: {e}");
            return std::process::ExitCode::from(69); // EX_UNAVAILABLE
        }
    };

    let response = match request_over(&mut stream, &request).await {
        Ok(response) => response,
        Err(e) => {
            eprintln!("archie: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if cli.json {
        match serde_json::to_string_pretty(&response) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("archie: cannot render JSON: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
        return exit_code(&response);
    }

    print!("{}", render(&response));
    exit_code(&response)
}

fn exit_code(response: &ControlResponse) -> std::process::ExitCode {
    if response.is_error() {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

fn render(response: &ControlResponse) -> String {
    match response {
        ControlResponse::Status(status) => render_status(status),
        ControlResponse::Topology {
            capabilities,
            nodes,
            totals,
        } => render_topology(capabilities, nodes, totals),
        ControlResponse::Modules {
            modules,
            load_order,
        } => render_modules(modules, load_order),
        ControlResponse::Providers { providers, active } => render_providers(providers, active),
        ControlResponse::Peers { peers } => render_peers(peers),
        ControlResponse::Devices { devices } => render_devices(devices),
        ControlResponse::DeviceOperations { operations } => render_device_operations(operations),
        ControlResponse::CortexTrajectories { trajectories } => {
            render_cortex_trajectories(trajectories)
        }
        ControlResponse::Models { models } => render_models(models),
        ControlResponse::Boot(report) => report.render(),
        ControlResponse::Metrics(snapshot) => {
            let mut out = String::new();
            for (name, value) in &snapshot.counters {
                out.push_str(&format!("{name:<40} {value}\n"));
            }
            for (name, value) in &snapshot.gauges {
                out.push_str(&format!("{name:<40} {value}\n"));
            }
            if out.is_empty() {
                out.push_str("no metrics recorded yet\n");
            }
            out
        }
        ControlResponse::Audit { events } => {
            if events.is_empty() {
                return "no audit records\n".to_string();
            }
            let mut out = String::new();
            for event in events {
                let action = event.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                let outcome = event.get("outcome").and_then(|v| v.as_str()).unwrap_or("?");
                let principal = event
                    .get("principal")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let resource = event
                    .get("resource")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let explanation = event
                    .get("explanation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push_str(&format!(
                    "{outcome:<16} {action:<24} {principal} -> {resource}\n    {explanation}\n"
                ));
            }
            out
        }
        ControlResponse::Pong { node_id, version } => {
            format!("archied {version} is running (node {})\n", short(node_id))
        }
        ControlResponse::Error { message } => format!("error: {message}\n"),
    }
}

fn render_status(status: &StatusView) -> String {
    let mut out = String::new();
    let field = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("{label:<22} {value}\n"));
    };

    out.push_str("ARCHIE OS\n\n");
    field(&mut out, "Node ID", &status.node_id);
    field(&mut out, "Hostname", &status.hostname);
    field(&mut out, "Profile", &status.profile);
    field(&mut out, "ArchieOS version", &status.archieos_version);
    field(
        &mut out,
        "System",
        &format!("{} {} ({})", status.os, status.kernel, status.architecture),
    );
    field(&mut out, "Uptime", &format_duration(status.uptime_ms));
    out.push('\n');

    field(
        &mut out,
        "CPU",
        &format!(
            "{} ({} physical / {} logical)",
            status.cpu_model, status.physical_cores, status.logical_cores
        ),
    );
    field(
        &mut out,
        "RAM",
        &format!(
            "{} free of {}",
            gib(status.memory_available_bytes),
            gib(status.memory_total_bytes)
        ),
    );
    field(
        &mut out,
        "Accelerators",
        &if status.accelerators.is_empty() {
            "none detected".to_string()
        } else {
            status.accelerators.join("\n                       ")
        },
    );
    field(
        &mut out,
        "Storage",
        &if status.storage.is_empty() {
            "none reported".to_string()
        } else {
            status.storage.join("\n                       ")
        },
    );
    field(
        &mut out,
        "Network",
        &if status.networks.is_empty() {
            "none reported".to_string()
        } else {
            status.networks.join("\n                       ")
        },
    );
    out.push('\n');

    field(
        &mut out,
        "Capabilities",
        &format!("{} advertised", status.capabilities.len()),
    );
    for capability in &status.capabilities {
        out.push_str(&format!("                       {capability}\n"));
    }
    if !status.capability_changes.is_empty() {
        field(
            &mut out,
            "Changed since boot",
            &status.capability_changes.join(" "),
        );
    }
    out.push('\n');

    field(
        &mut out,
        "Fabric",
        &format!("{} transport", status.fabric_transport),
    );
    field(
        &mut out,
        "Peers",
        &format!(
            "{} known, {} reachable",
            status.peers, status.reachable_peers
        ),
    );
    field(
        &mut out,
        "Modules",
        &format!(
            "{} loaded of {} installed",
            status.modules_loaded, status.modules_installed
        ),
    );
    field(
        &mut out,
        "Models",
        &if status.models_registered == 0 {
            "none registered".to_string()
        } else {
            format!("{} registered", status.models_registered)
        },
    );
    field(&mut out, "Tasks", &status.active_tasks.to_string());
    field(&mut out, "Health", &status.health);
    out
}

fn render_topology(
    capabilities: &BTreeMap<String, Vec<String>>,
    nodes: &[PeerView],
    totals: &TotalsView,
) -> String {
    let mut out = String::from("RESOURCE TOPOLOGY\n\n");

    out.push_str(&format!(
        "{} nodes ({} schedulable), {} logical cores, {} RAM",
        totals.nodes,
        totals.schedulable_nodes,
        totals.logical_cores,
        gib(totals.memory_bytes)
    ));
    if totals.accelerators > 0 {
        out.push_str(&format!(
            ", {} accelerators ({})",
            totals.accelerators,
            gib(totals.accelerator_bytes)
        ));
    }
    out.push_str("\n\nNODES\n");
    for node in nodes {
        out.push_str(&format!(
            "  {}  {:<20} {:<14} {:<10} {}\n",
            short(&node.node_id),
            node.hostname,
            node.profile,
            node.trust,
            if node.reachable {
                node.health.clone()
            } else {
                "unreachable".to_string()
            }
        ));
    }

    out.push_str("\nCAPABILITIES\n");
    if capabilities.is_empty() {
        out.push_str("  none advertised\n");
    }
    for (capability, providers) in capabilities {
        out.push_str(&format!("  {capability}\n"));
        for provider in providers {
            out.push_str(&format!("      {}\n", short(provider)));
        }
    }
    out
}

fn render_modules(modules: &[ModuleView], load_order: &[String]) -> String {
    if modules.is_empty() {
        return "no modules installed\n".to_string();
    }
    let mut out = String::from("MODULES\n\n");
    let width = modules
        .iter()
        .map(|m| m.id.len())
        .max()
        .unwrap_or(20)
        .max(20);

    for module in modules {
        out.push_str(&format!(
            "  {:<width$}  {:<10}  {:<14}  {}\n",
            module.id,
            module.version,
            module.state,
            module.explanation,
            width = width
        ));
    }

    if !load_order.is_empty() {
        out.push_str("\nLOAD ORDER\n");
        for (index, id) in load_order.iter().enumerate() {
            out.push_str(&format!("  {}. {id}\n", index + 1));
        }
    }
    out
}

fn render_providers(providers: &[ProviderView], active: &[String]) -> String {
    if providers.is_empty() {
        return "no data providers installed\n".to_string();
    }
    let mut out = String::from("DATA PROVIDERS\n\n");
    let width = providers
        .iter()
        .map(|p| p.id.len())
        .max()
        .unwrap_or(20)
        .max(20);

    for provider in providers {
        out.push_str(&format!(
            "  {:<width$}  {:<14}  {:<16}  {}\n",
            provider.id,
            provider.status,
            provider.capability,
            provider.explanation,
            width = width
        ));
    }

    out.push_str(&format!(
        "\n{} of {} will actually run; --json shows records and documented blind spots\n",
        active.len(),
        providers.len()
    ));
    out
}

fn render_peers(peers: &[PeerView]) -> String {
    if peers.is_empty() {
        // A lone node is a normal state, not an error, and the message should not read
        // like one.
        return "no peers discovered\n\nThis node is running alone on the fabric.\n".to_string();
    }
    let mut out = String::from("PEERS\n\n");
    for peer in peers {
        out.push_str(&format!(
            "  {}  {:<20} {:<14} trust={:<10} {}\n",
            short(&peer.node_id),
            peer.hostname,
            peer.profile,
            peer.trust,
            if peer.reachable {
                peer.health.clone()
            } else {
                "unreachable".to_string()
            }
        ));
        out.push_str(&format!(
            "      capabilities: {}\n",
            peer.capabilities.join(", ")
        ));
    }
    out
}

fn render_devices(devices: &[DeviceView]) -> String {
    if devices.is_empty() {
        return "no paired-device inventory records\n\nNo device adapter or command channel is active.\n"
            .to_string();
    }

    let mut out = String::from("DEVICE INVENTORY\n\n");
    for device in devices {
        out.push_str(&format!(
            "  {:<24} {:<16} pairing={:<10} key={}\n",
            device.device_reference, device.class, device.pairing, device.public_key_fingerprint,
        ));
        out.push_str(&format!(
            "      first seen={}  last seen={}\n",
            device.first_seen.as_millis(),
            device.last_seen.as_millis(),
        ));
    }
    out
}

fn render_device_operations(operations: &[DeviceOperationView]) -> String {
    if operations.is_empty() {
        return "no durable device operations\n\nThis view does not create, approve, cancel, or dispatch a command.\n"
            .to_string();
    }

    let mut out = String::from("DEVICE OPERATIONS (READ-ONLY)\n\n");
    for operation in operations {
        out.push_str(&format!(
            "  #{:<6} {:<24} state={:<10} trace={} digest={}\n",
            operation.operation_id,
            operation.device_reference,
            operation.state,
            operation.trace_fingerprint,
            operation.operation_sha256_fingerprint,
        ));
        out.push_str(&format!(
            "      prepared={}\n",
            operation.prepared_at.as_millis()
        ));
    }
    out
}

fn render_cortex_trajectories(trajectories: &[CortexTrajectoryView]) -> String {
    if trajectories.is_empty() {
        return "no Cortex correction trajectories\n\nThe graph writer/parser and offline trainer are not active from this view.\n"
            .to_string();
    }
    let mut out = String::from("CORTEX TRAJECTORIES (READ-ONLY)\n\n");
    for trajectory in trajectories {
        out.push_str(&format!(
            "  {:<24} {:<28} model={} payload={}\n",
            trajectory.trajectory_id,
            trajectory.correction_kind,
            trajectory.model_id,
            trajectory.payload_sha256_fingerprint,
        ));
        out.push_str(&format!(
            "      graph {} -> {}  selected={} omitted={} captured={}\n",
            trajectory.source_graph_snapshot_id,
            trajectory.resulting_graph_snapshot_id,
            trajectory.selected_context_count,
            trajectory.omitted_candidate_count,
            trajectory.captured_at.as_millis(),
        ));
    }
    out
}

fn render_models(models: &[ModelView]) -> String {
    if models.is_empty() {
        return "no model providers registered\n\nArchie can discover and register one \
                later; see docs/MODEL_ROUTING.md.\n"
            .to_string();
    }
    let mut out = String::from("MODELS\n\n");
    for model in models {
        out.push_str(&format!(
            "  {:<28} {:<18} {:<10} {:<10} ctx={} privacy<={}\n",
            model.id,
            model.provider,
            model.locality,
            model.availability,
            model.context_window,
            model.max_privacy_grade
        ));
    }
    out
}

/// First 12 hex characters of a node id — enough to be unambiguous in a table, short
/// enough to read.
fn short(node_id: &str) -> String {
    node_id.chars().take(12).collect()
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

fn format_duration(millis: i64) -> String {
    if millis < 0 {
        return "unknown".to_string();
    }
    let seconds = millis / 1000;
    let (days, hours, minutes) = (
        seconds / 86_400,
        (seconds % 86_400) / 3600,
        (seconds % 3600) / 60,
    );
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {}s", seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> StatusView {
        StatusView {
            node_id: "a".repeat(64),
            hostname: "brain-01".into(),
            profile: "brain-node".into(),
            archieos_version: "0.1.0".into(),
            os: "Arch Linux".into(),
            kernel: "6.10.0".into(),
            architecture: "x86_64".into(),
            uptime_ms: 3_725_000,
            cpu_model: "AMD Ryzen 9".into(),
            logical_cores: 16,
            physical_cores: 8,
            memory_total_bytes: 32 << 30,
            memory_available_bytes: 20 << 30,
            accelerators: vec!["NVIDIA RTX 3070 (8.0 GiB)".into()],
            storage: vec!["/ btrfs 400/900 GiB free".into()],
            networks: vec!["enp5s0 Lan 10.0.0.2".into()],
            capabilities: vec!["compute.cpu".into(), "compute.cuda".into()],
            peers: 2,
            reachable_peers: 1,
            modules_loaded: 3,
            modules_installed: 5,
            models_registered: 0,
            active_tasks: 0,
            health: "ok".into(),
            fabric_transport: "udp".into(),
            capability_changes: vec!["-compute.cuda".into()],
            // The rendering does not use it yet; a node with no stated place is the
            // ordinary case and the one worth having in the fixture.
            location: None,
        }
    }

    #[test]
    fn status_renders_every_field_the_spec_requires() {
        let rendered = render_status(&status());
        for expected in [
            "Node ID",
            "Hostname",
            "Profile",
            "ArchieOS version",
            "CPU",
            "RAM",
            "Accelerators",
            "Storage",
            "Network",
            "Capabilities",
            "Peers",
            "Modules",
            "Models",
            "Health",
        ] {
            assert!(rendered.contains(expected), "status is missing {expected}");
        }
        assert!(rendered.contains("RTX 3070"));
        assert!(rendered.contains("2 known, 1 reachable"));
        assert!(rendered.contains("3 loaded of 5 installed"));
    }

    #[test]
    fn status_surfaces_hardware_that_disappeared() {
        let rendered = render_status(&status());
        assert!(
            rendered.contains("-compute.cuda"),
            "a GPU that vanished since last boot must be visible"
        );
    }

    #[test]
    fn an_empty_peer_list_reads_as_normal_not_broken() {
        let rendered = render_peers(&[]);
        assert!(rendered.contains("running alone"));
        assert!(!rendered.to_lowercase().contains("error"));
    }

    #[test]
    fn an_empty_model_list_says_what_to_do_next() {
        let rendered = render_models(&[]);
        assert!(rendered.contains("no model providers registered"));
        assert!(rendered.contains("MODEL_ROUTING"));
    }

    #[test]
    fn device_inventory_is_visible_without_implying_a_live_adapter() {
        let rendered = render_devices(&[DeviceView {
            device_reference: "living-room-tv".into(),
            class: "television".into(),
            pairing: "paired".into(),
            public_key_fingerprint: "a1b2c3d4e5f60708".into(),
            first_seen: archie_types::Timestamp::from_millis(1),
            last_seen: archie_types::Timestamp::from_millis(2),
        }]);
        assert!(rendered.contains("living-room-tv"));
        assert!(rendered.contains("pairing=paired"));
        assert!(rendered.contains("a1b2c3d4e5f60708"));

        let empty = render_devices(&[]);
        assert!(empty.contains("No device adapter or command channel is active"));
    }

    #[test]
    fn device_operations_render_as_real_history_without_a_command_channel() {
        let rendered = render_device_operations(&[DeviceOperationView {
            operation_id: 42,
            device_reference: "living-room-tv".into(),
            state: "cancelled".into(),
            trace_fingerprint: "e1f2a3b4c5d6".into(),
            operation_sha256_fingerprint: "bbbbbbbbbbbbbbbb".into(),
            prepared_at: archie_types::Timestamp::from_millis(1),
        }]);
        assert!(rendered.contains("#42"));
        assert!(rendered.contains("state=cancelled"));
        assert!(rendered.contains("READ-ONLY"));

        let empty = render_device_operations(&[]);
        assert!(empty.contains("does not create, approve, cancel, or dispatch"));
    }

    #[test]
    fn modules_render_their_exclusion_reason() {
        let modules = vec![ModuleView {
            id: "science.physics".into(),
            version: "0.1.0".into(),
            state: "unsatisfied".into(),
            explanation: "node does not provide: compute.cuda".into(),
            dependencies: vec!["core.memory".into()],
        }];
        let rendered = render_modules(&modules, &[]);
        assert!(rendered.contains("science.physics"));
        assert!(
            rendered.contains("compute.cuda"),
            "an operator must be able to see WHY a module did not load"
        );
    }

    #[test]
    fn providers_render_their_exclusion_reason() {
        let providers = vec![
            ProviderView {
                id: "usgs.earthquakes".into(),
                title: "USGS Earthquake Hazards Program".into(),
                capability: "world.seismic".into(),
                status: "available".into(),
                explanation: "available".into(),
                records: vec!["observation".into(), "event".into()],
                known_gaps: vec![],
            },
            ProviderView {
                id: "nasa.firms".into(),
                title: "NASA FIRMS active fire detections".into(),
                capability: "world.wildfire".into(),
                status: "no-credentials".into(),
                explanation: "secret \"nasa.firms.map_key\" is not configured".into(),
                records: vec!["observation".into()],
                known_gaps: vec!["a detection is a thermal anomaly, not a confirmed fire".into()],
            },
        ];
        let rendered = render_providers(&providers, &["usgs.earthquakes".into()]);
        assert!(rendered.contains("usgs.earthquakes"));
        assert!(rendered.contains("nasa.firms"));
        assert!(
            rendered.contains("nasa.firms.map_key"),
            "an operator must be able to see WHY a provider is not running"
        );
        assert!(rendered.contains("1 of 2 will actually run"));
    }

    #[test]
    fn no_providers_installed_says_so_plainly() {
        assert_eq!(render_providers(&[], &[]), "no data providers installed\n");
    }

    #[test]
    fn module_load_order_is_numbered() {
        let rendered = render_modules(
            &[ModuleView {
                id: "core.archie".into(),
                version: "0.1.0".into(),
                state: "loaded".into(),
                explanation: "loaded".into(),
                dependencies: vec![],
            }],
            &["core.memory".into(), "core.archie".into()],
        );
        assert!(rendered.contains("1. core.memory"));
        assert!(rendered.contains("2. core.archie"));
    }

    #[test]
    fn topology_reports_totals_and_the_capability_index() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("compute.cuda".to_string(), vec!["b".repeat(64)]);
        let rendered = render_topology(
            &capabilities,
            &[],
            &TotalsView {
                nodes: 3,
                schedulable_nodes: 2,
                logical_cores: 48,
                memory_bytes: 96 << 30,
                accelerator_bytes: 24 << 30,
                accelerators: 3,
            },
        );
        assert!(rendered.contains("3 nodes (2 schedulable)"));
        assert!(rendered.contains("48 logical cores"));
        assert!(rendered.contains("compute.cuda"));
        assert!(rendered.contains("3 accelerators"));
    }

    #[test]
    fn node_ids_are_shortened_consistently() {
        assert_eq!(short(&"a".repeat(64)).len(), 12);
        assert_eq!(short("short"), "short");
    }

    #[test]
    fn durations_render_at_a_sensible_scale() {
        assert_eq!(format_duration(45_000), "45s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(format_duration(3_725_000), "1h 2m");
        assert_eq!(format_duration(90_061_000), "1d 1h 1m");
        assert_eq!(format_duration(-1), "unknown");
    }

    #[test]
    fn an_error_response_renders_and_fails() {
        let response = ControlResponse::error("something went wrong");
        assert!(render(&response).contains("something went wrong"));
    }

    #[test]
    fn every_command_maps_to_a_request() {
        for (command, expected) in [
            (Command::Status, "status"),
            (Command::Topology, "topology"),
            (Command::Modules, "modules"),
            (Command::Providers, "providers"),
            (Command::Peers, "peers"),
            (Command::Devices, "devices"),
            (Command::DeviceOperations { limit: 5 }, "device-operations"),
            (
                Command::CortexTrajectories { limit: 5 },
                "cortex-trajectories",
            ),
            (Command::Models, "models"),
            (Command::Boot, "boot"),
            (Command::Metrics, "metrics"),
            (Command::Ping, "ping"),
            (Command::Audit { limit: 5 }, "audit"),
        ] {
            assert_eq!(
                command
                    .to_request()
                    .expect("this command talks to the daemon")
                    .name(),
                expected
            );
        }
    }

    /// `enrol` writes files and must work with the daemon down, because a node that
    /// belongs to no fabric is one whose daemon is reporting a degraded boot stage.
    /// Routing it through the control socket would mean telling somebody to start the
    /// thing they are trying to fix.
    #[test]
    fn enrol_asks_the_daemon_for_nothing() {
        assert!(Command::Enrol { owner_key: None }.to_request().is_none());
        assert!(Command::Enrol {
            owner_key: Some("aabb".into())
        }
        .to_request()
        .is_none());
    }

    #[test]
    fn workstation_commands_are_local() {
        for command in [
            Command::Doctor,
            Command::Setup,
            Command::Start,
            Command::Stop,
        ] {
            assert!(command.to_request().is_none());
        }
    }

    #[test]
    fn iris_model_verification_is_local() {
        assert!(Command::Iris {
            command: IrisCommand::VerifyModel {
                manifest: None,
                checkpoint: None,
            },
        }
        .to_request()
        .is_none());
    }
}
