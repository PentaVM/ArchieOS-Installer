use archie_memory::{
    graph::*, CorrectionEvent, CorrectionKind, CorrectionTrajectory, GraphCorrectionReceipt,
};
use archie_state::StateStore;

fn graph() -> GraphSnapshot {
    GraphSnapshot {
        snapshot_id: "before".into(),
        captured_at_ms: 100,
        nodes: vec![GraphNode {
            id: "claim:motor".into(),
            kind: NodeKind::Claim,
            summary: "source asserted: old result".into(),
            aliases: vec![],
            provenance_ref: "evidence:1".into(),
            sensitivity: Sensitivity::LocalRestricted,
            observed_at_ms: 100,
            version: 1,
        }],
        edges: vec![],
    }
}
fn correction() -> CorrectionTrajectory {
    let digest = format!("sha256:{}", "a".repeat(64));
    CorrectionTrajectory {
        trajectory_id: "trajectory".into(),
        correction: CorrectionEvent {
            correction_id: "correction".into(),
            kind: CorrectionKind::EntityCorrection,
            operator_id: "operator:test".into(),
            graph_snapshot_id: "before".into(),
            query_digest: digest.clone(),
            raw_correction_ref: "vault:correction".into(),
            rationale_ref: "vault:rationale".into(),
            target_ids: vec!["claim:motor".into()],
            recorded_at_ms: 150,
        },
        graph_update: GraphCorrectionReceipt {
            correction_id: "correction".into(),
            source_graph_snapshot_id: "before".into(),
            resulting_graph_snapshot_id: "after".into(),
            graph_update_ref: "evidence:update".into(),
            applied_at_ms: 200,
        },
        selected_context_ids: vec!["claim:motor".into()],
        omitted_candidate_ids: vec![],
        response_ref: "vault:response".into(),
        response_digest: digest.clone(),
        corrected_interpretation_ref: "evidence:interpretation".into(),
        model_id: "baseline".into(),
        model_hash: digest,
        captured_at_ms: 201,
    }
}
fn successor() -> GraphSnapshot {
    let mut g = graph();
    g.snapshot_id = "after".into();
    g.captured_at_ms = 200;
    g.nodes[0].summary = "source asserted: corrected result".into();
    g.nodes[0].version = 2;
    g
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn corrected_truth_and_training_commit_together_and_history_survives() {
    let s = StateStore::open_test().unwrap();
    s.publish_cortex_graph(&graph(), None).unwrap();
    s.apply_cortex_correction(&correction(), &successor())
        .unwrap();
    assert_eq!(s.cortex_graph_at(199).unwrap().unwrap(), graph());
    assert_eq!(s.cortex_graph_at(200).unwrap().unwrap(), successor());
    assert_eq!(
        s.recent_cortex_correction_trajectories(100).unwrap().len(),
        1
    );
    s.apply_cortex_correction(&correction(), &successor())
        .unwrap();
    assert_eq!(
        s.recent_cortex_correction_trajectories(100).unwrap().len(),
        1
    );
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn trajectory_conflict_rolls_back_the_graph_publication() {
    let s = StateStore::open_test().unwrap();
    s.publish_cortex_graph(&graph(), None).unwrap();
    let mut conflict = correction();
    conflict.corrected_interpretation_ref = "evidence:different".into();
    s.record_cortex_correction_trajectory(&conflict).unwrap();
    assert!(s
        .apply_cortex_correction(&correction(), &successor())
        .is_err());
    assert_eq!(s.cortex_graph_at(300).unwrap().unwrap(), graph());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn stale_versions_missing_targets_and_forged_receipts_fail() {
    let s = StateStore::open_test().unwrap();
    s.publish_cortex_graph(&graph(), None).unwrap();
    let mut bad = successor();
    bad.nodes[0].version = 1;
    assert!(s.apply_cortex_correction(&correction(), &bad).is_err());
    let mut c = correction();
    c.correction.target_ids = vec!["unrelated".into()];
    assert!(s.apply_cortex_correction(&c, &successor()).is_err());
    c = correction();
    c.graph_update.applied_at_ms = 199;
    assert!(s.apply_cortex_correction(&c, &successor()).is_err());
    assert!(s.publish_cortex_graph(&successor(), None).is_err());
    s.publish_cortex_graph(&successor(), Some("before"))
        .unwrap();
    assert!(s
        .apply_cortex_correction(&correction(), &successor())
        .is_err());
}
#[test]
fn policy_export_excludes_private_nodes_and_dangling_edges() {
    let mut g = graph();
    let mut private = g.nodes[0].clone();
    private.id = "private:node".into();
    private.sensitivity = Sensitivity::Secret;
    private.summary = "DO_NOT_EXPORT".into();
    g.nodes.push(private);
    g.edges.push(GraphEdge {
        id: "edge".into(),
        source_id: "claim:motor".into(),
        target_id: "private:node".into(),
        kind: EdgeKind::Supports,
        provenance_ref: "evidence:edge".into(),
        observed_at_ms: 100,
        version: 1,
    });
    let bytes = g
        .encode_policy_view(&[Sensitivity::LocalRestricted], 100)
        .unwrap();
    assert!(!bytes.windows(13).any(|w| w == b"DO_NOT_EXPORT"));
    assert!(!bytes.windows(12).any(|w| w == b"private:node"));
    assert!(g.encode_policy_view(&[], 100).is_err());
}
