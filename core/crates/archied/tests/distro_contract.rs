//! Static M4 contracts for files copied into the Arch root filesystem.

use archie_core::config::{ArchieConfig, TransportKind};
use std::path::{Path, PathBuf};

fn airootfs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../distro/archiso/airootfs")
}

fn profile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../distro/archiso")
}

fn read(relative: &str) -> String {
    let path = airootfs().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn shipped_configuration_satisfies_the_runtime_contract() {
    let path = airootfs().join("etc/archie/archie.toml");
    let config = ArchieConfig::load(&path).unwrap();
    assert_eq!(config.fabric.transport, TransportKind::Udp);
    assert!(!config.policy.safety_boundary_present);
    assert!(config.validate().is_ok());
}

#[test]
fn service_runs_one_unprivileged_hardened_daemon() {
    let unit = read("etc/systemd/system/archied.service");
    for directive in [
        // Type=notify: readiness is what archied SAYS, not the fact that it was exec'd.
        // Under simple, anything ordered After=archied raced an unbound control socket.
        "Type=notify",
        "NotifyAccess=main",
        "WatchdogSec=60",
        "User=archie",
        "Group=archie",
        "ExecStart=/usr/bin/archied",
        "Restart=on-failure",
        "RestartPreventExitStatus=78",
        "RuntimeDirectory=archie",
        "StateDirectory=archie",
        "LogsDirectory=archie",
        "StandardOutput=journal+console",
        "NoNewPrivileges=yes",
        "CapabilityBoundingSet=",
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
    ] {
        assert!(
            unit.lines().any(|line| line == directive),
            "missing {directive}"
        );
    }
    assert!(!unit.contains("User=root"));
    assert!(!unit.contains("--text-logs"));
}

#[test]
fn account_directories_and_boot_enablement_are_declared() {
    let sysusers = read("usr/lib/sysusers.d/archie.conf");
    assert!(sysusers.lines().any(|line| line.starts_with("u archie ")));
    // The executor runs as its own account: it can create cgroups and spawn workers, and
    // must not be able to read the node's private key. Its group membership is what lets
    // it share a socket with archied by filesystem permission alone.
    assert!(sysusers
        .lines()
        .any(|line| line.starts_with("u archie-executor ")));
    assert!(sysusers
        .lines()
        .any(|line| line.trim() == "m archie-executor archie"));

    let tmpfiles = read("usr/lib/tmpfiles.d/archie.conf");
    for directory in ["/var/lib/archie", "/var/log/archie", "/run/archie"] {
        assert!(tmpfiles.lines().any(|line| line.contains(directory)));
    }
    assert!(tmpfiles.lines().any(|line| line.contains("/etc/archie")));

    let preset = read("usr/lib/systemd/system-preset/50-archie-os.preset");
    for service in [
        "archied.service",
        "archie-worldd.service",
        "archie-displayd.service",
        "archie-display.service",
        "archie-executord.service",
    ] {
        assert!(
            preset
                .lines()
                .any(|line| line == format!("enable {service}")),
            "preset does not enable {service}"
        );
    }
}

#[test]
fn archiso_profile_has_bios_uefi_and_only_runtime_packages() {
    let definition = std::fs::read_to_string(profile().join("profiledef.sh")).unwrap();
    for contract in [
        "iso_name=\"archieos\"",
        "iso_version=\"0.1.0\"",
        "'bios.syslinux'",
        "'uefi.systemd-boot'",
        "airootfs_image_type=\"squashfs\"",
    ] {
        assert!(definition.contains(contract), "missing {contract}");
    }

    let packages = std::fs::read_to_string(profile().join("packages.x86_64")).unwrap();
    for required in [
        "base",
        "linux",
        "linux-firmware",
        "mkinitcpio-archiso",
        "syslinux",
        "btrfs-progs",
    ] {
        assert!(
            packages.lines().any(|line| line == required),
            "missing {required}"
        );
    }

    // `archinstall` used to be required here and is deliberately gone. It is a Python
    // program, and the very next loop forbids `python` in the image — so the two
    // assertions contradicted each other, and the one that lost was the rule about not
    // shipping a language runtime. ArchieOS installs itself with its own native wizard,
    // which is the thing that has to be present instead.
    assert!(
        !packages.lines().any(|line| line == "archinstall"),
        "archinstall pulls a Python runtime into an image that forbids one"
    );
    let build = std::fs::read_to_string(profile().join("build.sh")).unwrap();
    assert!(
        build.contains("archie-install"),
        "the image ships no installer at all: the native wizard replaced archinstall and \
         something has to put it in /usr/bin"
    );
    for forbidden in ["cargo", "rust", "python", "xorg-server", "plasma-desktop"] {
        assert!(
            !packages.lines().any(|line| line == forbidden),
            "ships {forbidden}"
        );
    }
}

#[test]
fn release_builder_stages_only_linux_release_binaries_and_enables_boot() {
    let builder = std::fs::read_to_string(profile().join("build.sh")).unwrap();
    for contract in [
        "cargo build --locked --release",
        "-p archied -p archie-cli",
        "ELF 64-bit.*x86-64",
        "mkarchiso -v",
        "sha256sum",
        "multi-user.target.wants/archied.service",
        "systemd-firstboot.service",
        "ln -s /dev/null",
        "systemd-networkd.service",
        "systemd-resolved.service",
    ] {
        assert!(
            builder.contains(contract),
            "missing build contract {contract}"
        );
    }
    assert!(!airootfs().join("usr/bin/archie").exists());
    assert!(!airootfs().join("usr/bin/archied").exists());
}

#[test]
fn boot_entries_target_the_archiso_kernel_and_initramfs() {
    for relative in [
        "efiboot/loader/entries/01-archieos-linux.conf",
        "syslinux/syslinux-linux.cfg",
    ] {
        let entry = std::fs::read_to_string(profile().join(relative)).unwrap();
        assert!(entry.contains("vmlinuz-linux"));
        assert!(entry.contains("initramfs-linux.img"));
        assert!(entry.contains("archisobasedir=%INSTALL_DIR%"));
        assert!(entry.contains("archisosearchuuid=%ARCHISO_UUID%"));
        assert!(entry.contains("console=ttyS0,115200"));
    }
}

#[test]
fn source_overlay_contains_no_release_forbidden_content() {
    let forbidden_names = [".git", "target", "_legacy", "screenshots", "secrets"];
    let mut pending = vec![airootfs()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(!forbidden_names.contains(&name.as_ref()), "ships {name}");
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            }
        }
    }
}
