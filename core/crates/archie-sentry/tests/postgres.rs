//! Actual PostgreSQL 18: ignored without an explicitly isolated fixture.
use archie_postgres::PostgresStore;
use archie_sentry::engine::{ExecutionResult, ExecutionStatus, Stage};
use archie_sentry::{contracts::*, journal::*};
use archie_types::{Timestamp, TraceId};
use postgres::{Client, Config, NoTls};

struct Fixture {
    store: Option<PostgresStore>,
    config: Config,
    schema: String,
}
impl Fixture {
    fn open() -> Self {
        let config: Config = std::env::var("ARCHIE_SENTRY_TEST_DATABASE_URL")
            .expect("isolated ARCHIE_SENTRY_TEST_DATABASE_URL is required")
            .parse()
            .unwrap();
        let schema = format!(
            "archie_sentry_test_{}",
            TraceId::new().to_string().replace('-', "")
        );
        let store = PostgresStore::open_local(config.clone(), &schema).unwrap();
        Self {
            store: Some(store),
            config,
            schema,
        }
    }
    fn reopen(&mut self) {
        drop(self.store.take());
        self.store = Some(PostgresStore::open_local(self.config.clone(), &self.schema).unwrap());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.store.take());
        if let Ok(mut client) = self.config.connect(NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", self.schema));
        }
    }
}
fn intent() -> Intent {
    let now = Timestamp::now();
    Intent {
        schema_version: 1,
        intent_id: "persisted-intent".into(),
        operation_id: 42,
        trace_id: TraceId::new(),
        resource_id: "switch-office".into(),
        command: Command::SwitchSet { on: true },
        created_at: now,
        deadline_at: now.plus_millis(30000),
        max_observation_age_ms: 1000,
        verify: true,
    }
}
fn result(i: &Intent) -> ExecutionResult {
    ExecutionResult {
        schema_version: 1,
        intent_id: i.intent_id.clone(),
        operation_id: i.operation_id,
        resource_id: i.resource_id.clone(),
        intent_digest: i.digest().unwrap(),
        status: ExecutionStatus::Unavailable,
        lease_id: None,
        dispatched_at: None,
        before: None,
        after: None,
        stages: vec![Stage::Reserved],
        finished_at: Timestamp::now(),
        replayed: false,
    }
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_reservation_survives_connection_restart() {
    let mut fixture = Fixture::open();
    let i = intent();
    {
        let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
        assert!(journal
            .reserve(&i, &i.digest().unwrap(), Timestamp::now())
            .unwrap()
            .is_none());
    }
    fixture.reopen();
    let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
    let stored = journal.load(&i.intent_id).unwrap().unwrap();
    assert_eq!(stored.operation_id, 42);
    assert!(stored.result.is_none());
    assert!(journal
        .reserve(&i, &i.digest().unwrap(), Timestamp::now())
        .unwrap()
        .is_some());
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_refuses_intent_and_operation_identity_collisions() {
    let fixture = Fixture::open();
    let i = intent();
    let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
    journal
        .reserve(&i, &i.digest().unwrap(), Timestamp::now())
        .unwrap();
    let mut changed = i.clone();
    changed.command = Command::SwitchSet { on: false };
    assert!(matches!(
        journal.reserve(&changed, &changed.digest().unwrap(), Timestamp::now()),
        Err(JournalError::Collision)
    ));
    changed.intent_id = "other-intent".into();
    assert!(matches!(
        journal.reserve(&changed, &changed.digest().unwrap(), Timestamp::now()),
        Err(JournalError::Collision)
    ));
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_completed_result_reopens_and_cannot_be_replaced() {
    let mut fixture = Fixture::open();
    let i = intent();
    let r = result(&i);
    {
        let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
        journal
            .reserve(&i, &i.digest().unwrap(), Timestamp::now())
            .unwrap();
        journal.finish(&r).unwrap();
        journal.finish(&r).unwrap();
        let mut changed = r.clone();
        changed.status = ExecutionStatus::Denied;
        assert!(journal.finish(&changed).is_err());
    }
    fixture.reopen();
    let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
    assert_eq!(
        journal.load(&i.intent_id).unwrap().unwrap().result.unwrap(),
        r
    );
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_corrupted_receipt_hash_is_refused() {
    let fixture = Fixture::open();
    let i = intent();
    let r = result(&i);
    let mut journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
    journal
        .reserve(&i, &i.digest().unwrap(), Timestamp::now())
        .unwrap();
    journal.finish(&r).unwrap();
    let mut client: Client = fixture.config.connect(NoTls).unwrap();
    client
        .batch_execute(&format!(
            "UPDATE \"{}\".sentry_intents SET result_json='{{}}'",
            fixture.schema
        ))
        .unwrap();
    assert!(matches!(
        journal.load(&i.intent_id),
        Err(JournalError::Corrupt)
    ));
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_concurrent_connections_reserve_only_once() {
    let fixture = Fixture::open();
    let i = intent();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let outcomes = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let config = fixture.config.clone();
                let schema = fixture.schema.clone();
                let i = i.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    let store = PostgresStore::open_local(config, &schema).unwrap();
                    let mut journal = PostgresJournal::open(&store).unwrap();
                    barrier.wait();
                    journal
                        .reserve(&i, &i.digest().unwrap(), Timestamp::now())
                        .unwrap()
                        .is_none()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.into_iter().filter(|new| *new).count(), 1);
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn pg_rejects_changed_schema_migration() {
    let fixture = Fixture::open();
    let _journal = PostgresJournal::open(fixture.store.as_ref().unwrap()).unwrap();
    let mut client = fixture.config.connect(NoTls).unwrap();
    client
        .batch_execute(&format!(
            "UPDATE \"{}\".sentry_schema_migrations SET schema_sha256='invalid'",
            fixture.schema
        ))
        .unwrap();
    assert!(matches!(
        PostgresJournal::open(fixture.store.as_ref().unwrap()),
        Err(JournalError::Corrupt)
    ));
}
