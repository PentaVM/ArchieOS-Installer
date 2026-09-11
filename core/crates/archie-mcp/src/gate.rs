//! The admission gate every tool call passes through.
//!
//! # Why this is a type and not a check
//!
//! The rule is that a call goes
//!
//! ```text
//! Cerebra → capability registry → policy → MCP client → server
//! ```
//!
//! and never `model output → tool`. Rules of that shape are usually written as a
//! function somebody is supposed to call first, which works until the day a second call
//! site is added in a hurry.
//!
//! So [`AuthorizedCall`] has private fields and no public constructor. The only way to
//! obtain one is [`McpGate::authorize`], and a transport takes an `AuthorizedCall`
//! rather than a server id and a blob of arguments. A caller holding a model's proposed
//! tool call has a `ToolProposal` — which no transport accepts — and the only thing it
//! can do with it is submit it to the gate. The unauthorised path is not forbidden; it
//! is absent.
//!
//! # What the gate does not decide
//!
//! Whether the *operator* wants this to happen. A gate that returns
//! [`Admission::NeedsApproval`] is saying the call is legitimate and consequential, and
//! that somebody should be asked. Deciding is Cerebra's job, and asking is the
//! interface's.

use crate::manifest::{McpRegistry, McpServerManifest};
use archie_types::{Capability, CapabilitySet, PrivacyGrade};
use archie_vault::VaultRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The maximum size of an argument payload, before any of it is sent.
pub const MAX_ARGUMENT_BYTES: usize = 131_072;

/// A tool call somebody — usually a model — has proposed.
///
/// Carries no authority. Nothing accepts this type but the gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolProposal {
    pub server_id: String,
    pub tool: String,
    pub arguments: Value,
    /// The sensitivity of what is in `arguments`, as assessed by the caller that
    /// assembled them — normally from the context capsule they came out of.
    pub argument_grade: PrivacyGrade,
    pub task_id: String,
    pub trace_id: String,
}

impl ToolProposal {
    pub fn new(
        server_id: impl Into<String>,
        tool: impl Into<String>,
        arguments: Value,
        argument_grade: PrivacyGrade,
        task_id: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            server_id: server_id.into(),
            tool: tool.into(),
            arguments,
            argument_grade,
            task_id: task_id.into(),
            trace_id: trace_id.into(),
        }
    }
}

/// What the caller has been granted, and what it considers consequential.
#[derive(Debug, Clone, Default)]
pub struct CallerAuthority {
    granted: CapabilitySet,
    /// Tools that may not proceed without a person saying so, whatever else permits
    /// them. Named per server as `server_id/tool`.
    requires_approval: std::collections::BTreeSet<String>,
}

impl CallerAuthority {
    pub fn new(granted: CapabilitySet) -> Self {
        Self {
            granted,
            requires_approval: Default::default(),
        }
    }

    /// Mark a tool as one a person must approve each time.
    pub fn requiring_approval(mut self, server_id: &str, tool: &str) -> Self {
        self.requires_approval.insert(format!("{server_id}/{tool}"));
        self
    }

    pub fn grants(&self, capability: &Capability) -> bool {
        self.granted.provides(capability)
    }

    fn needs_approval(&self, server_id: &str, tool: &str) -> bool {
        self.requires_approval
            .contains(&format!("{server_id}/{tool}"))
    }
}

/// A call that has passed every gate.
///
/// Constructible only by [`McpGate::authorize`]: the fields are private and there is no
/// public constructor, which is what makes "a model cannot reach a tool directly" a
/// property of the program rather than of its documentation.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizedCall {
    server_id: String,
    tool: String,
    arguments: Value,
    /// Resolved at the execution boundary through the vault's broker. The credential
    /// itself is never here, and never in `arguments`.
    credential_ref: Option<VaultRef>,
    timeout_ms: u32,
    max_response_bytes: u32,
    task_id: String,
    trace_id: String,
}

impl AuthorizedCall {
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    /// The *name* of the credential the transport should broker. Never the credential.
    pub fn credential_ref(&self) -> Option<&VaultRef> {
        self.credential_ref.as_ref()
    }

    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub fn max_response_bytes(&self) -> u32 {
        self.max_response_bytes
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// The record of this call, for the trace. Arguments are described, not included.
    pub fn receipt(&self) -> CallReceipt {
        CallReceipt {
            server_id: self.server_id.clone(),
            tool: self.tool.clone(),
            task_id: self.task_id.clone(),
            trace_id: self.trace_id.clone(),
            argument_bytes: self.arguments.to_string().len(),
            brokered_credential: self.credential_ref.clone(),
        }
    }
}

/// What may be written down about a call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallReceipt {
    pub server_id: String,
    pub tool: String,
    pub task_id: String,
    pub trace_id: String,
    /// How much was sent, not what.
    pub argument_bytes: usize,
    pub brokered_credential: Option<VaultRef>,
}

/// The gate's verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    /// Proceed.
    Allowed(Box<AuthorizedCall>),
    /// Legitimate, and consequential enough that a person should say so first.
    NeedsApproval {
        call: Box<AuthorizedCall>,
        reason: &'static str,
    },
    Refused(Refusal),
}

impl Admission {
    pub fn allowed(&self) -> Option<&AuthorizedCall> {
        match self {
            Self::Allowed(call) => Some(call),
            _ => None,
        }
    }

    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }
}

/// Why a call did not pass. Stable codes: these end up in traces.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("no server registered as {0}")]
    UnknownServer(String),
    #[error("server {0} is registered but disabled")]
    ServerDisabled(String),
    #[error("{tool} is not on the operator's allowlist for {server_id}")]
    ToolNotAllowed { server_id: String, tool: String },
    #[error("caller lacks the capability {0} this server requires")]
    CapabilityNotGranted(Capability),
    #[error("arguments graded {grade} exceed what {server_id} may receive")]
    PrivacyExceeded {
        server_id: String,
        grade: PrivacyGrade,
    },
    #[error("a tool argument may never be graded secret; use a credential handle")]
    SecretInArguments,
    #[error("arguments are {size} bytes, over the {MAX_ARGUMENT_BYTES} permitted")]
    ArgumentsTooLarge { size: usize },
    #[error("arguments must be a JSON object")]
    ArgumentsNotAnObject,
}

/// Admits tool calls, or does not.
#[derive(Debug, Default, Clone)]
pub struct McpGate;

impl McpGate {
    /// The only way to obtain an [`AuthorizedCall`].
    ///
    /// Ordered so that the cheapest and most categorical refusals come first, and so
    /// that a refusal names the first rule that bound rather than an aggregate.
    pub fn authorize(
        registry: &McpRegistry,
        authority: &CallerAuthority,
        proposal: &ToolProposal,
    ) -> Admission {
        let server = match registry.get(&proposal.server_id) {
            Some(server) => server,
            None => {
                return Admission::Refused(Refusal::UnknownServer(proposal.server_id.clone()));
            }
        };
        if !server.enabled {
            return Admission::Refused(Refusal::ServerDisabled(server.id.clone()));
        }
        if !server.tools.permits(&proposal.tool) {
            return Admission::Refused(Refusal::ToolNotAllowed {
                server_id: server.id.clone(),
                tool: proposal.tool.clone(),
            });
        }
        // Every capability the server was declared to need must have been granted to
        // this caller. A server usable by anyone who can reach the registry would make
        // the capability model decorative.
        for capability in server.capabilities.iter() {
            if !authority.grants(capability) {
                return Admission::Refused(Refusal::CapabilityNotGranted(capability.clone()));
            }
        }
        if proposal.argument_grade == PrivacyGrade::Secret {
            return Admission::Refused(Refusal::SecretInArguments);
        }
        if !server.permits_privacy(proposal.argument_grade) {
            return Admission::Refused(Refusal::PrivacyExceeded {
                server_id: server.id.clone(),
                grade: proposal.argument_grade,
            });
        }
        if !proposal.arguments.is_object() && !proposal.arguments.is_null() {
            return Admission::Refused(Refusal::ArgumentsNotAnObject);
        }
        let size = proposal.arguments.to_string().len();
        if size > MAX_ARGUMENT_BYTES {
            return Admission::Refused(Refusal::ArgumentsTooLarge { size });
        }

        let call = Box::new(AuthorizedCall {
            server_id: server.id.clone(),
            tool: proposal.tool.clone(),
            arguments: proposal.arguments.clone(),
            credential_ref: server.secret_handle.clone(),
            timeout_ms: server.timeout_ms,
            max_response_bytes: server.max_response_bytes,
            task_id: proposal.task_id.clone(),
            trace_id: proposal.trace_id.clone(),
        });

        if authority.needs_approval(&server.id, &proposal.tool) {
            return Admission::NeedsApproval {
                call,
                reason: "operator marked this tool as requiring approval",
            };
        }
        Admission::Allowed(call)
    }

    /// The tools of `server` that are both advertised and allowed.
    ///
    /// The server's own listing is an input, not an authority: names it added since the
    /// manifest was written are not thereby granted.
    pub fn permitted_tools<'a>(
        server: &McpServerManifest,
        advertised: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        advertised
            .into_iter()
            .filter(|name| server.tools.permits(name))
            .map(str::to_string)
            .collect()
    }
}
