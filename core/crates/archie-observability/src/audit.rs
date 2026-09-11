//! The audit trail.
//!
//! An audit record answers "who authorised this, and on what grounds". It is written
//! for decisions that change authority or touch the physical world — not for ordinary
//! progress, which belongs in logs.
//!
//! Audit records are **append-only**. There is no update or delete in this API, and
//! that omission is the point.

use archie_types::{NodeId, TaskId, Timestamp, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// What happened to the request being audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Allowed,
    /// Allowed, but narrowed from what was asked for.
    AllowedScoped,
    Denied,
    /// Held pending a human decision.
    PendingApproval,
    /// Stopped by an integrity or safety guard, not by ordinary policy.
    HardStop,
}

impl AuditOutcome {
    pub fn granted_authority(&self) -> bool {
        matches!(self, Self::Allowed | Self::AllowedScoped)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::AllowedScoped => "allowed_scoped",
            Self::Denied => "denied",
            Self::PendingApproval => "pending_approval",
            Self::HardStop => "hard_stop",
        }
    }
}

/// One append-only audit record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub recorded_at: Timestamp,
    /// The node that made the decision.
    pub node: NodeId,
    pub trace_id: Option<TraceId>,
    pub task_id: Option<TaskId>,
    /// Dotted action name, e.g. `capability.grant`, `module.load`, `device.actuate`.
    pub action: String,
    /// Who or what asked. Free text, but must identify a principal.
    pub principal: String,
    /// What was acted upon.
    pub resource: String,
    pub outcome: AuditOutcome,
    /// Machine-readable reasons, e.g. `privacy_grade_not_allowed`.
    pub reason_codes: Vec<String>,
    /// Human-readable justification. Always present: an audit line without an
    /// explanation is not auditable.
    pub explanation: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

impl AuditEvent {
    pub fn new(
        node: NodeId,
        action: impl Into<String>,
        principal: impl Into<String>,
        resource: impl Into<String>,
        outcome: AuditOutcome,
        explanation: impl Into<String>,
    ) -> Self {
        Self {
            recorded_at: Timestamp::now(),
            node,
            trace_id: None,
            task_id: None,
            action: action.into(),
            principal: principal.into(),
            resource: resource.into(),
            outcome,
            reason_codes: Vec::new(),
            explanation: explanation.into(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_trace(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn with_reason(mut self, code: impl Into<String>) -> Self {
        self.reason_codes.push(code.into());
        self
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Whether this event records something that touched the physical world.
    ///
    /// Used to route safety-relevant events to durable storage even when the ordinary
    /// audit sink is under back-pressure.
    pub fn is_physical(&self) -> bool {
        self.action.starts_with("device.") || self.action.starts_with("safety.")
    }
}

/// Where audit records go.
///
/// A sink may not fail silently: `record` returns a result so that a caller granting
/// authority can refuse to proceed if the decision could not be written down.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("audit sink unavailable: {0}")]
    Unavailable(String),
}

/// An in-memory sink. For tests and for the early boot window before durable storage
/// is mounted.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.events.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Records matching a dotted action prefix.
    pub fn with_action_prefix(&self, prefix: &str) -> Vec<AuditEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.action.starts_with(prefix))
            .collect()
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let mut guard = self
            .events
            .lock()
            .map_err(|_| AuditError::Unavailable("audit mutex poisoned".into()))?;
        guard.push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeId {
        NodeId::from_public_key_bytes(&[1u8; 32])
    }

    #[test]
    fn events_are_appended_in_order() {
        let sink = InMemoryAuditSink::new();
        for action in ["a.one", "a.two", "a.three"] {
            sink.record(AuditEvent::new(
                node(),
                action,
                "operator",
                "res",
                AuditOutcome::Allowed,
                "because",
            ))
            .unwrap();
        }
        let actions: Vec<String> = sink.events().into_iter().map(|e| e.action).collect();
        assert_eq!(actions, vec!["a.one", "a.two", "a.three"]);
    }

    #[test]
    fn sink_has_no_delete_or_update() {
        // Compile-time property. This test documents the intent: the only mutation
        // available on the trait is `record`.
        let sink = InMemoryAuditSink::new();
        sink.record(AuditEvent::new(
            node(),
            "x.y",
            "p",
            "r",
            AuditOutcome::Denied,
            "no",
        ))
        .unwrap();
        assert_eq!(sink.len(), 1);
    }

    #[test]
    fn outcome_reports_whether_authority_was_granted() {
        assert!(AuditOutcome::Allowed.granted_authority());
        assert!(AuditOutcome::AllowedScoped.granted_authority());
        assert!(!AuditOutcome::Denied.granted_authority());
        assert!(!AuditOutcome::PendingApproval.granted_authority());
        assert!(!AuditOutcome::HardStop.granted_authority());
    }

    #[test]
    fn physical_actions_are_flagged() {
        let e = AuditEvent::new(
            node(),
            "device.actuate",
            "sentry",
            "arm-joint-3",
            AuditOutcome::Denied,
            "no lease",
        );
        assert!(e.is_physical());
        let e2 = AuditEvent::new(
            node(),
            "module.load",
            "core",
            "cognition.critic",
            AuditOutcome::Allowed,
            "ok",
        );
        assert!(!e2.is_physical());
    }

    #[test]
    fn filtering_by_action_prefix_works() {
        let sink = InMemoryAuditSink::new();
        for action in ["device.actuate", "device.read", "module.load"] {
            sink.record(AuditEvent::new(
                node(),
                action,
                "p",
                "r",
                AuditOutcome::Allowed,
                "ok",
            ))
            .unwrap();
        }
        assert_eq!(sink.with_action_prefix("device.").len(), 2);
    }

    #[test]
    fn event_round_trips_through_json() {
        let e = AuditEvent::new(
            node(),
            "capability.grant",
            "operator",
            "fs:/var/lib/archie",
            AuditOutcome::AllowedScoped,
            "scoped to one task",
        )
        .with_reason("scope_narrowed")
        .with_attribute("scope", "one_task");
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<AuditEvent>(&json).unwrap(), e);
    }
}
