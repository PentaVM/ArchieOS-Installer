//! Contract test for the complete booted-runtime control path.
//!
//! This deliberately uses the platform's real control endpoint rather than a duplex
//! buffer: it catches publication, connection, framing, dispatch, and response-shape
//! regressions in one place.

use archie_core::config::ArchieConfig;
use archie_core::control::{
    request_over, serve_connection, ControlRequest, ControlResponse, DeviceConnectionIntent,
    DevicePairingTransport,
};
use archie_core::{endpoint, DeviceEnrollmentError, Runtime};
use archie_memory::{
    CorrectionEvent, CorrectionKind, CorrectionTrajectory, GraphCorrectionReceipt,
};
use archie_state::{DeviceClass, DevicePairingState, StatePaths};
use archie_types::TraceId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_paths() -> StatePaths {
    let root = std::env::temp_dir().join(format!(
        "archie-runtime-control-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    let _ = std::fs::remove_dir_all(&root);
    StatePaths::rooted(root)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn every_read_only_request_round_trips_through_a_booted_runtime() {
    let paths = temp_paths();
    let runtime = Arc::new(
        Runtime::boot(paths.clone(), ArchieConfig::default())
            .await
            .unwrap(),
    );
    assert!(runtime.is_ready());
    let expected_node = runtime.node_id().to_string();
    runtime
        .state
        .propose_device_pairing(
            "living-room-tv",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            DeviceClass::Television,
        )
        .unwrap();
    runtime
        .state
        .confirm_device_pairing("living-room-tv")
        .unwrap();
    runtime
        .state
        .prepare_device_operation(
            "living-room-tv",
            TraceId::new(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
    runtime
        .state
        .record_cortex_correction_trajectory(&CorrectionTrajectory {
            trajectory_id: "trajectory:0001".into(),
            correction: CorrectionEvent {
                correction_id: "correction:0001".into(),
                kind: CorrectionKind::MissingContextLabel,
                operator_id: "operator:nelson".into(),
                graph_snapshot_id: "graph:before".into(),
                query_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                raw_correction_ref: "vault:corrections/0001".into(),
                rationale_ref: "knowledge:rationales/0001".into(),
                target_ids: vec!["simulation:motor-x".into()],
                recorded_at_ms: 1_700_000_000_000,
            },
            graph_update: GraphCorrectionReceipt {
                correction_id: "correction:0001".into(),
                source_graph_snapshot_id: "graph:before".into(),
                resulting_graph_snapshot_id: "graph:after".into(),
                graph_update_ref: "knowledge:updates/0001".into(),
                applied_at_ms: 1_700_000_000_001,
            },
            selected_context_ids: vec!["project:robot-arm".into()],
            omitted_candidate_ids: vec!["simulation:motor-x".into()],
            response_ref: "vault:responses/0001".into(),
            response_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            corrected_interpretation_ref: "knowledge:interpretations/0001".into(),
            model_id: "deterministic-v1".into(),
            model_hash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            captured_at_ms: 1_700_000_000_002,
        })
        .unwrap();

    let requests = [
        ControlRequest::Ping,
        ControlRequest::Status,
        ControlRequest::Topology,
        ControlRequest::Modules,
        ControlRequest::Providers,
        ControlRequest::Peers,
        ControlRequest::Devices,
        ControlRequest::DeviceOperations { limit: 20 },
        ControlRequest::CortexTrajectories { limit: 20 },
        ControlRequest::Models,
        ControlRequest::Boot,
        ControlRequest::Metrics,
        ControlRequest::Audit { limit: 20 },
    ];

    let listener = endpoint::bind(&paths).await.unwrap();
    let server_runtime = runtime.clone();
    let request_count = requests.len();
    let server = tokio::spawn(async move {
        for _ in 0..request_count {
            let stream = endpoint::accept(&listener).await.unwrap();
            let runtime = server_runtime.clone();
            serve_connection(stream, move |request| {
                let runtime = runtime.clone();
                async move { runtime.handle_control(request).await }
            })
            .await
            .unwrap();
        }
    });

    for request in requests {
        let mut client = endpoint::connect(&paths).await.unwrap();
        let response = request_over(&mut client, &request).await.unwrap();
        assert_contract(&request, &response, &expected_node);
        drop(client);
    }

    server.await.unwrap();
    endpoint::cleanup(&paths);
    drop(runtime);
    std::fs::remove_dir_all(paths.runtime_dir.parent().unwrap()).unwrap();
}

#[tokio::test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
async fn an_authenticated_archie_agent_enrols_without_a_second_manual_link_step() {
    let paths = temp_paths();
    let runtime = Runtime::boot(paths.clone(), ArchieConfig::default())
        .await
        .unwrap();

    let enrolled = runtime
        .enroll_authenticated_device(&DeviceConnectionIntent {
            device_reference: "mama-phone".into(),
            class: DeviceClass::Mobile,
            transport: DevicePairingTransport::ArchieFabric,
            authenticated_public_key: Some("a".repeat(64)),
        })
        .unwrap();
    assert_eq!(enrolled.pairing, DevicePairingState::Paired);
    assert_eq!(enrolled.public_key, "a".repeat(64));
    assert_eq!(
        runtime
            .state
            .get_device("mama-phone")
            .unwrap()
            .unwrap()
            .pairing,
        DevicePairingState::Paired
    );

    let needs_firmware_proof = runtime.enroll_authenticated_device(&DeviceConnectionIntent {
        device_reference: "living-room-tv".into(),
        class: DeviceClass::Television,
        transport: DevicePairingTransport::RokuEcp,
        authenticated_public_key: None,
    });
    assert!(matches!(
        needs_firmware_proof,
        Err(DeviceEnrollmentError::DeviceProofRequired {
            transport: DevicePairingTransport::RokuEcp,
            ..
        })
    ));
    assert!(runtime
        .state
        .get_device("living-room-tv")
        .unwrap()
        .is_none());

    drop(runtime);
    std::fs::remove_dir_all(paths.runtime_dir.parent().unwrap()).unwrap();
}

fn assert_contract(request: &ControlRequest, response: &ControlResponse, expected_node: &str) {
    match (request, response) {
        (
            ControlRequest::Ping,
            ControlResponse::Pong {
                node_id, version, ..
            },
        ) => {
            assert_eq!(node_id, expected_node);
            assert_eq!(version, "0.1.0");
        }
        (ControlRequest::Status, ControlResponse::Status(status)) => {
            assert_eq!(status.node_id, expected_node);
            assert_eq!(status.health, "ok");
        }
        (ControlRequest::Topology, ControlResponse::Topology { nodes, totals, .. }) => {
            assert!(nodes.iter().any(|node| node.node_id == expected_node));
            assert_eq!(totals.nodes, 1);
        }
        (ControlRequest::Modules, ControlResponse::Modules { modules, .. }) => {
            assert!(modules.is_empty());
        }
        (ControlRequest::Providers, ControlResponse::Providers { providers, active }) => {
            // Nothing staged in this temp tree, matching the empty-modules case above:
            // a booted node with no provider manifests installed is the ordinary state
            // of a fresh install, not a fault this contract should ever report as one.
            assert!(providers.is_empty());
            assert!(active.is_empty());
        }
        (ControlRequest::Peers, ControlResponse::Peers { peers }) => {
            assert!(peers.is_empty());
        }
        (ControlRequest::Devices, ControlResponse::Devices { devices }) => {
            assert_eq!(devices.len(), 1);
            assert_eq!(devices[0].device_reference, "living-room-tv");
            assert_eq!(devices[0].class, "television");
            assert_eq!(devices[0].pairing, "paired");
            assert_eq!(devices[0].public_key_fingerprint, "aaaaaaaaaaaaaaaa");
        }
        (
            ControlRequest::DeviceOperations { .. },
            ControlResponse::DeviceOperations { operations },
        ) => {
            assert_eq!(operations.len(), 1);
            assert_eq!(operations[0].operation_id, 1);
            assert_eq!(operations[0].device_reference, "living-room-tv");
            assert_eq!(operations[0].state, "prepared");
            assert_eq!(
                operations[0].operation_sha256_fingerprint,
                "bbbbbbbbbbbbbbbb"
            );
        }
        (
            ControlRequest::CortexTrajectories { .. },
            ControlResponse::CortexTrajectories { trajectories },
        ) => {
            assert_eq!(trajectories.len(), 1);
            assert_eq!(trajectories[0].trajectory_id, "trajectory:0001");
            assert_eq!(trajectories[0].correction_kind, "missing_context_label");
            assert_eq!(trajectories[0].payload_sha256_fingerprint.len(), 16);
            assert_ne!(
                trajectories[0].payload_sha256_fingerprint,
                "vault:responses/0001"
            );
        }
        (ControlRequest::Models, ControlResponse::Models { models }) => {
            assert!(models.is_empty());
        }
        (ControlRequest::Boot, ControlResponse::Boot(report)) => {
            assert!(report.is_ready());
        }
        (ControlRequest::Metrics, ControlResponse::Metrics(snapshot)) => {
            assert!(snapshot.counters.contains_key("control.request.metrics"));
        }
        (ControlRequest::Audit { .. }, ControlResponse::Audit { events }) => {
            assert!(events.len() <= 20);
        }
        _ => panic!("{} returned unexpected {response:?}", request.name()),
    }
}
