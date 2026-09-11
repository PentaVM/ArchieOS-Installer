use archie_memory::graph::Sensitivity;
use archie_state::StateStore;
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn real_hypertable_metrics_are_idempotent_and_bounded_by_time() {
    let s = StateStore::open_test().unwrap();
    for (time, value) in [(100, 2.0), (200, 4.0), (300, 30.0)] {
        s.record_metric("node:test", "cpu:ms", time, value, "probe:1")
            .unwrap();
    }
    s.record_metric("node:test", "cpu:ms", 100, 2.0, "probe:1")
        .unwrap();
    assert_eq!(
        s.metric_mean("node:test", "cpu:ms", 100, 200).unwrap(),
        Some(3.0)
    );
    assert!(s
        .record_metric("node:test", "cpu:ms", 100, 9.0, "probe:1")
        .is_err());
    assert!(s
        .record_metric("node:test", "cpu:ms", 400, f64::NAN, "probe:1")
        .is_err());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn vectors_preserve_history_dimensions_and_explicit_privacy_labels() {
    let s = StateStore::open_test().unwrap();
    s.record_embedding(
        "object:a",
        "model:test",
        &[1.0, 0.0, 0.0],
        Sensitivity::Public,
        "evidence:a",
        100,
    )
    .unwrap();
    s.record_embedding(
        "object:a",
        "model:test",
        &[1.0, 0.0, 0.0],
        Sensitivity::Public,
        "evidence:a",
        100,
    )
    .unwrap();
    s.record_embedding(
        "object:secret",
        "model:test",
        &[1.0, 0.0, 0.0],
        Sensitivity::Secret,
        "evidence:secret",
        100,
    )
    .unwrap();
    s.record_embedding(
        "object:a",
        "model:test",
        &[0.0, 1.0, 0.0],
        Sensitivity::Secret,
        "evidence:a2",
        200,
    )
    .unwrap();
    let old = s
        .nearest_embeddings(
            "model:test",
            &[1.0, 0.0, 0.0],
            150,
            &[Sensitivity::Public],
            10,
        )
        .unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].object_id, "object:a");
    assert_eq!(old[0].distance, 0.0);
    assert!(s
        .nearest_embeddings(
            "model:test",
            &[1.0, 0.0, 0.0],
            200,
            &[Sensitivity::Public],
            10
        )
        .unwrap()
        .is_empty());
    assert!(s
        .nearest_embeddings("model:test", &[1.0, 0.0], 150, &[Sensitivity::Public], 10)
        .unwrap()
        .is_empty());
    assert!(s
        .record_embedding(
            "object:a",
            "model:test",
            &[0.0, 0.0, 1.0],
            Sensitivity::Public,
            "evidence:a",
            100
        )
        .is_err());
}
