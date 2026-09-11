//! Which disk may be erased.
//!
//! The one decision in an installer that cannot be taken back.

use archie_installer::{assess, partition_path, survey, BlockDevice, Unsuitable};

fn disk(path: &str, size_bytes: u64) -> BlockDevice {
    BlockDevice {
        path: path.into(),
        model: "Test Disk".into(),
        size_bytes,
        removable: false,
        read_only: false,
        carries_live_medium: false,
        mounted_at: Vec::new(),
    }
}

const BIG: u64 = 500_000_000_000;
const MIN: u64 = 16_000_000_000;

/// The failure this whole module exists to prevent.
#[test]
fn the_medium_being_booted_from_is_never_a_target() {
    let mut stick = disk("/dev/sdb", 32_000_000_000);
    stick.carries_live_medium = true;
    stick.removable = true;

    assert_eq!(assess(&stick, MIN), Err(Unsuitable::LiveMedium));

    let survey = survey(&[disk("/dev/sda", BIG), stick], MIN);
    assert_eq!(survey.usable.len(), 1);
    assert_eq!(survey.usable[0].path, "/dev/sda");
}

/// A refusal is shown, not silently dropped.
///
/// Somebody who cannot find the disk they expected concludes the installer is broken and
/// goes looking for the flag that turns the protection off.
#[test]
fn a_refused_disk_is_kept_with_its_reason() {
    let mut stick = disk("/dev/sdb", 32_000_000_000);
    stick.carries_live_medium = true;

    let survey = survey(&[stick], MIN);
    assert!(survey.is_empty());
    assert_eq!(survey.refused.len(), 1);
    assert_eq!(survey.refused[0].1, Unsuitable::LiveMedium);
    assert!(
        survey.refused[0].1.to_string().contains("booted from"),
        "the reason has to be the one a person can act on: {}",
        survey.refused[0].1
    );
}

#[test]
fn a_disk_in_use_by_the_running_system_is_refused() {
    let mut mounted = disk("/dev/sdc", BIG);
    mounted.mounted_at = vec!["/home".into()];

    assert_eq!(
        assess(&mounted, MIN),
        Err(Unsuitable::InUse {
            mounted_at: vec!["/home".into()]
        })
    );
    assert!(assess(&mounted, MIN)
        .unwrap_err()
        .to_string()
        .contains("/home"));
}

#[test]
fn a_read_only_disk_is_refused() {
    let mut ro = disk("/dev/sr0", BIG);
    ro.read_only = true;
    assert_eq!(assess(&ro, MIN), Err(Unsuitable::ReadOnly));
}

#[test]
fn a_disk_too_small_to_hold_the_system_is_refused_with_both_numbers() {
    let small = disk("/dev/sdd", 4_000_000_000);
    let reason = assess(&small, MIN).unwrap_err();
    assert_eq!(
        reason,
        Unsuitable::TooSmall {
            needs_bytes: MIN,
            has_bytes: 4_000_000_000
        }
    );
    let text = reason.to_string();
    assert!(text.contains("16") && text.contains('4'), "{text}");
}

/// A disk can be several kinds of unsuitable at once, and only one reason is shown.
///
/// It has to be the most alarming true one. To somebody holding a read-only USB stick
/// that is also the live medium, the fact that matters is that it is the live medium.
#[test]
fn the_most_alarming_true_reason_is_the_one_reported() {
    let mut everything = disk("/dev/sdb", 1_000_000_000);
    everything.carries_live_medium = true;
    everything.read_only = true;
    everything.mounted_at = vec!["/run/media/x".into()];

    assert_eq!(assess(&everything, MIN), Err(Unsuitable::LiveMedium));

    // And with the live flag gone, the next one down.
    everything.carries_live_medium = false;
    assert_eq!(assess(&everything, MIN), Err(Unsuitable::ReadOnly));
    everything.read_only = false;
    assert!(matches!(
        assess(&everything, MIN),
        Err(Unsuitable::InUse { .. })
    ));
    everything.mounted_at.clear();
    assert!(matches!(
        assess(&everything, MIN),
        Err(Unsuitable::TooSmall { .. })
    ));
}

#[test]
fn an_ordinary_empty_disk_is_usable() {
    assert_eq!(assess(&disk("/dev/sda", BIG), MIN), Ok(()));
}

/// The bug that formats nothing on an NVMe machine.
///
/// `/dev/sda` + 1 is `/dev/sda1`, but `/dev/nvme0n1` + 1 is `/dev/nvme0n1p1`. An installer
/// that assumes the first form writes a filesystem to a path that does not exist — or,
/// on an unlucky machine, to one that does.
#[test]
fn partition_paths_follow_the_rule_the_kernel_actually_uses() {
    assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
    assert_eq!(partition_path("/dev/sda", 2), "/dev/sda2");
    assert_eq!(partition_path("/dev/vdb", 1), "/dev/vdb1");

    assert_eq!(partition_path("/dev/nvme0n1", 1), "/dev/nvme0n1p1");
    assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partition_path("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    assert_eq!(partition_path("/dev/loop0", 1), "/dev/loop0p1");

    // The case the rule exists for: without the `p`, this would be ambiguous with the
    // eleventh namespace of controller zero.
    assert_eq!(partition_path("/dev/nvme0n1", 11), "/dev/nvme0n1p11");
}

/// Sizes are shown the way the box the drive came in shows them.
///
/// A disk sold as 500 GB rendered as "465.7 GiB" invites somebody to believe they are
/// looking at a different disk than the one they meant.
#[test]
fn sizes_are_shown_in_the_units_a_person_recognises() {
    assert_eq!(disk("/dev/sda", 500_000_000_000).human_size(), "500.0 GB");
    assert_eq!(disk("/dev/sda", 1_000_000_000_000).human_size(), "1.0 TB");
    assert_eq!(disk("/dev/sda", 32_000_000_000).human_size(), "32.0 GB");
    assert_eq!(disk("/dev/sda", 512).human_size(), "512 B");
}
