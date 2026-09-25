//! Integration tests for the resource graph, module topology compiler, and model
//! registry.

use archie_registry::{
    DeviceAdapterRegistry, DeviceAdapterRegistryError, ModelRegistry, ModuleRegistry, NodeOrigin,
    NodeRecord, PermissionPolicy, ResourceGraph, TopologyContext,
};
use archie_state::TrustState;
use archie_types::{
    Accelerator, AcceleratorKind, Availability, Capability, CpuInfo, DataRetention,
    DeviceAdapterClass, DeviceAdapterManifest, DeviceAdapterOperation, DeviceAdapterOrigin,
    DeviceAdapterResourceBudget, HealthCheck, LinkClass, Locality, MemoryInfo, ModelDescriptor,
    ModelId, ModuleId, ModuleManifest, ModulePermission, NetworkInterface, NodeDescriptor,
    NodeHealth, NodeId, NodeLoad, NodeProfile, PrivacyGrade, ProviderKind, Timestamp, TokenCost,
    WorkerImpl,
};
use std::collections::BTreeMap;

// ------------------------------------------------------------------ fixtures

struct NodeBuilder {
    descriptor: NodeDescriptor,
    trust: TrustState,
    reachable: bool,
    origin: NodeOrigin,
}

impl NodeBuilder {
    fn new(seed: u8, hostname: &str) -> Self {
        let key = [seed; 32];
        Self {
            descriptor: NodeDescriptor {
                node_id: NodeId::from_public_key_bytes(&key),
                public_key: hex::encode(key),
                hostname: hostname.into(),
                profile: NodeProfile::Server,
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
                networks: vec![NetworkInterface {
                    name: "eth0".into(),
                    addresses: vec!["10.0.0.1".into()],
                    mac: None,
                    speed_mbps: Some(1000),
                    link_class: LinkClass::Lan,
                }],
                capabilities: [Capability::known("compute.cpu")].into_iter().collect(),
                loaded_models: vec![],
                modules: vec![],
                health: NodeHealth::Ok,
                load: NodeLoad::default(),
                announced_at: Timestamp::now(),
            },
            trust: TrustState::Trusted,
            reachable: true,
            origin: NodeOrigin::Peer,
        }
    }

    fn local(mut self) -> Self {
        self.origin = NodeOrigin::Local;
        self
    }

    fn profile(mut self, profile: NodeProfile) -> Self {
        self.descriptor.profile = profile;
        self
    }

    fn capability(mut self, name: &str) -> Self {
        self.descriptor
            .capabilities
            .insert(Capability::known_or_parse(name));
        self
    }

    fn cuda(mut self, gib: u64) -> Self {
        self.descriptor
            .capabilities
            .insert(Capability::parse("compute.cuda").unwrap());
        self.descriptor.accelerators.push(Accelerator {
            kind: AcceleratorKind::Cuda,
            vendor: "NVIDIA".into(),
            model: "RTX 3070".into(),
            memory_bytes: Some(gib << 30),
            driver_version: Some("560.0".into()),
            index: self.descriptor.accelerators.len() as u32,
        });
        self
    }

    fn memory_available(mut self, gib: u64) -> Self {
        self.descriptor.memory.available_bytes = gib << 30;
        self
    }

    fn link(mut self, class: LinkClass) -> Self {
        for interface in &mut self.descriptor.networks {
            interface.link_class = class;
        }
        self
    }

    fn health(mut self, health: NodeHealth) -> Self {
        self.descriptor.health = health;
        self
    }

    fn trust(mut self, trust: TrustState) -> Self {
        self.trust = trust;
        self
    }

    fn unreachable(mut self) -> Self {
        self.reachable = false;
        self
    }

    fn build(self) -> NodeRecord {
        NodeRecord {
            descriptor: self.descriptor,
            origin: self.origin,
            trust: self.trust,
            reachable: self.reachable,
            last_seen: Timestamp::now(),
        }
    }
}

/// Helper so the fixtures read cleanly.
trait CapabilityExt {
    fn known_or_parse(name: &str) -> Capability;
}
impl CapabilityExt for Capability {
    fn known_or_parse(name: &str) -> Capability {
        Capability::parse(name).expect("test capability must be valid")
    }
}

fn manifest(id: &str) -> ModuleManifest {
    ModuleManifest {
        id: ModuleId::parse(id).unwrap(),
        version: "0.1.0".into(),
        description: "test module".into(),
        required_capabilities: vec![],
        optional_capabilities: vec![],
        dependencies: vec![],
        permissions: vec![],
        architectures: vec![],
        profiles: vec![],
        priority: 100,
        worker: WorkerImpl::None,
        health_check: HealthCheck::None,
        config_schema: None,
        labels: BTreeMap::new(),
    }
}

fn native_device_module(id: &str) -> ModuleManifest {
    let mut module = manifest(id);
    module.permissions = vec![
        ModulePermission::DeviceCommunicate,
        ModulePermission::DeviceActuate,
    ];
    module.worker = WorkerImpl::Builtin {
        symbol: "archie_device_adapter".into(),
    };
    module
}

fn device_adapter(id: &str, module_id: &str, origin: DeviceAdapterOrigin) -> DeviceAdapterManifest {
    DeviceAdapterManifest {
        adapter_id: id.into(),
        module_id: ModuleId::parse(module_id).unwrap(),
        origin,
        supported_classes: vec![DeviceAdapterClass::Television],
        operations: vec![
            DeviceAdapterOperation::ObserveState,
            DeviceAdapterOperation::ApplyDesiredState,
        ],
        resource_budget: DeviceAdapterResourceBudget {
            max_resident_memory_mib: 64,
            max_cpu_millis_per_second: 100,
            max_gpu_memory_mib: 0,
        },
        dry_run_required: true,
        receipt_required: true,
    }
}

fn model(id: &str, provider: ProviderKind, locality: Locality) -> ModelDescriptor {
    ModelDescriptor {
        id: ModelId::parse(id).unwrap(),
        display_name: id.into(),
        provider,
        locality,
        node: None,
        context_window: 8192,
        max_output_tokens: 2048,
        capabilities: vec![Capability::parse("compute.inference").unwrap()],
        max_privacy_grade: PrivacyGrade::Restricted,
        retention: DataRetention::Unknown,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 50,
        required_accelerator_bytes: None,
        labels: BTreeMap::new(),
    }
}

// ------------------------------------------------------------ resource graph

#[test]
fn a_lone_local_node_is_a_valid_fabric() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "solo").local().build());

    assert_eq!(graph.len(), 1);
    assert_eq!(graph.totals().schedulable_nodes, 1);
    assert!(graph.local().is_some());
    assert!(graph.has_canonical_state_holder());
}

#[test]
fn totals_count_only_schedulable_nodes() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "good").local().build());
    graph.upsert(NodeBuilder::new(2, "offline").unreachable().build());
    graph.upsert(
        NodeBuilder::new(3, "sick")
            .health(NodeHealth::Unhealthy)
            .build(),
    );
    graph.upsert(
        NodeBuilder::new(4, "untrusted")
            .trust(TrustState::Unverified)
            .build(),
    );

    let totals = graph.totals();
    assert_eq!(totals.nodes, 4, "all nodes are known");
    assert_eq!(
        totals.schedulable_nodes, 1,
        "capacity Archie cannot use must not be advertised as available"
    );
    assert_eq!(totals.logical_cores, 8);
}

#[test]
fn each_eligibility_condition_can_independently_exclude_a_node() {
    for (label, record) in [
        (
            "unreachable",
            NodeBuilder::new(1, "n").unreachable().build(),
        ),
        (
            "unhealthy",
            NodeBuilder::new(2, "n")
                .health(NodeHealth::Unhealthy)
                .build(),
        ),
        (
            "revoked",
            NodeBuilder::new(3, "n").trust(TrustState::Revoked).build(),
        ),
        (
            "unverified",
            NodeBuilder::new(4, "n")
                .trust(TrustState::Unverified)
                .build(),
        ),
    ] {
        assert!(
            !record.schedulable(),
            "{label} node must not be schedulable"
        );
        assert!(!record.unschedulable_reasons().is_empty());
    }
}

#[test]
fn unschedulable_reasons_report_every_problem_at_once() {
    let record = NodeBuilder::new(1, "bad")
        .unreachable()
        .health(NodeHealth::Unhealthy)
        .trust(TrustState::Revoked)
        .build();
    assert_eq!(record.unschedulable_reasons().len(), 3);
}

#[test]
fn providers_of_finds_the_node_with_the_capability() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "cpu-only").local().build());
    graph.upsert(NodeBuilder::new(2, "gpu-box").cuda(8).build());

    let cuda = Capability::parse("compute.cuda").unwrap();
    let providers = graph.providers_of(&cuda);
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].descriptor.hostname, "gpu-box");
}

#[test]
fn an_untrusted_gpu_node_is_visible_but_not_schedulable() {
    // "nobody has a GPU" and "the only GPU node is untrusted" need different fixes,
    // so the graph must distinguish them.
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "cpu").local().build());
    graph.upsert(
        NodeBuilder::new(2, "gpu")
            .cuda(8)
            .trust(TrustState::Unverified)
            .build(),
    );

    let cuda = Capability::parse("compute.cuda").unwrap();
    assert!(graph.providers_of(&cuda).is_empty());
    assert_eq!(graph.all_providers_of(&cuda).len(), 1);
}

#[test]
fn providers_of_all_requires_every_capability() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "cpu").local().build());
    graph.upsert(NodeBuilder::new(2, "gpu").cuda(8).build());
    graph.upsert(
        NodeBuilder::new(3, "gpu-and-net")
            .cuda(8)
            .capability("network.internet")
            .build(),
    );

    let required = vec![
        Capability::parse("compute.cuda").unwrap(),
        Capability::parse("network.internet").unwrap(),
    ];
    let matches = graph.providers_of_all(&required);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].descriptor.hostname, "gpu-and-net");
}

#[test]
fn accelerator_memory_filter_excludes_nodes_that_cannot_fit_the_model() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "small-gpu").cuda(8).build());
    graph.upsert(NodeBuilder::new(2, "big-gpu").cuda(48).build());

    assert_eq!(graph.nodes_with_accelerator_memory(24 << 30).len(), 1);
    assert_eq!(graph.nodes_with_accelerator_memory(4 << 30).len(), 2);
    assert!(graph.nodes_with_accelerator_memory(128 << 30).is_empty());
}

#[test]
fn node_selection_prefers_local_then_free_memory_and_is_deterministic() {
    let mut graph = ResourceGraph::new();
    graph.upsert(
        NodeBuilder::new(1, "local")
            .local()
            .memory_available(2)
            .build(),
    );
    graph.upsert(
        NodeBuilder::new(2, "roomy-peer")
            .memory_available(64)
            .build(),
    );

    let required = vec![Capability::parse("compute.cpu").unwrap()];
    // Local wins despite having far less free memory: no network cost.
    let first = graph.select_node(&required, None).unwrap();
    assert_eq!(first.descriptor.hostname, "local");

    // Repeated calls agree.
    for _ in 0..5 {
        assert_eq!(
            graph
                .select_node(&required, None)
                .unwrap()
                .descriptor
                .hostname,
            "local"
        );
    }
}

#[test]
fn node_selection_falls_back_to_the_roomiest_peer() {
    let mut graph = ResourceGraph::new();
    graph.upsert(
        NodeBuilder::new(1, "peer-small")
            .memory_available(4)
            .build(),
    );
    graph.upsert(
        NodeBuilder::new(2, "peer-large")
            .memory_available(64)
            .build(),
    );

    let required = vec![Capability::parse("compute.cpu").unwrap()];
    assert_eq!(
        graph
            .select_node(&required, None)
            .unwrap()
            .descriptor
            .hostname,
        "peer-large"
    );
}

#[test]
fn node_selection_returns_none_when_nothing_qualifies() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "cpu").local().build());
    let required = vec![Capability::parse("simulation.cfd").unwrap()];
    assert!(graph.select_node(&required, None).is_none());
}

#[test]
fn tensor_parallel_is_refused_across_a_slow_link() {
    let mut graph = ResourceGraph::new();
    let fast_a = NodeBuilder::new(1, "fast-a")
        .link(LinkClass::FastLan)
        .build();
    let fast_b = NodeBuilder::new(2, "fast-b")
        .link(LinkClass::FastLan)
        .build();
    let slow = NodeBuilder::new(3, "slow").link(LinkClass::Overlay).build();
    let (a, b, c) = (fast_a.node_id(), fast_b.node_id(), slow.node_id());
    graph.upsert(fast_a);
    graph.upsert(fast_b);
    graph.upsert(slow);

    assert!(graph.permits_tensor_parallel(&a, &b), "10GbE pair is fine");
    assert!(
        !graph.permits_tensor_parallel(&a, &c),
        "splitting a model over a VPN turns every layer boundary into a round trip"
    );
    assert!(
        graph.permits_tensor_parallel(&a, &a),
        "same node is trivially fine"
    );
}

#[test]
fn tensor_parallel_is_refused_for_an_unknown_node() {
    let mut graph = ResourceGraph::new();
    let known = NodeBuilder::new(1, "known")
        .link(LinkClass::FastLan)
        .build();
    let id = known.node_id();
    graph.upsert(known);
    let stranger = NodeId::from_public_key_bytes(&[99u8; 32]);
    assert!(!graph.permits_tensor_parallel(&id, &stranger));
}

#[test]
fn capability_index_maps_capabilities_to_nodes() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "a").local().build());
    graph.upsert(NodeBuilder::new(2, "b").cuda(8).build());

    let index = graph.capability_index();
    let cpu = Capability::parse("compute.cpu").unwrap();
    let cuda = Capability::parse("compute.cuda").unwrap();
    assert_eq!(index.get(&cpu).map(Vec::len), Some(2));
    assert_eq!(index.get(&cuda).map(Vec::len), Some(1));
}

#[test]
fn graph_reports_what_appeared_and_disappeared() {
    let mut graph = ResourceGraph::new();
    let a = NodeBuilder::new(1, "a").local().build();
    let b = NodeBuilder::new(2, "b").build();
    let a_id = a.node_id();
    let b_id = b.node_id();
    graph.upsert(a);
    graph.upsert(b);
    let before = graph.node_ids();

    graph.remove(&b_id);
    let c = NodeBuilder::new(3, "c").build();
    let c_id = c.node_id();
    graph.upsert(c);

    assert_eq!(graph.appeared_since(&before), vec![c_id]);
    assert_eq!(graph.disappeared_since(&before), vec![b_id]);
    assert!(graph.node_ids().contains(&a_id));
}

#[test]
fn a_fabric_of_only_dev_vms_has_nowhere_durable_to_put_knowledge() {
    let mut graph = ResourceGraph::new();
    graph.upsert(
        NodeBuilder::new(1, "throwaway")
            .local()
            .profile(NodeProfile::DevVm)
            .build(),
    );
    assert!(
        !graph.has_canonical_state_holder(),
        "a disposable node must not silently become the only copy of canonical state"
    );

    graph.upsert(
        NodeBuilder::new(2, "server")
            .profile(NodeProfile::Server)
            .build(),
    );
    assert!(graph.has_canonical_state_holder());
}

#[test]
fn worst_link_class_bounds_fabric_wide_strategy() {
    let mut graph = ResourceGraph::new();
    graph.upsert(NodeBuilder::new(1, "fast").link(LinkClass::FastLan).build());
    assert_eq!(graph.worst_link_class(), LinkClass::FastLan);
    graph.upsert(
        NodeBuilder::new(2, "radio")
            .link(LinkClass::Constrained)
            .build(),
    );
    assert_eq!(graph.worst_link_class(), LinkClass::Constrained);
}

// ------------------------------------------------------------ module topology

fn context() -> TopologyContext {
    TopologyContext::new(
        [Capability::parse("compute.cpu").unwrap()]
            .into_iter()
            .collect(),
        "x86_64",
        "server",
    )
}

#[test]
fn a_satisfiable_module_loads() {
    let mut registry = ModuleRegistry::new();
    registry.register(manifest("core.archie")).unwrap();

    let topology = registry.compile_topology(&context());
    assert!(topology.is_loaded(&ModuleId::parse("core.archie").unwrap()));
    assert_eq!(topology.loaded_count(), 1);
}

#[test]
fn a_module_needing_absent_hardware_is_excluded_with_a_reason() {
    let mut registry = ModuleRegistry::new();
    let mut m = manifest("science.physics");
    m.required_capabilities
        .push(Capability::parse("compute.cuda").unwrap());
    registry.register(m).unwrap();

    let topology = registry.compile_topology(&context());
    assert_eq!(topology.loaded_count(), 0);
    let excluded = topology.excluded();
    assert_eq!(excluded.len(), 1);
    assert!(
        excluded[0].1.contains("compute.cuda"),
        "the explanation must name the missing capability, got: {}",
        excluded[0].1
    );
}

#[test]
fn architecture_and_profile_mismatches_are_reported_distinctly() {
    let mut registry = ModuleRegistry::new();
    let mut arm_only = manifest("devices.pi");
    arm_only.architectures.push("aarch64".into());
    registry.register(arm_only).unwrap();

    let mut gateway_only = manifest("devices.lora");
    gateway_only.profiles.push("edge-gateway".into());
    registry.register(gateway_only).unwrap();

    let topology = registry.compile_topology(&context());
    let explanations: Vec<String> = topology.excluded().into_iter().map(|(_, e)| e).collect();
    assert!(explanations.iter().any(|e| e.contains("x86_64")));
    assert!(explanations.iter().any(|e| e.contains("server")));
}

#[test]
fn a_module_requesting_an_ungranted_permission_is_denied() {
    let mut registry = ModuleRegistry::new();
    let mut m = manifest("devices.robot");
    m.permissions.push(ModulePermission::DeviceCommunicate);
    m.permissions.push(ModulePermission::DeviceActuate);
    registry.register(m).unwrap();

    // The default-safe policy grants nothing physical.
    let topology = registry.compile_topology(&context());
    assert_eq!(topology.loaded_count(), 0);
    assert!(topology.excluded()[0].1.contains("permission denied"));
}

#[test]
fn granting_the_permission_lets_the_module_load() {
    let mut registry = ModuleRegistry::new();
    let mut m = manifest("devices.robot");
    m.permissions.push(ModulePermission::DeviceCommunicate);
    registry.register(m).unwrap();

    let ctx = context().with_permissions(
        PermissionPolicy::default_safe().grant(ModulePermission::DeviceCommunicate),
    );
    assert_eq!(registry.compile_topology(&ctx).loaded_count(), 1);
}

#[test]
fn deny_all_blocks_even_read_only_modules() {
    let mut registry = ModuleRegistry::new();
    let mut m = manifest("cognition.critic");
    m.permissions.push(ModulePermission::ReadKnowledge);
    registry.register(m).unwrap();

    let ctx = context().with_permissions(PermissionPolicy::deny_all());
    assert_eq!(registry.compile_topology(&ctx).loaded_count(), 0);
}

#[test]
fn an_explicitly_disabled_module_does_not_load() {
    let mut registry = ModuleRegistry::new();
    registry.register(manifest("core.archie")).unwrap();
    let ctx = context().disable(ModuleId::parse("core.archie").unwrap());
    let topology = registry.compile_topology(&ctx);
    assert_eq!(topology.loaded_count(), 0);
    assert!(topology.excluded()[0].1.contains("disabled"));
}

#[test]
fn a_module_with_an_uninstalled_dependency_is_excluded() {
    let mut registry = ModuleRegistry::new();
    let mut m = manifest("cognition.critic");
    m.dependencies.push(ModuleId::parse("core.memory").unwrap());
    registry.register(m).unwrap();

    let topology = registry.compile_topology(&context());
    assert_eq!(topology.loaded_count(), 0);
    assert!(topology.excluded()[0].1.contains("core.memory"));
}

#[test]
fn dependency_failure_propagates_transitively() {
    // base needs CUDA (absent) -> middle depends on base -> top depends on middle.
    // All three must fail, not just base.
    let mut registry = ModuleRegistry::new();
    let mut base = manifest("core.gpu");
    base.required_capabilities
        .push(Capability::parse("compute.cuda").unwrap());
    registry.register(base).unwrap();

    let mut middle = manifest("cognition.middle");
    middle
        .dependencies
        .push(ModuleId::parse("core.gpu").unwrap());
    registry.register(middle).unwrap();

    let mut top = manifest("cognition.top");
    top.dependencies
        .push(ModuleId::parse("cognition.middle").unwrap());
    registry.register(top).unwrap();

    let topology = registry.compile_topology(&context());
    assert_eq!(
        topology.loaded_count(),
        0,
        "a module must not load behind a dependency that failed"
    );
    assert_eq!(topology.excluded().len(), 3);
}

#[test]
fn load_order_places_dependencies_first() {
    let mut registry = ModuleRegistry::new();
    registry.register(manifest("core.memory")).unwrap();

    let mut critic = manifest("cognition.critic");
    critic
        .dependencies
        .push(ModuleId::parse("core.memory").unwrap());
    // Give the dependent a *lower* priority number so that priority alone would order
    // it first — dependency order must win.
    critic.priority = 1;
    registry.register(critic).unwrap();

    let topology = registry.compile_topology(&context());
    let order: Vec<String> = topology.load_order.iter().map(|m| m.to_string()).collect();
    assert_eq!(order, vec!["core.memory", "cognition.critic"]);
}

#[test]
fn load_order_is_deterministic_across_runs() {
    let mut registry = ModuleRegistry::new();
    for id in ["core.a", "core.b", "core.c", "cognition.d"] {
        registry.register(manifest(id)).unwrap();
    }
    let first = registry.compile_topology(&context()).load_order;
    for _ in 0..5 {
        assert_eq!(registry.compile_topology(&context()).load_order, first);
    }
}

#[test]
fn priority_orders_independent_modules() {
    let mut registry = ModuleRegistry::new();
    let mut early = manifest("core.early");
    early.priority = 10;
    let mut late = manifest("core.late");
    late.priority = 900;
    registry.register(late).unwrap();
    registry.register(early).unwrap();

    let order: Vec<String> = registry
        .compile_topology(&context())
        .load_order
        .iter()
        .map(|m| m.to_string())
        .collect();
    assert_eq!(order, vec!["core.early", "core.late"]);
}

#[test]
fn a_dependency_cycle_is_detected_and_no_member_loads() {
    let mut registry = ModuleRegistry::new();
    let mut a = manifest("core.a");
    a.dependencies.push(ModuleId::parse("core.b").unwrap());
    let mut b = manifest("core.b");
    b.dependencies.push(ModuleId::parse("core.a").unwrap());
    registry.register(a).unwrap();
    registry.register(b).unwrap();

    let topology = registry.compile_topology(&context());
    assert!(!topology.cycles.is_empty(), "the cycle must be reported");
    assert_eq!(topology.loaded_count(), 0);
}

#[test]
fn an_invalid_manifest_is_marked_invalid_rather_than_crashing_the_compile() {
    let mut registry = ModuleRegistry::new();
    let mut bad = manifest("devices.halfbaked");
    // Actuate without communicate — rejected by manifest validation.
    bad.permissions.push(ModulePermission::DeviceActuate);
    registry.register(bad).unwrap();
    registry.register(manifest("core.archie")).unwrap();

    let topology = registry.compile_topology(&context());
    assert_eq!(
        topology.loaded_count(),
        1,
        "one bad manifest must not stop the rest of the topology"
    );
    assert!(topology.excluded()[0].1.contains("invalid manifest"));
}

#[test]
fn registering_a_duplicate_module_id_is_refused() {
    let mut registry = ModuleRegistry::new();
    registry.register(manifest("core.archie")).unwrap();
    assert!(registry.register(manifest("core.archie")).is_err());
}

#[test]
fn adapter_catalogue_keeps_generated_proposals_separate_from_native_contracts() {
    let mut modules = ModuleRegistry::new();
    modules
        .register(native_device_module("devices.television"))
        .unwrap();
    let mut adapters = DeviceAdapterRegistry::new();

    let proposal = device_adapter(
        "generated-living-room-tv",
        "devices.television",
        DeviceAdapterOrigin::GeneratedProposal,
    );
    adapters.submit_generated_proposal(proposal).unwrap();
    assert_eq!(adapters.generated_proposals().count(), 1);
    assert_eq!(adapters.admitted_native_contracts().count(), 0);

    let contract = device_adapter(
        "native-living-room-tv",
        "devices.television",
        DeviceAdapterOrigin::BuiltIntoIso,
    );
    adapters.admit_native_contract(contract, &modules).unwrap();
    assert_eq!(adapters.generated_proposals().count(), 1);
    assert_eq!(adapters.admitted_native_contracts().count(), 1);
    assert_eq!(adapters.len(), 2);
}

#[test]
fn adapter_catalogue_rejects_an_origin_or_module_that_cannot_be_admitted() {
    let mut adapters = DeviceAdapterRegistry::new();
    let native = device_adapter(
        "native-living-room-tv",
        "devices.television",
        DeviceAdapterOrigin::BuiltIntoIso,
    );
    assert!(matches!(
        adapters.submit_generated_proposal(native),
        Err(DeviceAdapterRegistryError::ProposalExpected)
    ));

    let proposal = device_adapter(
        "generated-living-room-tv",
        "devices.television",
        DeviceAdapterOrigin::GeneratedProposal,
    );
    let modules = ModuleRegistry::new();
    assert!(matches!(
        adapters.admit_native_contract(proposal, &modules),
        Err(DeviceAdapterRegistryError::NativeContractExpected)
    ));

    let missing_module = device_adapter(
        "missing-module-tv",
        "devices.television",
        DeviceAdapterOrigin::BuiltIntoIso,
    );
    assert!(matches!(
        adapters.admit_native_contract(missing_module, &modules),
        Err(DeviceAdapterRegistryError::MissingHostingModule(_))
    ));
}

#[test]
fn adapter_catalogue_rejects_shadowing_a_generated_proposal_with_native_code() {
    let mut modules = ModuleRegistry::new();
    modules
        .register(native_device_module("devices.television"))
        .unwrap();
    let mut adapters = DeviceAdapterRegistry::new();
    adapters
        .submit_generated_proposal(device_adapter(
            "living-room-tv",
            "devices.television",
            DeviceAdapterOrigin::GeneratedProposal,
        ))
        .unwrap();

    assert!(matches!(
        adapters.admit_native_contract(
            device_adapter(
                "living-room-tv",
                "devices.television",
                DeviceAdapterOrigin::BuiltIntoIso,
            ),
            &modules,
        ),
        Err(DeviceAdapterRegistryError::Duplicate(id)) if id == "living-room-tv"
    ));
}

#[test]
fn a_missing_module_directory_yields_an_empty_registry() {
    let missing = std::env::temp_dir().join("archie-no-such-module-dir-xyz");
    let _ = std::fs::remove_dir_all(&missing);
    let registry = ModuleRegistry::load_from_dir(&missing).unwrap();
    assert!(registry.is_empty());
}

#[test]
fn manifests_load_from_a_directory() {
    let dir = std::env::temp_dir().join(format!("archie-modules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("critic.toml"),
        r#"
id = "cognition.critic"
version = "0.1.0"
description = "Adversarial review"
priority = 50

[worker]
kind = "unimplemented"
reason = "not written yet"
"#,
    )
    .unwrap();
    // A non-manifest file must be ignored rather than breaking the load.
    std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();

    let registry = ModuleRegistry::load_from_dir(&dir).unwrap();
    assert_eq!(registry.len(), 1);
    assert!(registry
        .get(&ModuleId::parse("cognition.critic").unwrap())
        .is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_malformed_manifest_is_an_error_not_a_silent_skip() {
    let dir = std::env::temp_dir().join(format!("archie-badmod-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.toml"), "id = \"not valid toml").unwrap();

    // A typo in a manifest must not present as a missing feature.
    assert!(ModuleRegistry::load_from_dir(&dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

// -------------------------------------------------------------- model registry

#[test]
fn a_model_registers_and_is_retrievable() {
    let mut registry = ModelRegistry::new();
    let m = model("local.qwen3", ProviderKind::LlamaCpp, Locality::Local);
    let id = m.id.clone();
    registry.register(m).unwrap();
    assert!(registry.get(&id).is_some());
    assert_eq!(registry.len(), 1);
}

#[test]
fn a_cloud_model_claiming_to_be_local_is_refused() {
    // This is a security control: registering an OpenAI model as `local` would let
    // restricted data pass every downstream privacy check on its way off the machine.
    let mut registry = ModelRegistry::new();
    let sneaky = model("cloud.gpt", ProviderKind::OpenAi, Locality::Local);
    assert!(registry.register(sneaky).is_err());
    assert!(registry.is_empty());
}

#[test]
fn a_duplicate_model_id_is_refused_but_upsert_replaces() {
    let mut registry = ModelRegistry::new();
    registry
        .register(model("local.a", ProviderKind::LlamaCpp, Locality::Local))
        .unwrap();
    assert!(registry
        .register(model("local.a", ProviderKind::LlamaCpp, Locality::Local))
        .is_err());

    let mut updated = model("local.a", ProviderKind::LlamaCpp, Locality::Local);
    updated.availability = Availability::Offline;
    registry.upsert(updated).unwrap();
    assert_eq!(registry.len(), 1);
    assert_eq!(
        registry
            .get(&ModelId::parse("local.a").unwrap())
            .unwrap()
            .availability,
        Availability::Offline
    );
}

#[test]
fn a_zero_context_model_is_refused() {
    let mut registry = ModelRegistry::new();
    let mut broken = model("local.broken", ProviderKind::LlamaCpp, Locality::Local);
    broken.context_window = 0;
    assert!(registry.register(broken).is_err());
}

#[test]
fn registry_reports_whether_restricted_data_can_be_served() {
    let mut registry = ModelRegistry::new();
    let mut cloud = model("cloud.gpt", ProviderKind::OpenAi, Locality::Cloud);
    cloud.max_privacy_grade = PrivacyGrade::Internal;
    registry.register(cloud).unwrap();

    assert!(!registry.can_serve_privacy_grade(PrivacyGrade::Restricted));
    assert!(!registry.has_local_model());

    registry
        .register(model(
            "local.qwen3",
            ProviderKind::LlamaCpp,
            Locality::Local,
        ))
        .unwrap();
    assert!(registry.can_serve_privacy_grade(PrivacyGrade::Restricted));
    assert!(registry.has_local_model());
}

#[test]
fn offline_models_are_excluded_from_usable() {
    let mut registry = ModelRegistry::new();
    let mut offline = model("local.cold", ProviderKind::LlamaCpp, Locality::Local);
    offline.availability = Availability::Offline;
    registry.register(offline).unwrap();
    registry
        .register(model("local.warm", ProviderKind::LlamaCpp, Locality::Local))
        .unwrap();

    assert_eq!(registry.usable().len(), 1);
    assert_eq!(registry.usable()[0].id.as_str(), "local.warm");
}

// ------------------------------------------------- nested manifests & shipped modules

#[test]
fn manifests_load_from_nested_family_directories() {
    // The canonical tree groups modules by family on disk. Grouping is presentation
    // only, so the walk must descend and the ids must land in one flat namespace.
    let dir = std::env::temp_dir().join(format!("archie-nested-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("core")).unwrap();
    std::fs::create_dir_all(dir.join("cognition")).unwrap();

    std::fs::write(
        dir.join("core/archie.toml"),
        "id = \"core.archie\"\nversion = \"0.1.0\"\ndescription = \"core\"\n\n[worker]\nkind = \"none\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cognition/critic.toml"),
        "id = \"cognition.critic\"\nversion = \"0.1.0\"\ndescription = \"critic\"\n\n[worker]\nkind = \"none\"\n",
    )
    .unwrap();

    let registry = ModuleRegistry::load_from_dir(&dir).unwrap();
    assert_eq!(
        registry.len(),
        2,
        "the walk must descend into family directories"
    );
    assert!(registry
        .get(&ModuleId::parse("core.archie").unwrap())
        .is_some());
    assert!(registry
        .get(&ModuleId::parse("cognition.critic").unwrap())
        .is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_duplicate_id_in_a_different_family_is_still_refused() {
    // Identity is the `id`, not the path. Two families cannot both claim one module.
    let dir = std::env::temp_dir().join(format!("archie-dupfam-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let manifest =
        "id = \"core.archie\"\nversion = \"0.1.0\"\ndescription = \"d\"\n\n[worker]\nkind = \"none\"\n";
    std::fs::write(dir.join("a/m.toml"), manifest).unwrap();
    std::fs::write(dir.join("b/m.toml"), manifest).unwrap();

    assert!(ModuleRegistry::load_from_dir(&dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

/// The module tree this repository actually ships.
fn shipped_modules() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .join("modules")
}

#[test]
fn every_shipped_manifest_parses_and_validates() {
    // A broken manifest in the repository would present to an operator as a missing
    // feature rather than as the typo it is.
    let registry = ModuleRegistry::load_from_dir(shipped_modules())
        .expect("the shipped module tree must parse");
    assert!(
        registry.len() >= 10,
        "expected the canonical module set, found {}",
        registry.len()
    );
    for manifest in registry.manifests() {
        manifest
            .validate()
            .unwrap_or_else(|e| panic!("shipped manifest {} is invalid: {e}", manifest.id));
    }
}

#[test]
fn the_shipped_modules_form_an_acyclic_dependency_graph() {
    let registry = ModuleRegistry::load_from_dir(shipped_modules()).unwrap();
    let topology = registry.compile_topology(&context());
    assert!(
        topology.cycles.is_empty(),
        "shipped modules contain a dependency cycle: {:?}",
        topology.cycles
    );
}

#[test]
fn every_shipped_dependency_refers_to_a_shipped_module() {
    // A dependency on a module that does not exist is unsatisfiable forever, and shows
    // up only as a permanently-excluded module at boot.
    let registry = ModuleRegistry::load_from_dir(shipped_modules()).unwrap();
    let known: std::collections::BTreeSet<String> =
        registry.manifests().map(|m| m.id.to_string()).collect();
    for manifest in registry.manifests() {
        for dependency in &manifest.dependencies {
            assert!(
                known.contains(&dependency.to_string()),
                "{} depends on {}, which no shipped manifest provides",
                manifest.id,
                dependency
            );
        }
    }
}

#[test]
fn a_bare_cpu_node_loads_core_and_honestly_excludes_the_rest() {
    // The realistic first-boot case: a laptop with no GPU, no CAN bus, no CAD backend.
    // Archie Core must come up, and every module that cannot run must say why.
    let registry = ModuleRegistry::load_from_dir(shipped_modules()).unwrap();
    let ctx = TopologyContext::new(
        ["compute.cpu", "network.lan", "storage.object"]
            .into_iter()
            .map(|c| Capability::parse(c).unwrap())
            .collect(),
        "x86_64",
        "server",
    );
    let topology = registry.compile_topology(&ctx);

    assert!(
        topology.is_loaded(&ModuleId::parse("core.archie").unwrap()),
        "Archie Core must load on any node"
    );
    // No CAD backend is advertised, so the CAD module must be excluded with a reason
    // naming the capability rather than failing silently.
    let excluded: std::collections::BTreeMap<String, String> = topology
        .excluded()
        .into_iter()
        .map(|(id, why)| (id.to_string(), why))
        .collect();
    let cad = excluded
        .get("engineering.cad")
        .expect("engineering.cad cannot load without a CAD backend");
    assert!(
        cad.contains("cad.parametric") || cad.contains("core.nova"),
        "the exclusion must name what is missing, got: {cad}"
    );

    // Every excluded module carries a non-empty explanation.
    for (id, why) in &excluded {
        assert!(!why.is_empty(), "{id} was excluded with no explanation");
    }
}

#[test]
fn the_suit_module_is_refused_without_an_explicit_actuation_grant() {
    // devices.suit requests device-actuate. The default-safe policy must refuse it, so
    // that physical capability is never acquired by simply installing a module.
    let registry = ModuleRegistry::load_from_dir(shipped_modules()).unwrap();
    let ctx = TopologyContext::new(
        ["compute.cpu", "network.lan", "bus.can", "sensor.imu"]
            .into_iter()
            .map(|c| Capability::parse(c).unwrap())
            .collect(),
        "x86_64",
        "edge-gateway",
    );
    let topology = registry.compile_topology(&ctx);
    assert!(
        !topology.is_loaded(&ModuleId::parse("devices.suit").unwrap()),
        "a module requesting device-actuate must not load under the default policy"
    );
}

#[test]
fn no_shipped_module_names_an_interpreter_or_passes_for_runnable() {
    // ArchieOS ships no Python runtime. A module whose code does not exist says so, and
    // is never reported as loaded on a node that has everything else it needs.
    let registry = ModuleRegistry::load_from_dir(shipped_modules()).unwrap();
    let everything = TopologyContext::new(
        registry
            .manifests()
            .flat_map(|m| m.required_capabilities.iter().cloned())
            .collect(),
        "x86_64",
        "server",
    );
    let topology = registry.compile_topology(&everything);
    for manifest in registry.manifests() {
        if matches!(manifest.worker, WorkerImpl::Unimplemented { .. }) {
            assert!(
                !topology.is_loaded(&manifest.id),
                "{} has no implementation and must not load",
                manifest.id
            );
        }
    }
    // Given everything else, at least the modules with no dependencies left unmet say
    // plainly that they are unwritten.
    assert!(
        topology
            .excluded()
            .into_iter()
            .any(|(_, why)| why.contains("not implemented yet")),
        "an unwritten module is reported as unwritten"
    );
}
