use archie_policy::{
    Action, Constraints, Lease, PolicyConfig, PolicyEngine, Principal, Resource, Scope,
};
use archie_sentry::{contracts::*, engine::*, journal::*, registry::*};
use archie_types::{LeaseId, Timestamp, TraceId};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

fn intent() -> Intent {
    let now = Timestamp::now();
    Intent {
        schema_version: 1,
        intent_id: "intent-1".into(),
        operation_id: 1,
        trace_id: TraceId::new(),
        resource_id: "switch-office".into(),
        command: Command::SwitchSet { on: true },
        created_at: now,
        deadline_at: now.plus_millis(2000),
        max_observation_age_ms: 1000,
        verify: true,
    }
}
fn authority(request: &Intent) -> CoreAuthority {
    let human = Principal::Human {
        id: "operator".into(),
        authenticated: true,
    };
    let now = Timestamp::now();
    let lease = Lease {
        lease_id: LeaseId::new(),
        trace_id: request.trace_id,
        task_id: None,
        principal: human.clone(),
        resource: Resource::Device {
            device: request.resource_id.clone(),
        },
        scope: Scope::OneOperation {
            operation: request.operation_id,
        },
        constraints: Constraints {
            actions: [Action::Read, Action::Actuate].into_iter().collect(),
            max_uses: Some(1),
            expires_at: Some(now.plus_millis(3000)),
            requires_user_presence: true,
        },
        issued_at: now,
        uses: 0,
        revoked: false,
    };
    CoreAuthority::from_reviewed_intent(
        PolicyEngine::new(PolicyConfig {
            safety_boundary_present: true,
            ..PolicyConfig::default()
        }),
        lease,
        human,
        true,
        request.digest().unwrap(),
    )
    .unwrap()
}
fn registry() -> Arc<Mutex<Registry>> {
    let mut registry = Registry::default();
    registry
        .register(ResourceDescriptor {
            resource_id: "switch-office".into(),
            edge_id: "edge-home".into(),
            adapter_id: "fixture".into(),
            paired: true,
            capabilities: vec![CommandKind::SwitchSet],
        })
        .unwrap();
    Arc::new(Mutex::new(registry))
}

#[tokio::test]
async fn climate_preparation_revocation_is_checked_before_the_actuation_post() {
    use archie_sentry::adapters::home_assistant::HomeAssistant;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let mut i = intent();
    i.command = Command::ClimateSetTemperature {
        milli_celsius: 22500,
    };
    let a = authority(&i);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let revocable = a.clone();
    let posts = Arc::new(AtomicUsize::new(0));
    let sends = posts.clone();
    let server = tokio::spawn(async move {
        let mut configs = 0;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(request.len() < 8192);
            }
            let header = String::from_utf8_lossy(&request);
            let response = if header.starts_with("GET /api/config ") {
                configs += 1;
                if configs == 2 {
                    revocable.set_user_present(false).unwrap();
                }
                serde_json::json!({"unit_system":{"temperature":"°C"}})
            } else if header.starts_with("GET /api/states/climate.office ") {
                serde_json::json!({"entity_id":"climate.office","state":"heat",
                    "attributes":{"temperature":if sends.load(Ordering::SeqCst) == 0 {21.5} else {22.5}},
                    "last_reported":Timestamp::now().to_rfc3339()})
            } else {
                if header.starts_with("POST /api/services/climate/set_temperature ") {
                    sends.fetch_add(1, Ordering::SeqCst);
                } else {
                    assert!(header.starts_with("POST /api/services/homeassistant/update_entity "));
                }
                serde_json::json!([])
            };
            let bytes = serde_json::to_vec(&response).unwrap();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).as_bytes()).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        }
    });
    let mut routes = Registry::default();
    routes
        .register(ResourceDescriptor {
            resource_id: i.resource_id.clone(),
            edge_id: "edge-home".into(),
            adapter_id: "fixture".into(),
            paired: true,
            capabilities: vec![CommandKind::ClimateSetTemperature],
        })
        .unwrap();
    let mut adapter = HomeAssistant::new(
        "edge-home",
        "fixture",
        &base,
        "fixture-token",
        BTreeMap::from([(i.resource_id.clone(), "climate.office".into())]),
    )
    .unwrap();
    let mut engine = Engine::new(Arc::new(Mutex::new(routes)), MemoryJournal::default());
    let result = engine.execute(&i, &a, &mut adapter).await.unwrap();
    server.abort();
    assert_eq!(
        posts.load(Ordering::SeqCst),
        0,
        "revocation during units preparation must prevent POST"
    );
    assert_eq!(result.status, ExecutionStatus::Denied);
    assert!(result.dispatched_at.is_none());
}

struct SlowGuard(std::sync::atomic::AtomicUsize);
impl ExecutionGuard for SlowGuard {
    fn validate(&self, _: &Intent, _: &CoreAuthority) -> Result<(), EngineError> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
        Ok(())
    }
}

struct UnpairGuard {
    calls: std::sync::atomic::AtomicUsize,
    registry: Arc<Mutex<Registry>>,
}
impl ExecutionGuard for UnpairGuard {
    fn validate(&self, _: &Intent, _: &CoreAuthority) -> Result<(), EngineError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            self.registry
                .lock()
                .unwrap()
                .set_paired("switch-office", false)
                .unwrap();
        }
        Ok(())
    }
}
#[tokio::test]
async fn registry_revoked_during_core_guard_is_checked_again_before_dispatch() {
    let i = intent();
    let a = authority(&i);
    let mut edge = Switch::default();
    let registry = registry();
    let guard = Arc::new(UnpairGuard {
        calls: std::sync::atomic::AtomicUsize::new(0),
        registry: registry.clone(),
    });
    let mut engine = Engine::new(registry, MemoryJournal::default()).with_guard(guard);
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(edge.applications, 0);
    assert_eq!(r.status, ExecutionStatus::Denied);
}

#[derive(Default)]
struct FailFinishJournal(MemoryJournal);
impl Journal for FailFinishJournal {
    fn load(&mut self, id: &str) -> Result<Option<JournalRecord>, JournalError> {
        self.0.load(id)
    }
    fn reserve(
        &mut self,
        i: &Intent,
        digest: &str,
        at: Timestamp,
    ) -> Result<Option<JournalRecord>, JournalError> {
        self.0.reserve(i, digest, at)
    }
    fn finish(&mut self, _: &ExecutionResult) -> Result<(), JournalError> {
        Err(JournalError::Unavailable)
    }
}
#[tokio::test]
async fn failed_receipt_write_after_actuation_preserves_fence_and_prevents_resend() {
    let i = intent();
    let a = authority(&i);
    let mut edge = Switch::default();
    let mut engine = Engine::new(registry(), FailFinishJournal::default());
    assert!(matches!(
        engine.execute(&i, &a, &mut edge).await,
        Err(EngineError::Journal(JournalError::Unavailable))
    ));
    assert_eq!(edge.applications, 1);
    a.revoke().unwrap();
    let replay = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(replay.status, ExecutionStatus::Indeterminate);
    assert!(replay.replayed);
    assert_eq!(edge.applications, 1);
    assert_eq!(edge.observations, 2);
}
#[tokio::test]
async fn observation_that_ages_during_core_guard_cannot_dispatch() {
    let mut i = intent();
    i.max_observation_age_ms = 20;
    let a = authority(&i);
    let mut edge = Switch::default();
    let guard = Arc::new(SlowGuard(std::sync::atomic::AtomicUsize::new(0)));
    let mut engine = Engine::new(registry(), MemoryJournal::default()).with_guard(guard);
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(edge.applications, 0);
    assert_eq!(r.status, ExecutionStatus::Unavailable);
}
struct Switch {
    on: bool,
    apply_changes: bool,
    applications: u32,
    observations: u32,
    origin: ObservationOrigin,
    delay_ms: u64,
    revoke_after_observe: Option<CoreAuthority>,
    unpair_after_observe: Option<Arc<Mutex<Registry>>>,
    future_observation: bool,
}
impl Default for Switch {
    fn default() -> Self {
        Self {
            on: false,
            apply_changes: true,
            applications: 0,
            observations: 0,
            origin: ObservationOrigin::Measurement,
            delay_ms: 0,
            revoke_after_observe: None,
            unpair_after_observe: None,
            future_observation: false,
        }
    }
}
#[async_trait]
impl EdgeAdapter for Switch {
    fn adapter_id(&self) -> &str {
        "fixture"
    }
    fn edge_id(&self) -> &str {
        "edge-home"
    }
    async fn observe(
        &mut self,
        resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        self.observations += 1;
        if let Some(a) = &self.revoke_after_observe {
            a.revoke().unwrap();
        }
        if let Some(r) = &self.unpair_after_observe {
            r.lock()
                .unwrap()
                .set_paired("switch-office", false)
                .unwrap();
        }
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: "fixture".into(),
            evidence_id: format!("measurement-{}", self.observations),
            observed_at: Timestamp::now().plus_millis(if self.future_observation {
                5000
            } else {
                0
            }),
            origin: self.origin,
            state: ResourceState::Switch { on: self.on },
        })
    }
    async fn apply(
        &mut self,
        _: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        self.applications += 1;
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if self.apply_changes {
            if let Command::SwitchSet { on } = command {
                self.on = *on;
            }
        }
        Ok(AdapterAck { accepted: true })
    }
}

#[tokio::test]
async fn measured_postcondition_verifies_and_exact_replay_never_dispatches_twice() {
    let i = intent();
    let a = authority(&i);
    let mut edge = Switch::default();
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let first = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(first.status, ExecutionStatus::Verified);
    assert_eq!(
        first.after.unwrap().state,
        ResourceState::Switch { on: true }
    );
    let second = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(second.status, ExecutionStatus::Verified);
    assert!(second.replayed);
    assert_eq!(edge.applications, 1);
}
#[tokio::test]
async fn acknowledgement_without_observed_change_is_diverged() {
    let i = intent();
    let a = authority(&i);
    let mut edge = Switch {
        apply_changes: false,
        ..Switch::default()
    };
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(r.status, ExecutionStatus::Diverged);
    assert_eq!(r.after.unwrap().state, ResourceState::Switch { on: false });
    assert_eq!(edge.applications, 1);
}
#[tokio::test]
async fn emergency_pause_and_absent_operator_refuse_before_io() {
    for mode in [0, 1] {
        let i = intent();
        let a = authority(&i);
        if mode == 0 {
            a.set_emergency_pause(true).unwrap();
        } else {
            a.set_user_present(false).unwrap();
        }
        let mut edge = Switch::default();
        let mut engine = Engine::new(registry(), MemoryJournal::default());
        assert!(engine.execute(&i, &a, &mut edge).await.is_err());
        assert_eq!(edge.applications, 0);
        assert_eq!(edge.observations, 0);
    }
}
#[tokio::test]
async fn revocation_between_observation_and_dispatch_refuses_command() {
    let i = intent();
    let a = authority(&i);
    let mut edge = Switch {
        revoke_after_observe: Some(a.clone()),
        ..Switch::default()
    };
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(r.status, ExecutionStatus::Denied);
    assert_eq!(edge.applications, 0);
}
#[tokio::test]
async fn unpairing_between_observation_and_dispatch_refuses_command() {
    let i = intent();
    let a = authority(&i);
    let registry = registry();
    let mut edge = Switch {
        unpair_after_observe: Some(registry.clone()),
        ..Switch::default()
    };
    let mut engine = Engine::new(registry, MemoryJournal::default());
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(r.status, ExecutionStatus::Denied);
    assert_eq!(edge.applications, 0);
}
#[tokio::test]
async fn reviewed_payload_cannot_be_substituted_under_an_approved_operation() {
    let i = intent();
    let a = authority(&i);
    let mut changed = i.clone();
    changed.command = Command::SwitchSet { on: false };
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let mut edge = Switch::default();
    assert!(engine.execute(&changed, &a, &mut edge).await.is_err());
    assert_eq!(edge.applications, 0);
}
#[tokio::test]
async fn intent_and_operation_collisions_never_dispatch_new_payload() {
    let i = intent();
    let a = authority(&i);
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let mut edge = Switch::default();
    engine.execute(&i, &a, &mut edge).await.unwrap();
    let mut changed = i.clone();
    changed.command = Command::SwitchSet { on: false };
    assert!(engine
        .execute(&changed, &authority(&changed), &mut edge)
        .await
        .is_err());
    changed.intent_id = "another-intent".into();
    assert!(engine
        .execute(&changed, &authority(&changed), &mut edge)
        .await
        .is_err());
    assert_eq!(edge.applications, 1);
}
#[tokio::test]
async fn pending_durable_reservation_is_indeterminate_and_never_retried() {
    let i = intent();
    let a = authority(&i);
    let mut journal = MemoryJournal::default();
    journal
        .reserve(&i, &i.digest().unwrap(), Timestamp::now())
        .unwrap();
    let mut engine = Engine::new(registry(), journal);
    let mut edge = Switch::default();
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(r.status, ExecutionStatus::Indeterminate);
    assert!(r.replayed);
    assert_eq!(edge.applications, 0);
    assert_eq!(edge.observations, 0);
}
#[tokio::test]
async fn untrusted_or_future_observation_cannot_authorize_dispatch() {
    for mode in [0, 1] {
        let i = intent();
        let a = authority(&i);
        let mut edge = Switch {
            origin: if mode == 0 {
                ObservationOrigin::ModelHypothesis
            } else {
                ObservationOrigin::Measurement
            },
            future_observation: mode == 1,
            ..Switch::default()
        };
        let mut engine = Engine::new(registry(), MemoryJournal::default());
        let r = engine.execute(&i, &a, &mut edge).await.unwrap();
        assert_eq!(r.status, ExecutionStatus::Unavailable);
        assert_eq!(edge.applications, 0);
    }
}
#[tokio::test]
async fn timed_out_dispatch_is_ambiguous_and_never_retried() {
    let mut i = intent();
    i.deadline_at = i.created_at.plus_millis(40);
    let a = authority(&i);
    let mut edge = Switch {
        delay_ms: 200,
        ..Switch::default()
    };
    let mut engine = Engine::new(registry(), MemoryJournal::default());
    let r = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(r.status, ExecutionStatus::Indeterminate);
    let replay = engine.execute(&i, &a, &mut edge).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(edge.applications, 1);
}

struct CorruptReplay {
    record: JournalRecord,
}
impl Journal for CorruptReplay {
    fn load(&mut self, _: &str) -> Result<Option<JournalRecord>, JournalError> {
        Ok(Some(self.record.clone()))
    }
    fn reserve(
        &mut self,
        _: &Intent,
        _: &str,
        _: Timestamp,
    ) -> Result<Option<JournalRecord>, JournalError> {
        Err(JournalError::Unavailable)
    }
    fn finish(&mut self, _: &ExecutionResult) -> Result<(), JournalError> {
        Err(JournalError::Unavailable)
    }
}
#[tokio::test]
async fn corrupt_completed_receipt_cannot_claim_another_resource_was_verified() {
    let i = intent();
    let a = authority(&i);
    let receipt = ExecutionResult {
        schema_version: 1,
        intent_id: i.intent_id.clone(),
        operation_id: i.operation_id,
        resource_id: "another-switch".into(),
        intent_digest: i.digest().unwrap(),
        status: ExecutionStatus::Verified,
        lease_id: None,
        dispatched_at: None,
        before: None,
        after: None,
        stages: vec![],
        finished_at: Timestamp::now(),
        replayed: false,
    };
    let journal = CorruptReplay {
        record: JournalRecord {
            intent_id: i.intent_id.clone(),
            operation_id: i.operation_id,
            digest: i.digest().unwrap(),
            reserved_at: Timestamp::now(),
            result: Some(receipt),
        },
    };
    let mut engine = Engine::new(registry(), journal);
    let mut edge = Switch::default();
    assert!(engine.execute(&i, &a, &mut edge).await.is_err());
    assert_eq!(edge.applications, 0);
}
