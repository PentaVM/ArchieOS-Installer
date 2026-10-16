//! Two-phase proof. The harness restarts ONLY its owned isolated cluster between phases.
use archie_postgres::PostgresStore;
use archie_sentry::{contracts::*, engine::*, journal::*};
use archie_types::{Timestamp, TraceId};
#[test]
#[ignore = "requires an isolated cluster and explicit crash recovery phase"]
fn cluster_crash_preserves_dispatch_fence_and_completed_receipt() {
    let config: postgres::Config = std::env::var("ARCHIE_SENTRY_TEST_DATABASE_URL")
        .expect("isolated database required")
        .parse()
        .unwrap();
    assert_eq!(config.get_ports(), &[55439]);
    assert_eq!(config.get_dbname(), Some("sentry_fixture"));
    let phase =
        std::env::var("ARCHIE_SENTRY_TEST_RECOVERY_PHASE").expect("write or verify phase required");
    let store = PostgresStore::open_local(config.clone(), "archie_sentry_crash_fixture").unwrap();
    let mut journal = PostgresJournal::open(&store).unwrap();
    match phase.as_str() {
        "write" => {
            let now = Timestamp::now();
            for (id, operation) in [("crash-pending", 1), ("crash-completed", 2)] {
                let i = Intent {
                    schema_version: 1,
                    intent_id: id.into(),
                    operation_id: operation,
                    trace_id: TraceId::new(),
                    resource_id: "fixture-switch".into(),
                    command: Command::SwitchSet { on: true },
                    created_at: now,
                    deadline_at: now.plus_millis(30000),
                    max_observation_age_ms: 1000,
                    verify: true,
                };
                assert!(
                    journal.load(id).unwrap().is_none(),
                    "owned recovery schema must be unused"
                );
                assert!(journal
                    .reserve(&i, &i.digest().unwrap(), now)
                    .unwrap()
                    .is_none());
                if operation == 2 {
                    journal
                        .finish(&ExecutionResult {
                            schema_version: 1,
                            intent_id: i.intent_id.clone(),
                            operation_id: 2,
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
                        })
                        .unwrap();
                }
            }
        }
        "verify" => {
            let pending = journal
                .load("crash-pending")
                .unwrap()
                .expect("reservation survived crash");
            assert_eq!(pending.operation_id, 1);
            assert!(pending.result.is_none());
            let completed = journal
                .load("crash-completed")
                .unwrap()
                .expect("receipt survived crash");
            assert_eq!(completed.operation_id, 2);
            assert_eq!(
                completed.result.unwrap().status,
                ExecutionStatus::Unavailable
            );
        }
        _ => panic!("unsupported recovery phase"),
    }
    if phase == "verify" {
        drop(store);
        config
            .connect(postgres::NoTls)
            .unwrap()
            .batch_execute("DROP SCHEMA archie_sentry_crash_fixture CASCADE")
            .unwrap();
    }
}
