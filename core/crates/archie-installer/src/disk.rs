//! Which disk the install is allowed to destroy.
//!
//! This is the decision an installer gets wrong in the way that cannot be undone. Every
//! other mistake here — a bad hostname, the wrong profile, an unenrolled node — is a file
//! somebody edits afterwards. Choosing the wrong disk is somebody's photographs.
//!
//! So the choice is made from facts rather than from a list the person scrolls: a device
//! is a candidate or it is refused, and a refusal carries the reason. The facts come from
//! `lsblk` and `/proc/mounts`, which is an effect; deciding what they mean is pure, and
//! that is what lives here.
//!
//! # Why refusals are shown rather than filtered out
//!
//! The tempting design lists only the usable disks. It is worse. Somebody who cannot find
//! the disk they expected does not conclude the installer protected them — they conclude
//! it is broken, and they go looking for the flag that turns the protection off. A
//! refusal that says *this is the USB stick you are booting from* ends the search.

/// A whole disk as the running system currently sees it.
///
/// Deliberately not a `lsblk` row: the fields are the ones a decision depends on, so a
/// test can state a situation without pretending to be a kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    /// `/dev/sda`, `/dev/nvme0n1`.
    pub path: String,
    /// What the drive calls itself, for showing a person which physical object this is.
    pub model: String,
    pub size_bytes: u64,
    pub removable: bool,
    pub read_only: bool,
    /// This device, or one of its partitions, is the medium the live system booted from.
    pub carries_live_medium: bool,
    /// Where this device or its partitions are mounted right now.
    pub mounted_at: Vec<String>,
}

impl BlockDevice {
    /// Size in the units a person recognises off the box the drive came in.
    ///
    /// Decimal, not binary: a disk sold as 500 GB is 500 decimal GB, and showing it as
    /// "465.7 GiB" invites somebody to think they are looking at a different disk.
    pub fn human_size(&self) -> String {
        const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
        let mut value = self.size_bytes as f64;
        let mut unit = 0;
        while value >= 1000.0 && unit + 1 < UNITS.len() {
            value /= 1000.0;
            unit += 1;
        }
        if unit == 0 {
            format!("{} {}", self.size_bytes, UNITS[0])
        } else {
            format!("{value:.1} {}", UNITS[unit])
        }
    }
}

/// Why a device will not be offered as a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsuitable {
    /// The disk the live system is running from.
    LiveMedium,
    /// The kernel will not let us write to it.
    ReadOnly,
    /// Something is mounted from it, so it is in use by the running system.
    InUse { mounted_at: Vec<String> },
    /// ArchieOS will not fit.
    TooSmall { needs_bytes: u64, has_bytes: u64 },
}

impl std::fmt::Display for Unsuitable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LiveMedium => f.write_str(
                "this is the medium you booted from; installing onto it would erase the \
                 installer while it runs",
            ),
            Self::ReadOnly => f.write_str("the kernel reports this device as read-only"),
            Self::InUse { mounted_at } => write!(
                f,
                "in use by the running system (mounted at {})",
                mounted_at.join(", ")
            ),
            Self::TooSmall {
                needs_bytes,
                has_bytes,
            } => write!(
                f,
                "too small: ArchieOS needs about {} GB and this disk has {} GB",
                needs_bytes / 1_000_000_000,
                has_bytes / 1_000_000_000
            ),
        }
    }
}

/// The smallest disk an install will fit on, with room to be usable afterwards.
pub const MINIMUM_TARGET_BYTES: u64 = 16_000_000_000;

/// Whether this device may be installed onto, and if not, why not.
///
/// The order is the order of severity, because only the first reason is shown and the
/// person should hear the most alarming true thing about the disk. A read-only USB stick
/// that is also the live medium is, to the person holding it, the live medium.
pub fn assess(device: &BlockDevice, minimum_bytes: u64) -> Result<(), Unsuitable> {
    if device.carries_live_medium {
        return Err(Unsuitable::LiveMedium);
    }
    if device.read_only {
        return Err(Unsuitable::ReadOnly);
    }
    if !device.mounted_at.is_empty() {
        return Err(Unsuitable::InUse {
            mounted_at: device.mounted_at.clone(),
        });
    }
    if device.size_bytes < minimum_bytes {
        return Err(Unsuitable::TooSmall {
            needs_bytes: minimum_bytes,
            has_bytes: device.size_bytes,
        });
    }
    Ok(())
}

/// Every disk the machine has, sorted into what may be installed onto and what may not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Survey {
    pub usable: Vec<BlockDevice>,
    pub refused: Vec<(BlockDevice, Unsuitable)>,
}

impl Survey {
    /// True when there is nothing to install onto, which is a state the wizard has to
    /// handle rather than presenting an empty menu.
    pub fn is_empty(&self) -> bool {
        self.usable.is_empty()
    }
}

/// Sort the machine's disks. Refused devices are kept, with their reason.
pub fn survey(devices: &[BlockDevice], minimum_bytes: u64) -> Survey {
    let mut out = Survey::default();
    for device in devices {
        match assess(device, minimum_bytes) {
            Ok(()) => out.usable.push(device.clone()),
            Err(reason) => out.refused.push((device.clone(), reason)),
        }
    }
    out
}

/// The path of a partition on a disk.
///
/// `/dev/sda` + 1 is `/dev/sda1`, but `/dev/nvme0n1` + 1 is `/dev/nvme0n1p1`, and an
/// installer that assumes the first form formats nothing on an NVMe machine — or, worse,
/// writes a filesystem to a path that happens to exist. The rule the kernel actually
/// follows is that a `p` is inserted when the device name already ends in a digit,
/// because `nvme0n11` would otherwise be ambiguous.
pub fn partition_path(disk: &str, index: u32) -> String {
    let ends_in_digit = disk.chars().last().is_some_and(|c| c.is_ascii_digit());
    if ends_in_digit {
        format!("{disk}p{index}")
    } else {
        format!("{disk}{index}")
    }
}
