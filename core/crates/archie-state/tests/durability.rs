//! Durability contracts ported to PostgreSQL. Physical power-loss certification is separate.
use archie_state::StateStore;
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_committed_transaction_uses_synchronous_wal_and_durable_server_settings() {
    let store = StateStore::open_test().unwrap();
    let d = store.durability().unwrap();
    assert!(d.commits_are_durable());
    assert_eq!(d.synchronous, "on");
    assert!(d.fsync && d.full_page_writes && d.logged_tables && d.data_checksums);
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn durability_reads_the_actual_working_connection() {
    let store = StateStore::open_test().unwrap();
    store.test_sql("SET synchronous_commit=off").unwrap();
    assert!(!store.durability().unwrap().commits_are_durable());
    store.test_sql("SET synchronous_commit=on").unwrap();
    assert!(store.durability().unwrap().commits_are_durable());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn foreign_keys_reject_orphaned_device_operations() {
    let store = StateStore::open_test().unwrap();
    assert!(store.durability().unwrap().foreign_keys);
    assert!(store.test_sql("INSERT INTO device_operations(device_reference,trace_id,operation_sha256,prepared_at,state) VALUES('missing','trace:test','invalid',1,'prepared')").is_err());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn corrupted_schema_is_refused_before_a_reopen_succeeds() {
    let path = std::env::temp_dir().join(format!(
        "archie-schema-durability-{}.json",
        std::process::id()
    ));
    let store = StateStore::open(&path).unwrap();
    store
        .test_sql("UPDATE control_schema_migrations SET hash='damaged'")
        .unwrap();
    drop(store);
    assert!(StateStore::open(&path).is_err());
    std::fs::remove_file(path).unwrap();
}
#[test]
fn malformed_configuration_is_refused_before_boot() {
    let path = std::env::temp_dir().join(format!(
        "archie-invalid-durability-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, b"not a database configuration").unwrap();
    assert!(StateStore::open(&path).is_err());
    std::fs::remove_file(path).unwrap();
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_healthy_database_reopens_with_the_same_durability() {
    let path = std::env::temp_dir().join(format!(
        "archie-healthy-durability-{}.json",
        std::process::id()
    ));
    let first = StateStore::open(&path).unwrap().durability().unwrap();
    assert_eq!(
        StateStore::open(&path).unwrap().durability().unwrap(),
        first
    );
    std::fs::remove_file(path).unwrap();
}
