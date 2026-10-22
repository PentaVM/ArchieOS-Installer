//! Managed, text-only Codex app-server connection. Credentials remain with Codex.
//! A child process on this machine still performs cloud inference.

use archie_types::PrivacyGrade;
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;

const MAX_FRAME: usize = 256 * 1024;
const MAX_TOTAL: usize = 2 * 1024 * 1024;
const MAX_PROMPT: usize = 64 * 1024;
const MAX_TEXT: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum CodexError {
    #[error("managed subscription accepts public text only; deployment retention is not verified")]
    PrivacyDenied,
    #[error("invalid managed-client request or limits")]
    InvalidRequest,
    #[error("official client is unavailable or exited")]
    Process,
    #[error("official client exited with status {0:?}")]
    ProcessExit(Option<i32>),
    #[error("official client protocol is invalid or uncorrelated")]
    Protocol,
    #[error("official client returned an error")]
    Remote,
    #[error("managed ChatGPT account identity is unavailable")]
    AuthenticationRequired,
    #[error("conversation account or domain differs from its stored binding")]
    SessionMismatch,
    #[error("official client exceeded its output budget")]
    OutputLimit,
    #[error("model turn did not complete")]
    Incomplete,
    #[error("model requested a tool; execution requires the Core tool boundary")]
    ToolDenied,
    #[error("managed-client total deadline expired")]
    Timeout,
    #[error("managed-client turn cancelled")]
    Cancelled,
}

/// Only non-secret, observed metadata. The account identifier is hashed locally.
#[derive(Debug, Clone, Serialize)]
pub struct CodexAccount {
    pub auth_type: Option<String>,
    pub plan: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexSession {
    pub domain: ContextDomain,
    pub account_id: String,
    /// The official `thread.id`, never the session tree root.
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexCompletion {
    pub text: String,
    pub session: CodexSession,
    pub turn_id: String,
}

#[derive(Clone)]
pub struct CodexClient {
    executable: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    timeout: Duration,
}

impl CodexClient {
    pub fn new(
        executable: impl AsRef<OsStr>,
        cwd: &Path,
        timeout: Duration,
    ) -> Result<Self, CodexError> {
        if timeout.is_zero() || timeout > Duration::from_secs(600) || !cwd.is_dir() {
            return Err(CodexError::InvalidRequest);
        }
        let mut args = vec!["app-server".into(), "--stdio".into()];
        // All configuration is per child; no user/client config is written.
        for (key, value) in inference_config().as_object().expect("object") {
            args.push("-c".into());
            args.push(format!("{key}={value}"));
        }
        Ok(Self {
            executable: PathBuf::from(executable.as_ref()),
            args,
            cwd: cwd.to_owned(),
            timeout,
        })
    }

    #[cfg(test)]
    fn fixture(executable: &str, args: Vec<String>, cwd: &Path, timeout: Duration) -> Self {
        Self {
            executable: executable.into(),
            args,
            cwd: cwd.into(),
            timeout,
        }
    }

    pub async fn account(&self) -> Result<CodexAccount, CodexError> {
        let mut process = JsonProcess::spawn(self)?;
        let result = tokio::time::timeout(self.timeout, async {
            process.initialize().await?;
            observed_account(
                &process
                    .rpc("account/read", json!({"refreshToken":false}))
                    .await?,
            )
        })
        .await
        .map_err(|_| CodexError::Timeout)
        .and_then(|v| v);
        process.close().await;
        result
    }

    /// A catalog is diagnostic metadata, not proof of inference entitlement.
    pub async fn models(&self) -> Result<Vec<String>, CodexError> {
        let mut process = JsonProcess::spawn(self)?;
        let result = tokio::time::timeout(self.timeout, async {
            process.initialize().await?;
            let reply = process.rpc("model/list", json!({"limit":100})).await?;
            let models = reply
                .get("data")
                .and_then(Value::as_array)
                .ok_or(CodexError::Protocol)?;
            models
                .iter()
                .map(|m| {
                    m.get("id")
                        .and_then(Value::as_str)
                        .filter(|id| valid_id(id))
                        .map(str::to_owned)
                        .ok_or(CodexError::Protocol)
                })
                .collect()
        })
        .await
        .map_err(|_| CodexError::Timeout)
        .and_then(|v| v);
        process.close().await;
        result
    }

    pub async fn complete(
        &self,
        domain: &ContextDomain,
        privacy: PrivacyGrade,
        model: &str,
        prompt: &str,
        session: Option<&CodexSession>,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<CodexCompletion, CodexError> {
        self.complete_for_account(
            domain,
            privacy,
            model,
            prompt,
            session,
            session.map(|s| s.account_id.as_str()),
            cancel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn complete_for_account(
        &self,
        domain: &ContextDomain,
        privacy: PrivacyGrade,
        model: &str,
        prompt: &str,
        session: Option<&CodexSession>,
        expected_account: Option<&str>,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<CodexCompletion, CodexError> {
        self.complete_bounded_for_account(
            domain,
            privacy,
            model,
            prompt,
            session,
            expected_account,
            MAX_TEXT,
            cancel,
        )
        .await
    }

    /// The official protocol currently offers byte/time bounds here, not a hard
    /// generation-token limit. Service callers must explicitly choose that mode.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_bounded_for_account(
        &self,
        domain: &ContextDomain,
        privacy: PrivacyGrade,
        model: &str,
        prompt: &str,
        session: Option<&CodexSession>,
        expected_account: Option<&str>,
        max_output_bytes: usize,
        mut cancel: Option<watch::Receiver<bool>>,
    ) -> Result<CodexCompletion, CodexError> {
        // Subscription retention is unknown; even Internal must fail closed.
        if privacy != PrivacyGrade::Public {
            return Err(CodexError::PrivacyDenied);
        }
        if !valid_id(model)
            || prompt.is_empty()
            || prompt.len() > MAX_PROMPT
            || !(1..=MAX_TEXT).contains(&max_output_bytes)
        {
            return Err(CodexError::InvalidRequest);
        }
        if session.is_some_and(|s| s.domain != *domain || !valid_id(&s.thread_id)) {
            return Err(CodexError::SessionMismatch);
        }
        if cancel.as_ref().is_some_and(|c| *c.borrow()) {
            return Err(CodexError::Cancelled);
        }
        let mut process = JsonProcess::spawn(self)?;
        let result = {
            let operation = process.turn(
                domain,
                model,
                prompt,
                session,
                expected_account,
                max_output_bytes,
            );
            tokio::select! {
                result=tokio::time::timeout(self.timeout,operation)=>result.map_err(|_|CodexError::Timeout).and_then(|v|v),
                ()=cancelled(&mut cancel)=>Err(CodexError::Cancelled),
            }
        };
        if matches!(
            result,
            Err(CodexError::Timeout | CodexError::Cancelled | CodexError::ToolDenied)
        ) {
            process.interrupt().await;
        }
        process.close().await;
        result
    }
}

async fn cancelled(cancel: &mut Option<watch::Receiver<bool>>) {
    if let Some(rx) = cancel {
        loop {
            if *rx.borrow() {
                return;
            }
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    } else {
        std::future::pending::<()>().await;
    }
}

fn inference_config() -> Value {
    json!({
        "model_provider":"openai", "web_search":"disabled", "mcp_servers":{}, "plugins":{},
        "features.shell_tool":false,"features.apps":false,"features.plugins":false,
        "features.remote_plugin":false,"features.multi_agent":false,"features.hooks":false,
        "features.memories":false,"features.goals":false,"features.shell_snapshot":false,
        "features.skill_mcp_dependency_install":false,"tools.view_image":false,
        "project_doc_max_bytes":0,"sandbox_mode":"read-only","approval_policy":"never"
    })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
}

fn observed_account(reply: &Value) -> Result<CodexAccount, CodexError> {
    let account = reply.get("account").filter(|a| !a.is_null());
    let auth_type = account
        .and_then(|a| a.get("type"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let plan = account
        .and_then(|a| a.get("planType"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    // No guessed identity from an email/plan: different workspaces may share both.
    let account_id = if auth_type.as_deref() == Some("chatgpt") {
        reply
            .pointer("/workspaceRouting/chatgptAccountId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
            .map(|id| {
                format!(
                    "{:x}",
                    Sha256::digest(format!("codex-managed-chatgpt\0{id}").as_bytes())
                )
            })
    } else {
        None
    };
    Ok(CodexAccount {
        auth_type,
        plan,
        account_id,
    })
}

struct JsonProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
    total: usize,
    pending: VecDeque<Value>,
    active: Option<(String, String)>,
}

impl JsonProcess {
    fn spawn(config: &CodexClient) -> Result<Self, CodexError> {
        let mut command = Command::new(&config.executable);
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command.spawn().map_err(|_| CodexError::Process)?;
        let input = child.stdin.take().ok_or(CodexError::Process)?;
        let output = BufReader::new(child.stdout.take().ok_or(CodexError::Process)?);
        Ok(Self {
            child,
            input,
            output,
            next_id: 1,
            total: 0,
            pending: VecDeque::new(),
            active: None,
        })
    }
    async fn send(&mut self, value: Value) -> Result<(), CodexError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| CodexError::Protocol)?;
        if bytes.len() > MAX_FRAME {
            return Err(CodexError::OutputLimit);
        }
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| CodexError::Process)?;
        self.input.flush().await.map_err(|_| CodexError::Process)
    }
    async fn read(&mut self) -> Result<Value, CodexError> {
        let mut frame = Vec::new();
        loop {
            let bytes = self
                .output
                .fill_buf()
                .await
                .map_err(|_| CodexError::Process)?;
            if bytes.is_empty() {
                return Err(CodexError::ProcessExit(
                    self.child.try_wait().ok().flatten().and_then(|s| s.code()),
                ));
            }
            let end = bytes.iter().position(|b| *b == b'\n').map(|p| p + 1);
            let n = end.unwrap_or(bytes.len());
            if frame.len() + n > MAX_FRAME || self.total + n > MAX_TOTAL {
                return Err(CodexError::OutputLimit);
            }
            frame.extend_from_slice(&bytes[..n]);
            self.output.consume(n);
            self.total += n;
            if end.is_some() {
                break;
            }
        }
        let value: Value = serde_json::from_slice(&frame).map_err(|_| CodexError::Protocol)?;
        if !value.is_object() {
            return Err(CodexError::Protocol);
        }
        if value.get("id").is_some() && value.get("method").is_some() {
            // Never provide auth tokens, approvals or dynamic-tool execution.
            return Err(CodexError::ToolDenied);
        }
        Ok(value)
    }
    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, CodexError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let v = self.read().await?;
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if v.get("error").is_some() {
                    return Err(CodexError::Remote);
                }
                return v.get("result").cloned().ok_or(CodexError::Protocol);
            }
            if v.get("id").is_some() {
                return Err(CodexError::Protocol);
            }
            if self.pending.len() >= 128 {
                return Err(CodexError::OutputLimit);
            }
            self.pending.push_back(v);
        }
    }
    async fn initialize(&mut self) -> Result<(), CodexError> {
        self.rpc(
            "initialize",
            json!({"clientInfo":{"name":"archie_managed_inference","version":"0.1.0"}}),
        )
        .await?;
        self.send(json!({"method":"initialized","params":{}})).await
    }
    async fn turn(
        &mut self,
        domain: &ContextDomain,
        model: &str,
        prompt: &str,
        session: Option<&CodexSession>,
        expected_account: Option<&str>,
        max_output_bytes: usize,
    ) -> Result<CodexCompletion, CodexError> {
        self.initialize().await?;
        let account = observed_account(
            &self
                .rpc("account/read", json!({"refreshToken":false}))
                .await?,
        )?;
        let account_id = account
            .account_id
            .ok_or(CodexError::AuthenticationRequired)?;
        if session.is_some_and(|s| s.account_id != account_id)
            || expected_account.is_some_and(|expected| expected != account_id)
        {
            return Err(CodexError::SessionMismatch);
        }
        let mut params = json!({"model":model,"approvalPolicy":"never","sandbox":"read-only","config":inference_config(),
            "developerInstructions":"Return a text answer using only the supplied context. Tools and filesystem access are unavailable. Your answer is an unverified proposal; Core retains all authority."});
        let method = if let Some(session) = session {
            params["threadId"] = json!(session.thread_id);
            params["excludeTurns"] = json!(true);
            "thread/resume"
        } else {
            "thread/start"
        };
        let thread = self.rpc(method, params).await?;
        let thread_id = thread
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .filter(|s| valid_id(s))
            .ok_or(CodexError::Protocol)?
            .to_owned();
        if session.is_some_and(|s| s.thread_id != thread_id) {
            return Err(CodexError::SessionMismatch);
        }
        self.pending.clear();
        let started=self.rpc("turn/start",json!({"threadId":thread_id,"input":[{"type":"text","text":prompt,"text_elements":[]}],"model":model,"approvalPolicy":"never","sandboxPolicy":{"type":"readOnly"}})).await?;
        let turn_id = started
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .filter(|s| valid_id(s))
            .ok_or(CodexError::Protocol)?
            .to_owned();
        self.active = Some((thread_id.clone(), turn_id.clone()));
        // Item IDs are opaque. Preserve first observed protocol order while
        // applying duplicate updates in place, independently of lexical IDs.
        let mut messages: Vec<(String, String, Option<String>)> = Vec::new();
        loop {
            let event = if let Some(event) = self.pending.pop_front() {
                event
            } else {
                self.read().await?
            };
            let method = event
                .get("method")
                .and_then(Value::as_str)
                .ok_or(CodexError::Protocol)?;
            if !matches!(method, "item/completed" | "item/started" | "turn/completed") {
                continue;
            }
            let p = event.get("params").ok_or(CodexError::Protocol)?;
            if p.get("threadId").and_then(Value::as_str) != Some(&thread_id) {
                return Err(CodexError::Protocol);
            }
            if method == "turn/completed" {
                if p.pointer("/turn/id").and_then(Value::as_str) != Some(&turn_id) {
                    return Err(CodexError::Protocol);
                }
                if p.pointer("/turn/status").and_then(Value::as_str) != Some("completed") {
                    return Err(CodexError::Incomplete);
                }
                let has_final = messages
                    .iter()
                    .any(|(_, _, phase)| phase.as_deref() == Some("final_answer"));
                let text = messages
                    .into_iter()
                    .filter(|(_, _, phase)| {
                        if has_final {
                            phase.as_deref() == Some("final_answer")
                        } else {
                            !matches!(phase.as_deref(), Some("commentary" | "partial_answer"))
                        }
                    })
                    .map(|(_, text, _)| text)
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.len() > max_output_bytes {
                    return Err(CodexError::OutputLimit);
                }
                if text.is_empty() {
                    return Err(CodexError::Incomplete);
                }
                self.active = None;
                return Ok(CodexCompletion {
                    text,
                    session: CodexSession {
                        domain: domain.clone(),
                        account_id,
                        thread_id,
                    },
                    turn_id,
                });
            }
            if p.get("turnId").and_then(Value::as_str) != Some(&turn_id) {
                return Err(CodexError::Protocol);
            }
            let item = p.get("item").ok_or(CodexError::Protocol)?;
            match item.get("type").and_then(Value::as_str) {
                Some("agentMessage") if method == "item/completed" => {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or(CodexError::Protocol)?;
                    let id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or(CodexError::Protocol)?;
                    let existing = messages.iter().position(|(old_id, _, _)| old_id == id);
                    let old_len = existing.map_or(0, |i| messages[i].1.len());
                    if text.len()
                        + messages
                            .iter()
                            .map(|(_, text, _)| text.len())
                            .sum::<usize>()
                        - old_len
                        > max_output_bytes
                    {
                        return Err(CodexError::OutputLimit);
                    }
                    let message = (
                        id.to_owned(),
                        text.to_owned(),
                        item.get("phase").and_then(Value::as_str).map(str::to_owned),
                    );
                    if let Some(index) = existing {
                        messages[index] = message;
                    } else {
                        if messages.len() >= 128 {
                            return Err(CodexError::OutputLimit);
                        }
                        messages.push(message);
                    }
                }
                Some("agentMessage" | "userMessage" | "reasoning") => {}
                _ => return Err(CodexError::ToolDenied),
            }
        }
    }
    async fn interrupt(&mut self) {
        if let Some((thread, turn)) = self.active.take() {
            let _=tokio::time::timeout(Duration::from_millis(100),self.send(json!({"id":self.next_id,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}}))).await;
        }
    }
    async fn close(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::PrivacyGrade;
    use archie_vault::ContextDomain;
    use std::time::Duration;

    // This is a process protocol fixture, never evidence of provider access.
    fn fixture(mode: &str) -> (tempfile::TempDir, CodexClient) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.py");
        let log_path = dir.path().join("rpc.log").to_string_lossy().into_owned();
        std::fs::write(&script, format!(r#"
import json, sys, time
MODE = {mode:?}
LOG = {log_path:?}
def send(v):
    print(json.dumps(v), flush=True)
for line in sys.stdin:
    r = json.loads(line); m = r.get('method'); i = r.get('id')
    with open(LOG, 'a') as log: log.write(str(m)+'\n')
    if m == 'initialize': send({{'id':i,'result':{{'userAgent':'fixture'}}}})
    elif m == 'account/read':
        account = 'fixture-account'
        if MODE == 'switch-account':
            import pathlib
            marker = pathlib.Path(LOG+'.account-seen')
            if marker.exists(): account = 'other-account'
            marker.write_text('seen')
        send({{'id':i,'result':{{'account':{{'type':'chatgpt','email':'fixture@example.invalid','planType':'unknown'}},'workspaceRouting':{{'chatgptAccountId':account}},'requiresOpenaiAuth':True}}}})
    elif m in ('thread/start','thread/resume'):
        assert r['params']['approvalPolicy'] == 'never'
        assert r['params']['sandbox'] == 'read-only'
        assert r['params']['config']['features.shell_tool'] is False
        send({{'id':i,'result':{{'thread':{{'id':'thread-1','sessionId':'different-session-root'}}}}}})
    elif m == 'turn/start':
        send({{'id':i,'result':{{'turn':{{'id':'turn-1','status':'inProgress'}}}}}})
        if MODE == 'hang': time.sleep(30)
        elif MODE == 'oversize': print('x'*300000,flush=True)
        elif MODE == 'tool': send({{'id':123,'method':'item/commandExecution/requestApproval','params':{{}}}})
        else:
            items = [{{'type':'agentMessage','id':'message-1','text':'protocol reply'}}]
            if MODE == 'ordered': items = [{{'type':'agentMessage','id':'z','text':'first','phase':None}},{{'type':'agentMessage','id':'a','text':'second'}},{{'type':'agentMessage','id':'z','text':'first updated','phase':None}}]
            if MODE == 'phased': items = [{{'type':'agentMessage','id':'a','text':'progress','phase':'commentary'}},{{'type':'agentMessage','id':'b','text':'draft','phase':'partial_answer'}},{{'type':'agentMessage','id':'z','text':'final one','phase':'final_answer'}},{{'type':'agentMessage','id':'c','text':'final two','phase':'final_answer'}}]
            if MODE == 'progress-only': items = [{{'type':'agentMessage','id':'a','text':'still working','phase':'commentary'}}]
            for item in items: send({{'method':'item/completed','params':{{'threadId':'thread-1','turnId':'turn-1','item':item}}}})
            send({{'method':'turn/completed','params':{{'threadId':'wrong' if MODE == 'wrong-thread' else 'thread-1','turn':{{'id':'turn-1','status':'failed' if MODE == 'fail' else 'completed'}}}}}})
"#)).unwrap();
        let client = CodexClient::fixture(
            "python3",
            vec!["-u".into(), script.to_string_lossy().into()],
            dir.path(),
            Duration::from_millis(700),
        );
        (dir, client)
    }

    #[tokio::test]
    async fn first_turn_pins_account_before_any_thread_or_capsule_rpc() {
        let (dir, client) = fixture("switch-account");
        let expected = client.account().await.unwrap().account_id.unwrap();
        let result = client
            .complete_for_account(
                &ContextDomain::personal("owner").unwrap(),
                PrivacyGrade::Public,
                "fixture-model",
                "account A admitted capsule",
                None,
                Some(&expected),
                None,
            )
            .await;
        assert!(matches!(result, Err(CodexError::SessionMismatch)));
        let log = std::fs::read_to_string(dir.path().join("rpc.log")).unwrap();
        assert!(!log.contains("thread/start"));
        assert!(!log.contains("turn/start"));
    }

    #[tokio::test]
    async fn explicit_managed_byte_bound_is_enforced_without_claiming_a_token_cap() {
        let (_dir, client) = fixture("ok");
        assert!(matches!(
            client
                .complete_bounded_for_account(
                    &ContextDomain::personal("owner").unwrap(),
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    None,
                    None,
                    3,
                    None,
                )
                .await,
            Err(CodexError::OutputLimit)
        ));
    }

    #[tokio::test]
    async fn answers_keep_event_order_deduplicate_updates_and_prefer_final_phase() {
        for (mode, expected) in [
            ("ordered", "first updated\nsecond"),
            ("phased", "final one\nfinal two"),
        ] {
            let (_dir, client) = fixture(mode);
            let answer = client
                .complete(
                    &ContextDomain::personal("owner").unwrap(),
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    None,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(answer.text, expected, "mode {mode}");
        }
        let (_dir, client) = fixture("progress-only");
        assert!(matches!(
            client
                .complete(
                    &ContextDomain::personal("owner").unwrap(),
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    None,
                    None
                )
                .await,
            Err(CodexError::Incomplete)
        ));
    }

    #[tokio::test]
    async fn managed_process_requires_observed_completed_turn_and_keeps_thread_id() {
        let (_dir, client) = fixture("ok");
        let domain = ContextDomain::personal("owner").unwrap();
        let result = client
            .complete(
                &domain,
                PrivacyGrade::Public,
                "fixture-model",
                "public question",
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.text, "protocol reply");
        assert_eq!(result.session.thread_id, "thread-1");
        assert_ne!(result.session.thread_id, "different-session-root");
        assert_eq!(result.session.domain, domain);
        assert_eq!(result.session.account_id.len(), 64);
        let second = client
            .complete(
                &domain,
                PrivacyGrade::Public,
                "fixture-model",
                "second public question",
                Some(&result.session),
                None,
            )
            .await
            .unwrap();
        assert_eq!(second.session, result.session);
    }

    #[tokio::test]
    async fn resume_refuses_different_domain_or_observed_account() {
        let (_dir, client) = fixture("ok");
        let domain = ContextDomain::personal("owner").unwrap();
        let mut session = CodexSession {
            domain: ContextDomain::work("org", "project").unwrap(),
            account_id: "x".repeat(64),
            thread_id: "thread-1".into(),
        };
        assert!(matches!(
            client
                .complete(
                    &domain,
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    Some(&session),
                    None
                )
                .await,
            Err(CodexError::SessionMismatch)
        ));
        session.domain = domain.clone();
        assert!(matches!(
            client
                .complete(
                    &domain,
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    Some(&session),
                    None
                )
                .await,
            Err(CodexError::SessionMismatch)
        ));
    }

    #[tokio::test]
    async fn failed_or_uncorrelated_events_never_become_success() {
        for mode in ["fail", "wrong-thread", "tool", "oversize"] {
            let (_dir, client) = fixture(mode);
            let result = client
                .complete(
                    &ContextDomain::personal("owner").unwrap(),
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    None,
                    None,
                )
                .await;
            assert!(result.is_err(), "{mode} incorrectly succeeded");
        }
    }

    #[tokio::test]
    async fn total_deadline_and_cancel_terminate_owned_process() {
        let (_dir, mut client) = fixture("hang");
        client.timeout = Duration::from_millis(100);
        let domain = ContextDomain::personal("owner").unwrap();
        let start = std::time::Instant::now();
        assert!(matches!(
            client
                .complete(
                    &domain,
                    PrivacyGrade::Public,
                    "fixture-model",
                    "test",
                    None,
                    None
                )
                .await,
            Err(CodexError::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
        let (send, recv) = tokio::sync::watch::channel(true);
        let result = client
            .complete(
                &domain,
                PrivacyGrade::Public,
                "fixture-model",
                "test",
                None,
                Some(recv),
            )
            .await;
        assert!(matches!(result, Err(CodexError::Cancelled)));
        drop(send);
    }

    #[tokio::test]
    async fn cloud_subscription_refuses_private_context_before_spawn() {
        let client = CodexClient::new(
            "definitely-missing-command",
            std::path::Path::new("."),
            Duration::from_secs(1),
        )
        .unwrap();
        for privacy in [
            PrivacyGrade::Internal,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
            PrivacyGrade::Secret,
        ] {
            assert!(matches!(
                client
                    .complete(
                        &ContextDomain::personal("owner").unwrap(),
                        privacy,
                        "fixture-model",
                        "never sent",
                        None,
                        None
                    )
                    .await,
                Err(CodexError::PrivacyDenied)
            ));
        }
    }
}
