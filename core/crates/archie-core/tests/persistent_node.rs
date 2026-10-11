//! M2 acceptance contract: one root remains one node across restarts.

use archie_core::config::ArchieConfig;
use archie_core::control::{ControlRequest, ControlResponse};
use archie_core::Runtime;
use archie_state::StatePaths;

fn temp_paths() -> StatePaths {
    let root = std::env::temp_dir().join(format!(
        "archie-persistent-node-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let _ = std::fs::remove_dir_all(&root);
    StatePaths::rooted(root)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn identity_state_and_self_topology_survive_a_restart() {
    let paths = temp_paths();

    // The model manifests the image stages, so the node boots knowing IRIS.
    let shipped = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .join("models/iris/iris-beta-2-1.toml");
    std::fs::create_dir_all(paths.models().join("iris")).unwrap();
    std::fs::copy(shipped, paths.models().join("iris/iris-beta-2-1.toml")).unwrap();

    let first = Runtime::boot(paths.clone(), ArchieConfig::default())
        .await
        .unwrap();
    let node_id = first.node_id();
    let public_key = std::fs::read(paths.identity_public()).unwrap();
    assert!(paths.identity_key().is_file());
    assert_eq!(
        first.state.schema_version().unwrap(),
        archie_state::SCHEMA_VERSION
    );
    assert_eq!(first.state.boot_count(&node_id).unwrap(), 1);
    assert_local_contract(&first, &node_id.to_string()).await;
    drop(first);

    let second = Runtime::boot(paths.clone(), ArchieConfig::default())
        .await
        .unwrap();
    assert_eq!(second.node_id(), node_id);
    assert_eq!(std::fs::read(paths.identity_public()).unwrap(), public_key);
    assert_eq!(second.state.boot_count(&node_id).unwrap(), 2);
    assert_local_contract(&second, &node_id.to_string()).await;
    drop(second);

    std::fs::remove_dir_all(paths.runtime_dir.parent().unwrap()).unwrap();
}

async fn assert_local_contract(runtime: &Runtime, expected_node: &str) {
    assert!(runtime.is_ready());

    match runtime.handle_control(ControlRequest::Status).await {
        ControlResponse::Status(status) => {
            assert_eq!(status.node_id, expected_node);
            assert!(status.capabilities.iter().any(|c| c == "compute.cpu"));
            assert_eq!(status.peers, 0);
            assert_eq!(status.health, "ok");
            assert_eq!(status.models_registered, 1, "the staged IRIS manifest is registered");
        }
        response => panic!("unexpected status response: {response:?}"),
    }

    match runtime.handle_control(ControlRequest::Topology).await {
        ControlResponse::Topology { nodes, totals, .. } => {
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].node_id, expected_node);
            assert_eq!(totals.nodes, 1);
            assert_eq!(totals.schedulable_nodes, 1);
        }
        response => panic!("unexpected topology response: {response:?}"),
    }
}
