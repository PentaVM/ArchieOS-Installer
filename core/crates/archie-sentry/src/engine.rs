//! Core-owned execution. There is no deserializable authority or auto-issued lease.
use crate::{
    contracts::{ContractError, Intent, Observation},
    journal::{Journal, JournalError, JournalRecord},
    registry::{Registry, ResourceDescriptor},
};
use archie_policy::{
    Action, CapabilityRequest, Lease, Operation, PolicyEngine, Principal, Resource, Scope,
};
use archie_types::{LeaseId, Timestamp};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterError {
    Transport,
    Authentication,
    Protocol,
    Unsupported,
    Unavailable,
}
impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "adapter {self:?}")
    }
}
impl std::error::Error for AdapterError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterAck {
    pub accepted: bool,
}

#[async_trait]
pub trait EdgeAdapter: Send {
    fn edge_id(&self) -> &str;
    fn adapter_id(&self) -> &str;
    async fn observe(&mut self, resource: &ResourceDescriptor)
        -> Result<Observation, AdapterError>;
    /// Complete awaited protocol preparation before Core's final revocation check.
    /// This must not actuate. `apply` must proceed directly to its dispatch request.
    async fn preflight(
        &mut self,
        _resource: &ResourceDescriptor,
        _command: &crate::contracts::Command,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &crate::contracts::Command,
    ) -> Result<AdapterAck, AdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Verified,
    Diverged,
    Denied,
    Unavailable,
    Indeterminate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Resolved,
    Authorized,
    Reserved,
    ObservedBefore,
    DispatchStarted,
    Acknowledged,
    ObservedAfter,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    pub schema_version: u16,
    pub intent_id: String,
    pub operation_id: u64,
    pub resource_id: String,
    pub intent_digest: String,
    pub status: ExecutionStatus,
    pub lease_id: Option<LeaseId>,
    pub dispatched_at: Option<Timestamp>,
    pub before: Option<Observation>,
    pub after: Option<Observation>,
    pub stages: Vec<Stage>,
    pub finished_at: Timestamp,
    pub replayed: bool,
}
impl ExecutionResult {
    pub fn validate_for(&self, intent: &Intent) -> Result<(), JournalError> {
        if self.schema_version != 1
            || self.intent_id != intent.intent_id
            || self.operation_id != intent.operation_id
            || self.resource_id != intent.resource_id
            || self.intent_digest != intent.digest().map_err(|_| JournalError::Corrupt)?
        {
            return Err(JournalError::Corrupt);
        }
        if matches!(
            self.status,
            ExecutionStatus::Verified | ExecutionStatus::Diverged
        ) {
            let before = self.before.as_ref().ok_or(JournalError::Corrupt)?;
            let after = self.after.as_ref().ok_or(JournalError::Corrupt)?;
            let sent = self.dispatched_at.ok_or(JournalError::Corrupt)?;
            if self.lease_id.is_none()
                || before.observed_at > sent
                || before.evidence_id == after.evidence_id
                || !self.stages.contains(&Stage::DispatchStarted)
                || !self.stages.contains(&Stage::Acknowledged)
                || !self.stages.contains(&Stage::ObservedAfter)
                || before.adapter_id != after.adapter_id
                || after
                    .validate_after(intent, &before.adapter_id, sent, self.finished_at)
                    .is_err()
                || (self.status == ExecutionStatus::Verified)
                    != intent.command.matches(&after.state)
            {
                return Err(JournalError::Corrupt);
            }
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("Core authority refused execution")]
    Authority,
    #[error("execution deadline expired or clock rolled back")]
    Deadline,
    #[error("Core execution lock poisoned")]
    Poisoned,
}

struct AuthorityState {
    policy: PolicyEngine,
    lease: Lease,
    principal: Principal,
    user_present: bool,
    reviewed_digest: String,
}
/// Construct only from a durable, reviewed operation in trusted Core code.
/// This is not a network proof. Remote Edge authentication belongs to transport.
#[derive(Clone)]
pub struct CoreAuthority(Arc<Mutex<AuthorityState>>);
impl CoreAuthority {
    /// Applies the composing Core's policy without issuing any authority.
    pub fn check_policy(&self, intent: &Intent, policy: &PolicyEngine) -> Result<(), EngineError> {
        let state = self.0.lock().map_err(|_| EngineError::Poisoned)?;
        if state.reviewed_digest != intent.digest()? {
            return Err(EngineError::Authority);
        }
        let request = CapabilityRequest {
            trace_id: intent.trace_id,
            task_id: None,
            principal: state.principal.clone(),
            resource: Resource::Device {
                device: intent.resource_id.clone(),
            },
            scope: Scope::OneOperation {
                operation: intent.operation_id,
            },
            constraints: state.lease.constraints.clone(),
            justification: "Core Sentry dispatch guard".into(),
        };
        if !policy.decide(&request).decision.grants_authority() {
            return Err(EngineError::Authority);
        }
        Ok(())
    }
    pub fn from_reviewed_intent(
        policy: PolicyEngine,
        lease: Lease,
        principal: Principal,
        user_present: bool,
        reviewed_digest: String,
    ) -> Result<Self, EngineError> {
        if reviewed_digest.len() != 64
            || !reviewed_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(EngineError::Authority);
        }
        Ok(Self(Arc::new(Mutex::new(AuthorityState {
            policy,
            lease,
            principal,
            user_present,
            reviewed_digest,
        }))))
    }
    pub fn revoke(&self) -> Result<(), EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::Poisoned)?
            .lease
            .revoke();
        Ok(())
    }
    pub fn set_emergency_pause(&self, paused: bool) -> Result<(), EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::Poisoned)?
            .policy
            .set_emergency_pause(paused);
        Ok(())
    }
    pub fn set_user_present(&self, present: bool) -> Result<(), EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::Poisoned)?
            .user_present = present;
        Ok(())
    }
    fn authorize(
        &self,
        intent: &Intent,
        now: Timestamp,
        consume: bool,
    ) -> Result<LeaseId, EngineError> {
        let mut state = self.0.lock().map_err(|_| EngineError::Poisoned)?;
        let expiry = state
            .lease
            .constraints
            .expires_at
            .ok_or(EngineError::Authority)?;
        if state.reviewed_digest != intent.digest()?
            || state.lease.trace_id != intent.trace_id
            || state.lease.task_id.is_some()
            || !state.principal.is_authenticated_human()
            || state.lease.scope
                != (Scope::OneOperation {
                    operation: intent.operation_id,
                })
            || state.lease.constraints.max_uses != Some(1)
            || !state.lease.constraints.requires_user_presence
            || expiry
                .0
                .checked_sub(state.lease.issued_at.0)
                .is_none_or(|ms| !(1..=300000).contains(&ms))
        {
            return Err(EngineError::Authority);
        }
        let request = CapabilityRequest {
            trace_id: intent.trace_id,
            task_id: None,
            principal: state.principal.clone(),
            resource: Resource::Device {
                device: intent.resource_id.clone(),
            },
            scope: Scope::OneOperation {
                operation: intent.operation_id,
            },
            constraints: state.lease.constraints.clone(),
            justification: "typed Sentry dispatch".into(),
        };
        if !state.policy.decide(&request).decision.grants_authority() {
            return Err(EngineError::Authority);
        }
        state
            .lease
            .authorises_one_operation(
                &Operation {
                    principal: state.principal.clone(),
                    resource: Resource::Device {
                        device: intent.resource_id.clone(),
                    },
                    action: Action::Actuate,
                    task_id: None,
                    user_present: state.user_present,
                },
                intent.operation_id,
                now,
            )
            .map_err(|_| EngineError::Authority)?;
        let id = state.lease.lease_id;
        if consume {
            state.lease.consume();
        }
        Ok(id)
    }
}

/// Trusted Core checks of live pairing, preparation and policy.
pub trait ExecutionGuard: Send + Sync {
    fn validate(&self, intent: &Intent, authority: &CoreAuthority) -> Result<(), EngineError>;
}
pub struct Engine<J: Journal> {
    registry: Arc<Mutex<Registry>>,
    journal: J,
    guard: Option<Arc<dyn ExecutionGuard>>,
}
impl<J: Journal> Engine<J> {
    pub fn new(registry: Arc<Mutex<Registry>>, journal: J) -> Self {
        Self {
            registry,
            journal,
            guard: None,
        }
    }
    pub fn with_guard(mut self, guard: Arc<dyn ExecutionGuard>) -> Self {
        self.guard = Some(guard);
        self
    }
    fn resolve(
        &self,
        intent: &Intent,
        edge: &str,
        adapter: &str,
    ) -> Result<ResourceDescriptor, EngineError> {
        Ok(self
            .registry
            .lock()
            .map_err(|_| EngineError::Poisoned)?
            .resolve(intent, edge, adapter)?
            .clone())
    }
    pub async fn execute(
        &mut self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
    ) -> Result<ExecutionResult, EngineError> {
        let digest = intent.digest()?;
        if let Some(record) = self.journal.load(&intent.intent_id)? {
            return replay(intent, &digest, record);
        }
        if let Some(guard) = &self.guard {
            guard.validate(intent, authority)?;
        }
        let start = Instant::now();
        let mut budget = Budget::new(intent, start)?;
        let edge_id = adapter.edge_id().to_owned();
        let adapter_id = adapter.adapter_id().to_owned();
        let resource = self.resolve(intent, &edge_id, &adapter_id)?;
        let lease_id = authority.authorize(intent, Timestamp::now(), false)?;
        if let Some(record) = self.journal.reserve(intent, &digest, Timestamp::now())? {
            return replay(intent, &digest, record);
        }
        let mut result = ExecutionResult {
            schema_version: 1,
            intent_id: intent.intent_id.clone(),
            operation_id: intent.operation_id,
            resource_id: intent.resource_id.clone(),
            intent_digest: digest,
            status: ExecutionStatus::Unavailable,
            lease_id: Some(lease_id),
            dispatched_at: None,
            before: None,
            after: None,
            stages: vec![Stage::Resolved, Stage::Authorized, Stage::Reserved],
            finished_at: Timestamp::now(),
            replayed: false,
        };
        self.run(
            intent,
            authority,
            adapter,
            &resource,
            &mut budget,
            &mut result,
        )
        .await;
        result.finished_at = Timestamp::now();
        result.validate_for(intent)?;
        self.journal.finish(&result)?;
        Ok(result)
    }
    async fn run(
        &self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
        resource: &ResourceDescriptor,
        budget: &mut Budget,
        result: &mut ExecutionResult,
    ) {
        let duration = match budget.remaining(intent) {
            Ok(d) => d,
            Err(_) => return,
        };
        let before = match tokio::time::timeout(duration, adapter.observe(resource)).await {
            Ok(Ok(value)) => value,
            _ => return,
        };
        if before
            .validate_after(
                intent,
                &resource.adapter_id,
                intent.created_at,
                Timestamp::now(),
            )
            .is_err()
        {
            return;
        }
        result.before = Some(before);
        result.stages.push(Stage::ObservedBefore);
        let duration = match budget.remaining(intent) {
            Ok(d) => d,
            Err(_) => return,
        };
        if !matches!(
            tokio::time::timeout(duration, adapter.preflight(resource, &intent.command)).await,
            Ok(Ok(()))
        ) {
            return;
        }
        if self
            .guard
            .as_ref()
            .is_some_and(|g| g.validate(intent, authority).is_err())
        {
            result.status = ExecutionStatus::Denied;
            return;
        }
        if self
            .resolve(intent, &resource.edge_id, &resource.adapter_id)
            .is_err()
            || adapter.edge_id() != resource.edge_id
            || adapter.adapter_id() != resource.adapter_id
        {
            result.status = ExecutionStatus::Denied;
            return;
        }
        if result.before.as_ref().is_none_or(|b| {
            b.validate_after(
                intent,
                &resource.adapter_id,
                intent.created_at,
                Timestamp::now(),
            )
            .is_err()
        }) {
            return;
        }
        // Storage checks may have waited: recompute the budget before dispatch.
        let duration = match budget.remaining(intent) {
            Ok(d) => d,
            Err(_) => return,
        };
        if authority.authorize(intent, Timestamp::now(), true).is_err() {
            result.status = ExecutionStatus::Denied;
            return;
        }
        let dispatched_at = Timestamp::now();
        result.dispatched_at = Some(dispatched_at);
        result.stages.push(Stage::DispatchStarted);
        result.status = ExecutionStatus::Indeterminate;
        let ack =
            match tokio::time::timeout(duration, adapter.apply(resource, &intent.command)).await {
                Ok(Ok(value)) => value,
                _ => return,
            };
        if !ack.accepted {
            return;
        }
        result.stages.push(Stage::Acknowledged);
        let duration = match budget.remaining(intent) {
            Ok(d) => d,
            Err(_) => return,
        };
        let after = match tokio::time::timeout(duration, adapter.observe(resource)).await {
            Ok(Ok(value)) => value,
            _ => return,
        };
        if after
            .validate_after(
                intent,
                &resource.adapter_id,
                dispatched_at,
                Timestamp::now(),
            )
            .is_err()
            || result
                .before
                .as_ref()
                .is_some_and(|b| b.evidence_id == after.evidence_id)
        {
            return;
        }
        result.status = if intent.command.matches(&after.state) {
            ExecutionStatus::Verified
        } else {
            ExecutionStatus::Diverged
        };
        result.after = Some(after);
        result.stages.push(Stage::ObservedAfter);
    }
}

fn replay(
    intent: &Intent,
    digest: &str,
    record: JournalRecord,
) -> Result<ExecutionResult, EngineError> {
    if record.digest != digest
        || record.operation_id != intent.operation_id
        || record.intent_id != intent.intent_id
    {
        return Err(JournalError::Collision.into());
    }
    let mut result = record.result.unwrap_or_else(|| ExecutionResult {
        schema_version: 1,
        intent_id: intent.intent_id.clone(),
        operation_id: intent.operation_id,
        resource_id: intent.resource_id.clone(),
        intent_digest: digest.into(),
        status: ExecutionStatus::Indeterminate,
        lease_id: None,
        dispatched_at: None,
        before: None,
        after: None,
        stages: vec![Stage::Reserved],
        finished_at: Timestamp::now(),
        replayed: true,
    });
    result.replayed = true;
    result.validate_for(intent)?;
    Ok(result)
}

struct Budget {
    start: Instant,
    initial: Duration,
    last_wall: Timestamp,
}
impl Budget {
    fn new(intent: &Intent, start: Instant) -> Result<Self, EngineError> {
        let now = Timestamp::now();
        if now < intent.created_at || now >= intent.deadline_at {
            return Err(EngineError::Deadline);
        }
        Ok(Self {
            start,
            initial: Duration::from_millis((intent.deadline_at.0 - now.0) as u64),
            last_wall: now,
        })
    }
    fn remaining(&mut self, intent: &Intent) -> Result<Duration, EngineError> {
        let now = Timestamp::now();
        if now < self.last_wall || now >= intent.deadline_at {
            return Err(EngineError::Deadline);
        }
        self.last_wall = now;
        let monotonic = self
            .initial
            .checked_sub(self.start.elapsed())
            .ok_or(EngineError::Deadline)?;
        Ok(monotonic.min(Duration::from_millis((intent.deadline_at.0 - now.0) as u64)))
    }
}
