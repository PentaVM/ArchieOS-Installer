//! Bounded process boundary for the pinned Google Harness² worker.
//! The supplied handler is trusted Core code: it must freshly admit each stage
//! before using the ordinary router/runtime. This module grants no authority.
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdout, Command};
use tokio::time::{timeout_at, Instant};

pub const UPSTREAM_COMMIT: &str = "1cfe254e8d25b15f37048c3730134686abe60335";
const MAX_FRAME: u64 = 1_048_576;
const FILES: [&str; 4] = [
    "LongTermMEMORY.md",
    "guardrails.md",
    "systemprompt.md",
    "tool_descriptions/tool_guidance.md",
];

/// Trusted launch configuration. Arguments precede the worker path. Windows WSL
/// callers explicitly configure Linux paths and a non-login `env -i` launcher.
#[derive(Debug, Clone)]
pub struct WorkerProgram {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub worker: PathBuf,
    pub state_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub reference: String,
    pub sha256: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness2Limits {
    pub max_calls: u64,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    /// None means monetary bounds are unsupported; token/call/time bounds remain.
    pub max_cost_micros: Option<u64>,
    pub per_call_input_tokens: u64,
    pub per_call_output_tokens: u64,
    pub per_call_cost_micros: Option<u64>,
    pub max_frame_bytes: u64,
    pub deadline_unix_ms: u64,
}

/// An already admitted task. No endpoint, credential or execution lease travels
/// to the worker. The model/config digest includes the parent's account binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness2Task {
    pub run_id: String,
    pub task_id: String,
    pub task_family: String,
    pub domain: ContextDomain,
    pub focus_id: String,
    pub objective_ref: String,
    pub policy_revision_ref: String,
    pub model_identity_sha256: String,
    pub input_sha256: String,
    pub task: String,
    pub evidence: Vec<Evidence>,
    pub base_harness: BTreeMap<String, String>,
    pub limits: Harness2Limits,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub run_id: String,
    pub task_id: String,
    pub domain: ContextDomain,
    pub focus_id: String,
    pub objective_ref: String,
    pub policy_revision_ref: String,
    pub stage: String,
    pub role: String,
    pub response_format: String,
    pub prompt: String,
    pub input_sha256: String,
    pub reserved_input_tokens: u64,
    pub max_output_tokens: u64,
    pub reserved_cost_micros: Option<u64>,
    pub evidence_refs: Vec<String>,
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_micros: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StageReply {
    pub text: String,
    pub usage: StageUsage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub run_id: String,
    pub task_id: String,
    pub domain: ContextDomain,
    pub policy_revision_ref: String,
    pub identity_sha256: String,
    pub base_artifact: String,
    pub chosen_artifact: String,
    pub harness: BTreeMap<String, String>,
    pub experience: BTreeMap<String, String>,
    pub base_artifact_sha256: String,
    pub chosen_artifact_sha256: String,
    pub harness_sha256: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PromotionDecision {
    pub approved: bool,
    pub reason_code: String,
}

pub type StageFuture<'a> = Pin<Box<dyn Future<Output = Result<StageReply, String>> + Send + 'a>>;
pub type PromotionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PromotionDecision, String>> + Send + 'a>>;

/// Returning a stage reply asserts fresh authorization under the supplied
/// domain/Focus/objective/Policy, plus enforcement by the selected runtime.
/// Every retry/reflection is a distinct request and needs the same admission.
pub trait Harness2Handler: Send {
    fn infer<'a>(&'a mut self, request: &'a StageRequest) -> StageFuture<'a>;

    /// Promotion has no automatic authority. A trusted assessor may override this
    /// to check the complete harness/lesson and baseline/final artifacts.
    fn promote<'a>(&'a mut self, _request: &'a PromotionRequest) -> PromotionFuture<'a> {
        Box::pin(async {
            Ok(PromotionDecision {
                approved: false,
                reason_code: "no_trusted_assessor".into(),
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CumulativeUsage {
    pub calls: u64,
    pub charged_input_tokens: u64,
    pub charged_output_tokens: u64,
    pub charged_cost_micros: Option<u64>,
    pub observed_input_tokens: Option<u64>,
    pub observed_output_tokens: Option<u64>,
    pub observed_cost_micros: Option<u64>,
}

impl Default for CumulativeUsage {
    fn default() -> Self {
        Self {
            calls: 0,
            charged_input_tokens: 0,
            charged_output_tokens: 0,
            charged_cost_micros: Some(0),
            observed_input_tokens: Some(0),
            observed_output_tokens: Some(0),
            observed_cost_micros: Some(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub chosen_label: String,
    pub chosen_run_id: String,
    pub trivial: bool,
    pub ok: bool,
    pub harness_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promotion {
    pub status: String,
    pub reason_code: String,
    pub improvement_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryEntry {
    pub reference: String,
    pub manifest_path: String,
    pub sha256: String,
    pub harness_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryResult {
    pub committed: Vec<LibraryEntry>,
    pub reused_tasks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness2Result {
    pub protocol_version: u32,
    pub status: String,
    pub upstream_commit: String,
    pub identity_sha256: String,
    pub run_id: String,
    pub task_id: String,
    pub domain: ContextDomain,
    pub ledger: Value,
    pub usage: CumulativeUsage,
    pub selection: Selection,
    pub promotion: Promotion,
    pub library: LibraryResult,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerFailure {
    protocol_version: u32,
    status: String,
    error_code: String,
    upstream_commit: String,
    usage: Option<CumulativeUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ResultMessage {
    Success(Box<Harness2Result>),
    Failure(WorkerFailure),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerMessage {
    Inference(StageRequest),
    Promotion(PromotionRequest),
    Result(ResultMessage),
}

#[derive(Debug, thiserror::Error)]
pub enum Harness2Error {
    #[error("invalid Harness² task")]
    InvalidRequest,
    #[error("invalid trusted Harness² launch configuration")]
    InvalidProgram,
    #[error("Harness² process I/O failed")]
    Io(#[from] std::io::Error),
    #[error("Harness² worker protocol was refused")]
    Protocol,
    #[error("Harness² aggregate budget was exhausted")]
    Budget,
    #[error("Harness² total deadline expired")]
    Deadline,
    #[error("the trusted Harness² handler refused the stage")]
    Handler,
    #[error("Harness² worker failed: {code}")]
    Worker {
        code: String,
        usage: Option<CumulativeUsage>,
    },
}

impl Harness2Error {
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidProgram => "invalid_program",
            Self::Io(_) => "process_io",
            Self::Protocol => "protocol_violation",
            Self::Budget => "budget_exhausted",
            Self::Deadline => "deadline_exceeded",
            Self::Handler => "handler_refused",
            Self::Worker { code, .. } => code,
        }
    }
}

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

fn reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
}

fn valid_tree(tree: &BTreeMap<String, String>, base: Option<&BTreeMap<String, String>>) -> bool {
    tree.keys().map(String::as_str).eq(FILES)
        && tree
            .values()
            .all(|v| !v.trim().is_empty() && !v.contains('\0'))
        && tree.values().map(String::len).sum::<usize>() <= 24_000
        && base.is_none_or(|b| tree.get("guardrails.md") == b.get("guardrails.md"))
}

fn now_ms() -> Result<u64, Harness2Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Harness2Error::Deadline)?
        .as_millis()
        .try_into()
        .map_err(|_| Harness2Error::Deadline)
}

fn validate_task(task: &Harness2Task) -> Result<(), Harness2Error> {
    let l = &task.limits;
    let ids = [&task.run_id, &task.task_id, &task.task_family];
    let refs = [
        &task.focus_id,
        &task.objective_ref,
        &task.policy_revision_ref,
    ];
    if !ids.into_iter().all(|s| segment(s))
        || !refs.into_iter().all(|s| reference(s))
        || !hash(&task.model_identity_sha256)
        || task.input_sha256 != sha(&task.task)
        || task.task.is_empty()
        || task.task.len() > 32_768
        || !(1..=64).contains(&task.evidence.len())
        || task
            .evidence
            .iter()
            .any(|e| !reference(&e.reference) || e.text.len() > 32_768 || e.sha256 != sha(&e.text))
        || task
            .evidence
            .iter()
            .map(|e| &e.reference)
            .collect::<BTreeSet<_>>()
            .len()
            != task.evidence.len()
        || !valid_tree(&task.base_harness, None)
        || !(1..=128).contains(&l.max_calls)
        || !(4096..=MAX_FRAME).contains(&l.max_frame_bytes)
        || !(1..=500_000).contains(&l.per_call_input_tokens)
        || !(1..=32_768).contains(&l.per_call_output_tokens)
        || [l.max_input_tokens, l.max_output_tokens]
            .into_iter()
            .any(|n| n == 0 || n > (1u64 << 53) - 1)
        || l.max_cost_micros.is_some() != l.per_call_cost_micros.is_some()
        || [l.max_cost_micros, l.per_call_cost_micros]
            .into_iter()
            .flatten()
            .any(|n| n == 0 || n > (1u64 << 53) - 1)
    {
        return Err(Harness2Error::InvalidRequest);
    }
    if l.deadline_unix_ms <= now_ms()? {
        return Err(Harness2Error::Deadline);
    }
    Ok(())
}

fn sorted_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> =
                map.into_iter().map(|(k, v)| (k, sorted_json(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted_json).collect()),
        other => other,
    }
}

fn canonical(value: Value) -> Result<String, Harness2Error> {
    serde_json::to_string(&sorted_json(value)).map_err(|_| Harness2Error::Protocol)
}

fn identity(task: &Harness2Task) -> Result<String, Harness2Error> {
    let mut value = serde_json::to_value(task).map_err(|_| Harness2Error::InvalidRequest)?;
    value["limits"]
        .as_object_mut()
        .ok_or(Harness2Error::Protocol)?
        .remove("deadline_unix_ms");
    value["upstream_commit"] = UPSTREAM_COMMIT.into();
    Ok(sha(&canonical(value)?))
}

struct FrameReader {
    output: ChildStdout,
    pending: Vec<u8>,
    maximum: usize,
}

impl FrameReader {
    async fn next(&mut self) -> Result<Vec<u8>, Harness2Error> {
        loop {
            if let Some(index) = self.pending.iter().position(|b| *b == b'\n') {
                if index > self.maximum {
                    return Err(Harness2Error::Protocol);
                }
                let line = self.pending.drain(..=index).collect::<Vec<_>>();
                return Ok(line[..line.len() - 1].to_vec());
            }
            if self.pending.len() > self.maximum {
                return Err(Harness2Error::Protocol);
            }
            let mut chunk = [0u8; 4096];
            let count = self.output.read(&mut chunk).await?;
            if count == 0 {
                return Err(Harness2Error::Protocol);
            }
            self.pending.extend_from_slice(&chunk[..count]);
        }
    }
}

fn within(used: u64, reserve: u64, maximum: u64) -> bool {
    used.checked_add(reserve)
        .is_some_and(|total| total <= maximum)
}

fn within_monetary(used: Option<u64>, reserve: Option<u64>, maximum: Option<u64>) -> bool {
    match (used, reserve, maximum) {
        (Some(used), Some(reserve), Some(maximum)) => within(used, reserve, maximum),
        (None, None, None) => true,
        _ => false,
    }
}

fn settle(
    charged: &mut u64,
    observed: &mut Option<u64>,
    reserve: u64,
    actual: Option<u64>,
) -> Result<(), Harness2Error> {
    if let Some(amount) = actual {
        if amount > reserve {
            return Err(Harness2Error::Budget);
        }
        *charged -= reserve - amount;
        *observed = observed.and_then(|old| old.checked_add(amount));
    } else {
        *observed = None;
    }
    Ok(())
}

struct RunState<'a> {
    task: &'a Harness2Task,
    identity: String,
    seen: BTreeSet<String>,
    usage: CumulativeUsage,
    promotion: Option<PromotionDecision>,
    promoted_harness_sha256: Option<String>,
}

impl RunState<'_> {
    fn references(&self) -> Vec<String> {
        self.task
            .evidence
            .iter()
            .map(|e| e.reference.clone())
            .collect()
    }

    fn admit_stage(&mut self, r: &StageRequest) -> Result<(), Harness2Error> {
        let t = self.task;
        let l = &t.limits;
        let (role, form, dependencies): (&str, &str, &[&str]) = match r.stage.as_str() {
            "base1" => ("solver", "artifact", &[]),
            "inv1.a1" | "dom1.a1" => ("proposer", "harness", &["base1"]),
            "inv1" => ("solver", "artifact", &["inv1.a1"]),
            "dom1" => ("solver", "artifact", &["dom1.a1"]),
            "c1.a1" => ("proposer", "harness", &["inv1", "dom1"]),
            "c1" => ("solver", "artifact", &["c1.a1"]),
            "experience.seq1" => ("reflection", "experience", &["c1"]),
            _ => return Err(Harness2Error::Protocol),
        };
        if self.promotion.is_some()
            || r.protocol_version != 1
            || r.run_id != t.run_id
            || r.task_id != t.task_id
            || r.domain != t.domain
            || r.focus_id != t.focus_id
            || r.objective_ref != t.objective_ref
            || r.policy_revision_ref != t.policy_revision_ref
            || r.input_sha256 != sha(&r.prompt)
            || r.request_id != format!("stage-{}", self.usage.calls + 1)
            || r.evidence_refs != self.references()
            || r.deadline_unix_ms != l.deadline_unix_ms
            || r.role != role
            || r.response_format != form
            || self.seen.contains(&r.stage)
            || dependencies.iter().any(|d| !self.seen.contains(*d))
            || r.reserved_input_tokens != r.prompt.len() as u64 + 256
            || r.reserved_input_tokens > l.per_call_input_tokens
            || r.max_output_tokens != l.per_call_output_tokens
            || r.reserved_cost_micros != l.per_call_cost_micros
        {
            return Err(Harness2Error::Protocol);
        }
        if self.usage.calls >= l.max_calls
            || !within(
                self.usage.charged_input_tokens,
                r.reserved_input_tokens,
                l.max_input_tokens,
            )
            || !within(
                self.usage.charged_output_tokens,
                r.max_output_tokens,
                l.max_output_tokens,
            )
            || !within_monetary(
                self.usage.charged_cost_micros,
                r.reserved_cost_micros,
                l.max_cost_micros,
            )
        {
            return Err(Harness2Error::Budget);
        }
        self.seen.insert(r.stage.clone());
        self.usage.calls += 1;
        self.usage.charged_input_tokens += r.reserved_input_tokens;
        self.usage.charged_output_tokens += r.max_output_tokens;
        if let (Some(charged), Some(reserve)) =
            (&mut self.usage.charged_cost_micros, r.reserved_cost_micros)
        {
            *charged += reserve;
        }
        Ok(())
    }

    fn settle(&mut self, stage: &StageRequest, usage: &StageUsage) -> Result<(), Harness2Error> {
        settle(
            &mut self.usage.charged_input_tokens,
            &mut self.usage.observed_input_tokens,
            stage.reserved_input_tokens,
            usage.input_tokens,
        )?;
        settle(
            &mut self.usage.charged_output_tokens,
            &mut self.usage.observed_output_tokens,
            stage.max_output_tokens,
            usage.output_tokens,
        )?;
        match (
            &mut self.usage.charged_cost_micros,
            stage.reserved_cost_micros,
        ) {
            (Some(charged), Some(reserve)) => settle(
                charged,
                &mut self.usage.observed_cost_micros,
                reserve,
                usage.cost_micros,
            ),
            (None, None) => {
                self.usage.observed_cost_micros = self
                    .usage
                    .observed_cost_micros
                    .zip(usage.cost_micros)
                    .and_then(|(previous, actual)| previous.checked_add(actual));
                Ok(())
            }
            _ => Err(Harness2Error::Protocol),
        }
    }

    fn admit_promotion(&self, r: &PromotionRequest) -> Result<(), Harness2Error> {
        let t = self.task;
        if self.promotion.is_some()
            || self.seen.len() != 8
            || r.protocol_version != 1
            || r.request_id != "promotion-1"
            || r.run_id != t.run_id
            || r.task_id != t.task_id
            || r.domain != t.domain
            || r.policy_revision_ref != t.policy_revision_ref
            || r.identity_sha256 != self.identity
            || r.evidence_refs != self.references()
            || r.base_artifact.is_empty()
            || r.chosen_artifact.is_empty()
            || r.base_artifact_sha256 != sha(&r.base_artifact)
            || r.chosen_artifact_sha256 != sha(&r.chosen_artifact)
            || !valid_tree(&r.harness, Some(&t.base_harness))
            || r.harness_sha256
                != sha(&canonical(
                    serde_json::to_value(&r.harness).map_err(|_| Harness2Error::Protocol)?,
                )?)
            || ["cue", "lesson", "evidence"]
                .iter()
                .any(|k| r.experience.get(*k).is_none_or(|v| v.trim().is_empty()))
            || r.experience.keys().any(|k| {
                !["cue", "lesson", "evidence", "confidence", "edits"].contains(&k.as_str())
            })
        {
            return Err(Harness2Error::Protocol);
        }
        Ok(())
    }

    fn finish(&self, r: &Harness2Result, program: &WorkerProgram) -> Result<(), Harness2Error> {
        let t = self.task;
        let l = &t.limits;
        if r.protocol_version != 1
            || r.upstream_commit != UPSTREAM_COMMIT
            || r.identity_sha256 != self.identity
            || r.run_id != t.run_id
            || r.task_id != t.task_id
            || r.domain != t.domain
            || r.ledger["R"] != 4
            || r.ledger["P"] != 3
            || !r.selection.ok
            || !r.selection.trivial
            || r.selection.chosen_label != "c1"
            // This is upstream's 16-hex SHA-1 display fingerprint, not Archie's
            // full SHA-256 artifact/identity digest.
            || r.selection.harness_fingerprint.len() != 16
            || !r.selection.harness_fingerprint.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || r.promotion.improvement_verified
            || !segment(&r.promotion.reason_code)
            || r.library.committed.len() > 1
            || r.library.reused_tasks.len() > 64
            || r.library.reused_tasks.iter().any(|s| !segment(s))
            || r.usage.calls > l.max_calls
            || r.usage.charged_input_tokens > l.max_input_tokens
            || r.usage.charged_output_tokens > l.max_output_tokens
            || !within_monetary(r.usage.charged_cost_micros, l.max_cost_micros.map(|_| 0), l.max_cost_micros)
        {
            return Err(Harness2Error::Protocol);
        }
        if r.status == "completed" {
            if self.seen.len() != 8 || r.usage != self.usage {
                return Err(Harness2Error::Protocol);
            }
            match self.promotion.as_ref() {
                Some(promotion) => {
                    if r.promotion.status
                        != if promotion.approved {
                            "promoted"
                        } else {
                            "baseline_retained"
                        }
                        || r.promotion.reason_code != promotion.reason_code
                    {
                        return Err(Harness2Error::Protocol);
                    }
                }
                // A reflection that found no lesson (upstream's NO_ENTRY) offers nothing:
                // the baseline stays and nothing can have been committed.
                None => {
                    if r.promotion.status != "baseline_retained"
                        || r.promotion.reason_code != "no_lesson"
                        || !r.library.committed.is_empty()
                    {
                        return Err(Harness2Error::Protocol);
                    }
                }
            }
        } else if r.status != "resumed" || !self.seen.is_empty() || self.promotion.is_some() {
            return Err(Harness2Error::Protocol);
        }
        if (r.promotion.status == "promoted") != (r.library.committed.len() == 1)
            || !["promoted", "baseline_retained"].contains(&r.promotion.status.as_str())
        {
            return Err(Harness2Error::Protocol);
        }
        let namespace = sha(&canonical(serde_json::json!([
            t.domain.path(),
            t.task_family,
            t.model_identity_sha256,
            UPSTREAM_COMMIT
        ]))?);
        for entry in &r.library.committed {
            let expected = program
                .state_root
                .join(&namespace)
                .join("library")
                .join(&t.task_id)
                .join("manifest.json");
            if PathBuf::from(&entry.manifest_path) != expected
                || entry.reference != format!("harness2/{namespace}/{}", t.task_id)
                || !hash(&entry.sha256)
                || !hash(&entry.harness_sha256)
                || self
                    .promoted_harness_sha256
                    .as_ref()
                    .is_some_and(|h| h != &entry.harness_sha256)
            {
                return Err(Harness2Error::Protocol);
            }
        }
        Ok(())
    }
}

async fn run_inner<H: Harness2Handler>(
    program: &WorkerProgram,
    task: &Harness2Task,
    handler: &mut H,
) -> Result<Harness2Result, Harness2Error> {
    let mut command = Command::new(&program.executable);
    command
        .args(&program.arguments)
        .arg(&program.worker)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    for key in ["SYSTEMROOT", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command.spawn()?;
    let mut input = child.stdin.take().ok_or(Harness2Error::Protocol)?;
    let mut frames = FrameReader {
        output: child.stdout.take().ok_or(Harness2Error::Protocol)?,
        pending: Vec::new(),
        maximum: task.limits.max_frame_bytes as usize,
    };
    let mut start = serde_json::to_value(task).map_err(|_| Harness2Error::InvalidRequest)?;
    start["type"] = "start".into();
    start["protocol_version"] = 1.into();
    start["state_root"] = program
        .state_root
        .to_str()
        .ok_or(Harness2Error::InvalidProgram)?
        .into();
    let encoded = canonical(start)?;
    if encoded.len() > frames.maximum {
        return Err(Harness2Error::InvalidRequest);
    }
    input.write_all(encoded.as_bytes()).await?;
    input.write_all(b"\n").await?;
    input.flush().await?;
    let mut state = RunState {
        task,
        identity: identity(task)?,
        seen: BTreeSet::new(),
        usage: CumulativeUsage {
            charged_cost_micros: task.limits.max_cost_micros.map(|_| 0),
            ..CumulativeUsage::default()
        },
        promotion: None,
        promoted_harness_sha256: None,
    };
    loop {
        let frame = frames.next().await?;
        let message: WorkerMessage =
            serde_json::from_slice(&frame).map_err(|_| Harness2Error::Protocol)?;
        let response = match message {
            WorkerMessage::Inference(stage) => {
                state.admit_stage(&stage)?;
                let reply = handler
                    .infer(&stage)
                    .await
                    .map_err(|_| Harness2Error::Handler)?;
                if reply.text.trim().is_empty()
                    || reply.text.len() as u64 > stage.max_output_tokens.saturating_mul(8)
                {
                    return Err(Harness2Error::Protocol);
                }
                state.settle(&stage, &reply.usage)?;
                serde_json::json!({"type":"inference_result","protocol_version":1,"request_id":stage.request_id,
                    "status":"completed","admitted":true,"domain":task.domain,"policy_revision_ref":task.policy_revision_ref,
                    "text":reply.text,"usage":reply.usage})
            }
            WorkerMessage::Promotion(promotion) => {
                state.admit_promotion(&promotion)?;
                let reply = handler
                    .promote(&promotion)
                    .await
                    .map_err(|_| Harness2Error::Handler)?;
                if !segment(&reply.reason_code) {
                    return Err(Harness2Error::Protocol);
                }
                let response = serde_json::json!({"type":"promotion_result","protocol_version":1,
                    "request_id":promotion.request_id,"admitted":true,"domain":task.domain,
                    "policy_revision_ref":task.policy_revision_ref,"approved":reply.approved,"reason_code":reply.reason_code});
                state.promoted_harness_sha256 = Some(promotion.harness_sha256);
                state.promotion = Some(reply);
                response
            }
            WorkerMessage::Result(ResultMessage::Success(result)) => {
                state.finish(&result, program)?;
                drop(input);
                if !frames.pending.is_empty() || !child.wait().await?.success() {
                    return Err(Harness2Error::Protocol);
                }
                return Ok(*result);
            }
            WorkerMessage::Result(ResultMessage::Failure(failure)) => {
                if failure.protocol_version != 1
                    || failure.status != "failed"
                    || failure.upstream_commit != UPSTREAM_COMMIT
                    || !segment(&failure.error_code)
                {
                    return Err(Harness2Error::Protocol);
                }
                return Err(Harness2Error::Worker {
                    code: failure.error_code,
                    usage: failure.usage,
                });
            }
        };
        let encoded = canonical(response)?;
        if encoded.len() > frames.maximum {
            return Err(Harness2Error::Protocol);
        }
        input.write_all(encoded.as_bytes()).await?;
        input.write_all(b"\n").await?;
        input.flush().await?;
    }
}

/// A single absolute deadline covers process startup, every callback, all I/O and
/// child exit. Dropping this future cancels by killing its owned child process.
pub async fn run_worker<H: Harness2Handler>(
    program: &WorkerProgram,
    task: Harness2Task,
    handler: &mut H,
) -> Result<Harness2Result, Harness2Error> {
    validate_task(&task)?;
    let absolute =
        |p: &std::path::Path| p.is_absolute() || p.to_str().is_some_and(|s| s.starts_with('/'));
    if !absolute(&program.executable)
        || !absolute(&program.worker)
        || !absolute(&program.state_root)
        || program.arguments.iter().any(|a| a.contains('\0'))
    {
        return Err(Harness2Error::InvalidProgram);
    }
    let remaining = task
        .limits
        .deadline_unix_ms
        .checked_sub(now_ms()?)
        .ok_or(Harness2Error::Deadline)?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(remaining))
        .ok_or(Harness2Error::InvalidRequest)?;
    timeout_at(deadline, run_inner(program, &task, handler))
        .await
        .map_err(|_| Harness2Error::Deadline)?
}
