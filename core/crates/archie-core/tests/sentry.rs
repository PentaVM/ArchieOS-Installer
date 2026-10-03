use archie_core::sentry::SentryService;
use archie_core::{config::ArchieConfig, Runtime};
use archie_policy::{
    Action, Constraints, Lease, PolicyConfig, PolicyEngine, Principal, Resource, Scope,
};
use archie_sentry::{contracts::*, engine::*, registry::*};
use archie_state::StatePaths;
use archie_state::{DeviceClass, StateStore};
use archie_types::{LeaseId, Timestamp, TraceId};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

fn request() -> Intent {
    let now = Timestamp::now();
    Intent {
        schema_version: 1,
        intent_id: "core-switch-on".into(),
        operation_id: 1,
        trace_id: TraceId::new(),
        resource_id: "office-switch".into(),
        command: Command::SwitchSet { on: true },
        created_at: now,
        deadline_at: now.plus_millis(30000),
        max_observation_age_ms: 1000,
        verify: true,
    }
}
fn fixture() -> Arc<StateStore> {
    let state = Arc::new(StateStore::open_test().unwrap());
    state
        .propose_device_pairing("office-switch", &"a".repeat(64), DeviceClass::HomeAssistant)
        .unwrap();
    state.confirm_device_pairing("office-switch").unwrap();
    state
}
fn policy() -> PolicyEngine {
    PolicyEngine::new(PolicyConfig {
        safety_boundary_present: true,
        ..PolicyConfig::default()
    })
}
fn service(state: Arc<StateStore>, policy: PolicyEngine) -> SentryService {
    let mut registry = Registry::default();
    registry
        .register(ResourceDescriptor {
            resource_id: "office-switch".into(),
            edge_id: "core-fixture".into(),
            adapter_id: "switch-fixture".into(),
            paired: true,
            capabilities: vec![CommandKind::SwitchSet],
        })
        .unwrap();
    SentryService::new(state, Arc::new(Mutex::new(registry)), policy)
}
fn authority(i: &Intent) -> CoreAuthority {
    let human = Principal::Human {
        id: "fixture-operator".into(),
        authenticated: true,
    };
    let now = Timestamp::now();
    let lease = Lease {
        lease_id: LeaseId::new(),
        trace_id: i.trace_id,
        task_id: None,
        principal: human.clone(),
        resource: Resource::Device {
            device: i.resource_id.clone(),
        },
        scope: Scope::OneOperation {
            operation: i.operation_id,
        },
        constraints: Constraints {
            actions: [Action::Read, Action::Actuate].into_iter().collect(),
            max_uses: Some(1),
            expires_at: Some(now.plus_millis(30000)),
            requires_user_presence: true,
        },
        issued_at: now,
        uses: 0,
        revoked: false,
    };
    CoreAuthority::from_reviewed_intent(policy(), lease, human, true, i.digest().unwrap()).unwrap()
}
struct Switch {
    state: Arc<StateStore>,
    revoke: bool,
    on: bool,
    sends: usize,
    observations: usize,
}
#[async_trait]
impl EdgeAdapter for Switch {
    fn edge_id(&self) -> &str {
        "core-fixture"
    }
    fn adapter_id(&self) -> &str {
        "switch-fixture"
    }
    async fn observe(&mut self, r: &ResourceDescriptor) -> Result<Observation, AdapterError> {
        self.observations += 1;
        if self.revoke {
            self.state.revoke_device(&r.resource_id).unwrap();
        }
        Ok(Observation {
            resource_id: r.resource_id.clone(),
            adapter_id: self.adapter_id().into(),
            evidence_id: format!("core-measurement-{}", self.observations),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state: ResourceState::Switch { on: self.on },
        })
    }
    async fn apply(
        &mut self,
        _: &ResourceDescriptor,
        c: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        self.sends += 1;
        if let Command::SwitchSet { on } = c {
            self.on = *on;
        }
        Ok(AdapterAck { accepted: true })
    }
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn core_preparation_binds_allocated_id_and_rechecks_durable_pairing() {
    let state = fixture();
    let service = service(state.clone(), policy());
    let i = service.prepare(request()).unwrap();
    let a = authority(&i);
    let stored = state.get_device_operation(i.operation_id).unwrap().unwrap();
    assert_eq!(stored.operation_sha256, i.digest().unwrap());
    assert_eq!(stored.trace_id, i.trace_id);
    state
        .validate_prepared_device_operation(
            i.operation_id,
            &i.resource_id,
            i.trace_id,
            &i.digest().unwrap(),
        )
        .unwrap();
    let mut edge = Switch {
        state: state.clone(),
        revoke: true,
        on: false,
        sends: 0,
        observations: 0,
    };
    let result = service.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Denied);
    assert_eq!(edge.sends, 0);
    assert_eq!(edge.observations, 1);
    assert!(state
        .validate_prepared_device_operation(
            i.operation_id,
            &i.resource_id,
            i.trace_id,
            &i.digest().unwrap()
        )
        .is_err());
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn default_core_policy_refuses_external_authority_before_io() {
    let state = fixture();
    let service = service(state.clone(), PolicyEngine::default());
    let i = service.prepare(request()).unwrap();
    let a = authority(&i);
    let mut edge = Switch {
        state,
        revoke: false,
        on: false,
        sends: 0,
        observations: 0,
    };
    assert!(service.execute(&i, &a, &mut edge).await.is_err());
    assert_eq!(edge.sends, 0);
    assert_eq!(edge.observations, 0);
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn cancelled_preparation_and_substituted_intent_are_refused() {
    let state = fixture();
    let service = service(state.clone(), policy());
    let i = service.prepare(request()).unwrap();
    let mut changed = i.clone();
    changed.command = Command::SwitchSet { on: false };
    let mut edge = Switch {
        state: state.clone(),
        revoke: false,
        on: false,
        sends: 0,
        observations: 0,
    };
    assert!(service
        .execute(&changed, &authority(&changed), &mut edge)
        .await
        .is_err());
    state.cancel_device_operation(i.operation_id).unwrap();
    assert!(service
        .execute(&i, &authority(&i), &mut edge)
        .await
        .is_err());
    assert_eq!(edge.observations, 0);
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn core_executes_once_and_replays_historical_result_after_cancellation() {
    let state = fixture();
    let service = service(state.clone(), policy());
    let i = service.prepare(request()).unwrap();
    let a = authority(&i);
    let mut edge = Switch {
        state: state.clone(),
        revoke: false,
        on: false,
        sends: 0,
        observations: 0,
    };
    let first = service.execute(&i, &a, &mut edge).await.unwrap();
    assert_eq!(first.status, ExecutionStatus::Verified);
    state.cancel_device_operation(i.operation_id).unwrap();
    a.revoke().unwrap();
    let replay = service.execute(&i, &a, &mut edge).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.status, ExecutionStatus::Verified);
    assert_eq!(edge.sends, 1);
    assert_eq!(edge.observations, 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn booted_runtime_composes_sentry_with_its_real_store_and_policy() {
    let root = std::env::temp_dir().join(format!("archie-sentry-runtime-{}", TraceId::new()));
    let runtime = Runtime::boot(StatePaths::rooted(root), ArchieConfig::default())
        .await
        .unwrap();
    assert!(runtime.is_ready());
    runtime
        .state
        .propose_device_pairing("office-switch", &"a".repeat(64), DeviceClass::HomeAssistant)
        .unwrap();
    runtime
        .state
        .confirm_device_pairing("office-switch")
        .unwrap();
    let mut registry = Registry::default();
    registry
        .register(ResourceDescriptor {
            resource_id: "office-switch".into(),
            edge_id: "core-fixture".into(),
            adapter_id: "switch-fixture".into(),
            paired: true,
            capabilities: vec![CommandKind::SwitchSet],
        })
        .unwrap();
    let service = runtime.sentry_service(Arc::new(Mutex::new(registry)));
    let i = service.prepare(request()).unwrap();
    let mut edge = Switch {
        state: runtime.state.clone(),
        revoke: false,
        on: false,
        sends: 0,
        observations: 0,
    };
    assert!(service
        .execute(&i, &authority(&i), &mut edge)
        .await
        .is_err());
    assert_eq!(edge.sends, 0);
    assert_eq!(edge.observations, 0);
}
