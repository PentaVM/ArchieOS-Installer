//! Node descriptors: the self-asserted view a node publishes about itself.
//!
//! # Trust boundary
//!
//! Everything in [`NodeDescriptor`] is *claimed by the node itself*. A descriptor is
//! signed, so a peer knows **who** said it, but not that it is **true** — a node can
//! lie about having 8 GPUs. Trust is therefore deliberately NOT a field here; it is
//! held by the receiver in its own peer table (see `archie-registry`). A node must
//! never be able to assert its own trust level.

use crate::capability::CapabilitySet;
use crate::ids::{ModelId, ModuleId, NodeId};
use crate::time::Timestamp;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The role a node was installed to play. Chosen by the operator at install time, not
/// inferred from hardware: a workstation with a big GPU is still a workstation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeProfile {
    /// Headless orchestrator. Runs Archie Core, owns the task DAG.
    Server,
    /// Interactive machine with a display; may also contribute compute.
    Workstation,
    /// Dedicated inference/accelerator host.
    BrainNode,
    /// Small always-on node bridging to devices, radios and buses.
    EdgeGateway,
    /// Disposable development instance. Never trusted to hold canonical state.
    DevVm,
}

impl NodeProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Workstation => "workstation",
            Self::BrainNode => "brain-node",
            Self::EdgeGateway => "edge-gateway",
            Self::DevVm => "dev-vm",
        }
    }

    /// Whether this profile is permitted to hold canonical knowledge and task state.
    ///
    /// A dev VM is explicitly excluded so that throwaway instances cannot become the
    /// only copy of something that matters.
    pub fn may_hold_canonical_state(&self) -> bool {
        !matches!(self, Self::DevVm)
    }
}

impl fmt::Display for NodeProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub model: String,
    pub architecture: String,
    /// Physical cores where detectable, otherwise equal to `logical_cores`.
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub base_frequency_mhz: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
}

impl MemoryInfo {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    /// Fraction of RAM in use, 0.0..=1.0. Returns 0.0 rather than NaN on a zero total.
    pub fn utilization(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes() as f32 / self.total_bytes as f32
    }
}

/// The class of an accelerator. Open by design; `Other` carries the raw string so an
/// unrecognised device is still schedulable by exact name rather than being dropped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AcceleratorKind {
    Cuda,
    Rocm,
    Tenstorrent,
    Npu,
    Other(String),
}

impl AcceleratorKind {
    /// The capability this accelerator implies, if any.
    pub fn capability_name(&self) -> Option<&str> {
        match self {
            Self::Cuda => Some("compute.cuda"),
            Self::Rocm => Some("compute.rocm"),
            Self::Tenstorrent => Some("compute.tenstorrent"),
            Self::Npu => Some("compute.npu"),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accelerator {
    pub kind: AcceleratorKind,
    pub vendor: String,
    pub model: String,
    /// Dedicated device memory. `None` when it could not be determined; callers must
    /// treat unknown as "do not place a memory-sensitive workload here" rather than
    /// assuming zero or infinity.
    pub memory_bytes: Option<u64>,
    pub driver_version: Option<String>,
    /// Index within this host, e.g. CUDA ordinal.
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageVolume {
    pub mount_point: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    /// True when the volume is on rotational media, where known.
    pub rotational: Option<bool>,
}

/// How good a link is. The scheduler must never assume all nodes are equally close;
/// tensor-parallel placement across a slow link is a correctness-of-design problem,
/// not just a performance one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkClass {
    /// Same machine.
    Loopback,
    /// >= 10 GbE, or better. Splitting a model across this is plausible.
    FastLan,
    /// 1 GbE / Wi-Fi. Move whole workers, not tensors.
    Lan,
    /// VPN or overlay. Latency is unpredictable.
    Overlay,
    /// Low-rate radio. Telemetry only, never a control path for compute.
    Constrained,
}

impl LinkClass {
    /// Whether it is ever acceptable to split one model's layers across this link.
    pub fn permits_tensor_parallel(&self) -> bool {
        matches!(self, Self::Loopback | Self::FastLan)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub addresses: Vec<String>,
    pub mac: Option<String>,
    /// Negotiated link speed in Mbps where the OS reports it.
    pub speed_mbps: Option<u64>,
    pub link_class: LinkClass,
}

/// Coarse health. Deliberately small: a node is either usable, usable-but-impaired, or
/// not usable. Fine-grained diagnostics belong in observability, not in the field the
/// scheduler branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeHealth {
    Ok,
    Degraded,
    Unhealthy,
    /// No recent contact. Distinct from `Unhealthy`: we do not know that it is broken.
    Unreachable,
}

impl NodeHealth {
    pub fn schedulable(&self) -> bool {
        matches!(self, Self::Ok | Self::Degraded)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Unhealthy => "unhealthy",
            Self::Unreachable => "unreachable",
        }
    }
}

impl fmt::Display for NodeHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Instantaneous load. Sampled, not authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NodeLoad {
    pub cpu_utilization: f32,
    pub memory_utilization: f32,
    pub running_jobs: u32,
    pub queued_jobs: u32,
}

impl Default for NodeLoad {
    fn default() -> Self {
        Self {
            cpu_utilization: 0.0,
            memory_utilization: 0.0,
            running_jobs: 0,
            queued_jobs: 0,
        }
    }
}

/// The complete self-description a node publishes to the fabric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDescriptor {
    pub node_id: NodeId,
    /// Hex-encoded Ed25519 public key. `node_id` must equal its derivation.
    pub public_key: String,
    pub hostname: String,
    pub profile: NodeProfile,
    pub archieos_version: String,
    pub os: String,
    pub kernel: String,
    pub architecture: String,

    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub accelerators: Vec<Accelerator>,
    pub storage: Vec<StorageVolume>,
    pub networks: Vec<NetworkInterface>,

    pub capabilities: CapabilitySet,
    pub loaded_models: Vec<ModelId>,
    pub modules: Vec<ModuleId>,

    pub health: NodeHealth,
    pub load: NodeLoad,
    /// When the node generated this descriptor, by its own clock.
    pub announced_at: Timestamp,
}

impl NodeDescriptor {
    /// Total dedicated accelerator memory, counting only devices that reported it.
    ///
    /// Devices with unknown memory are excluded rather than assumed zero; the caller
    /// can detect the difference with [`Self::has_accelerator_with_unknown_memory`].
    pub fn total_accelerator_memory_bytes(&self) -> u64 {
        self.accelerators
            .iter()
            .filter_map(|a| a.memory_bytes)
            .sum()
    }

    pub fn has_accelerator_with_unknown_memory(&self) -> bool {
        self.accelerators.iter().any(|a| a.memory_bytes.is_none())
    }

    /// Best link class this node has, used as a coarse reachability hint.
    pub fn best_link_class(&self) -> LinkClass {
        self.networks
            .iter()
            .map(|n| n.link_class)
            .min()
            .unwrap_or(LinkClass::Constrained)
    }

    /// Check the internal consistency a receiver can verify without external data.
    ///
    /// This is the *cheap* half of validation; signature verification lives in
    /// `archie-fabric`. Returns the list of problems rather than a bool so the fabric
    /// can log precisely why a peer was rejected.
    pub fn self_consistency_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();

        match hex::decode(&self.public_key) {
            Ok(bytes) => match <[u8; 32]>::try_from(bytes) {
                Ok(key) => {
                    if !self.node_id.matches_public_key(&key) {
                        problems.push(
                            "node_id is not the derivation of the advertised public_key".into(),
                        );
                    }
                }
                Err(_) => problems.push("public_key must be 32 bytes".into()),
            },
            Err(_) => problems.push("public_key must be hex".into()),
        }

        if self.hostname.trim().is_empty() {
            problems.push("hostname must not be empty".into());
        }
        if self.archieos_version.trim().is_empty() {
            problems.push("archieos_version must not be empty".into());
        }
        if self.cpu.logical_cores == 0 {
            problems.push("cpu.logical_cores must be at least 1".into());
        }
        if self.cpu.physical_cores > self.cpu.logical_cores {
            problems.push("cpu.physical_cores cannot exceed logical_cores".into());
        }
        if self.memory.total_bytes == 0 {
            problems.push("memory.total_bytes must be non-zero".into());
        }
        if self.memory.available_bytes > self.memory.total_bytes {
            problems.push("memory.available_bytes cannot exceed total_bytes".into());
        }
        for volume in &self.storage {
            if volume.available_bytes > volume.total_bytes {
                problems.push(format!(
                    "storage {}: available_bytes exceeds total_bytes",
                    volume.mount_point
                ));
            }
        }
        // An accelerator that claims a capability the manifest omits means the probe and
        // the manifest disagree, which would make scheduling decisions unsound.
        for accelerator in &self.accelerators {
            if let Some(name) = accelerator.kind.capability_name() {
                if let Ok(cap) = crate::capability::Capability::parse(name) {
                    if !self.capabilities.provides(&cap) {
                        problems.push(format!(
                            "accelerator {} implies capability {name} which is not advertised",
                            accelerator.model
                        ));
                    }
                }
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;

    fn descriptor() -> NodeDescriptor {
        let key = [4u8; 32];
        NodeDescriptor {
            node_id: NodeId::from_public_key_bytes(&key),
            public_key: hex::encode(key),
            hostname: "test-node".into(),
            profile: NodeProfile::DevVm,
            archieos_version: "0.1.0".into(),
            os: "Arch Linux".into(),
            kernel: "6.10.0".into(),
            architecture: "x86_64".into(),
            cpu: CpuInfo {
                model: "Test CPU".into(),
                architecture: "x86_64".into(),
                physical_cores: 4,
                logical_cores: 8,
                base_frequency_mhz: 3000,
            },
            memory: MemoryInfo {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 8 * 1024 * 1024 * 1024,
                swap_total_bytes: 0,
            },
            accelerators: vec![],
            storage: vec![],
            networks: vec![],
            capabilities: [Capability::known("compute.cpu")].into_iter().collect(),
            loaded_models: vec![],
            modules: vec![],
            health: NodeHealth::Ok,
            load: NodeLoad::default(),
            announced_at: Timestamp::now(),
        }
    }

    #[test]
    fn a_well_formed_descriptor_has_no_problems() {
        assert!(descriptor().self_consistency_problems().is_empty());
    }

    #[test]
    fn detects_node_id_not_matching_key() {
        let mut d = descriptor();
        d.node_id = NodeId::from_public_key_bytes(&[99u8; 32]);
        let problems = d.self_consistency_problems();
        assert!(problems.iter().any(|p| p.contains("not the derivation")));
    }

    #[test]
    fn detects_impossible_memory() {
        let mut d = descriptor();
        d.memory.available_bytes = d.memory.total_bytes + 1;
        assert!(d
            .self_consistency_problems()
            .iter()
            .any(|p| p.contains("available_bytes cannot exceed")));
    }

    #[test]
    fn detects_accelerator_capability_mismatch() {
        let mut d = descriptor();
        d.accelerators.push(Accelerator {
            kind: AcceleratorKind::Cuda,
            vendor: "NVIDIA".into(),
            model: "RTX 3070".into(),
            memory_bytes: Some(8 * 1024 * 1024 * 1024),
            driver_version: None,
            index: 0,
        });
        // capabilities still only list compute.cpu
        assert!(d
            .self_consistency_problems()
            .iter()
            .any(|p| p.contains("compute.cuda")));
    }

    #[test]
    fn accelerator_memory_excludes_unknown() {
        let mut d = descriptor();
        d.capabilities.insert(Capability::known("compute.cuda"));
        d.accelerators.push(Accelerator {
            kind: AcceleratorKind::Cuda,
            vendor: "NVIDIA".into(),
            model: "RTX 3070".into(),
            memory_bytes: Some(1000),
            driver_version: None,
            index: 0,
        });
        d.accelerators.push(Accelerator {
            kind: AcceleratorKind::Cuda,
            vendor: "NVIDIA".into(),
            model: "Unknown".into(),
            memory_bytes: None,
            driver_version: None,
            index: 1,
        });
        assert_eq!(d.total_accelerator_memory_bytes(), 1000);
        assert!(d.has_accelerator_with_unknown_memory());
    }

    #[test]
    fn dev_vm_may_not_hold_canonical_state() {
        assert!(!NodeProfile::DevVm.may_hold_canonical_state());
        assert!(NodeProfile::Server.may_hold_canonical_state());
    }

    #[test]
    fn only_fast_links_permit_tensor_parallel() {
        assert!(LinkClass::Loopback.permits_tensor_parallel());
        assert!(LinkClass::FastLan.permits_tensor_parallel());
        assert!(!LinkClass::Lan.permits_tensor_parallel());
        assert!(!LinkClass::Overlay.permits_tensor_parallel());
        assert!(!LinkClass::Constrained.permits_tensor_parallel());
    }

    #[test]
    fn unreachable_is_not_schedulable_but_degraded_is() {
        assert!(NodeHealth::Ok.schedulable());
        assert!(NodeHealth::Degraded.schedulable());
        assert!(!NodeHealth::Unhealthy.schedulable());
        assert!(!NodeHealth::Unreachable.schedulable());
    }

    #[test]
    fn descriptor_round_trips_through_json() {
        let d = descriptor();
        let json = serde_json::to_string(&d).unwrap();
        let back: NodeDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}
