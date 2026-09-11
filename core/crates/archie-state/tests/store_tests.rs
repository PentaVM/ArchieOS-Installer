//! Integration tests for the durable store.
//!
//! These exercise the store through its public API only, which is how `archied` uses it.

use archie_memory::{
    CorrectionEvent, CorrectionKind, CorrectionTrajectory, GraphCorrectionReceipt,
};
use archie_observability::{AuditEvent, AuditOutcome, AuditSink};
use archie_state::{
    CortexCorrectionTrajectoryWrite, DeviceClass, DeviceOperationState, DevicePairingState,
    StateStore, TrustState, SCHEMA_VERSION,
};
use archie_types::{
    Capability, CpuInfo, DeviceAdapterClass, DeviceAdapterManifest, DeviceAdapterOperation,
    DeviceAdapterOrigin, DeviceAdapterResourceBudget, MemoryInfo, NodeDescriptor, NodeHealth,
    NodeId, NodeLoad, NodeProfile, QueueClass, Task, TaskDependency, TaskStatus, Timestamp,
    TraceId,
};

fn descriptor_with(key_seed: u8, capabilities: &[&str]) -> NodeDescriptor {
    let key = [key_seed; 32];
    NodeDescriptor {
        node_id: NodeId::from_public_key_bytes(&key),
        public_key: hex::encode(key),
        hostname: format!("node-{key_seed}"),
        profile: NodeProfile::DevVm,
        archieos_version: "0.1.0".into(),
        os: "Arch Linux".into(),
        kernel: "6.10.0".into(),
        architecture: "x86_64".into(),
        cpu: CpuInfo {
            model: "Test".into(),
            architecture: "x86_64".into(),
            physical_cores: 4,
            logical_cores: 8,
            base_frequency_mhz: 3000,
        },
        memory: MemoryInfo {
            total_bytes: 16 << 30,
            available_bytes: 8 << 30,
            swap_total_bytes: 0,
        },
        accelerators: vec![],
        storage: vec![],
        networks: vec![],
        capabilities: capabilities
            .iter()
            .map(|c| Capability::parse(*c).unwrap())
            .collect(),
        loaded_models: vec![],
        modules: vec![],
        health: NodeHealth::Ok,
        load: NodeLoad::default(),
        announced_at: Timestamp::now(),
    }
}

fn ready_task(title: &str, class: QueueClass) -> Task {
    let mut task = Task::new(title, TraceId::new(), class);
    task.transition_to(TaskStatus::Ready).unwrap();
    task
}

fn generated_adapter_proposal(adapter_id: &str) -> DeviceAdapterManifest {
    DeviceAdapterManifest::generated_proposal(
        adapter_id.into(),
        archie_types::ModuleId::parse("devices.television").unwrap(),
        vec![DeviceAdapterClass::Television],
        vec![
            DeviceAdapterOperation::ObserveState,
            DeviceAdapterOperation::ApplyDesiredState,
        ],
        DeviceAdapterResourceBudget {
            max_resident_memory_mib: 64,
            max_cpu_millis_per_second: 100,
            max_gpu_memory_mib: 0,
        },
    )
    .unwrap()
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn fresh_database_is_migrated_to_current_schema() {
    let store = StateStore::open_test().unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn migration_is_idempotent_across_reopens() {
    let dir = tempdir("migrate");
    let path = dir.join("database.json");
    for _ in 0..3 {
        let store = StateStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn first_boot_reports_no_previous_comparison() {
    let store = StateStore::open_test().unwrap();
    let delta = store
        .record_boot(&descriptor_with(1, &["compute.cpu"]))
        .unwrap();
    assert!(!delta.had_previous_boot);
    assert!(delta.is_unchanged());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn second_boot_reports_hardware_that_appeared_and_disappeared() {
    let store = StateStore::open_test().unwrap();
    store
        .record_boot(&descriptor_with(1, &["compute.cpu", "compute.cuda"]))
        .unwrap();

    // The GPU is gone this boot, and a LoRa radio was plugged in.
    let delta = store
        .record_boot(&descriptor_with(1, &["compute.cpu", "radio.lora"]))
        .unwrap();

    assert!(delta.had_previous_boot);
    assert_eq!(delta.appeared, vec![Capability::known("radio.lora")]);
    assert_eq!(delta.disappeared, vec![Capability::known("compute.cuda")]);
    assert!(!delta.is_unchanged());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn identical_boot_reports_no_change() {
    let store = StateStore::open_test().unwrap();
    store
        .record_boot(&descriptor_with(1, &["compute.cpu"]))
        .unwrap();
    let delta = store
        .record_boot(&descriptor_with(1, &["compute.cpu"]))
        .unwrap();
    assert!(delta.had_previous_boot);
    assert!(delta.is_unchanged());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn boot_history_is_per_node() {
    let store = StateStore::open_test().unwrap();
    let a = descriptor_with(1, &["compute.cpu"]);
    let b = descriptor_with(2, &["compute.cpu"]);
    store.record_boot(&a).unwrap();
    store.record_boot(&a).unwrap();
    store.record_boot(&b).unwrap();
    assert_eq!(store.boot_count(&a.node_id).unwrap(), 2);
    assert_eq!(store.boot_count(&b.node_id).unwrap(), 1);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn tasks_round_trip_through_storage() {
    let store = StateStore::open_test().unwrap();
    let task = ready_task("investigate tendon fatigue", QueueClass::DeepIdle);
    store.put_task(&task).unwrap();

    let loaded = store
        .get_task(&task.task_id)
        .unwrap()
        .expect("task must exist");
    assert_eq!(loaded, task);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn putting_a_task_twice_updates_rather_than_duplicates() {
    let store = StateStore::open_test().unwrap();
    let mut task = ready_task("t", QueueClass::Active);
    store.put_task(&task).unwrap();
    task.transition_to(TaskStatus::Active).unwrap();
    store.put_task(&task).unwrap();

    assert_eq!(store.task_count().unwrap(), 1);
    assert_eq!(
        store.get_task(&task.task_id).unwrap().unwrap().status,
        TaskStatus::Active
    );
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn an_invalid_task_is_refused_before_it_reaches_disk() {
    let store = StateStore::open_test().unwrap();
    let mut task = ready_task("t", QueueClass::Active);
    // Claim terminal status without a completion time.
    task.status = TaskStatus::Succeeded;
    task.completed_at = None;
    assert!(store.put_task(&task).is_err());
    assert_eq!(store.task_count().unwrap(), 0);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn tasks_can_be_listed_by_status_in_creation_order() {
    let store = StateStore::open_test().unwrap();
    for _ in 0..3 {
        store
            .put_task(&ready_task("ready", QueueClass::Active))
            .unwrap();
    }
    let mut running = ready_task("running", QueueClass::Active);
    running.transition_to(TaskStatus::Active).unwrap();
    running.transition_to(TaskStatus::Running).unwrap();
    store.put_task(&running).unwrap();

    assert_eq!(store.tasks_with_status(TaskStatus::Ready).unwrap().len(), 3);
    assert_eq!(
        store.tasks_with_status(TaskStatus::Running).unwrap().len(),
        1
    );
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn active_graph_pulls_in_terminal_dependencies() {
    let store = StateStore::open_test().unwrap();

    // A finished prerequisite.
    let mut done = ready_task("prerequisite", QueueClass::Active);
    done.transition_to(TaskStatus::Active).unwrap();
    done.transition_to(TaskStatus::Running).unwrap();
    done.transition_to(TaskStatus::Succeeded).unwrap();
    store.put_task(&done).unwrap();

    // A live task that depends on it.
    let mut blocked = Task::new("dependent", TraceId::new(), QueueClass::Active);
    blocked
        .depends_on
        .push(TaskDependency::on_success(done.task_id));
    blocked.transition_to(TaskStatus::Blocked).unwrap();
    store.put_task(&blocked).unwrap();

    let graph = store.load_active_graph().unwrap();
    // Without the terminal dependency loaded, the blocked task could never be judged
    // runnable, and the whole subtree would stall.
    assert!(graph.get(&done.task_id).is_some());
    assert_eq!(graph.runnable().len(), 1);
    assert_eq!(graph.runnable()[0].task_id, blocked.task_id);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn state_survives_a_reopen() {
    let dir = tempdir("persist");
    let path = dir.join("database.json");
    let task = ready_task("durable", QueueClass::Background);
    {
        let store = StateStore::open(&path).unwrap();
        store.put_task(&task).unwrap();
        store
            .record_boot(&descriptor_with(5, &["compute.cpu"]))
            .unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        assert_eq!(store.get_task(&task.task_id).unwrap().unwrap(), task);
        assert_eq!(store.task_count().unwrap(), 1);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_new_peer_is_pinned_on_first_contact() {
    let store = StateStore::open_test().unwrap();
    let d = descriptor_with(9, &["compute.cpu"]);
    let peer = store
        .observe_peer(&d.node_id, &d.public_key, Some(&d))
        .unwrap();
    assert_eq!(peer.trust, TrustState::Pinned);
    assert_eq!(store.list_peers().unwrap().len(), 1);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn re_observing_a_peer_updates_last_seen_but_keeps_first_seen() {
    let store = StateStore::open_test().unwrap();
    let d = descriptor_with(9, &["compute.cpu"]);
    let first = store.observe_peer(&d.node_id, &d.public_key, None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = store
        .observe_peer(&d.node_id, &d.public_key, Some(&d))
        .unwrap();
    assert_eq!(first.first_seen, second.first_seen);
    assert!(second.last_seen >= first.last_seen);
    assert_eq!(store.list_peers().unwrap().len(), 1);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_peer_presenting_a_different_key_is_refused() {
    let store = StateStore::open_test().unwrap();
    let d = descriptor_with(9, &["compute.cpu"]);
    store.observe_peer(&d.node_id, &d.public_key, None).unwrap();

    // Same claimed id, different key: either a reinstall or an impersonation attempt.
    // Either way the operator must decide, so this must not silently re-pin.
    let result = store.observe_peer(&d.node_id, &hex::encode([0xABu8; 32]), None);
    assert!(result.is_err());

    let peers = store.list_peers().unwrap();
    assert_eq!(peers[0].public_key, d.public_key);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn trust_is_operator_controlled_and_gates_scheduling() {
    let store = StateStore::open_test().unwrap();
    let d = descriptor_with(9, &["compute.cpu"]);
    store.observe_peer(&d.node_id, &d.public_key, None).unwrap();

    store
        .set_peer_trust(&d.node_id, TrustState::Trusted)
        .unwrap();
    assert_eq!(store.list_peers().unwrap()[0].trust, TrustState::Trusted);

    store
        .set_peer_trust(&d.node_id, TrustState::Revoked)
        .unwrap();
    let peer = &store.list_peers().unwrap()[0];
    assert_eq!(peer.trust, TrustState::Revoked);
    assert!(!peer.trust.schedulable());
}

#[test]
fn trust_levels_gate_the_right_things() {
    assert!(!TrustState::Unverified.schedulable());
    assert!(TrustState::Pinned.schedulable());
    assert!(TrustState::Trusted.schedulable());
    assert!(!TrustState::Revoked.schedulable());

    // Merely pinned is not enough to receive sensitive data.
    assert!(!TrustState::Pinned.may_receive_sensitive_data());
    assert!(TrustState::Trusted.may_receive_sensitive_data());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn setting_trust_on_an_unknown_peer_fails() {
    let store = StateStore::open_test().unwrap();
    let unknown = NodeId::from_public_key_bytes(&[0xEEu8; 32]);
    assert!(store.set_peer_trust(&unknown, TrustState::Trusted).is_err());
}

fn device_key(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
}

const OPERATION_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CORTEX_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn cortex_trajectory(correction_id: &str, trajectory_id: &str) -> CorrectionTrajectory {
    CorrectionTrajectory {
        trajectory_id: trajectory_id.into(),
        correction: CorrectionEvent {
            correction_id: correction_id.into(),
            kind: CorrectionKind::MissingContextLabel,
            operator_id: "operator:nelson".into(),
            graph_snapshot_id: "graph:before".into(),
            query_digest: CORTEX_DIGEST.into(),
            raw_correction_ref: "vault:corrections/0001".into(),
            rationale_ref: "knowledge:correction-rationales/0001".into(),
            target_ids: vec!["simulation:motor-x".into()],
            recorded_at_ms: 1_700_000_000_000,
        },
        graph_update: GraphCorrectionReceipt {
            correction_id: correction_id.into(),
            source_graph_snapshot_id: "graph:before".into(),
            resulting_graph_snapshot_id: "graph:after".into(),
            graph_update_ref: "knowledge:updates/0001".into(),
            applied_at_ms: 1_700_000_000_001,
        },
        selected_context_ids: vec!["project:robot-arm".into()],
        omitted_candidate_ids: vec!["simulation:motor-x".into()],
        response_ref: "vault:responses/0001".into(),
        response_digest: CORTEX_DIGEST.into(),
        corrected_interpretation_ref: "knowledge:interpretations/0001".into(),
        model_id: "deterministic-v1".into(),
        model_hash: CORTEX_DIGEST.into(),
        captured_at_ms: 1_700_000_000_002,
    }
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_device_key_is_pinned_but_needs_explicit_pairing_confirmation() {
    let store = StateStore::open_test().unwrap();
    let proposed = store
        .propose_device_pairing("living-room-tv", &device_key(0x11), DeviceClass::Television)
        .unwrap();
    assert_eq!(proposed.pairing, DevicePairingState::Proposed);
    assert!(!proposed.pairing.may_receive_commands());

    store.confirm_device_pairing("living-room-tv").unwrap();
    let paired = store.get_device("living-room-tv").unwrap().unwrap();
    assert_eq!(paired.pairing, DevicePairingState::Paired);
    assert!(paired.pairing.may_receive_commands());
    assert_eq!(paired.class, DeviceClass::Television);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn a_device_cannot_replace_its_pinned_key_or_class() {
    let store = StateStore::open_test().unwrap();
    store
        .propose_device_pairing("edge-router", &device_key(0x22), DeviceClass::Router)
        .unwrap();

    assert!(store
        .propose_device_pairing("edge-router", &device_key(0x23), DeviceClass::Router)
        .is_err());
    assert!(store
        .propose_device_pairing("edge-router", &device_key(0x22), DeviceClass::Camera)
        .is_err());

    let stored = store.get_device("edge-router").unwrap().unwrap();
    assert_eq!(stored.public_key, device_key(0x22));
    assert_eq!(stored.class, DeviceClass::Router);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn revocation_is_sticky_across_later_sightings_and_confirmation_attempts() {
    let store = StateStore::open_test().unwrap();
    store
        .propose_device_pairing("bedroom-roku", &device_key(0x33), DeviceClass::Television)
        .unwrap();
    store.confirm_device_pairing("bedroom-roku").unwrap();
    store.revoke_device("bedroom-roku").unwrap();

    let reobserved = store
        .propose_device_pairing("bedroom-roku", &device_key(0x33), DeviceClass::Television)
        .unwrap();
    assert_eq!(reobserved.pairing, DevicePairingState::Revoked);
    assert!(!reobserved.pairing.may_receive_commands());
    assert!(store.confirm_device_pairing("bedroom-roku").is_err());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn device_inventory_survives_a_reopen() {
    let dir = tempdir("device-inventory");
    let path = dir.join("database.json");
    {
        let store = StateStore::open(&path).unwrap();
        store
            .propose_device_pairing("office-projector", &device_key(0x44), DeviceClass::Display)
            .unwrap();
        store.confirm_device_pairing("office-projector").unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        let devices = store.list_devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_reference, "office-projector");
        assert_eq!(devices[0].pairing, DevicePairingState::Paired);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn malformed_device_identity_is_refused_before_storage() {
    let store = StateStore::open_test().unwrap();
    assert!(store
        .propose_device_pairing(
            "living room tv; unsafe",
            &device_key(0x55),
            DeviceClass::Television,
        )
        .is_err());
    assert!(store
        .propose_device_pairing("living-room-tv", "NOT-A-KEY", DeviceClass::Television)
        .is_err());
    assert!(store.list_devices().unwrap().is_empty());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn prepared_device_operations_require_a_currently_paired_device() {
    let store = StateStore::open_test().unwrap();
    store
        .propose_device_pairing("living-room-tv", &device_key(0x66), DeviceClass::Television)
        .unwrap();
    assert!(store
        .prepare_device_operation("living-room-tv", TraceId::new(), OPERATION_DIGEST)
        .is_err());

    store.confirm_device_pairing("living-room-tv").unwrap();
    let prepared = store
        .prepare_device_operation("living-room-tv", TraceId::new(), OPERATION_DIGEST)
        .unwrap();
    assert_ne!(prepared.operation_id, 0);
    assert_eq!(prepared.state, DeviceOperationState::Prepared);
    assert_eq!(prepared.operation_sha256, OPERATION_DIGEST);
    assert_eq!(
        store
            .get_device_operation(prepared.operation_id)
            .unwrap()
            .unwrap(),
        prepared
    );
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn prepared_device_operation_is_immutable_and_cancellation_is_sticky() {
    let store = StateStore::open_test().unwrap();
    store
        .propose_device_pairing("living-room-tv", &device_key(0x77), DeviceClass::Television)
        .unwrap();
    store.confirm_device_pairing("living-room-tv").unwrap();
    let prepared = store
        .prepare_device_operation("living-room-tv", TraceId::new(), OPERATION_DIGEST)
        .unwrap();

    store
        .cancel_device_operation(prepared.operation_id)
        .unwrap();
    store
        .cancel_device_operation(prepared.operation_id)
        .unwrap();
    let cancelled = store
        .get_device_operation(prepared.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.state, DeviceOperationState::Cancelled);
    assert_eq!(cancelled.operation_sha256, OPERATION_DIGEST);
    assert_eq!(store.recent_device_operations(1).unwrap(), vec![cancelled]);
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn invalid_device_operation_digests_never_reach_storage() {
    let store = StateStore::open_test().unwrap();
    store
        .propose_device_pairing("living-room-tv", &device_key(0x88), DeviceClass::Television)
        .unwrap();
    store.confirm_device_pairing("living-room-tv").unwrap();
    assert!(store
        .prepare_device_operation("living-room-tv", TraceId::new(), "not-a-digest")
        .is_err());
    assert!(store.recent_device_operations(10).unwrap().is_empty());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn prepared_device_operation_survives_a_reopen_without_becoming_authority() {
    let dir = tempdir("device-operation");
    let path = dir.join("database.json");
    let operation_id;
    {
        let store = StateStore::open(&path).unwrap();
        store
            .propose_device_pairing("living-room-tv", &device_key(0x99), DeviceClass::Television)
            .unwrap();
        store.confirm_device_pairing("living-room-tv").unwrap();
        operation_id = store
            .prepare_device_operation("living-room-tv", TraceId::new(), OPERATION_DIGEST)
            .unwrap()
            .operation_id;
    }
    {
        let store = StateStore::open(&path).unwrap();
        let prepared = store.get_device_operation(operation_id).unwrap().unwrap();
        assert_eq!(prepared.state, DeviceOperationState::Prepared);
        store.cancel_device_operation(operation_id).unwrap();
        assert_eq!(
            store
                .get_device_operation(operation_id)
                .unwrap()
                .unwrap()
                .state,
            DeviceOperationState::Cancelled
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn generated_adapter_proposals_persist_as_inactive_hashed_history() {
    let dir = tempdir("adapter-proposal");
    let path = dir.join("database.json");
    let second_id;
    {
        let store = StateStore::open(&path).unwrap();
        let first = store
            .record_generated_adapter_proposal(&generated_adapter_proposal("living-room-tv"))
            .unwrap();
        let second = store
            .record_generated_adapter_proposal(&generated_adapter_proposal("office-tv"))
            .unwrap();
        second_id = second.proposal_id;
        assert_ne!(first.proposal_id, second.proposal_id);
        assert_eq!(
            first.manifest.origin,
            DeviceAdapterOrigin::GeneratedProposal
        );
        assert_eq!(first.manifest_sha256.len(), 64);
        let proposals = store.recent_generated_adapter_proposals(10).unwrap();
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].manifest.adapter_id, "office-tv");
        assert_eq!(proposals[1].manifest.adapter_id, "living-room-tv");
    }
    {
        let store = StateStore::open(&path).unwrap();
        let proposal = store
            .recent_generated_adapter_proposals(1)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(proposal.proposal_id, second_id);
        assert_eq!(proposal.manifest.adapter_id, "office-tv");
        assert_eq!(
            proposal.manifest.origin,
            DeviceAdapterOrigin::GeneratedProposal
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn cortex_trajectories_are_atomic_idempotent_and_require_graph_receipts() {
    let store = StateStore::open_test().unwrap();
    let trajectory = cortex_trajectory("correction:0001", "trajectory:0001");
    let first = store
        .record_cortex_correction_trajectory(&trajectory)
        .unwrap();
    assert!(matches!(
        first,
        CortexCorrectionTrajectoryWrite::Recorded(_)
    ));

    let second = store
        .record_cortex_correction_trajectory(&trajectory)
        .unwrap();
    assert!(matches!(
        second,
        CortexCorrectionTrajectoryWrite::AlreadyRecorded(_)
    ));
    let records = store.recent_cortex_correction_trajectories(10).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].trajectory, trajectory);
    assert_eq!(records[0].payload_sha256.len(), 64);

    let conflicting = cortex_trajectory("correction:0001", "trajectory:other");
    assert!(store
        .record_cortex_correction_trajectory(&conflicting)
        .is_err());
    assert_eq!(
        store
            .recent_cortex_correction_trajectories(10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn cortex_trajectory_survives_reopen_without_raw_payloads() {
    let dir = tempdir("cortex-trajectory");
    let path = dir.join("database.json");
    let trajectory = cortex_trajectory("correction:0002", "trajectory:0002");
    {
        let store = StateStore::open(&path).unwrap();
        store
            .record_cortex_correction_trajectory(&trajectory)
            .unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        let stored = store.recent_cortex_correction_trajectories(1).unwrap();
        assert_eq!(stored[0].trajectory.response_ref, "vault:responses/0001");
        assert_eq!(stored[0].trajectory.response_digest, CORTEX_DIGEST);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn native_adapter_contracts_cannot_enter_generated_proposal_history() {
    let store = StateStore::open_test().unwrap();
    let mut native_contract = generated_adapter_proposal("living-room-tv");
    native_contract.origin = DeviceAdapterOrigin::BuiltIntoIso;
    assert!(store
        .record_generated_adapter_proposal(&native_contract)
        .is_err());
    assert!(store
        .recent_generated_adapter_proposals(10)
        .unwrap()
        .is_empty());
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn audit_events_persist_and_read_back_newest_first() {
    let store = StateStore::open_test().unwrap();
    let node = NodeId::from_public_key_bytes(&[1u8; 32]);
    for action in ["first.action", "second.action", "third.action"] {
        store
            .record(AuditEvent::new(
                node,
                action,
                "operator",
                "resource",
                AuditOutcome::Allowed,
                "because the operator said so",
            ))
            .unwrap();
    }
    let recent = store.recent_audit(2).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].action, "third.action");
    assert_eq!(recent[1].action, "second.action");
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn audit_survives_a_reopen() {
    let dir = tempdir("audit");
    let path = dir.join("database.json");
    let node = NodeId::from_public_key_bytes(&[2u8; 32]);
    {
        let store = StateStore::open(&path).unwrap();
        store
            .record(AuditEvent::new(
                node,
                "device.actuate",
                "sentry",
                "joint-3",
                AuditOutcome::Denied,
                "no lease held",
            ))
            .unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        let events = store.recent_audit(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, AuditOutcome::Denied);
        assert!(events[0].is_physical());
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A unique temporary directory for this test process.
fn tempdir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("archie-state-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
