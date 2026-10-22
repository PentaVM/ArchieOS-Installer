//! Operator CLI for shared inference and memory. Official clients retain credentials.
use archie_core::{
    control::{request_over, ControlRequest, ControlResponse},
    endpoint,
};
use archie_model_runtime::routed::Routed;
use archie_policy::PolicyEngine;
use archie_state::{StatePaths, StateStore};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, VaultRef};
use archie_workstation::{
    ai::{
        InferenceBudget, InferenceOutcome, InferenceRequest, InferenceStorage, RunnerChoice,
        UnifiedRuntime,
    },
    ai_policy::{RuntimeAuthority, RuntimeProfile},
    codex::CodexClient,
    harness::{save_harness, HarnessKind, HarnessSpec},
    harness_runtime::{create_program_config, load_program, run_harness, HarnessRunRequest},
};
use clap::Subcommand;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Debug, Subcommand)]
pub enum AiCommand {
    /// Create operator scope once; never overwrites an existing policy.
    Init {
        #[arg(long)]
        domain: String,
        /// Register the installed official client for daemon use.
        #[arg(long)]
        codex: Option<PathBuf>,
    },
    /// Read official account metadata; credentials stay with the client.
    Account {
        #[arg(long, default_value = "codex")]
        codex: PathBuf,
    },
    /// Model discovery alone does not prove inference access.
    Models {
        #[arg(long, default_value = "codex")]
        codex: PathBuf,
    },
    /// Run a bounded proposal through Core and persist its attributed result.
    Ask {
        #[arg(long)]
        domain: String,
        #[arg(long, default_value = "conversation")]
        focus: String,
        #[arg(long)]
        prompt_file: PathBuf,
        #[arg(long, default_value = "public")]
        privacy: String,
        #[arg(long, default_value = "api")]
        runner: String,
        #[arg(long)]
        model: Option<String>,
        /// For explicit standalone diagnostics; Core uses trusted configuration.
        #[arg(long, default_value = "codex")]
        codex: PathBuf,
        /// Hard generation token cap for APIs (default 1024).
        #[arg(long, conflicts_with = "managed_byte_limit")]
        max_tokens: Option<u32>,
        /// Explicit plan byte limit. Managed plans expose no hard token cap.
        #[arg(long)]
        managed_byte_limit: Option<usize>,
        #[arg(long)]
        evidence: Vec<String>,
        /// Permit standalone diagnostics only when no daemon is reachable.
        #[arg(long)]
        standalone: bool,
    },
    /// Reopen committed experience from receipt JSON returned by Ask.
    Recall {
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        standalone: bool,
    },
    /// Configure pinned Python and worker once. Use private POSIX installed paths.
    ConfigureHarness2 {
        #[arg(long)]
        python: PathBuf,
        #[arg(long)]
        worker: PathBuf,
    },
    /// Actual pinned Harness2, eight mediated stages; promotion defaults to denied.
    Harness2 {
        #[arg(long)]
        domain: String,
        #[arg(long, default_value = "harness")]
        focus: String,
        #[arg(long)]
        task_family: String,
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        prompt_file: PathBuf,
        #[arg(long, default_value = "api")]
        runner: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value_t = 300000)]
        duration_ms: u64,
        /// Accept managed-plan byte/time bounds without claiming a hard token cap.
        #[arg(long)]
        accept_managed_bounds: bool,
        #[arg(long)]
        standalone: bool,
        #[arg(long, default_value = "codex")]
        codex: PathBuf,
    },
}
fn domain(value: &str) -> Result<ContextDomain, String> {
    ContextDomain::try_from(value.to_owned()).map_err(|e| e.to_string())
}
fn read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "cannot open bounded input")?;
    if !file
        .metadata()
        .map_err(|_| "cannot inspect input")?
        .is_file()
    {
        return Err("input is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read input")?;
    if bytes.is_empty() || bytes.len() as u64 > max {
        return Err("input exceeds byte budget".into());
    }
    Ok(bytes)
}
fn prompt(path: &Path, max: u64) -> Result<String, String> {
    String::from_utf8(read(path, max)?).map_err(|_| "prompt must be UTF-8".into())
}
fn client(paths: &StatePaths, executable: &Path) -> Result<CodexClient, String> {
    let cwd = paths.runtime_dir.join("ai-client");
    std::fs::create_dir_all(&cwd).map_err(|_| "cannot create managed-client directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cwd, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "cannot restrict managed-client directory")?;
    }
    CodexClient::new(executable.as_os_str(), &cwd, Duration::from_secs(60))
        .map_err(|e| e.to_string())
}
async fn open(paths: &StatePaths, codex: Option<CodexClient>) -> Result<UnifiedRuntime, String> {
    let database = paths.database();
    let store =
        tokio::task::spawn_blocking(move || StateStore::open(database).map_err(|e| e.to_string()))
            .await
            .map_err(|_| "database worker failed")??;
    Ok(UnifiedRuntime {
        storage: Arc::new(InferenceStorage {
            store: Arc::new(store),
            authority: RuntimeAuthority::new(
                paths.config_dir.join("inference.toml"),
                Arc::new(RwLock::new(PolicyEngine::default())),
            ),
            vault_dir: paths.state_dir.join("inference/vault"),
        }),
        catalog: Routed::from_paths(paths.models(), paths.inference_providers())
            .map_err(|e| e.to_string())?,
        codex,
    })
}
/// Once connected, an error is terminal. Never replay a possibly submitted turn
/// or open another StateStore writer because a reply was lost.
async fn daemon(
    paths: &StatePaths,
    request: &ControlRequest,
    standalone: bool,
    duration_ms: u64,
) -> Result<Option<ControlResponse>, String> {
    let connection = tokio::time::timeout(Duration::from_secs(2), endpoint::connect(paths)).await;
    let mut stream =
        match connection {
            Ok(Ok(stream)) => stream,
            _ if standalone => return Ok(None),
            _ => return Err(
                "Core is unavailable; start archied or explicitly select --standalone diagnostics"
                    .into(),
            ),
        };
    let response = tokio::time::timeout(
        Duration::from_millis(duration_ms.saturating_add(3000)),
        request_over(&mut stream, request),
    )
    .await
    .map_err(|_| "Core response deadline expired; request was not replayed")?
    .map_err(|_| "Core reply failed; request was not replayed")?;
    match response {
        ControlResponse::Error { message } => Err(message),
        other => Ok(Some(other)),
    }
}
fn choice(runner: &str, model: &Option<String>) -> Result<RunnerChoice, String> {
    match runner {
        "api" => Ok(model
            .as_ref()
            .map(|model| RunnerChoice::ApiPinned {
                model: model.clone(),
            })
            .unwrap_or(RunnerChoice::Api)),
        "codex" => Ok(RunnerChoice::Codex {
            model: model
                .clone()
                .ok_or("--model is required for managed Codex")?,
        }),
        _ => Err("runner must be api or codex".into()),
    }
}
fn budget(
    runner: &str,
    tokens: Option<u32>,
    bytes: Option<usize>,
) -> Result<InferenceBudget, String> {
    match (runner,tokens,bytes) {
        ("api",_,None)=>Ok(InferenceBudget::StrictTokens),
        ("codex",None,Some(n)) if (1..=65536).contains(&n)=>Ok(InferenceBudget::ManagedByteTime{max_output_bytes:n}),
        _=>Err("API uses token bounds; Codex requires --managed-byte-limit 1..65536 and no --max-tokens".into()),
    }
}
fn print_answer(result: &InferenceOutcome, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(result).map_err(|_| "cannot encode result")?
        );
    } else {
        if let Some(text) = &result.text {
            println!("{text}");
        }
        println!(
            "receipt={} model={} status={:?}",
            result.receipt.receipt_id, result.receipt.model_id, result.receipt.status
        );
        if let Some(bounds) = &result.budget {
            println!(
                "enforced_budget={}",
                serde_json::to_string(bounds).map_err(|_| "cannot encode budget")?
            );
        }
    }
    if result.receipt.status != archie_memory::inference::InferenceStatus::Completed {
        return Err("inference did not complete; attributed failure was persisted".into());
    }
    Ok(())
}
pub async fn run(command: &AiCommand, paths: &StatePaths, json: bool) -> Result<(), String> {
    match command {
        AiCommand::Init {
            domain: value,
            codex,
        } => {
            RuntimeProfile::for_domain(domain(value)?)
                .create(&paths.config_dir.join("inference.toml"))?;
            if let Some(executable) = codex {
                let mut spec = HarnessSpec::official(HarnessKind::Codex);
                spec.command = Some(
                    executable
                        .to_str()
                        .ok_or("Codex path must be UTF-8")?
                        .to_owned(),
                );
                save_harness(&paths.inference_harnesses(), spec).map_err(|e| e.to_string())?;
            }
            println!(
                "operator inference scope created; PostgreSQL configuration is {}",
                paths.database().display()
            );
        }
        AiCommand::Account { codex } => println!(
            "{}",
            serde_json::to_string_pretty(
                &client(paths, codex)?
                    .account()
                    .await
                    .map_err(|e| e.to_string())?
            )
            .map_err(|_| "cannot encode account status")?
        ),
        AiCommand::Models { codex } => println!(
            "{}",
            serde_json::to_string_pretty(
                &client(paths, codex)?
                    .models()
                    .await
                    .map_err(|e| e.to_string())?
            )
            .map_err(|_| "cannot encode catalog")?
        ),
        AiCommand::ConfigureHarness2 { python, worker } => {
            create_program_config(
                &paths.state_dir.join("inference/harness2.toml"),
                python,
                worker,
            )?;
            println!("Harness2 worker configured; no inference or promotion performed");
        }
        AiCommand::Ask {
            domain: value,
            focus,
            prompt_file,
            privacy,
            runner,
            model,
            codex,
            max_tokens,
            managed_byte_limit,
            evidence,
            standalone,
        } => {
            let privacy = match privacy.as_str() {
                "public" => PrivacyGrade::Public,
                "internal" => PrivacyGrade::Internal,
                "sensitive" => PrivacyGrade::Sensitive,
                "restricted" => PrivacyGrade::Restricted,
                _ => return Err("invalid privacy grade; secrets never reach inference".into()),
            };
            let request = InferenceRequest {
                domain: domain(value)?,
                focus: focus.clone(),
                privacy,
                prompt: prompt(prompt_file, 65536)?,
                evidence: evidence
                    .iter()
                    .map(|r| VaultRef::parse(r).map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?,
                runner: choice(runner, model)?,
                max_output_tokens: max_tokens.unwrap_or(1024),
                recall_memory: true,
                reply_schema: None,
                budget: budget(runner, *max_tokens, *managed_byte_limit)?,
            };
            let result = match daemon(
                paths,
                &ControlRequest::AiInfer {
                    request: request.clone(),
                },
                *standalone,
                600000,
            )
            .await?
            {
                Some(ControlResponse::AiResult { outcome }) => *outcome,
                Some(_) => return Err("unexpected Core inference response".into()),
                None => {
                    open(
                        paths,
                        if runner == "codex" {
                            Some(client(paths, codex)?)
                        } else {
                            None
                        },
                    )
                    .await?
                    .execute(&request, None)
                    .await?
                }
            };
            print_answer(&result, json)?;
        }
        AiCommand::Recall {
            receipt,
            standalone,
        } => {
            let value: serde_json::Value = serde_json::from_slice(&read(receipt, 262144)?)
                .map_err(|_| "invalid receipt JSON")?;
            let receipt: archie_memory::inference::InferenceReceipt =
                serde_json::from_value(value.get("receipt").cloned().unwrap_or(value))
                    .map_err(|_| "invalid receipt JSON")?;
            let result = match daemon(
                paths,
                &ControlRequest::AiRecall {
                    binding: receipt.binding.clone(),
                    receipt_id: receipt.receipt_id.clone(),
                },
                *standalone,
                10000,
            )
            .await?
            {
                Some(ControlResponse::AiResult { outcome }) => *outcome,
                Some(_) => return Err("unexpected Core memory response".into()),
                None => {
                    open(paths, None)
                        .await?
                        .recall(receipt.binding, receipt.receipt_id)
                        .await?
                }
            };
            print_answer(&result, true)?;
        }
        AiCommand::Harness2 {
            domain: value,
            focus,
            task_family,
            run_id,
            prompt_file,
            runner,
            model,
            duration_ms,
            accept_managed_bounds,
            standalone,
            codex,
        } => {
            if runner == "codex" && !*accept_managed_bounds {
                return Err("managed Harness2 requires --accept-managed-bounds (byte/time limits; no hard token cap)".into());
            }
            let request = HarnessRunRequest {
                domain: domain(value)?,
                focus: focus.clone(),
                task_family: task_family.clone(),
                run_id: run_id.clone(),
                prompt: prompt(prompt_file, 16384)?,
                runner: choice(runner, &Some(model.clone()))?,
                duration_ms: *duration_ms,
            };
            let outcome = match daemon(
                paths,
                &ControlRequest::AiHarness {
                    request: request.clone(),
                },
                *standalone,
                (*duration_ms).min(300000),
            )
            .await?
            {
                Some(ControlResponse::AiHarnessResult { outcome }) => *outcome,
                Some(_) => return Err("unexpected Core Harness2 response".into()),
                None => {
                    let program = load_program(
                        &paths.state_dir.join("inference/harness2.toml"),
                        paths.state_dir.join("inference/harness2"),
                        None,
                    )?;
                    let mut runtime = open(
                        paths,
                        if runner == "codex" {
                            Some(client(paths, codex)?)
                        } else {
                            None
                        },
                    )
                    .await?;
                    run_harness(&mut runtime, &program, &request, None).await?
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&outcome)
                    .map_err(|_| "cannot encode Harness2 result")?
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_plan_requires_explicit_bounds_without_claiming_token_enforcement() {
        assert!(budget("codex", Some(100), None).is_err());
        assert!(budget("codex", None, None).is_err());
        assert!(budget("codex", Some(100), Some(100)).is_err());
        assert!(budget("api", None, Some(100)).is_err());
        assert_eq!(
            budget("codex", None, Some(100)).unwrap(),
            InferenceBudget::ManagedByteTime {
                max_output_bytes: 100
            }
        );
    }
    #[tokio::test]
    async fn lost_reply_never_falls_back_to_another_writer_or_turn() {
        let root =
            std::env::temp_dir().join(format!("archie-cli-no-replay-{}", std::process::id()));
        let paths = StatePaths::rooted(&root);
        let listener = endpoint::bind(&paths).await.unwrap();
        let server = tokio::spawn(async move {
            let mut stream = endpoint::accept(&listener).await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut bytes = [0; 1024];
            assert!(stream.read(&mut bytes).await.unwrap() > 0);
        });
        assert!(daemon(&paths, &ControlRequest::Ping, true, 1000)
            .await
            .is_err());
        server.await.unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
