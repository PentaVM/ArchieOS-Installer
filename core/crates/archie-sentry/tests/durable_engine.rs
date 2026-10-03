//! End-to-end policy, measurement and durable replay against an isolated database.
use archie_policy::{
    Action, Constraints, Lease, PolicyConfig, PolicyEngine, Principal, Resource, Scope,
};
use archie_postgres::PostgresStore;
use archie_sentry::{contracts::*, engine::*, journal::*, registry::*};
use archie_types::{LeaseId, Timestamp, TraceId};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

struct Switch {
    on: bool,
    sends: usize,
    measurements: usize,
}
#[async_trait]
impl EdgeAdapter for Switch {
    fn edge_id(&self) -> &str {
        "fixture-edge"
    }
    fn adapter_id(&self) -> &str {
        "fixture-switch"
    }
    async fn observe(&mut self, r: &ResourceDescriptor) -> Result<Observation, AdapterError> {
        self.measurements += 1;
        Ok(Observation {
            resource_id: r.resource_id.clone(),
            adapter_id: self.adapter_id().into(),
            evidence_id: format!("fixture-{}", self.measurements),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state: ResourceState::Switch { on: self.on },
        })
    }
    async fn apply(
        &mut self,
        _: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        self.sends += 1;
        if let Command::SwitchSet { on } = command {
            self.on = *on;
        }
        Ok(AdapterAck { accepted: true })
    }
}
fn request() -> Intent {
    let now = Timestamp::now();
    Intent {
        schema_version: 1,
        intent_id: "durable-switch-on".into(),
        operation_id: 1,
        trace_id: TraceId::new(),
        resource_id: "fixture-switch".into(),
        command: Command::SwitchSet { on: true },
        created_at: now,
        deadline_at: now.plus_millis(30000),
        max_observation_age_ms: 1000,
        verify: true,
    }
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
    CoreAuthority::from_reviewed_intent(
        PolicyEngine::new(PolicyConfig {
            safety_boundary_present: true,
            ..PolicyConfig::default()
        }),
        lease,
        human,
        true,
        i.digest().unwrap(),
    )
    .unwrap()
}
fn registry() -> Arc<Mutex<Registry>> {
    let mut r = Registry::default();
    r.register(ResourceDescriptor {
        resource_id: "fixture-switch".into(),
        edge_id: "fixture-edge".into(),
        adapter_id: "fixture-switch".into(),
        paired: true,
        capabilities: vec![CommandKind::SwitchSet],
    })
    .unwrap();
    Arc::new(Mutex::new(r))
}
fn config() -> postgres::Config {
    std::env::var("ARCHIE_SENTRY_TEST_DATABASE_URL")
        .expect("isolated database required")
        .parse()
        .unwrap()
}
fn cleanup(config: &postgres::Config, schema: &str) {
    config
        .connect(postgres::NoTls)
        .unwrap()
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn measured_result_replays_after_restart_with_zero_new_io() {
    let config = config();
    let schema = format!(
        "archie_sentry_e2e_{}",
        TraceId::new().to_string().replace('-', "")
    );
    let i = request();
    let a = authority(&i);
    let mut edge = Switch {
        on: false,
        sends: 0,
        measurements: 0,
    };
    let first = {
        let store = PostgresStore::open_local(config.clone(), &schema).unwrap();
        let journal = PostgresJournal::open(&store).unwrap();
        Engine::new(registry(), journal)
            .execute(&i, &a, &mut edge)
            .await
            .unwrap()
    };
    assert_eq!(first.status, ExecutionStatus::Verified);
    a.revoke().unwrap();
    let second = {
        let store = PostgresStore::open_local(config.clone(), &schema).unwrap();
        Engine::new(registry(), PostgresJournal::open(&store).unwrap())
            .execute(&i, &a, &mut edge)
            .await
            .unwrap()
    };
    assert!(second.replayed);
    assert_eq!(second.status, ExecutionStatus::Verified);
    assert_eq!(edge.sends, 1);
    assert_eq!(edge.measurements, 2);
    // The synchronous client must be created and destroyed off the Tokio runtime.
    std::thread::spawn(move || cleanup(&config, &schema))
        .join()
        .unwrap();
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn unfinished_durable_reservation_replays_indeterminate_without_io() {
    let config = config();
    let schema = format!(
        "archie_sentry_e2e_{}",
        TraceId::new().to_string().replace('-', "")
    );
    let i = request();
    let a = authority(&i);
    let mut edge = Switch {
        on: false,
        sends: 0,
        measurements: 0,
    };
    {
        let store = PostgresStore::open_local(config.clone(), &schema).unwrap();
        PostgresJournal::open(&store)
            .unwrap()
            .reserve(&i, &i.digest().unwrap(), Timestamp::now())
            .unwrap();
    }
    {
        let store = PostgresStore::open_local(config.clone(), &schema).unwrap();
        let result = Engine::new(registry(), PostgresJournal::open(&store).unwrap())
            .execute(&i, &a, &mut edge)
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Indeterminate);
        assert!(result.replayed);
    }
    assert_eq!(edge.sends, 0);
    assert_eq!(edge.measurements, 0);
    std::thread::spawn(move || cleanup(&config, &schema))
        .join()
        .unwrap();
}
