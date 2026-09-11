//! Reading what disks the machine has.
//!
//! Running `lsblk` is an effect and cannot be tested here. Understanding what it said is
//! pure, and it is where this goes wrong: a partition table is a tree, the live medium
//! announces itself only through a mount point three levels down, and a parser that reads
//! the top level and stops will cheerfully offer somebody the USB stick they booted from.
//!
//! So the command is one function and the parsing is another, and the parsing is tested
//! against real `lsblk` output.

use crate::disk::BlockDevice;

/// Where archiso mounts the medium it booted from.
///
/// A device carrying any of these is the live medium. `bootmnt` is the ISO itself;
/// `cowspace` and the squashfs mounts are on it or backed by it, and a device holding one
/// of them is not a device to install onto.
const LIVE_MOUNT_PREFIX: &str = "/run/archiso";

/// The command whose output [`parse`] expects.
///
/// Bytes rather than human sizes (`-b`), JSON rather than columns (`-J`), and every field
/// a decision depends on. Kept next to the parser so the two cannot drift apart.
pub const LSBLK_ARGV: [&str; 5] = [
    "lsblk",
    "-J",
    "-b",
    "-o",
    "NAME,PATH,SIZE,RM,RO,MODEL,TYPE,MOUNTPOINTS",
];

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("lsblk did not return JSON: {0}")]
    NotJson(#[from] serde_json::Error),

    #[error("lsblk returned JSON without a blockdevices array")]
    NoBlockDevices,
}

/// Turn `lsblk -J` output into the disks a person could install onto.
///
/// Only whole disks are returned. Partitions, loop devices and ROM drives are not things
/// an install targets, and listing them invites somebody to pick one.
pub fn parse(lsblk_json: &str) -> Result<Vec<BlockDevice>, ProbeError> {
    let root: serde_json::Value = serde_json::from_str(lsblk_json)?;
    let devices = root
        .get("blockdevices")
        .and_then(|v| v.as_array())
        .ok_or(ProbeError::NoBlockDevices)?;

    let mut out = Vec::new();
    for node in devices {
        if node.get("type").and_then(|v| v.as_str()) != Some("disk") {
            continue;
        }
        let path = match node.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => continue,
        };

        let mut mounted_at = Vec::new();
        collect_mounts(node, &mut mounted_at);

        let carries_live_medium = mounted_at.iter().any(|m| m.starts_with(LIVE_MOUNT_PREFIX));

        // A live medium is reported as mounted at its archiso paths, which is true but
        // says nothing a person would act on. Keeping it out of `mounted_at` means the
        // refusal reads "this is the medium you booted from" rather than a list of mount
        // points nobody recognises — the same fact, in words that end the question.
        mounted_at.retain(|m| !m.starts_with(LIVE_MOUNT_PREFIX));

        out.push(BlockDevice {
            path,
            model: node
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown model".to_string()),
            size_bytes: node.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            removable: node.get("rm").and_then(|v| v.as_bool()).unwrap_or(false),
            read_only: node.get("ro").and_then(|v| v.as_bool()).unwrap_or(false),
            carries_live_medium,
            mounted_at,
        });
    }
    Ok(out)
}

/// Every mount point on this device or anything nested inside it.
///
/// Recursive because the tree is: a disk holds partitions, and a partition can hold an
/// LVM volume or a crypt device that is the thing actually mounted. A disk whose third
/// level is mounted is as much in use as one whose first level is.
fn collect_mounts(node: &serde_json::Value, into: &mut Vec<String>) {
    if let Some(points) = node.get("mountpoints").and_then(|v| v.as_array()) {
        for point in points {
            if let Some(path) = point.as_str() {
                into.push(path.to_string());
            }
        }
    }
    // Older lsblk reports a single `mountpoint` string instead of the array.
    if let Some(path) = node.get("mountpoint").and_then(|v| v.as_str()) {
        into.push(path.to_string());
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            collect_mounts(child, into);
        }
    }
}
