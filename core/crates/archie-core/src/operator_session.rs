//! One authenticated connection owns one short-lived confirmation.
use crate::{
    control::{ControlRequest, ControlResponse},
    operator::KernelOperator,
    Runtime,
};
use archie_policy::{Action, CapabilityRequest, Constraints, Principal, Resource, Scope};
use archie_sentry::{
    contracts::{Command, Intent},
    engine::{CoreAuthority, EngineError, ExecutionGuard},
    journal::Journal,
    registry::{Registry, ResourceDescriptor},
};
use archie_types::{Timestamp, TraceId};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

struct SessionGuard {
    peer: KernelOperator,
    connected: Arc<AtomicBool>,
}
impl ExecutionGuard for SessionGuard {
    fn validate(&self, _: &Intent, _: &CoreAuthority) -> Result<(), EngineError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(EngineError::Authority);
        }
        self.peer.check_present()
    }
}
struct OperatorSession {
    core: Arc<Runtime>,
    peer: Option<KernelOperator>,
    pending: Option<Intent>,
    review_started: Option<std::time::Instant>,
    connected: Arc<AtomicBool>,
}
impl Drop for OperatorSession {
    fn drop(&mut self) {
        self.connected.store(false, Ordering::SeqCst);
        if let Some(intent) = self.pending.take() {
            let _ = self.core.state.cancel_device_operation(intent.operation_id);
        }
    }
}
impl OperatorSession {
    fn peer(&self) -> Result<&KernelOperator, EngineError> {
        let peer = self.peer.as_ref().ok_or(EngineError::Authority)?;
        peer.check_present()?;
        Ok(peer)
    }
    fn service(&self) -> Result<crate::sentry::SentryService, EngineError> {
        let binding = self
            .core
            .config
            .sentry
            .home_assistant
            .as_ref()
            .ok_or(EngineError::Authority)?;
        let mut registry = Registry::default();
        for resource in &binding.resources {
            let device = self
                .core
                .state
                .get_device(&resource.resource_id)
                .map_err(|_| EngineError::Authority)?;
            let paired = device.is_some_and(|d| {
                d.pairing == archie_state::DevicePairingState::Paired
                    && d.class == archie_state::DeviceClass::HomeAssistant
            });
            registry.register(ResourceDescriptor {
                resource_id: resource.resource_id.clone(),
                edge_id: "local-core".into(),
                adapter_id: "home-assistant".into(),
                paired,
                capabilities: resource
                    .capabilities()
                    .map_err(|_| EngineError::Authority)?,
            })?;
        }
        Ok(self.core.sentry_service(Arc::new(Mutex::new(registry))))
    }
    fn prepare(
        &mut self,
        resource_id: String,
        command: Command,
    ) -> Result<ControlResponse, EngineError> {
        let uid = self.peer()?.uid();
        if self.pending.is_some() {
            return Err(EngineError::Authority);
        }
        let binding = self
            .core
            .config
            .sentry
            .home_assistant
            .as_ref()
            .ok_or(EngineError::Authority)?;
        let resource = binding
            .resources
            .iter()
            .find(|r| r.resource_id == resource_id)
            .ok_or(EngineError::Authority)?;
        if !resource
            .capabilities()
            .map_err(|_| EngineError::Authority)?
            .contains(&command.capability())
        {
            return Err(EngineError::Authority);
        }
        let now = Timestamp::now();
        let intent = self.service()?.prepare(Intent {
            schema_version: 1,
            intent_id: TraceId::new().to_string(),
            operation_id: 1,
            trace_id: TraceId::new(),
            resource_id,
            command,
            created_at: now,
            deadline_at: now.plus_millis(30000),
            max_observation_age_ms: 3000,
            verify: true,
        })?;
        let digest = intent.digest()?;
        self.pending = Some(intent.clone());
        self.review_started = Some(std::time::Instant::now());
        self.core
            .state
            .append_audit(
                &archie_observability::AuditEvent::new(
                    self.core.node_id(),
                    "device.sentry.prepare",
                    format!("human:unix-uid-{uid}"),
                    format!("device:{}", intent.resource_id),
                    archie_observability::AuditOutcome::PendingApproval,
                    "typed intention prepared; no dispatch authority granted",
                )
                .with_trace(intent.trace_id)
                .with_attribute("intent_digest", &digest),
            )
            .map_err(|_| EngineError::Authority)?;
        Ok(ControlResponse::SentryPrepared {
            intent: Box::new(intent),
            digest,
        })
    }
    async fn approve(
        &mut self,
        intent_id: String,
        digest: String,
    ) -> Result<ControlResponse, EngineError> {
        let intent = self.pending.take().ok_or(EngineError::Authority)?;
        let review_started = self.review_started.take().ok_or(EngineError::Authority)?;
        if intent.intent_id != intent_id
            || intent.digest()? != digest
            || Timestamp::now() > intent.deadline_at
            || review_started.elapsed() > std::time::Duration::from_secs(30)
        {
            let _ = self.core.state.cancel_device_operation(intent.operation_id);
            return Err(EngineError::Authority);
        }
        let peer = self.peer()?.clone();
        let principal = Principal::Human {
            id: format!("unix-uid-{}", peer.uid()),
            authenticated: true,
        };
        let request = CapabilityRequest {
            trace_id: intent.trace_id,
            task_id: None,
            principal: principal.clone(),
            resource: Resource::Device {
                device: intent.resource_id.clone(),
            },
            scope: Scope::OneOperation {
                operation: intent.operation_id,
            },
            constraints: Constraints {
                actions: [Action::Read, Action::Actuate].into_iter().collect(),
                max_uses: Some(1),
                expires_at: Some(intent.deadline_at),
                requires_user_presence: true,
            },
            justification:
                "exact typed intent confirmed on authenticated interactive local session".into(),
        };
        let decision = self.core.policy().decide(&request);
        self.core
            .state
            .append_audit(
                &decision
                    .to_audit(self.core.node_id(), &request)
                    .with_attribute("intent_digest", &digest)
                    .with_attribute("operation_id", intent.operation_id.to_string()),
            )
            .map_err(|_| EngineError::Authority)?;
        let lease = decision.lease.ok_or(EngineError::Authority)?;
        let authority = CoreAuthority::from_reviewed_intent(
            self.core.policy().clone(),
            lease,
            principal,
            true,
            digest,
        )?;
        let binding = self
            .core
            .config
            .sentry
            .home_assistant
            .as_ref()
            .ok_or(EngineError::Authority)?;
        let mut adapter = binding.adapter().map_err(|_| EngineError::Authority)?;
        let guard = Arc::new(SessionGuard {
            peer,
            connected: self.connected.clone(),
        });
        let result = self
            .service()?
            .execute_with_guard(&intent, &authority, &mut adapter, guard)
            .await?;
        Ok(ControlResponse::SentryResult(Box::new(result)))
    }
    async fn handle(&mut self, request: ControlRequest) -> ControlResponse {
        let outcome = match request {
            ControlRequest::SentryPrepare {
                resource_id,
                command,
            } => self.prepare(resource_id, command),
            ControlRequest::SentryApprove { intent_id, digest } => {
                self.approve(intent_id, digest).await
            }
            ControlRequest::SentryCancel => {
                if self.peer().is_err() {
                    return ControlResponse::error("authenticated local operator session required");
                }
                if let Some(intent) = self.pending.take() {
                    if self
                        .core
                        .state
                        .cancel_device_operation(intent.operation_id)
                        .is_err()
                    {
                        return ControlResponse::error(
                            "Sentry cancellation could not be persisted",
                        );
                    }
                }
                self.review_started = None;
                Ok(ControlResponse::SentryCancelled)
            }
            ControlRequest::SentryReceipt { intent_id } => self.receipt(&intent_id),
            other => return self.core.handle_control(other).await,
        };
        outcome.unwrap_or_else(|_| {
            ControlResponse::error("Sentry operation refused or persistence unavailable")
        })
    }
    fn receipt(&self, intent_id: &str) -> Result<ControlResponse, EngineError> {
        self.peer()?;
        archie_sentry::contracts::validate_id(intent_id)?;
        let record = self
            .core
            .state
            .sentry_journal()?
            .load(intent_id)?
            .ok_or(EngineError::Authority)?;
        Ok(ControlResponse::SentryReceipt(Box::new(record)))
    }
}

/// Context is captured before splitting. EOF/pipelining during a mutation drops
/// execution; a reservation already written remains a durable dispatch fence.
pub async fn serve_operator_connection(
    stream: crate::endpoint::ControlStream,
    core: Arc<Runtime>,
) -> crate::control::Result<()> {
    use crate::control::{decode, encode, MAX_CONTROL_MESSAGE_BYTES};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let peer = KernelOperator::from_stream(&stream, &core.config.sentry.operator_uids).ok();
    let mut session = OperatorSession {
        core,
        peer,
        pending: None,
        review_started: None,
        connected: Arc::new(AtomicBool::new(true)),
    };
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        let count = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            (&mut reader)
                .take(MAX_CONTROL_MESSAGE_BYTES as u64 + 1)
                .read_line(&mut line),
        )
        .await
        .map_err(|_| {
            crate::control::ControlError::Malformed("control session timed out".into())
        })??;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_CONTROL_MESSAGE_BYTES {
            return Err(crate::control::ControlError::TooLarge);
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = match decode::<ControlRequest>(&line) {
            Ok(request) if request.is_mutating() => {
                tokio::select! { biased;
                    _ = reader.fill_buf() => { return Ok(()); }
                    response = session.handle(request) => response,
                }
            }
            Ok(request) => session.handle(request).await,
            Err(_) => ControlResponse::error("malformed control request"),
        };
        write_half.write_all(&encode(&response)?).await?;
        write_half.flush().await?;
    }
}
