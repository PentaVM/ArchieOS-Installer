use archie_memory::cognitive::{CognitiveEvent, CognitivePayload, FocusRecord};
use archie_memory::graph::Sensitivity;
use archie_state::{snapshots, StateStore};
use postgres::NoTls;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn focus(revision: u64) -> CognitiveEvent {
    CognitiveEvent {
        schema_version: 1,
        event_id: format!("event:snapshot:{revision}"),
        recorded_at_ms: revision as i64 * 100,
        payload: CognitivePayload::FocusUpdated(FocusRecord {
            focus_id: "focus:snapshot".into(),
            revision,
            objective_ref: "objective:snapshot".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 100,
            updated_at_ms: revision as i64 * 100,
            activations: vec![],
        }),
    }
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn full_timescale_restore_preserves_history_and_rejects_archive_damage() {
    let url = std::env::var("ARCHIE_TEST_ADMIN_DATABASE_URL")
        .expect("isolated operator PostgreSQL fixture required");
    let mut config: postgres::Config = url.parse().unwrap();
    let mut admin = config.connect(NoTls).unwrap();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = format!("archie_snapshot_fixture_{suffix}");
    admin
        .batch_execute(&format!("CREATE DATABASE {database} TEMPLATE template0"))
        .unwrap();
    config.dbname(&database);
    let mut setup = config.connect(NoTls).unwrap();
    setup.batch_execute("CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'").unwrap();
    drop(setup);
    let source_url = format!("{}/{}", url.rsplit_once('/').unwrap().0, database);
    let root = std::env::temp_dir().join(format!("archie_snapshot_{suffix}"));
    fs::create_dir(&root).unwrap();
    let settings = root.join("database.json");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(&settings)
        .unwrap()
        .write_all(
            &serde_json::to_vec(
                &serde_json::json!({"connection":source_url,"schema":"archie_snapshot"}),
            )
            .unwrap(),
        )
        .unwrap();
    let source = StateStore::open(&settings).unwrap();
    let first = source.record_cognitive_event(&focus(1)).unwrap();
    let acknowledged = source.cognitive_outbox(10).unwrap().remove(0);
    source
        .acknowledge_cognitive_outbox(acknowledged.sequence, &acknowledged.payload_sha256)
        .unwrap();
    let watermark = source.record_cognitive_event(&focus(2)).unwrap();
    assert!(watermark > first);
    let pending = source.cognitive_outbox(10).unwrap();
    source
        .record_metric("node:a", "cpu:ms", 100, 2.0, "probe:a")
        .unwrap();
    source
        .record_metric("node:a", "cpu:ms", 200, 4.0, "probe:b")
        .unwrap();
    source
        .record_embedding(
            "object:a",
            "model:a",
            &[1.0, 0.0],
            Sensitivity::Public,
            "evidence:a",
            100,
        )
        .unwrap();
    let directory = root.join("snapshot");
    let manifest = snapshots::capture(&settings, &directory).unwrap();
    assert_eq!(
        manifest
            .tables
            .iter()
            .find(|t| t.table == "metric_samples")
            .unwrap()
            .rows,
        2
    );
    assert!(
        snapshots::capture(&settings, &directory).is_err(),
        "never overwrite an archive"
    );
    source
        .record_metric("node:a", "cpu:ms", 300, 99.0, "probe:c")
        .unwrap();
    let receipt = snapshots::restore_verify(&settings, &directory).unwrap();
    assert_ne!(receipt.database, database);
    let restored_url = format!("{}/{}", url.rsplit_once('/').unwrap().0, receipt.database);
    let restored = StateStore::connect(&restored_url, "archie_snapshot").unwrap();
    assert_eq!(restored.cognitive_high_watermark().unwrap(), watermark);
    assert_eq!(restored.cognitive_outbox(10).unwrap(), pending);
    assert_eq!(
        restored.cognitive_events(0, watermark, 1000, 10).unwrap(),
        source.cognitive_events(0, watermark, 1000, 10).unwrap()
    );
    assert!(
        restored.record_cognitive_event(&focus(3)).unwrap() > watermark,
        "restored sequences must never reuse an identity"
    );
    assert_eq!(
        restored.metric_mean("node:a", "cpu:ms", 100, 300).unwrap(),
        Some(3.0)
    );
    assert_eq!(
        restored
            .nearest_embeddings("model:a", &[1.0, 0.0], 300, &[Sensitivity::Public], 10)
            .unwrap()[0]
            .distance,
        0.0
    );
    assert_eq!(
        source.metric_mean("node:a", "cpu:ms", 100, 300).unwrap(),
        Some(35.0)
    );
    assert!(
        snapshots::restore_verify(&settings, &directory).is_err(),
        "never overwrite verification receipt"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(directory.join("database.dump"))
        .unwrap()
        .write_all(b"damage")
        .unwrap();
    assert!(snapshots::restore_verify(&settings, &directory)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    drop(restored);
    drop(source);
    // Only databases freshly created by this test are eligible for cleanup.
    admin
        .batch_execute(&format!("DROP DATABASE {}", receipt.database))
        .unwrap();
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .unwrap();
    fs::remove_file(settings).unwrap(); // Remove fixture credentials, retain diagnostics.
}
