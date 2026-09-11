//! Hardware probing.
//!
//! Builds the [`NodeDescriptor`] that the fabric announces. Everything here is
//! best-effort by design: an unknown value becomes `None` or is omitted, never a
//! fabricated default. A probe that guesses "8 GB of VRAM" when it cannot read the
//! device is worse than one that admits it does not know, because the scheduler will
//! believe it.
//!
//! # Accelerator detection
//!
//! Two independent paths, tried in order:
//!
//! 1. `nvidia-smi` — authoritative for NVIDIA, including memory and driver version.
//! 2. PCI class scan under `/sys/bus/pci/devices` — vendor-agnostic, but yields no
//!    memory size.
//!
//! The second is a fallback rather than a supplement; results are de-duplicated so a
//! card is not counted twice.

use archie_types::{
    Accelerator, AcceleratorKind, Capability, CapabilitySet, CpuInfo, LinkClass, MemoryInfo,
    NetworkInterface, NodeDescriptor, NodeHealth, NodeId, NodeLoad, NodeProfile, StorageVolume,
    Timestamp, ARCHIEOS_VERSION,
};
use std::path::Path;
use std::process::Command;
use sysinfo::{Disks, Networks, System};

/// A complete hardware picture of this machine.
#[derive(Debug, Clone)]
pub struct HardwareProbe {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub accelerators: Vec<Accelerator>,
    pub storage: Vec<StorageVolume>,
    pub networks: Vec<NetworkInterface>,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub architecture: String,
    /// Anything the probe could not determine, for the boot report.
    pub warnings: Vec<String>,
}

impl HardwareProbe {
    /// Inspect this machine.
    pub fn run() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        let mut warnings = Vec::new();

        let cpu = probe_cpu(&system, &mut warnings);
        let memory = MemoryInfo {
            total_bytes: system.total_memory(),
            available_bytes: system.available_memory(),
            swap_total_bytes: system.total_swap(),
        };
        if memory.total_bytes == 0 {
            warnings.push("could not determine total system memory".into());
        }

        let accelerators = probe_accelerators(&mut warnings);
        let storage = probe_storage();
        let networks = probe_networks();

        let hostname = System::host_name().unwrap_or_else(|| {
            warnings.push("hostname unavailable; using 'unknown-host'".into());
            "unknown-host".to_string()
        });

        Self {
            cpu,
            memory,
            accelerators,
            storage,
            networks,
            hostname,
            os: System::name().unwrap_or_else(|| "unknown".into()),
            kernel: System::kernel_version().unwrap_or_else(|| "unknown".into()),
            architecture: System::cpu_arch(),
            warnings,
        }
    }

    /// Derive the capability set this hardware implies.
    ///
    /// Capabilities are *derived* rather than configured so the manifest cannot drift
    /// from the machine. Operators add extra capabilities through configuration; they
    /// do not hand-write the hardware ones.
    pub fn derive_capabilities(&self) -> CapabilitySet {
        let mut capabilities = CapabilitySet::new();

        // Every node can compute something.
        capabilities.insert(Capability::known("compute.cpu"));

        for accelerator in &self.accelerators {
            if let Some(name) = accelerator.kind.capability_name() {
                if let Ok(capability) = Capability::parse(name) {
                    capabilities.insert(capability);
                }
            }
        }
        // Any accelerator makes this node a plausible inference host.
        if !self.accelerators.is_empty() {
            capabilities.insert(Capability::known("compute.inference"));
        }

        if !self.storage.is_empty() {
            capabilities.insert(Capability::known("storage.object"));
        }
        if !self.networks.is_empty() {
            capabilities.insert(Capability::known("network.lan"));
        }
        capabilities
    }

    /// Assemble the descriptor this node will announce.
    pub fn to_descriptor(
        &self,
        node_id: NodeId,
        public_key: String,
        profile: NodeProfile,
        extra_capabilities: impl IntoIterator<Item = Capability>,
    ) -> NodeDescriptor {
        let mut capabilities = self.derive_capabilities();
        for capability in extra_capabilities {
            capabilities.insert(capability);
        }

        NodeDescriptor {
            node_id,
            public_key,
            hostname: self.hostname.clone(),
            profile,
            archieos_version: ARCHIEOS_VERSION.to_string(),
            os: self.os.clone(),
            kernel: self.kernel.clone(),
            architecture: self.architecture.clone(),
            cpu: self.cpu.clone(),
            memory: self.memory,
            accelerators: self.accelerators.clone(),
            storage: self.storage.clone(),
            networks: self.networks.clone(),
            capabilities,
            loaded_models: vec![],
            modules: vec![],
            health: NodeHealth::Ok,
            load: NodeLoad {
                cpu_utilization: 0.0,
                memory_utilization: self.memory.utilization(),
                running_jobs: 0,
                queued_jobs: 0,
            },
            announced_at: Timestamp::now(),
        }
    }
}

fn probe_cpu(system: &System, warnings: &mut Vec<String>) -> CpuInfo {
    let cpus = system.cpus();
    let logical_cores = cpus.len().max(1) as u32;
    let physical_cores = system
        .physical_core_count()
        .map(|n| n as u32)
        .unwrap_or_else(|| {
            warnings.push("physical core count unavailable; assuming logical".into());
            logical_cores
        })
        .min(logical_cores);

    CpuInfo {
        model: cpus
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
        architecture: System::cpu_arch(),
        physical_cores: physical_cores.max(1),
        logical_cores,
        base_frequency_mhz: cpus.first().map(|c| c.frequency()).unwrap_or(0),
    }
}

fn probe_storage() -> Vec<StorageVolume> {
    let disks = Disks::new_with_refreshed_list();
    let mut volumes: Vec<StorageVolume> = disks
        .list()
        .iter()
        .map(|disk| StorageVolume {
            mount_point: disk.mount_point().display().to_string(),
            filesystem: disk.file_system().to_string_lossy().to_string(),
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
            rotational: None,
        })
        .collect();
    // Stable order so two boots of the same machine produce comparable manifests.
    volumes.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    volumes.dedup_by(|a, b| a.mount_point == b.mount_point);
    volumes
}

fn probe_networks() -> Vec<NetworkInterface> {
    let networks = Networks::new_with_refreshed_list();
    let mut interfaces: Vec<NetworkInterface> = networks
        .list()
        .iter()
        .map(|(name, data)| {
            let addresses: Vec<String> = data
                .ip_networks()
                .iter()
                .map(|ip| ip.addr.to_string())
                .collect();
            let mac = data.mac_address().to_string();
            NetworkInterface {
                name: name.clone(),
                addresses,
                mac: if mac == "00:00:00:00:00:00" {
                    None
                } else {
                    Some(mac)
                },
                speed_mbps: None,
                link_class: classify_link(name),
            }
        })
        .collect();
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    interfaces
}

/// Classify a link from its interface name.
///
/// A coarse heuristic, and deliberately pessimistic: an interface we cannot classify is
/// assumed to be ordinary LAN rather than fast, because over-estimating a link is what
/// leads to splitting a model across something that cannot carry it.
fn classify_link(name: &str) -> LinkClass {
    let lower = name.to_ascii_lowercase();

    // Constrained links are matched FIRST. `lora0` also starts with "lo", and
    // classifying a LoRa radio as loopback would tell the scheduler it may split a
    // model across it — the exact mistake `LinkClass` exists to prevent.
    if lower.starts_with("lora") || lower.starts_with("can") {
        return LinkClass::Constrained;
    }
    // Loopback is `lo`, optionally with an index (`lo0`) — never an arbitrary `lo*`.
    if lower == "lo"
        || lower
            .strip_prefix("lo")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    {
        return LinkClass::Loopback;
    }
    if lower.starts_with("wg") || lower.starts_with("tun") || lower.starts_with("tap") {
        return LinkClass::Overlay;
    }
    LinkClass::Lan
}

fn probe_accelerators(warnings: &mut Vec<String>) -> Vec<Accelerator> {
    let mut accelerators = probe_nvidia_smi();
    if accelerators.is_empty() {
        accelerators = probe_pci_class();
        if !accelerators.is_empty() {
            warnings.push(
                "accelerators detected by PCI scan only; device memory is unknown and \
                 memory-sensitive placement will skip this node"
                    .into(),
            );
        }
    }
    accelerators
}

/// Query `nvidia-smi`. Absent tool or non-zero exit simply means "no NVIDIA here".
fn probe_nvidia_smi() -> Vec<Accelerator> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .filter_map(|(index, line)| {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.is_empty() {
                return None;
            }
            Some(Accelerator {
                kind: AcceleratorKind::Cuda,
                vendor: "NVIDIA".to_string(),
                model: fields[0].to_string(),
                // Reported in MiB. A field we cannot parse becomes None rather than 0,
                // so "unknown" stays distinguishable from "none".
                memory_bytes: fields
                    .get(1)
                    .and_then(|m| m.parse::<u64>().ok())
                    .map(|mib| mib * 1024 * 1024),
                driver_version: fields.get(2).map(|d| d.to_string()),
                index: index as u32,
            })
        })
        .collect()
}

/// Scan `/sys/bus/pci/devices` for display/compute controllers.
///
/// Yields no memory size, which is why it is a fallback.
fn probe_pci_class() -> Vec<Accelerator> {
    let root = Path::new("/sys/bus/pci/devices");
    if !root.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices: Vec<(String, Accelerator)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let class = read_sysfs_hex(&path.join("class"));
        // 0x0300xx = VGA controller, 0x0302xx = 3D controller, 0x1200xx = processing
        // accelerator.
        let Some(class) = class else { continue };
        let base = class >> 16;
        if base != 0x03 && base != 0x12 {
            continue;
        }

        let Some(vendor) = read_sysfs_hex(&path.join("vendor")) else {
            continue;
        };
        let (kind, vendor_name) = match vendor {
            0x10de => (AcceleratorKind::Cuda, "NVIDIA"),
            0x1002 => (AcceleratorKind::Rocm, "AMD"),
            0x1e52 => (AcceleratorKind::Tenstorrent, "Tenstorrent"),
            _ => (
                AcceleratorKind::Other(format!("pci:{vendor:04x}")),
                "unknown",
            ),
        };

        let slot = entry.file_name().to_string_lossy().to_string();
        devices.push((
            slot,
            Accelerator {
                kind,
                vendor: vendor_name.to_string(),
                model: read_sysfs_string(&path.join("device"))
                    .unwrap_or_else(|| "unknown".to_string()),
                memory_bytes: None,
                driver_version: None,
                index: 0,
            },
        ));
    }

    // Order by PCI slot so indices are stable across boots.
    devices.sort_by(|a, b| a.0.cmp(&b.0));
    devices
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut accelerator))| {
            accelerator.index = index as u32;
            accelerator
        })
        .collect()
}

fn read_sysfs_hex(path: &Path) -> Option<u32> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    u32::from_str_radix(stripped, 16).ok()
}

fn read_sysfs_string(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probing_this_machine_produces_a_usable_picture() {
        let probe = HardwareProbe::run();
        assert!(
            probe.cpu.logical_cores >= 1,
            "a machine has at least one core"
        );
        assert!(probe.memory.total_bytes > 0, "a machine has memory");
        assert!(!probe.hostname.is_empty());
        assert!(!probe.architecture.is_empty());
    }

    #[test]
    fn physical_cores_never_exceed_logical_cores() {
        let probe = HardwareProbe::run();
        assert!(probe.cpu.physical_cores <= probe.cpu.logical_cores);
        assert!(probe.cpu.physical_cores >= 1);
    }

    #[test]
    fn every_probed_machine_advertises_cpu_compute() {
        let capabilities = HardwareProbe::run().derive_capabilities();
        assert!(capabilities.provides(&Capability::known("compute.cpu")));
    }

    #[test]
    fn derived_capabilities_match_detected_accelerators() {
        // The descriptor self-consistency check rejects a manifest whose accelerators
        // imply a capability it does not advertise, so these must agree by construction.
        let probe = HardwareProbe::run();
        let capabilities = probe.derive_capabilities();
        for accelerator in &probe.accelerators {
            if let Some(name) = accelerator.kind.capability_name() {
                assert!(
                    capabilities.provides(&Capability::parse(name).unwrap()),
                    "accelerator {} implies {name}, which was not derived",
                    accelerator.model
                );
            }
        }
    }

    #[test]
    fn the_descriptor_built_from_a_probe_is_self_consistent() {
        let key = [7u8; 32];
        let node_id = NodeId::from_public_key_bytes(&key);
        let descriptor =
            HardwareProbe::run().to_descriptor(node_id, hex::encode(key), NodeProfile::DevVm, []);
        assert_eq!(
            descriptor.self_consistency_problems(),
            Vec::<String>::new(),
            "a descriptor from a real probe must pass the checks peers apply to it"
        );
    }

    #[test]
    fn extra_capabilities_are_merged_in() {
        let key = [7u8; 32];
        let descriptor = HardwareProbe::run().to_descriptor(
            NodeId::from_public_key_bytes(&key),
            hex::encode(key),
            NodeProfile::Server,
            [Capability::known("simulation.fea")],
        );
        assert!(descriptor
            .capabilities
            .provides(&Capability::known("simulation.fea")));
        assert!(descriptor
            .capabilities
            .provides(&Capability::known("compute.cpu")));
    }

    #[test]
    fn storage_volumes_are_ordered_and_unique() {
        let volumes = probe_storage();
        let mounts: Vec<&String> = volumes.iter().map(|v| &v.mount_point).collect();
        let mut sorted = mounts.clone();
        sorted.sort();
        assert_eq!(mounts, sorted, "volume order must be stable across boots");

        let unique: std::collections::HashSet<_> = mounts.iter().collect();
        assert_eq!(unique.len(), mounts.len(), "a mount point must appear once");
    }

    #[test]
    fn reported_storage_is_internally_consistent() {
        for volume in probe_storage() {
            assert!(
                volume.available_bytes <= volume.total_bytes,
                "{} reports more free space than it has",
                volume.mount_point
            );
        }
    }

    #[test]
    fn link_classification_is_pessimistic_for_unknown_interfaces() {
        assert_eq!(classify_link("lo"), LinkClass::Loopback);
        assert_eq!(classify_link("wg0"), LinkClass::Overlay);
        assert_eq!(classify_link("tun0"), LinkClass::Overlay);
        assert_eq!(classify_link("lo0"), LinkClass::Loopback);
        // Regression: `lora0` shares the "lo" prefix with loopback. Misclassifying it
        // would advertise a low-rate radio as suitable for tensor-parallel placement.
        assert_eq!(classify_link("lora0"), LinkClass::Constrained);
        assert_eq!(classify_link("can0"), LinkClass::Constrained);
        assert!(!classify_link("lora0").permits_tensor_parallel());
        // An unrecognised NIC must not be assumed fast.
        assert_eq!(classify_link("enp5s0"), LinkClass::Lan);
        assert!(!classify_link("enp5s0").permits_tensor_parallel());
    }

    #[test]
    fn an_absent_nvidia_smi_yields_no_accelerators_rather_than_an_error() {
        // On a machine with no NVIDIA tooling this returns empty; on one with a GPU it
        // returns real devices. Both are valid — the point is that it never panics.
        let detected = probe_nvidia_smi();
        for accelerator in &detected {
            assert_eq!(accelerator.kind, AcceleratorKind::Cuda);
            assert!(!accelerator.model.is_empty());
        }
    }

    #[test]
    fn pci_scan_is_a_no_op_off_linux() {
        // Guards against the scan panicking when /sys does not exist.
        let detected = probe_pci_class();
        if !Path::new("/sys/bus/pci/devices").is_dir() {
            assert!(detected.is_empty());
        }
    }

    #[test]
    fn unknown_accelerator_memory_stays_unknown() {
        // Never fabricate a size: the scheduler treats unknown as "do not place
        // memory-sensitive work here", and a fabricated 0 or default would defeat that.
        for accelerator in probe_pci_class() {
            assert!(
                accelerator.memory_bytes.is_none(),
                "the PCI fallback cannot know memory size and must not invent one"
            );
        }
    }

    #[test]
    fn sysfs_hex_parsing_handles_the_kernel_format() {
        let dir = std::env::temp_dir().join(format!("archie-sysfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vendor");
        std::fs::write(&path, "0x10de\n").unwrap();
        assert_eq!(read_sysfs_hex(&path), Some(0x10de));

        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(read_sysfs_hex(&path), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
