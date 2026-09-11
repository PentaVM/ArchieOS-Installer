//! Understanding what `lsblk` said.
//!
//! The live medium announces itself through a mount point on a partition, which is one
//! level down from the disk. A parser that reads the top level and stops will offer
//! somebody the USB stick they are booting from, and the JSON here is the shape real
//! `lsblk` produces so that the test fails when the parser is that one.

use archie_installer::{assess, parse, survey, Unsuitable};

/// A laptop booted from a USB stick: one NVMe drive with a Windows install on it, and the
/// stick the installer is running from.
const LIVE_LAPTOP: &str = r#"{
  "blockdevices": [
    {
      "name": "nvme0n1", "path": "/dev/nvme0n1", "size": 512110190592,
      "rm": false, "ro": false, "model": "SAMSUNG MZVLB512HBJQ", "type": "disk",
      "mountpoints": [null],
      "children": [
        { "name": "nvme0n1p1", "path": "/dev/nvme0n1p1", "size": 272629760,
          "rm": false, "ro": false, "model": null, "type": "part",
          "mountpoints": [null] },
        { "name": "nvme0n1p2", "path": "/dev/nvme0n1p2", "size": 511833206784,
          "rm": false, "ro": false, "model": null, "type": "part",
          "mountpoints": [null] }
      ]
    },
    {
      "name": "sda", "path": "/dev/sda", "size": 30752000000,
      "rm": true, "ro": false, "model": "SanDisk Ultra", "type": "disk",
      "mountpoints": [null],
      "children": [
        { "name": "sda1", "path": "/dev/sda1", "size": 2000000000,
          "rm": true, "ro": false, "model": null, "type": "part",
          "mountpoints": ["/run/archiso/bootmnt"] }
      ]
    },
    {
      "name": "loop0", "path": "/dev/loop0", "size": 1900000000,
      "rm": false, "ro": true, "model": null, "type": "loop",
      "mountpoints": ["/run/archiso/airootfs"]
    }
  ]
}"#;

/// The whole point: the stick is recognised through its partition, not through itself.
#[test]
fn the_live_medium_is_found_one_level_down() {
    let devices = parse(LIVE_LAPTOP).expect("parse");
    let stick = devices
        .iter()
        .find(|d| d.path == "/dev/sda")
        .expect("the stick is a disk and is listed");

    assert!(
        stick.carries_live_medium,
        "the mount point that gives it away is on sda1, not on sda"
    );
    assert_eq!(assess(stick, 16_000_000_000), Err(Unsuitable::LiveMedium));
}

/// And the internal drive, which is what the person is trying to install onto, is offered.
#[test]
fn the_internal_drive_is_the_one_offered() {
    let devices = parse(LIVE_LAPTOP).expect("parse");
    let survey = survey(&devices, 16_000_000_000);

    assert_eq!(survey.usable.len(), 1);
    assert_eq!(survey.usable[0].path, "/dev/nvme0n1");
    assert_eq!(survey.usable[0].model, "SAMSUNG MZVLB512HBJQ");
    assert_eq!(survey.usable[0].human_size(), "512.1 GB");
}

/// Loop devices are how the live root is mounted. They are not disks and must not be
/// offered, or somebody installs ArchieOS onto the squashfs it is running from.
#[test]
fn loop_devices_are_not_offered() {
    let devices = parse(LIVE_LAPTOP).expect("parse");
    assert!(
        !devices.iter().any(|d| d.path.contains("loop")),
        "a loop device is not something an install targets"
    );
    assert_eq!(devices.len(), 2, "two disks: the NVMe and the stick");
}

/// A mounted internal disk is in use, and the mount points are reported so the refusal
/// can say which ones.
#[test]
fn mounts_below_the_top_level_make_a_disk_in_use() {
    let json = r#"{
      "blockdevices": [
        { "name": "sda", "path": "/dev/sda", "size": 500000000000,
          "rm": false, "ro": false, "model": "WDC WD5000", "type": "disk",
          "mountpoints": [null],
          "children": [
            { "name": "sda1", "path": "/dev/sda1", "size": 500000000000,
              "type": "part", "mountpoints": ["/home", "/var/data"] }
          ]
        }
      ]
    }"#;
    let devices = parse(json).expect("parse");
    assert_eq!(devices[0].mounted_at, vec!["/home", "/var/data"]);
    assert!(matches!(
        assess(&devices[0], 16_000_000_000),
        Err(Unsuitable::InUse { .. })
    ));
}

/// Two levels down: a partition holding an encrypted volume that is the thing mounted.
///
/// A disk whose third level is mounted is as much in use as one whose first level is.
#[test]
fn nesting_deeper_than_one_level_is_still_in_use() {
    let json = r#"{
      "blockdevices": [
        { "name": "sda", "path": "/dev/sda", "size": 500000000000,
          "type": "disk", "mountpoints": [null],
          "children": [
            { "name": "sda1", "type": "part", "mountpoints": [null],
              "children": [
                { "name": "vault", "type": "crypt", "mountpoints": ["/secrets"] }
              ]
            }
          ]
        }
      ]
    }"#;
    let devices = parse(json).expect("parse");
    assert_eq!(devices[0].mounted_at, vec!["/secrets"]);
}

/// Older `lsblk` reports one `mountpoint` string rather than a `mountpoints` array. The
/// installer runs on whatever the live image shipped, so both are read.
#[test]
fn the_older_single_mountpoint_field_is_also_read() {
    let json = r#"{
      "blockdevices": [
        { "name": "sda", "path": "/dev/sda", "size": 30000000000,
          "type": "disk", "mountpoint": null,
          "children": [
            { "name": "sda1", "type": "part", "mountpoint": "/run/archiso/bootmnt" }
          ]
        }
      ]
    }"#;
    let devices = parse(json).expect("parse");
    assert!(devices[0].carries_live_medium);
}

/// The archiso mount points are true and useless to a person. Saying "this is the medium
/// you booted from" ends the question; listing `/run/archiso/bootmnt` does not.
#[test]
fn the_live_mounts_do_not_leak_into_the_reason_shown() {
    let devices = parse(LIVE_LAPTOP).expect("parse");
    let stick = devices.iter().find(|d| d.path == "/dev/sda").unwrap();
    assert!(
        stick.mounted_at.is_empty(),
        "otherwise the refusal reads as a mount-point list: {:?}",
        stick.mounted_at
    );
}

/// A drive with no model string still has to be identifiable in a menu.
#[test]
fn a_disk_without_a_model_is_still_listed() {
    let json = r#"{
      "blockdevices": [
        { "name": "vda", "path": "/dev/vda", "size": 40000000000,
          "type": "disk", "model": null, "mountpoints": [null] }
      ]
    }"#;
    let devices = parse(json).expect("parse");
    assert_eq!(devices[0].model, "unknown model");
    assert_eq!(assess(&devices[0], 16_000_000_000), Ok(()));
}

#[test]
fn output_that_is_not_lsblk_output_is_an_error_rather_than_an_empty_list() {
    assert!(parse("not json at all").is_err());
    assert!(
        parse(r#"{"something_else": []}"#).is_err(),
        "an empty list would read as a machine with no disks, which is a different fact"
    );
}
