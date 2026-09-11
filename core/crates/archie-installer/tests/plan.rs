//! What the installer will do, checked without a disk.

use archie_core::config::ArchieConfig;
use archie_installer::plan::{Action, IdentityFiles};
use archie_installer::{build, config, Answers, BlockDevice};
use archie_types::NodeProfile;

fn target(path: &str) -> BlockDevice {
    BlockDevice {
        path: path.into(),
        model: "SAMSUNG MZVLB512HBJQ".into(),
        size_bytes: 512_110_190_592,
        removable: false,
        read_only: false,
        carries_live_medium: false,
        mounted_at: Vec::new(),
    }
}

fn answers(ai: bool) -> Answers {
    Answers::new("archie", "nelson", NodeProfile::Workstation, ai).expect("valid answers")
}

fn identity(owned: bool) -> IdentityFiles {
    IdentityFiles {
        node_key_source: "/run/archie-install/node.key".into(),
        membership_json: r#"{"owner_public_key":"aa","node_public_key":"bb"}"#.into(),
        owner_key_source: owned.then(|| "/run/archie-install/owner.key".into()),
    }
}

fn plan_for(disk: &str, ai: bool, owned: bool) -> archie_installer::InstallPlan {
    let a = answers(ai);
    let toml = config::render(&a, "aabbccdd11223344", None);
    build(
        &target(disk),
        &a,
        &identity(owned),
        &toml,
        &archie_installer::MapChoice::Skip,
    )
}

fn argv_of(step: &archie_installer::Step) -> Vec<String> {
    match &step.action {
        Action::Run { argv } | Action::InTarget { argv } => argv.clone(),
        _ => Vec::new(),
    }
}

#[test]
fn disk_boot_is_prepared_before_initramfs_without_changing_firmware_order() {
    let plan = plan_for("/dev/nvme0n1", true, true);
    let finalize = plan
        .steps
        .iter()
        .position(|s| {
            argv_of(s)
                .first()
                .is_some_and(|a| a == "/usr/lib/archie/finalize-install-target")
        })
        .unwrap();
    let initramfs = plan
        .steps
        .iter()
        .position(|s| argv_of(s).first().is_some_and(|a| a == "mkinitcpio"))
        .unwrap();
    assert!(finalize < initramfs);
    assert_eq!(argv_of(&plan.steps[finalize])[1], "/dev/nvme0n1p2");
    assert!(plan
        .steps
        .iter()
        .any(|s| argv_of(s) == ["bootctl", "--no-variables", "install"]));
    assert!(plan.steps.iter().take(finalize).any(|s| matches!(&s.action, Action::Write {path,contents,mode} if path == "/etc/archie/install-target-device" && contents == "/dev/nvme0n1p2\n" && *mode == 0o600)));
}

/// The NVMe naming bug, checked where it would actually bite.
///
/// `disk.rs` proves `partition_path`. This proves the plan uses it, which is the part that
/// would still be wrong if somebody wrote `format!("{disk}1")` here.
#[test]
fn the_plan_formats_the_partitions_that_will_exist() {
    let nvme = plan_for("/dev/nvme0n1", true, true);
    let all: Vec<String> = nvme.steps.iter().flat_map(argv_of).collect();

    assert!(all.contains(&"/dev/nvme0n1p1".to_string()));
    assert!(all.contains(&"/dev/nvme0n1p2".to_string()));
    assert!(
        !all.iter().any(|a| a == "/dev/nvme0n11"),
        "that path is the eleventh partition, and on this disk it does not exist"
    );

    let sata = plan_for("/dev/sda", true, true);
    let all: Vec<String> = sata.steps.iter().flat_map(argv_of).collect();
    assert!(all.contains(&"/dev/sda1".to_string()));
    assert!(all.contains(&"/dev/sda2".to_string()));
}

/// A plan is printed, and on a failure it is the first thing pasted into a bug report.
#[test]
fn no_step_of_the_plan_can_hold_a_secret() {
    let plan = plan_for("/dev/sda", true, true);
    let rendered = format!("{plan:?}");

    // The one step that concerns a password names the user and nothing else.
    let password_steps: Vec<_> = plan
        .steps
        .iter()
        .filter(|s| matches!(s.action, Action::SetPassword { .. }))
        .collect();
    assert_eq!(password_steps.len(), 1);
    match &password_steps[0].action {
        Action::SetPassword { user } => assert_eq!(user, "nelson"),
        other => panic!("{other:?}"),
    }

    // And the private keys are referred to by path, never by value. If somebody changes
    // `IdentityFiles` to carry hex, this is what notices.
    for step in &plan.steps {
        if let Action::InstallKey { from, .. } = &step.action {
            assert!(
                from.starts_with("/run/"),
                "a key in the plan must be a path into the staging tmpfs, not material"
            );
        }
    }
    assert!(
        !rendered.contains("BEGIN"),
        "no key blocks in a printed plan"
    );
}

/// Private keys land at 0600 and the certificate does not, because one of them is what a
/// node shows to strangers.
#[test]
fn the_private_keys_are_the_only_things_written_unreadable() {
    let plan = plan_for("/dev/sda", true, true);

    let mut node_key_mode = None;
    let mut owner_key_mode = None;
    let mut membership_mode = None;
    for step in &plan.steps {
        match &step.action {
            Action::InstallKey { to, mode, .. } if to.ends_with("node.key") => {
                node_key_mode = Some(*mode)
            }
            Action::InstallKey { to, mode, .. } if to.ends_with("owner.key") => {
                owner_key_mode = Some(*mode)
            }
            Action::Write { path, mode, .. } if path.ends_with("membership.json") => {
                membership_mode = Some(*mode)
            }
            _ => {}
        }
    }
    assert_eq!(node_key_mode, Some(0o600));
    assert_eq!(owner_key_mode, Some(0o600));
    assert_eq!(membership_mode, Some(0o644));
}

/// A machine that joined somebody else's fabric is not handed the ability to admit others.
#[test]
fn joining_an_existing_fabric_writes_no_owner_key() {
    let plan = plan_for("/dev/sda", true, false);
    assert!(
        !plan.steps.iter().any(|s| matches!(
            &s.action,
            Action::InstallKey { to, .. } if to.ends_with("owner.key")
        )),
        "the owner key was lent to issue one certificate, not given away"
    );
    // The node key is still installed: it belongs to this machine.
    assert!(plan.steps.iter().any(|s| matches!(
        &s.action,
        Action::InstallKey { to, .. } if to.ends_with("node.key")
    )));
}

/// The count shown above the confirmation has to be the real one.
#[test]
fn the_destructive_steps_are_counted_and_come_first() {
    let plan = plan_for("/dev/sda", true, true);
    assert_eq!(plan.destructive_steps(), 5);

    let last_destructive = plan
        .steps
        .iter()
        .rposition(|s| s.destroys_data)
        .expect("there are destructive steps");
    let first_write = plan
        .steps
        .iter()
        .position(|s| matches!(s.action, Action::Write { .. }))
        .expect("there are writes");
    assert!(
        last_destructive < first_write,
        "the disk is erased and formatted before anything is put on it"
    );
}

/// The warning names the disk three ways, because somebody with two identical drives
/// cannot tell /dev/sdb from /dev/sdc and should not have to.
#[test]
fn the_warning_identifies_the_physical_object() {
    let plan = plan_for("/dev/nvme0n1", true, true);
    let warning = plan.destruction_warning();

    assert!(warning.contains("/dev/nvme0n1"));
    assert!(warning.contains("SAMSUNG MZVLB512HBJQ"));
    assert!(warning.contains("512.1 GB"));
    assert!(warning.contains("erased"));
}

/// Declining the AI module is written down, not merely left unmentioned.
#[test]
fn the_cortex_answer_is_recorded_either_way() {
    let enabled: Vec<String> = plan_for("/dev/sda", true, true)
        .steps
        .iter()
        .flat_map(argv_of)
        .collect();
    assert!(enabled
        .windows(3)
        .any(|w| w == ["systemctl", "enable", "archie-cortex.socket"]));

    let declined: Vec<String> = plan_for("/dev/sda", false, true)
        .steps
        .iter()
        .flat_map(argv_of)
        .collect();
    assert!(declined
        .windows(3)
        .any(|w| w == ["systemctl", "disable", "archie-cortex.socket"]));
    assert!(
        !declined
            .windows(3)
            .any(|w| w == ["systemctl", "enable", "archie-cortex.socket"]),
        "a machine where the module was declined must not also enable it"
    );
}

/// The live image logs straight into a root shell. That is right for something you boot
/// to look at and wrong for something you install, and it is exactly the kind of setting
/// that survives an install unnoticed.
#[test]
fn the_live_autologin_does_not_survive_the_install() {
    let plan = plan_for("/dev/sda", true, true);
    let all: Vec<String> = plan.steps.iter().flat_map(argv_of).collect();
    assert!(
        all.iter().any(|a| a.contains("autologin.conf")),
        "an installed machine that logs in with no password is one anybody at the \
         keyboard owns"
    );
}

/// The configuration the installer writes is parsed by the loader the daemon runs.
///
/// The file is generated as text so it can keep its comments, which trades a guarantee
/// for readability. This buys the guarantee back.
#[test]
fn the_generated_configuration_is_one_the_daemon_will_accept() {
    for profile in config::OFFERED_PROFILES {
        for ai in [true, false] {
            let a = Answers::new("archie", "nelson", profile, ai).expect("valid");
            let text = config::render(&a, "aabbccdd11223344", None);
            let parsed = ArchieConfig::from_toml(&text)
                .unwrap_or_else(|e| panic!("{profile} / ai={ai} produced unusable config: {e}"));
            assert_eq!(parsed.node.profile, profile);
        }
    }
}

/// Somebody comparing two machines needs to see whether they are in the same fabric
/// without running anything.
#[test]
fn the_configuration_records_which_fabric_this_is() {
    let a = answers(true);
    let text = config::render(&a, "aabbccdd11223344", None);
    assert!(text.contains("aabbccdd11223344"));
    assert!(
        text.contains("membership.json"),
        "and says where the fact actually lives, so nobody tries to change fabric by \
         editing this file"
    );
}

#[test]
fn every_profile_offered_has_something_written_beside_it() {
    for profile in config::OFFERED_PROFILES {
        let text = config::describe(profile);
        assert!(text.len() > 20, "{profile} has no description");
    }
    assert_eq!(
        config::OFFERED_PROFILES[0],
        NodeProfile::Workstation,
        "the first entry is the one most people want, so most people answer correctly"
    );
}

/// A map that is downloaded and then not recorded is a map nothing can find.
#[test]
fn the_configuration_says_where_the_map_went() {
    let a = answers(true);

    let with = config::render(
        &a,
        "aabbccdd11223344",
        Some("/var/lib/archie/maps/chile.osm.pbf"),
    );
    assert!(with.contains("map_data = \"/var/lib/archie/maps/chile.osm.pbf\""));
    let parsed = ArchieConfig::from_toml(&with).expect("the daemon must accept it");
    assert_eq!(
        parsed.world.map_data.as_deref(),
        Some("/var/lib/archie/maps/chile.osm.pbf")
    );

    // And an install that skipped it says nothing rather than pointing at a file that
    // was never fetched.
    let without = config::render(&a, "aabbccdd11223344", None);
    assert!(!without.contains("map_data ="));
    assert_eq!(
        ArchieConfig::from_toml(&without)
            .expect("still valid")
            .world
            .map_data,
        None
    );
}

/// The download is last, and its failure must not cost the install.
#[test]
fn the_map_is_fetched_only_after_the_machine_is_complete() {
    let a = answers(true);
    let toml = config::render(
        &a,
        "aabbccdd11223344",
        Some("/var/lib/archie/maps/x.osm.pbf"),
    );
    let plan = build(
        &target("/dev/sda"),
        &a,
        &identity(true),
        &toml,
        &archie_installer::MapChoice::Download {
            region: archie_installer::MapRegion::new(
                "Chile",
                "https://download.geofabrik.de/south-america/chile-latest.osm.pbf",
            ),
            destination: std::path::PathBuf::from("/var/lib/archie/maps"),
        },
    );

    let all: Vec<String> = plan.steps.iter().flat_map(argv_of).collect();
    assert!(all.iter().any(|a| a.contains("chile-latest.osm.pbf")));

    // The destination that was chosen, not one the plan decided for itself. Asking where
    // it should go and then putting it somewhere else is the whole feature failing
    // quietly — and on a machine where the choice was made for want of room, it fails by
    // filling the disk it was meant to avoid.
    assert!(
        all.iter().any(|a| a.contains("/mnt/var/lib/archie/maps")),
        "the plan must use the directory the person named, under the install mount: {all:?}"
    );
    assert!(
        !all.iter().any(|a| a.contains("/mnt/mnt")),
        "the mount point is added once, by the plan, and not again by the answer"
    );

    let download = plan
        .steps
        .iter()
        .position(|s| s.explain == "Download or reuse the verified regional map cache")
        .expect("the map is downloaded");
    let bootloader = plan
        .steps
        .iter()
        .position(|s| s.explain == "Install the bootloader")
        .expect("the bootloader is installed");
    assert!(
        download > bootloader,
        "a download that dies half way must leave a machine that already boots"
    );

    // And the checksum is checked, because a truncated four-gigabyte download is the
    // failure that actually happens.
    assert!(
        all.iter().any(|a| a == "archie-map-fetch"),
        "an unverified archive is one nobody can tell from a complete one"
    );

    // Skipping adds none of it.
    let skipped = plan_for("/dev/sda", true, true);
    assert!(!skipped.steps.iter().any(|s| s.explain.contains("map")));
}

#[test]
fn a_separate_map_partition_is_mounted_before_fstab_and_never_formatted() {
    let mut plan = plan_for("/dev/sda", true, true);
    archie_installer::plan::attach_map_volume(&mut plan, "/dev/sdb1", "/srv/maps").unwrap();
    let mount = plan
        .steps
        .iter()
        .position(|s| argv_of(s) == ["mount", "--", "/dev/sdb1", "/mnt/srv/maps"])
        .unwrap();
    let fstab = plan
        .steps
        .iter()
        .position(|s| argv_of(s).iter().any(|a| a.contains("genfstab")))
        .unwrap();
    assert!(mount < fstab);
    assert!(!plan
        .steps
        .iter()
        .any(|s| s.destroys_data && argv_of(s).iter().any(|a| a.contains("sdb"))));
    assert!(
        archie_installer::plan::attach_map_volume(&mut plan, "/dev/sdb1;id", "/srv/maps").is_err()
    );
}
