//! In-process protocol servers for adapter tests.
//!
//! Each server implements its protocol with state of its own — a broker that
//! keeps retained messages, a printer that queues jobs, a receiver that runs
//! applications — so an adapter goes through the same path it would against a
//! device and the postcondition is measured from that state. These are
//! protocol tests on loopback: none of them is a physical device, and passing
//! them verifies no hardware.
#![allow(dead_code)]

pub mod cast;
pub mod home_assistant;
pub mod http;
pub mod igd;
pub mod ipp;
pub mod matter;
pub mod mqtt;
pub mod onvif;
pub mod roku;

use archie_policy::{
    Action, Constraints, Lease, PolicyConfig, PolicyEngine, Principal, Resource, Scope,
};
use archie_sentry::{
    contracts::{Command, Intent},
    engine::{CoreAuthority, EdgeAdapter, Engine, ExecutionResult},
    journal::MemoryJournal,
    registry::{Registry, ResourceDescriptor},
};
use archie_types::{LeaseId, Timestamp, TraceId};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

static OPERATION: AtomicU64 = AtomicU64::new(1);

/// An approved intent for one command, valid for `deadline_ms`.
pub fn intent(resource: &str, command: Command, deadline_ms: i64) -> Intent {
    let now = Timestamp::now();
    let operation = OPERATION.fetch_add(1, Ordering::SeqCst);
    Intent {
        schema_version: 1,
        intent_id: format!("intent-{operation}"),
        operation_id: operation,
        trace_id: TraceId::new(),
        resource_id: resource.into(),
        command,
        created_at: now,
        deadline_at: now.plus_millis(deadline_ms),
        max_observation_age_ms: 5000,
        verify: true,
    }
}

/// The authority Core grants after an operator approved this exact digest.
pub fn authority(request: &Intent) -> CoreAuthority {
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
            expires_at: Some(request.deadline_at.plus_millis(1000)),
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

/// Run one command through the engine: Intent → policy → approval →
/// observation before → dispatch → observation after.
pub async fn execute(
    resource: &ResourceDescriptor,
    adapter: &mut dyn EdgeAdapter,
    command: Command,
) -> ExecutionResult {
    let mut registry = Registry::default();
    registry.register(resource.clone()).unwrap();
    let mut engine = Engine::new(Arc::new(Mutex::new(registry)), MemoryJournal::default());
    let request = intent(&resource.resource_id, command, 20_000);
    let grant = authority(&request);
    engine.execute(&request, &grant, adapter).await.unwrap()
}
