use archie_memory::{
    cognitive::FocusRecord,
    graph::{ContextQuery, GraphSnapshot, Sensitivity},
};
fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../protocols/fixtures")
            .join(name),
    )
    .unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[test]
fn rust_graph_and_persisted_focus_match_cpp_wire_fixtures() {
    let graph: GraphSnapshot = serde_json::from_str(&fixture("cortex-graph.json")).unwrap();
    assert_eq!(
        hex(&graph
            .encode_policy_view(&[Sensitivity::LocalRestricted], 1000)
            .unwrap()),
        fixture("cortex-graph-v1.hex").trim()
    );
    let query: ContextQuery = serde_json::from_str(&fixture("cortex-query.json")).unwrap();
    let focus: FocusRecord = serde_json::from_str(&fixture("cortex-focus.json")).unwrap();
    assert_eq!(
        hex(&query.encode(Some(&focus)).unwrap()),
        fixture("cortex-focus-request-v2.hex").trim()
    );
    let mut future = focus;
    future.updated_at_ms = 1001;
    assert!(query.encode(Some(&future)).is_err());
}

#[test]
fn prospective_v3_matches_native_fixture_and_rejects_unbounded_or_duplicate_cues() {
    let mut query: ContextQuery =
        serde_json::from_str(&fixture("cortex-prospective-query.json")).unwrap();
    assert_eq!(
        hex(&query.encode(None).unwrap()),
        fixture("cortex-prospective-request-v3.hex").trim()
    );
    query.retrieval_cues.push(query.retrieval_cues[0].clone());
    assert!(query.encode(None).is_err());
    query.retrieval_cues.truncate(1);
    query.retrieval_cues[0].question = "q".repeat(257);
    assert!(query.encode(None).is_err());
}
