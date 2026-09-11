//! Everything the install will do, decided before any of it is done.
//!
//! The wizard does not discover its next action as it goes. It builds the whole sequence
//! first, shows it, and only then runs it. That costs nothing and buys three things: the
//! person can read what is about to happen to their disk, a test can assert on the
//! sequence without a disk existing, and a failure halfway through can name the step it
//! stopped at rather than leaving somebody to guess how far it got.

use crate::answers::Answers;
use crate::disk::{partition_path, BlockDevice};

/// The EFI system partition. 512 MB is the size that stops being a problem: a kernel and
/// a fallback kernel fit, and the machines that shipped with 100 MB partitions are the
/// ones whose owners later discover they cannot install an update.
pub const ESP_BYTES: u64 = 512 * 1024 * 1024;

/// Where the target filesystem is assembled while the installer runs.
pub const TARGET_ROOT: &str = "/mnt";

/// One thing the installer does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run a command on the live system.
    Run { argv: Vec<String> },
    /// Run a command inside the installed system, via `arch-chroot`.
    InTarget { argv: Vec<String> },
    /// Write a file into the installed system.
    Write {
        /// Path relative to the root of the installed system, e.g. `/etc/hostname`.
        path: String,
        contents: String,
        /// Unix mode. Identity material is `0o600`; everything else `0o644`.
        mode: u32,
    },
    /// Move a key file that already exists on the live system into the installed one.
    ///
    /// Carries paths, never key material — for the same reason [`Action::SetPassword`]
    /// carries no password. The keys are generated into a tmpfs on the live system before
    /// the disk is touched, because a fabric certificate has to name the node public key
    /// and so the key must exist before the plan that installs it does.
    InstallKey {
        /// Path on the live system.
        from: String,
        /// Path inside the installed system.
        to: String,
        mode: u32,
    },
    /// Set the password for a user.
    ///
    /// Deliberately carries no password. A plan is printed to a terminal, and on a
    /// failure it is the first thing somebody pastes into a bug report — so a plan that
    /// *can* hold a secret is a plan that will eventually publish one. The wizard holds
    /// the password in memory for the seconds between asking and piping it to `chpasswd`,
    /// and it never reaches this type, a log line, or a process argument list.
    SetPassword { user: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What this does, in the words used to tell a person before it happens.
    pub explain: String,
    pub action: Action,
    /// Whether this step destroys data that was on the disk beforehand.
    ///
    /// Tracked so the confirmation can say how many such steps there are, and so the
    /// executor can refuse to begin any of them without one.
    pub destroys_data: bool,
}

impl Step {
    fn run(explain: &str, argv: &[&str]) -> Self {
        Self {
            explain: explain.into(),
            action: Action::Run {
                argv: argv.iter().map(|s| s.to_string()).collect(),
            },
            destroys_data: false,
        }
    }

    fn destructive(explain: &str, argv: &[&str]) -> Self {
        Self {
            destroys_data: true,
            ..Self::run(explain, argv)
        }
    }

    fn in_target(explain: &str, argv: &[&str]) -> Self {
        Self {
            explain: explain.into(),
            action: Action::InTarget {
                argv: argv.iter().map(|s| s.to_string()).collect(),
            },
            destroys_data: false,
        }
    }

    fn write(explain: &str, path: &str, contents: String, mode: u32) -> Self {
        Self {
            explain: explain.into(),
            action: Action::Write {
                path: path.into(),
                contents,
                mode,
            },
            destroys_data: false,
        }
    }
}

/// Where the identity this install will carry currently lives.
///
/// Deciding *which fabric* a node belongs to is `archie_fabric::enrolment_plan`'s job;
/// putting the result on a disk is this one's. The private keys are referred to by path
/// so that neither this type nor the printed plan ever holds one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityFiles {
    /// Path on the live system to the freshly generated node key.
    pub node_key_source: String,
    /// The membership certificate. Public: it is what a node shows to strangers.
    pub membership_json: String,
    /// Path on the live system to the owner key — present only when this install started
    /// its own fabric. A machine joining an existing one is never given a copy.
    pub owner_key_source: Option<String>,
}

/// The whole install, as a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub target: BlockDevice,
    pub steps: Vec<Step>,
}

impl InstallPlan {
    /// How many steps will destroy what is currently on the disk.
    pub fn destructive_steps(&self) -> usize {
        self.steps.iter().filter(|s| s.destroys_data).count()
    }

    /// The line shown immediately above the confirmation prompt.
    ///
    /// Names the disk by path *and* model *and* size, because somebody with two identical
    /// drives cannot tell `/dev/sdb` from `/dev/sdc` and should not have to.
    pub fn destruction_warning(&self) -> String {
        format!(
            "Everything on {} ({}, {}) will be erased.",
            self.target.path,
            self.target.model,
            self.target.human_size()
        )
    }
}

/// Build the install.
pub fn build(
    target: &BlockDevice,
    answers: &Answers,
    identity: &IdentityFiles,
    config_toml: &str,
    map: &crate::mapdata::MapChoice,
) -> InstallPlan {
    let disk = target.path.as_str();
    let esp = partition_path(disk, 1);
    let root = partition_path(disk, 2);
    let esp_size = format!("--new=1:0:+{}M", ESP_BYTES / (1024 * 1024));
    let target_boot = format!("{TARGET_ROOT}/boot");

    let mut steps = vec![
        Step::destructive(
            "Erase the existing partition table",
            &["sgdisk", "--zap-all", disk],
        ),
        Step::destructive(
            "Create the EFI system partition",
            &[
                "sgdisk",
                &esp_size,
                "--typecode=1:ef00",
                "--change-name=1:ARCHIEOS-ESP",
                disk,
            ],
        ),
        Step::destructive(
            "Create the root partition across the rest of the disk",
            &[
                "sgdisk",
                "--new=2:0:0",
                "--typecode=2:8304",
                "--change-name=2:ARCHIEOS-ROOT",
                disk,
            ],
        ),
        Step::run("Re-read the partition table", &["partprobe", disk]),
        Step::destructive(
            "Format the EFI partition",
            &["mkfs.fat", "-F32", "-n", "ARCHIE-ESP", &esp],
        ),
        Step::destructive(
            "Format the root partition",
            &["mkfs.ext4", "-F", "-L", "archieos", &root],
        ),
        Step::run("Mount the new root", &["mount", &root, TARGET_ROOT]),
        Step::run("Create the boot directory", &["mkdir", "-p", &target_boot]),
        Step::run("Mount the EFI partition", &["mount", &esp, &target_boot]),
        Step::run(
            "Copy the live system onto the disk",
            &[
                "rsync",
                "-aAXH",
                "--info=progress2",
                "--exclude=/run/archiso",
                "/run/archiso/airootfs/",
                TARGET_ROOT,
            ],
        ),
        Step::write(
            "Set the hostname",
            "/etc/hostname",
            format!("{}\n", answers.hostname()),
            0o644,
        ),
        Step::write(
            "Write the node configuration",
            "/etc/archie/archie.toml",
            config_toml.to_string(),
            0o644,
        ),
        Step::write(
            "Bind installation finalization to the reviewed root partition",
            "/etc/archie/install-target-device",
            format!("{root}\n"),
            0o600,
        ),
    ];

    // Identity, written after the copy so that nothing carried over from the live image
    // can already be sitting at these paths.
    steps.push(Step {
        explain: "Install the private key that is this node identity".into(),
        action: Action::InstallKey {
            from: identity.node_key_source.clone(),
            to: "/var/lib/archie/identity/node.key".into(),
            mode: 0o600,
        },
        destroys_data: false,
    });
    steps.push(Step::write(
        "Write the certificate admitting this node to its fabric",
        "/var/lib/archie/identity/membership.json",
        format!("{}\n", identity.membership_json),
        0o644,
    ));
    if let Some(owner_key_source) = &identity.owner_key_source {
        steps.push(Step {
            explain: "Install the owner key that admits nodes to this fabric".into(),
            action: Action::InstallKey {
                from: owner_key_source.clone(),
                to: "/var/lib/archie/identity/owner.key".into(),
                mode: 0o600,
            },
            destroys_data: false,
        });
    }

    steps.extend([
        Step::run(
            "Record the filesystems so the installed system can mount them",
            &["sh", "-c", "genfstab -U /mnt >> /mnt/etc/fstab"],
        ),
        Step::in_target(
            "Create your account",
            &[
                "useradd",
                "--create-home",
                "--groups",
                "wheel",
                answers.username(),
            ],
        ),
        Step::write(
            "Allow the owner account to administer the system with its password",
            "/etc/sudoers.d/10-archie-owner",
            "%wheel ALL=(ALL:ALL) ALL\n".into(),
            0o440,
        ),
        Step {
            explain: format!("Set the password for {}", answers.username()),
            action: Action::SetPassword {
                user: answers.username().to_string(),
            },
            destroys_data: false,
        },
        // The live image logs straight into a shell with no password. That is right for
        // something you boot to look at, wrong for something you install, and exactly the
        // kind of setting that survives an install unnoticed.
        Step::in_target(
            "Remove the automatic login the live system uses",
            &[
                "rm",
                "-f",
                "/etc/systemd/system/getty@tty1.service.d/autologin.conf",
            ],
        ),
        Step::in_target(
            "Prepare disk boot and remove the live-only graphical environment",
            &["/usr/lib/archie/finalize-install-target", &root],
        ),
        Step::in_target(
            "Build the boot image for this machine",
            &["mkinitcpio", "-P"],
        ),
        Step::in_target(
            "Install the bootloader",
            &["bootctl", "--no-variables", "install"],
        ),
        Step::in_target(
            "Start the ArchieOS control plane at boot",
            &["systemctl", "enable", "archied.service"],
        ),
        Step::in_target(
            "Start the execution daemon at boot",
            &["systemctl", "enable", "archie-executord.service"],
        ),
    ]);

    // The AI module is a service, so the answer is an enable or a disable rather than a
    // line in a config file nobody reads. Written either way: a machine where the module
    // was declined should say so, not merely fail to mention it.
    steps.push(if answers.enable_ai_module() {
        Step::in_target(
            "Enable the Analytical Cortex",
            &["systemctl", "enable", "archie-cortex.socket"],
        )
    } else {
        Step::in_target(
            "Leave the Analytical Cortex switched off",
            &["systemctl", "disable", "archie-cortex.socket"],
        )
    });

    // The vector map, when one was asked for. Last, because it is the only step that
    // needs a network and the only one whose failure should not cost the install: by the
    // time it runs the machine is already complete and bootable, and a download that dies
    // half way leaves a system that works with the globe it shipped with.
    if let crate::mapdata::MapChoice::Download {
        region,
        destination,
    } = map
    {
        let target_path = format!("{TARGET_ROOT}{}", destination.display());
        steps.push(Step::run(
            "Download or reuse the verified regional map cache",
            &[
                "archie-map-fetch",
                "--url",
                &region.url,
                "--destination",
                &target_path,
            ],
        ));
    }

    steps.push(Step::run(
        "Unmount the installed system",
        &["umount", "-R", TARGET_ROOT],
    ));

    InstallPlan {
        target: target.clone(),
        steps,
    }
}

/// Mount an existing separate map filesystem before genfstab. Never format it.
/// The caller must re-probe its physical parent and live-medium protection.
pub fn attach_map_volume(
    plan: &mut InstallPlan,
    device: &str,
    destination: &str,
) -> Result<(), String> {
    if !crate::mapdata::safe_device(device) || !crate::mapdata::safe_destination(destination) {
        return Err("invalid separate map volume".into());
    }
    let index = plan
        .steps
        .iter()
        .position(|s| match &s.action {
            Action::Run { argv } => argv.iter().any(|a| a.contains("genfstab")),
            _ => false,
        })
        .ok_or("filesystem table step missing")?;
    let path = format!("{TARGET_ROOT}{destination}");
    plan.steps.insert(
        index,
        Step::run(
            "Mount the existing map partition without formatting",
            &["mount", "--", device, &path],
        ),
    );
    plan.steps.insert(
        index,
        Step::run("Create the map mount point", &["mkdir", "-p", "--", &path]),
    );
    Ok(())
}
