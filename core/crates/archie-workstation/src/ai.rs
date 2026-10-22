//! One attributed text-inference path through Core constraints, router, Vault and
//! PostgreSQL. External processes return proposals; no tool authority is granted.
use crate::{
    ai_policy::{RuntimeAuthority, RuntimeGrant},
    codex::CodexClient,
    live::vault_io,
};
use archie_memory::resume::resume_context;
use archie_memory::{
    cognitive::{CognitiveEvent, CognitivePayload, FocusRecord},
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus, RetainedTaskState},
    inference::{
        sha256, InferenceBinding, InferenceReceipt, InferenceStatus, ObservedUsage,
        OpaqueSessionId, ScopedReference,
    },
    storage::{CheckpointStore, EventStore},
};
use archie_model_runtime::{
    routed::{Question, Routed},
    ChatMessage, ChatRequest, InferenceClient, MessageRole, ProviderEndpoint,
};
use archie_state::StateStore;
use archie_types::{Locality, ModelDescriptor, ModelRole, PrivacyGrade, Timestamp};
use archie_vault::{
    ContextDomain, ContextGateway, ContextRequest, Disclosure, EntryClass, VaultRef,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InferenceBudget {
    #[default]
    StrictTokens,
    ManagedByteTime {
        max_output_bytes: usize,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    /// None for managed protocols without an actual generation-token cap.
    pub generation_token_limit: Option<u32>,
    pub max_output_bytes: usize,
    pub max_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "runner", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunnerChoice {
    Api,
    ApiPinned { model: String },
    Codex { model: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    pub domain: ContextDomain,
    /// Operator's conversation/task label. Authorization comes from the profile.
    pub focus: String,
    pub privacy: PrivacyGrade,
    pub prompt: String,
    pub evidence: Vec<VaultRef>,
    pub runner: RunnerChoice,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub budget: InferenceBudget,
    #[serde(default = "recall_by_default")]
    pub recall_memory: bool,
}
fn recall_by_default() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceOutcome {
    pub receipt: InferenceReceipt,
    /// Returned to the operator, never inserted into verified facts.
    pub text: Option<String>,
    /// Recall never invents historical budget observations.
    #[serde(default)]
    pub budget: Option<BudgetReport>,
}
struct AdmittedContext {
    text: String,
    references: Vec<VaultRef>,
}
struct Prepared {
    head: String,
    policy_revision: String,
    grant: RuntimeGrant,
}

pub struct UnifiedRuntime {
    pub storage: Arc<InferenceStorage>,
    pub catalog: Routed,
    pub codex: Option<CodexClient>,
}
/// Blocking PostgreSQL and filesystem work always runs outside Tokio workers.
pub struct InferenceStorage {
    pub store: Arc<StateStore>,
    pub authority: RuntimeAuthority,
    pub vault_dir: PathBuf,
}
impl InferenceStorage {
    pub fn focus_id(domain: &ContextDomain, label: &str) -> Result<String, String> {
        if label.is_empty()
            || label.len() > 64
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err("invalid Focus label".into());
        }
        Ok(format!(
            "ai:{}",
            sha256(format!("{}\0{label}", domain.path()).as_bytes())
        ))
    }
    fn focus(&self, domain: &ContextDomain, label: &str) -> Result<FocusRecord, String> {
        let (profile, _) = self.authority.profile()?;
        if !profile.allowed_domains.contains(domain) {
            return Err("domain is outside operator scope".into());
        }
        let focus_id = Self::focus_id(domain, label)?;
        let objective_ref = domain
            .reference(&format!(
                "objectives/{}",
                &sha256(focus_id.as_bytes())[..24]
            ))
            .map_err(|_| "invalid objective reference")?
            .to_string();
        if let Some(focus) = self
            .store
            .focus_at(&focus_id, i64::MAX)
            .map_err(|e| e.to_string())?
        {
            if focus.objective_ref != objective_ref || focus.suspended {
                return Err("Focus suspended or objective differs".into());
            }
            return Ok(focus);
        }
        let now = now();
        let focus = FocusRecord {
            focus_id: focus_id.clone(),
            revision: 1,
            objective_ref: objective_ref.clone(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 86400000,
            updated_at_ms: now,
            activations: vec![],
        };
        self.store
            .append_event(&CognitiveEvent {
                schema_version: 1,
                event_id: format!("focus:{}", uuid::Uuid::new_v4()),
                recorded_at_ms: now,
                payload: CognitivePayload::FocusUpdated(focus.clone()),
            })
            .map_err(|e| e.to_string())?;
        self.store
            .append_checkpoint(
                &ExecutionCheckpoint {
                    schema_version: 1,
                    checkpoint_id: format!("root:{}", uuid::Uuid::new_v4()),
                    focus_id,
                    branch_id: "inference-main".into(),
                    parent_checkpoint_id: None,
                    objective_ref,
                    evidence_ref: "core:operator-focus".into(),
                    recorded_at_ms: now,
                    kind: ExecutionKind::Task,
                    status: ExecutionStatus::Current,
                    state: RetainedTaskState::default(),
                },
                None,
            )
            .map_err(|e| e.to_string())?;
        Ok(focus)
    }
}
impl UnifiedRuntime {
    pub async fn execute(
        &mut self,
        request: &InferenceRequest,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<InferenceOutcome, String> {
        self.execute_inner(request, cancel, None).await
    }
    pub async fn execute_for_account(
        &mut self,
        request: &InferenceRequest,
        cancel: Option<watch::Receiver<bool>>,
        expected_account: &str,
    ) -> Result<InferenceOutcome, String> {
        self.execute_inner(request, cancel, Some(expected_account))
            .await
    }
    async fn execute_inner(
        &mut self,
        request: &InferenceRequest,
        cancel: Option<watch::Receiver<bool>>,
        expected_account: Option<&str>,
    ) -> Result<InferenceOutcome, String> {
        let (profile, _) = self.storage.authority.profile()?;
        match (&request.runner, &request.budget) {
            (RunnerChoice::Codex { .. }, InferenceBudget::StrictTokens) =>
                return Err("hard generation-token cap is unsupported by managed Codex; explicitly select managed byte/time limits".into()),
            (RunnerChoice::Codex { .. }, InferenceBudget::ManagedByteTime { max_output_bytes })
                if (1..=65536).contains(max_output_bytes) => {},
            (RunnerChoice::Api | RunnerChoice::ApiPinned { .. }, InferenceBudget::StrictTokens) => {},
            _ => return Err("invalid runner budget mode or byte bound".into()),
        }
        if request.prompt.is_empty()
            || request.prompt.len() > profile.max_prompt_bytes
            || request.evidence.len() > 32
            || request.max_output_tokens == 0
            || request.max_output_tokens > profile.max_output_tokens
            || request.evidence.iter().any(|r| !request.domain.contains(r))
            || request.privacy == PrivacyGrade::Secret
        {
            return Err("invalid inference input, domain or budget".into());
        }
        let owned_request = request.clone();
        let focus = self
            .storage
            .blocking(move |storage| storage.focus(&owned_request.domain, &owned_request.focus))
            .await?;
        let timeout = Duration::from_millis(profile.max_duration_ms);
        let start = Instant::now();
        match &request.runner {
            RunnerChoice::Codex { model } => {
                // A managed subscription is a cloud runner with unknown retention.
                if request.privacy != PrivacyGrade::Public {
                    return Err(
                        "subscription retention is unverified; public context required".into(),
                    );
                }
                let client = self
                    .codex
                    .as_ref()
                    .ok_or("official Codex client is not configured")?
                    .clone();
                let account = client
                    .account()
                    .await
                    .map_err(|e| e.to_string())?
                    .account_id
                    .ok_or("managed account identity unavailable")?;
                let binding = self.storage.binding(
                    &focus,
                    &request.domain,
                    "codex",
                    "openai-managed",
                    &account,
                );
                if expected_account.is_some_and(|expected| expected != binding.account_id) {
                    return Err("managed account differs from frozen admission identity".into());
                }
                let (mut prepared, context) = self
                    .storage
                    .prepare_context(
                        binding.clone(),
                        request.clone(),
                        model.clone(),
                        Locality::Cloud,
                    )
                    .await?;
                let prompt = format!(
                    "{SYSTEM}\n\n{}\n\nCurrent request:\n{}",
                    context.text, request.prompt
                );
                let max_output_bytes = match request.budget {
                    InferenceBudget::ManagedByteTime { max_output_bytes } => max_output_bytes,
                    InferenceBudget::StrictTokens => unreachable!("validated above"),
                };
                let dispatched = self.storage.authority.dispatch(&mut prepared.grant);
                let refused = dispatched.is_err();
                let completed = if let Ok(granted) = dispatched {
                    let remaining = granted.min(timeout.saturating_sub(start.elapsed()));
                    if remaining.is_zero() {
                        Err(crate::codex::CodexError::Timeout)
                    } else {
                        tokio::time::timeout(
                            remaining,
                            client.complete_bounded_for_account(
                                &request.domain,
                                request.privacy,
                                model,
                                &prompt,
                                // Mutable provider threads are never Core snapshots. Only
                                // committed Vault/context evidence crosses this boundary.
                                None,
                                Some(&binding.account_id),
                                max_output_bytes,
                                cancel,
                            ),
                        )
                        .await
                        .map_err(|_| crate::codex::CodexError::Timeout)
                        .and_then(|result| result)
                    }
                } else {
                    Err(crate::codex::CodexError::Timeout)
                };
                let (status, text, session, error) = match completed {
                    Ok(answer)
                        if answer.session.account_id == binding.account_id
                            && answer.session.domain == binding.domain =>
                    {
                        (
                            InferenceStatus::Completed,
                            Some(answer.text),
                            Some(OpaqueSessionId::new(answer.session.thread_id)?),
                            None,
                        )
                    }
                    Err(crate::codex::CodexError::Cancelled) => (
                        InferenceStatus::Cancelled,
                        None,
                        None,
                        Some("cancelled".into()),
                    ),
                    Ok(_) | Err(crate::codex::CodexError::SessionMismatch) => (
                        InferenceStatus::Failed,
                        None,
                        None,
                        Some("managed_account_mismatch".into()),
                    ),
                    Err(crate::codex::CodexError::Timeout) => (
                        InferenceStatus::Failed,
                        None,
                        None,
                        Some(
                            if refused {
                                "dispatch_refused"
                            } else {
                                "deadline_expired"
                            }
                            .into(),
                        ),
                    ),
                    Err(_) => (
                        InferenceStatus::Failed,
                        None,
                        None,
                        Some("managed_client_failed".into()),
                    ),
                };
                self.storage
                    .commit(Attempt {
                        binding,
                        model: model.clone(),
                        prompt,
                        request: request.clone(),
                        prepared,
                        evidence: context.references,
                        status,
                        text,
                        session,
                        error,
                        usage: ObservedUsage {
                            duration_ms: Some(start.elapsed().as_millis() as u64),
                            ..Default::default()
                        },
                    })
                    .await
            }
            RunnerChoice::Api | RunnerChoice::ApiPinned { .. } => {
                // A pinned model is asked in the role its manifest offers: one that only
                // reasons (Bonsai with its reasoning on) is a reasoning worker. The router
                // still applies the privacy, locality and context gates to it.
                let role = match &request.runner {
                    RunnerChoice::ApiPinned { model } => model
                        .parse()
                        .ok()
                        .and_then(|id| self.catalog.registry().get(&id))
                        .filter(|descriptor| {
                            !descriptor.provides(&ModelRole::CognitiveResponse.capability())
                                && descriptor.provides(&ModelRole::ReasoningWorker.capability())
                        })
                        .map_or(ModelRole::CognitiveResponse, |_| ModelRole::ReasoningWorker),
                    _ => ModelRole::CognitiveResponse,
                };
                let question = Question {
                    role,
                    privacy: request.privacy,
                    system: SYSTEM,
                    user: &request.prompt,
                    image: None,
                    max_tokens: request.max_output_tokens,
                    timeout,
                };
                let candidates = self
                    .catalog
                    .candidates(&question)
                    .map_err(|e| e.to_string())?;
                let mut last = None;
                let mut too_large = false;
                let candidates: Vec<_> = candidates
                    .into_iter()
                    .filter(|id| match &request.runner {
                        RunnerChoice::ApiPinned { model } => id.as_str() == model,
                        _ => true,
                    })
                    .collect();
                for id in candidates.into_iter().take(4) {
                    if start.elapsed() >= timeout || cancel.as_ref().is_some_and(|c| *c.borrow()) {
                        break;
                    }
                    let descriptor = self
                        .catalog
                        .registry()
                        .get(&id)
                        .ok_or("selected model disappeared")?
                        .clone();
                    let provider = descriptor
                        .labels
                        .get("provider-id")
                        .cloned()
                        .unwrap_or_else(|| descriptor.provider.as_str().to_owned());
                    let account = api_account(&descriptor);
                    let binding =
                        self.storage
                            .binding(&focus, &request.domain, "api", &provider, &account);
                    if expected_account.is_some_and(|expected| expected != binding.account_id) {
                        return Err(
                            "provider account differs from frozen admission identity".into()
                        );
                    }
                    let (mut prepared, context) = self
                        .storage
                        .prepare_context(
                            binding.clone(),
                            request.clone(),
                            id.as_str().into(),
                            descriptor.locality,
                        )
                        .await?;
                    let prompt =
                        format!("{}\n\nCurrent request:\n{}", context.text, request.prompt);
                    let mut wire = ChatRequest {
                        model: archie_model_runtime::provider_model_id(&descriptor)
                            .map_err(|e| e.to_string())?
                            .into(),
                        messages: vec![
                            ChatMessage {
                                role: MessageRole::System,
                                content: SYSTEM.into(),
                            },
                            ChatMessage {
                                role: MessageRole::User,
                                content: prompt.clone(),
                            },
                        ],
                        max_tokens: Some(request.max_output_tokens),
                        temperature: None,
                        stream: false,
                    };
                    // Count the full recalled capsule, not only the fresh question.
                    // A small model must not block a larger fallback candidate.
                    if !descriptor.fits_context(
                        ((SYSTEM.len() + prompt.len()) / 3 + 1).min(u32::MAX as usize) as u32,
                        request.max_output_tokens,
                    ) {
                        too_large = true;
                        continue;
                    }
                    let granted = self
                        .storage
                        .authority
                        .dispatch(&mut prepared.grant)
                        .unwrap_or(Duration::ZERO);
                    let remaining = granted.min(timeout.saturating_sub(start.elapsed()));
                    let outcome = if remaining.is_zero() {
                        (
                            InferenceStatus::Failed,
                            None,
                            Some("original_lease_expired".into()),
                            None,
                        )
                    } else {
                        let endpoint = ProviderEndpoint::for_model(&descriptor)
                            .map_err(|e| e.to_string())?
                            .with_limits(remaining, 2 * 1024 * 1024)
                            .map_err(|e| e.to_string())?;
                        let client = InferenceClient::new(endpoint).map_err(|e| e.to_string())?;
                        wire.model = archie_model_runtime::provider_model_id(&descriptor)
                            .map_err(|e| e.to_string())?
                            .into();
                        let remaining = prepared
                            .grant
                            .remaining()
                            .unwrap_or(Duration::ZERO)
                            .min(timeout.saturating_sub(start.elapsed()));
                        if remaining.is_zero() {
                            (
                                InferenceStatus::Failed,
                                None,
                                Some("original_lease_expired".into()),
                                None,
                            )
                        } else {
                            let operation = client.chat(&descriptor, request.privacy, &wire);
                            tokio::select! {
                                result=tokio::time::timeout(remaining,operation)=>match result {
                                    Ok(Ok(reply))=> {
                                        let text=reply.choices.first().map(|c|c.message.content.clone()).filter(|s|!s.is_empty());
                                        if let Some(text)=text {(InferenceStatus::Completed,Some(text),None,reply.usage.map(|u|(u.prompt_tokens,u.completion_tokens)))}
                                        else {(InferenceStatus::Failed,None,Some("empty_response".into()),None)}
                                    },
                                    Ok(Err(error))=>(InferenceStatus::Failed,None,Some(provider_error_code(&error).into()),None),
                                    Err(_)=>(InferenceStatus::Failed,None,Some("deadline_expired".into()),None),
                                },
                                ()=wait_cancel(cancel.clone())=>(InferenceStatus::Cancelled,None,Some("cancelled".into()),None),
                            }
                        }
                    };
                    let result = self
                        .storage
                        .commit(Attempt {
                            binding,
                            model: id.as_str().into(),
                            prompt,
                            request: request.clone(),
                            prepared,
                            evidence: context.references,
                            status: outcome.0,
                            text: outcome.1,
                            session: None,
                            error: outcome.2,
                            usage: ObservedUsage {
                                input_tokens: outcome.3.map(|u| u.0),
                                output_tokens: outcome.3.map(|u| u.1),
                                duration_ms: Some(start.elapsed().as_millis() as u64),
                            },
                        })
                        .await?;
                    if result.receipt.status != InferenceStatus::Failed {
                        return Ok(result);
                    }
                    last = Some(result);
                }
                last.ok_or_else(|| {
                    if too_large {
                        "assembled context exceeds every candidate model budget".into()
                    } else {
                        "no model attempt completed within its budget".into()
                    }
                })
            }
        }
    }

    pub async fn recall(
        &self,
        binding: InferenceBinding,
        receipt_id: String,
    ) -> Result<InferenceOutcome, String> {
        self.storage
            .blocking(move |storage| storage.recall(&binding, &receipt_id))
            .await
    }

    pub async fn prepare_harness_focus(
        &self,
        domain: ContextDomain,
        focus: String,
    ) -> Result<(FocusRecord, String), String> {
        self.storage
            .blocking(move |storage| {
                let focus = storage.focus(&domain, &focus)?;
                let (_, revision) = storage.authority.profile()?;
                Ok((focus, revision))
            })
            .await
    }
}
struct Attempt {
    binding: InferenceBinding,
    model: String,
    prompt: String,
    request: InferenceRequest,
    prepared: Prepared,
    evidence: Vec<VaultRef>,
    status: InferenceStatus,
    text: Option<String>,
    session: Option<OpaqueSessionId>,
    error: Option<String>,
    usage: ObservedUsage,
}
impl InferenceStorage {
    /// One OS file lock coordinates every runtime/process sharing this Vault.
    /// It includes key creation and is held through filesystem verification and
    /// PostgreSQL commit; a whole-snapshot writer cannot erase another receipt.
    fn lock_vault(&self) -> Result<std::fs::File, String> {
        std::fs::create_dir_all(&self.vault_dir).map_err(|_| "cannot create runtime Vault")?;
        if !std::fs::symlink_metadata(&self.vault_dir)
            .map_err(|_| "runtime Vault unavailable")?
            .is_dir()
        {
            return Err("runtime Vault must be a real directory".into());
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(self.vault_dir.join(".inference.lock"))
            .map_err(|_| "cannot open runtime Vault lock")?;
        let wait = Duration::from_millis(self.authority.profile()?.0.max_duration_ms.min(5000));
        let started = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < wait =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => return Err("runtime Vault lock unavailable or timed out".into()),
            }
        }
    }
    async fn blocking<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Self) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || work(&storage))
            .await
            .map_err(|_| "storage worker failed")?
    }
    async fn prepare_context(
        self: &Arc<Self>,
        binding: InferenceBinding,
        request: InferenceRequest,
        model: String,
        locality: Locality,
    ) -> Result<(Prepared, AdmittedContext), String> {
        self.blocking(move |storage| {
            let prepared = storage.prepare(&binding, request.privacy, &model)?;
            let context = storage.context(&binding, &request, locality)?;
            Ok((prepared, context))
        })
        .await
    }
    async fn commit(self: &Arc<Self>, attempt: Attempt) -> Result<InferenceOutcome, String> {
        self.blocking(move |storage| {
            storage.finish(
                &attempt.binding,
                &attempt.model,
                &attempt.prompt,
                &attempt.request,
                &attempt.prepared,
                &attempt.evidence,
                &attempt.status,
                attempt.text,
                attempt.session,
                attempt.error,
                attempt.usage,
            )
        })
        .await
    }
    fn binding(
        &self,
        focus: &FocusRecord,
        domain: &ContextDomain,
        runner: &str,
        provider: &str,
        account: &str,
    ) -> InferenceBinding {
        InferenceBinding {
            schema_version: 1,
            domain: domain.clone(),
            focus_id: focus.focus_id.clone(),
            task_id: uuid::Uuid::new_v4().to_string(),
            objective_ref: focus.objective_ref.clone(),
            runner_id: runner.into(),
            provider_id: provider.into(),
            account_id: account.into(),
        }
    }
    fn prepare(
        &self,
        binding: &InferenceBinding,
        privacy: PrivacyGrade,
        model: &str,
    ) -> Result<Prepared, String> {
        let now = now();
        let grant = self.authority.admit_grant(binding, privacy, model, now)?;
        let resumed = resume_context(self.store.as_ref(), &self.authority, &binding.focus_id, now)?;
        self.store
            .bind_inference(binding)
            .map_err(|e| e.to_string())?;
        Ok(Prepared {
            head: resumed.execution.checkpoint_id,
            policy_revision: grant.pinned.policy_revision_ref.clone(),
            grant,
        })
    }
    fn context(
        &self,
        binding: &InferenceBinding,
        request: &InferenceRequest,
        locality: Locality,
    ) -> Result<AdmittedContext, String> {
        let _vault_lock = self.lock_vault()?;
        let mut references = request.evidence.clone();
        if request.recall_memory {
            for receipt in self
                .store
                .recent_session_inferences(binding, 8)
                .map_err(|e| e.to_string())?
            {
                if let Some(result) = receipt.result_ref {
                    if !references.contains(&result.reference) {
                        references.push(result.reference);
                    }
                }
            }
        }
        let mut vault =
            vault_io::open_vault(&self.vault_dir).map_err(|_| "cannot open runtime Vault")?;
        let entries = vault.sealed_entries();
        for reference in &references {
            if !binding.domain.contains(reference) {
                return Err("recalled evidence crosses domain".into());
            }
            let entry = entries
                .iter()
                .find(|e| e.reference == *reference)
                .ok_or("recalled evidence is missing")?;
            if entry.grade > request.privacy {
                return Err("recalled evidence exceeds request privacy grade".into());
            }
        }
        let mut context_request = ContextRequest::new(
            &binding.task_id,
            &binding.task_id,
            "attributed text inference",
            locality,
            600000,
        )
        .in_domain(binding.domain.clone());
        context_request.references = references.clone();
        let capsule = ContextGateway::new()
            .assemble(&mut vault, &context_request, now())
            .map_err(|_| "Vault refused scoped context")?;
        let rendered = capsule
            .render(&binding.task_id, now())
            .map_err(|_| "context capsule expired")?;
        if rendered.len() + request.prompt.len() > self.authority.profile()?.0.max_prompt_bytes {
            return Err("assembled memory exceeds input budget".into());
        }
        Ok(AdmittedContext {
            text: rendered,
            references,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        binding: &InferenceBinding,
        model: &str,
        prompt: &str,
        request: &InferenceRequest,
        prepared: &Prepared,
        evidence: &[VaultRef],
        status: &InferenceStatus,
        mut text: Option<String>,
        mut session: Option<OpaqueSessionId>,
        mut error: Option<String>,
        usage: ObservedUsage,
    ) -> Result<InferenceOutcome, String> {
        let _vault_lock = self.lock_vault()?;
        let now = now();
        let mut status = *status;
        if prepared.grant.remaining().is_err() {
            status = InferenceStatus::Failed;
            text = None;
            session = None;
            error = Some("original_lease_expired".into());
        }
        // A valid but oversized answer is a durable failed attempt, never an
        // unattributed abort that skips the receipt and the next candidate.
        if text.as_ref().is_some_and(|t| t.len() > 262144) {
            status = InferenceStatus::Failed;
            text = None;
            session = None;
            error = Some("output_exceeds_result_budget".into());
        }
        // A changed profile, paused Focus or emergency pause can never publish a
        // newly successful recalled answer merely because inference completed.
        let pinned = self.authority.admit(binding, request.privacy, model, now)?;
        let resumed = resume_context(self.store.as_ref(), &self.authority, &binding.focus_id, now)?;
        if resumed.execution.checkpoint_id != prepared.head
            || pinned.policy_revision_ref != prepared.policy_revision
        {
            return Err("Focus or policy changed while inference was running".into());
        }
        let receipt_id = uuid::Uuid::new_v4().to_string();
        let mut result_ref = None;
        let mut output_sha256 = None;
        if let Some(text) = &text {
            if text.is_empty() {
                return Err("empty model output cannot be sealed".into());
            }
            let reference = binding
                .domain
                .reference(&format!("inference/{receipt_id}"))
                .map_err(|_| "invalid output reference")?;
            let mut vault =
                vault_io::open_vault(&self.vault_dir).map_err(|_| "cannot open runtime Vault")?;
            vault
                .store(
                    "unverified model proposal",
                    reference.clone(),
                    EntryClass::PrivateMemory,
                    request.privacy,
                    text,
                    now,
                    None,
                )
                .map_err(|_| "cannot seal model proposal")?;
            vault_io::persist_vault(&self.vault_dir, &vault)
                .map_err(|_| "cannot persist sealed proposal")?;
            // Each open holds the Vault's own transaction lock until dropped, so
            // release the writer before reopening from disk in this same process.
            drop(vault);
            // Verify actual disk artifact before committing its authority-free receipt.
            let verified = {
                let mut reopened = vault_io::open_vault(&self.vault_dir)
                    .map_err(|_| "cannot reopen sealed proposal")?;
                matches!(
                    reopened
                        .disclose(
                            "verify output artifact",
                            &reference,
                            PrivacyGrade::Restricted,
                            now,
                        )
                        .map_err(|_| "cannot verify sealed proposal")?,
                    Disclosure::Content { text: restored, .. } if restored == *text
                )
            };
            if !verified {
                self.remove_staged(&reference, "unverifiable staged output")?;
                return Err("sealed proposal integrity mismatch".into());
            }
            output_sha256 = Some(sha256(text.as_bytes()));
            result_ref = Some(ScopedReference {
                reference,
                account_id: binding.account_id.clone(),
            });
        }
        let mut receipt = InferenceReceipt {
            schema_version: 1,
            receipt_id,
            checkpoint_id: format!("inference-checkpoint:{}", uuid::Uuid::new_v4()),
            binding: binding.clone(),
            session_id: session,
            model_id: model.into(),
            request_sha256: sha256(prompt.as_bytes()),
            output_sha256,
            admitted_evidence_refs: evidence
                .iter()
                .cloned()
                .map(|reference| ScopedReference {
                    reference,
                    account_id: binding.account_id.clone(),
                })
                .collect(),
            result_ref,
            status,
            error_code: error,
            observed_at_ms: now,
            usage,
        };
        let mut checkpoint = ExecutionCheckpoint {
            schema_version: 1,
            checkpoint_id: receipt.checkpoint_id.clone(),
            focus_id: binding.focus_id.clone(),
            branch_id: resumed.execution.branch_id.clone(),
            parent_checkpoint_id: Some(resumed.execution.checkpoint_id.clone()),
            objective_ref: binding.objective_ref.clone(),
            evidence_ref: receipt.provenance_ref(),
            recorded_at_ms: now,
            kind: if status == InferenceStatus::Completed {
                ExecutionKind::Result
            } else {
                ExecutionKind::Attempt
            },
            status: ExecutionStatus::Current,
            state: RetainedTaskState {
                artifact_refs: receipt
                    .result_ref
                    .iter()
                    .map(|r| r.reference.to_string())
                    .collect(),
                ..Default::default()
            },
        };
        let expected = Some(resumed.execution.checkpoint_id.as_str());
        if receipt.status == InferenceStatus::Completed {
            let committed = self.store.commit_inference_before(
                &receipt,
                &checkpoint,
                expected,
                prepared.grant.expires_at_ms(),
            );
            match committed {
                Ok(()) => {}
                Err(_) if prepared.grant.remaining().is_err() => {
                    // The guarded SQL transaction rolled back. Remove only this
                    // attempt's staged output, retain a failed audit checkpoint,
                    // and never renew the original model grant at completion.
                    if let Some(result) = &receipt.result_ref {
                        self.remove_staged(&result.reference, "expired inference attempt")?;
                    }
                    receipt.status = InferenceStatus::Failed;
                    receipt.error_code = Some("original_lease_expired".into());
                    receipt.result_ref = None;
                    receipt.output_sha256 = None;
                    receipt.session_id = None;
                    receipt.observed_at_ms = Timestamp::now().as_millis();
                    checkpoint.recorded_at_ms = receipt.observed_at_ms;
                    checkpoint.kind = ExecutionKind::Attempt;
                    checkpoint.state = RetainedTaskState::default();
                    text = None;
                    self.store
                        .commit_inference(&receipt, &checkpoint, expected)
                        .map_err(|e| e.to_string())?;
                }
                Err(error) => {
                    // The transaction rolled back (CAS loss, storage error): the
                    // staged output can never be recalled, so it must not linger.
                    if let Some(result) = &receipt.result_ref {
                        self.remove_staged(&result.reference, "uncommitted inference attempt")?;
                    }
                    return Err(error.to_string());
                }
            }
        } else {
            self.store
                .commit_inference(&receipt, &checkpoint, expected)
                .map_err(|e| e.to_string())?;
        }
        Ok(InferenceOutcome {
            receipt,
            text,
            budget: Some(BudgetReport {
                generation_token_limit: if request.budget == InferenceBudget::StrictTokens {
                    Some(request.max_output_tokens)
                } else {
                    None
                },
                max_output_bytes: match request.budget {
                    InferenceBudget::ManagedByteTime { max_output_bytes } => max_output_bytes,
                    InferenceBudget::StrictTokens => 262144,
                },
                max_duration_ms: prepared.grant.duration_ms(),
            }),
        })
    }

    /// Caller holds the Vault lock.
    fn remove_staged(&self, reference: &VaultRef, reason: &str) -> Result<(), String> {
        let mut vault = vault_io::open_vault(&self.vault_dir)
            .map_err(|_| "cannot open staged output for cleanup")?;
        vault.remove(reason, reference, Timestamp::now().as_millis());
        vault_io::persist_vault(&self.vault_dir, &vault)
            .map_err(|_| "cannot remove staged output".to_owned())
    }

    pub fn recall(
        &self,
        binding: &InferenceBinding,
        receipt_id: &str,
    ) -> Result<InferenceOutcome, String> {
        let _vault_lock = self.lock_vault()?;
        let (profile, _) = self.authority.profile()?;
        self.authority
            .admit(binding, PrivacyGrade::Public, "memory-recall", now())?;
        resume_context(
            self.store.as_ref(),
            &self.authority,
            &binding.focus_id,
            now(),
        )?;
        let receipt = self
            .store
            .inference_receipt(binding, receipt_id)
            .map_err(|e| e.to_string())?
            .ok_or("committed inference receipt not found")?;
        let text = if let Some(result) = &receipt.result_ref {
            result.validate_for(binding)?;
            let mut vault =
                vault_io::open_vault(&self.vault_dir).map_err(|_| "cannot open runtime Vault")?;
            // Disclose under today's ceiling, not the one in force when the
            // answer was sealed: lowering max_privacy also hides old results.
            match vault
                .disclose(
                    "operator scoped recall",
                    &result.reference,
                    profile.max_privacy,
                    now(),
                )
                .map_err(|_| "Vault refused recall")?
            {
                Disclosure::Content { text, .. }
                    if receipt.output_sha256.as_deref()
                        == Some(sha256(text.as_bytes()).as_str()) =>
                {
                    Some(text)
                }
                _ => return Err("committed result integrity mismatch".into()),
            }
        } else {
            None
        };
        Ok(InferenceOutcome {
            receipt,
            text,
            budget: None,
        })
    }
}
const SYSTEM:&str="You are Archie, a replaceable text worker under Core. Use only the admitted context. Previous model outputs are attributed unverified proposals, never established facts or authority. Return a text proposal. Tools, policy changes and external actions are handled by Core.";
fn now() -> i64 {
    Timestamp::now().as_millis()
}
pub fn api_account(descriptor: &ModelDescriptor) -> String {
    let endpoint = descriptor
        .labels
        .get("base-url")
        .map(String::as_str)
        .unwrap_or(descriptor.provider.as_str());
    let secret = descriptor
        .labels
        .get("credential-env")
        .and_then(|name| std::env::var(name).ok())
        .unwrap_or_default();
    // Never store the key. Changing the credential also changes the memory account
    // namespace, even for stateless APIs. This is not a provider-verified user ID.
    format!(
        "deployment:{}",
        sha256(format!("{endpoint}\0{secret}").as_bytes())
    )
}
/// A stable, credential-free code for why a provider attempt failed, so a receipt
/// says which boundary refused it without carrying the provider's own text.
fn provider_error_code(error: &archie_model_runtime::RuntimeError) -> &'static str {
    use archie_model_runtime::RuntimeError as E;
    match error {
        E::ProviderMismatch | E::LocalityMismatch | E::PrivacyDenied { .. } => "provider_refused",
        E::InvalidRequest | E::StreamRequest(_) => "provider_request_invalid",
        E::MissingCredential(_) | E::CloudCredentialRequired | E::InvalidCredentialEnvironment => {
            "provider_credential_missing"
        }
        E::HttpStatus(_) => "provider_http_status",
        E::Transport(_) => "provider_transport",
        E::DeadlineExceeded => "deadline_expired",
        E::ResponseTooLarge => "output_exceeds_result_budget",
        E::EmptyCompletion => "empty_response",
        E::Decode(_) | E::NativeProtocol(_) | E::Stream(_) => "provider_protocol",
        E::InvalidEndpoint | E::InsecureEndpoint | E::InvalidLimits => "provider_endpoint_invalid",
    }
}

async fn wait_cancel(mut cancel: Option<watch::Receiver<bool>>) {
    if let Some(receiver) = &mut cancel {
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    } else {
        std::future::pending::<()>().await;
    }
}
