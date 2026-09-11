//! M3 acceptance contract: two booted runtimes form an authenticated useful fabric.

use archie_core::config::ArchieConfig;
use archie_core::control::{ControlRequest, ControlResponse};
use archie_core::{Runtime, TransportOutcome};
use archie_fabric::{Enrolment, FabricEvent, LoopbackBus, NodeIdentity, OwnerIdentity};
use archie_state::{StatePaths, TrustState};
use std::sync::Arc;
use std::time::Duration;

/// Do to a node what the installer does: give it a key, have an owner admit it, and
/// leave the certificate where the runtime will find it.
///
/// Before this existed the two nodes below simply found each other, because anything
/// that could sign was admitted. Now a fabric is something somebody has to have made,
/// and the test has to make one — which is a more honest model of what a two-node
/// install actually involves.
fn enrol_into(paths: &StatePaths, owner: &OwnerIdentity) {
    let node = NodeIdentity::load_or_create(paths.identity_key()).expect("node key");
    let membership = owner.admit(&node.public_key_hex(), 1);
    Enrolment::new(&node.public_key_hex(), membership)
        .expect("the owner admitted this node")
        .save(paths.membership())
        .expect("write membership");
}

fn temp_paths(name: &str) -> StatePaths {
    let root = std::env::temp_dir().join(format!(
        "archie-two-node-{name}-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let _ = std::fs::remove_dir_all(&root);
    StatePaths::rooted(root)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn two_nodes_authenticate_persist_and_expose_each_other_to_control() {
    let paths_a = temp_paths("a");
    let paths_b = temp_paths("b");

    // One household, two machines.
    let owner = OwnerIdentity::generate();
    enrol_into(&paths_a, &owner);
    enrol_into(&paths_b, &owner);

    let bus = LoopbackBus::new();

    // Attach both endpoints before either node announces so neither can miss a
    // broadcast while the second runtime is booting.
    let endpoint_a = bus.endpoint();
    let endpoint_b = bus.endpoint();
    let node_a = Runtime::boot_with_transport(
        paths_a.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(endpoint_a)),
    )
    .await
    .unwrap();
    let node_b = Runtime::boot_with_transport(
        paths_b.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(endpoint_b)),
    )
    .await
    .unwrap();

    let id_a = node_a.node_id();
    let id_b = node_b.node_id();
    assert_ne!(id_a, id_b);

    node_a.fabric.announce().await.unwrap();
    let seen_by_b = tokio::time::timeout(Duration::from_secs(2), node_b.fabric.receive_once())
        .await
        .unwrap()
        .unwrap();
    assert_authenticated_discovery(&seen_by_b, &id_a.to_string());
    node_b.apply_fabric_event(&seen_by_b).await;

    node_b.fabric.announce().await.unwrap();
    let seen_by_a = tokio::time::timeout(Duration::from_secs(2), node_a.fabric.receive_once())
        .await
        .unwrap()
        .unwrap();
    assert_authenticated_discovery(&seen_by_a, &id_b.to_string());
    node_a.apply_fabric_event(&seen_by_a).await;

    assert_peer_contract(&node_a, &id_b.to_string()).await;
    assert_peer_contract(&node_b, &id_a.to_string()).await;

    for (runtime, peer_id) in [(&node_a, id_b), (&node_b, id_a)] {
        let peers = runtime.state.list_peers().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].node_id, peer_id);
        assert_eq!(peers[0].trust, TrustState::Pinned);
        assert!(peers[0].descriptor.is_some());
    }

    drop(node_a);
    drop(node_b);
    std::fs::remove_dir_all(paths_a.runtime_dir.parent().unwrap()).unwrap();
    std::fs::remove_dir_all(paths_b.runtime_dir.parent().unwrap()).unwrap();
}

fn assert_authenticated_discovery(event: &FabricEvent, expected_peer: &str) {
    match event {
        FabricEvent::PeerDiscovered(peer) => {
            assert_eq!(peer.node_id.to_string(), expected_peer);
            assert!(peer.authenticated);
            assert_eq!(peer.descriptor.node_id, peer.node_id);
            assert_eq!(peer.public_key, peer.descriptor.public_key);
        }
        event => panic!("unexpected fabric event: {event:?}"),
    }
}

async fn assert_peer_contract(runtime: &Runtime, expected_peer: &str) {
    match runtime.handle_control(ControlRequest::Peers).await {
        ControlResponse::Peers { peers } => {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].node_id, expected_peer);
            assert_eq!(peers[0].trust, "pinned");
            assert!(peers[0].reachable);
            assert!(peers[0]
                .capabilities
                .iter()
                .any(|capability| capability == "compute.cpu"));
        }
        response => panic!("unexpected peers response: {response:?}"),
    }

    match runtime.handle_control(ControlRequest::Topology).await {
        ControlResponse::Topology { nodes, totals, .. } => {
            assert_eq!(nodes.len(), 2);
            assert_eq!(totals.nodes, 2);
            assert_eq!(totals.schedulable_nodes, 2);
            assert!(nodes.iter().any(|node| node.node_id == expected_peer));
        }
        response => panic!("unexpected topology response: {response:?}"),
    }
}

/// Two people, two installs, one network — through two real booted runtimes.
///
/// `archie-fabric` proves the certificate logic and that the admission check is wired
/// into `handle_inbound`. This proves the last link: that a node booting from disk picks
/// up which fabric it belongs to, and that two which belong to different ones stay
/// apart even sharing a bus.
#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn two_households_do_not_form_one_fabric() {
    let paths_a = temp_paths("household-a");
    let paths_b = temp_paths("household-b");

    // Two owners who have never met.
    enrol_into(&paths_a, &OwnerIdentity::generate());
    enrol_into(&paths_b, &OwnerIdentity::generate());

    let bus = LoopbackBus::new();
    let endpoint_a = bus.endpoint();
    let endpoint_b = bus.endpoint();

    let node_a = Runtime::boot_with_transport(
        paths_a.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(endpoint_a)),
    )
    .await
    .unwrap();
    let node_b = Runtime::boot_with_transport(
        paths_b.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(endpoint_b)),
    )
    .await
    .unwrap();

    assert_ne!(
        node_a.fabric.fabric_fingerprint(),
        node_b.fabric.fabric_fingerprint(),
        "two installs that generated their own owner keys are two fabrics"
    );

    node_a.fabric.announce().await.unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(2), node_b.fabric.receive_once())
        .await
        .expect("the packet arrives; it is the admission that refuses it")
        .unwrap();

    match seen {
        FabricEvent::Rejected(reason) => {
            assert_eq!(
                reason.code(),
                "not_a_member",
                "rejected, but for the wrong reason: {reason:?}"
            );
        }
        other => panic!("a stranger's node joined the fabric: {other:?}"),
    }
    assert_eq!(node_b.fabric.peer_count().await, 0);
}

/// A machine installed and never enrolled runs, and talks to nobody.
#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn an_unenrolled_node_boots_and_stays_alone() {
    let paths = temp_paths("unenrolled");
    let bus = LoopbackBus::new();
    let endpoint = bus.endpoint();

    let stranger_paths = temp_paths("stranger");
    enrol_into(&stranger_paths, &OwnerIdentity::generate());
    let stranger_endpoint = bus.endpoint();

    // No enrol_into for this one: it has a key and no membership, which is the state a
    // fresh install is in before anybody decides what it belongs to.
    let alone = Runtime::boot_with_transport(
        paths.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(endpoint)),
    )
    .await
    .expect("a node with no fabric still boots; it is not a broken machine");

    assert!(!alone.fabric.is_enrolled());
    assert_eq!(alone.fabric.fabric_fingerprint(), None);

    let stranger = Runtime::boot_with_transport(
        stranger_paths.clone(),
        ArchieConfig::default(),
        TransportOutcome::ready(Arc::new(stranger_endpoint)),
    )
    .await
    .unwrap();

    stranger.fabric.announce().await.unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(2), alone.fabric.receive_once())
        .await
        .expect("the packet arrives")
        .unwrap();
    assert!(
        matches!(seen, FabricEvent::Rejected(_)),
        "belonging to no fabric is not belonging to every fabric: {seen:?}"
    );
    assert_eq!(alone.fabric.peer_count().await, 0);
}
