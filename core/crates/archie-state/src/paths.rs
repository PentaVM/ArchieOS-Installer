//! Filesystem layout.
//!
//! ArchieOS follows the ordinary Linux conventions:
//!
//! | Path | Contents | Survives reboot |
//! |---|---|---|
//! | `/etc/archie` | Operator configuration. Read-only to the daemon. | yes |
//! | `/var/lib/archie` | Identity and derived filesystem artifacts. | yes |
//! | `/var/lib/archie-postgres` | Durable PostgreSQL cluster. | yes |
//! | `/var/log/archie` | Logs not going to journald. | yes |
//! | `/run/archie` | Runtime sockets and pid files. | no |
//!
//! Mutable state is never written next to the binaries. On an ArchieOS install the
//! system partition is intended to be treated as replaceable; anything that matters
//! lives under `/var/lib/archie` on its own Btrfs subvolume so it survives a rollback
//! of the system subvolume.
//!
//! For development, `ARCHIE_ROOT` relocates the entire tree under one directory, which
//! is what makes it possible to run several simulated nodes on one machine.

use std::path::{Path, PathBuf};

/// Resolved locations for one ArchieOS instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl StatePaths {
    /// Production layout.
    pub fn system() -> Self {
        Self {
            config_dir: PathBuf::from("/etc/archie"),
            state_dir: PathBuf::from("/var/lib/archie"),
            log_dir: PathBuf::from("/var/log/archie"),
            runtime_dir: PathBuf::from("/run/archie"),
        }
    }

    /// Everything under one root. Used by `ARCHIE_ROOT`, tests, and multi-node
    /// simulation on a single machine.
    pub fn rooted(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_dir: root.join("etc"),
            state_dir: root.join("var/lib"),
            log_dir: root.join("var/log"),
            runtime_dir: root.join("run"),
        }
    }

    /// Resolve from the environment.
    ///
    /// `ARCHIE_ROOT` wins if set; otherwise the system layout. Reading the environment
    /// here rather than at each call site keeps the "where does state live" decision in
    /// exactly one place.
    pub fn from_env() -> Self {
        match std::env::var_os("ARCHIE_ROOT") {
            Some(root) if !root.is_empty() => Self::rooted(root),
            _ => Self::system(),
        }
    }

    /// The Ed25519 private key. Must be mode 0600 and never leave the node.
    pub fn identity_key(&self) -> PathBuf {
        self.state_dir.join("identity/node.key")
    }

    /// The public half, safe to publish.
    pub fn identity_public(&self) -> PathBuf {
        self.state_dir.join("identity/node.pub")
    }

    /// The certificate admitting this node to a fabric.
    ///
    /// Absent on a machine nobody has enrolled, which is a real state rather than a
    /// broken one: such a node runs perfectly well and talks to nobody.
    pub fn membership(&self) -> PathBuf {
        self.state_dir.join("identity/membership.json")
    }

    /// The owner key, on the one machine that holds it.
    ///
    /// Present only where someone chose to keep it. It is needed to *admit* a node, not
    /// to run one, so a fabric whose owner key lives on a USB stick in a drawer works
    /// exactly as well until it wants to grow.
    pub fn owner_key(&self) -> PathBuf {
        self.state_dir.join("identity/owner.key")
    }

    /// Private PostgreSQL connection/schema configuration (server side only).
    pub fn database(&self) -> PathBuf {
        self.config_dir.join("database.json")
    }

    /// Peers whose keys have been pinned.
    pub fn known_peers(&self) -> PathBuf {
        self.state_dir.join("fabric/known_peers.toml")
    }

    /// Worker checkpoints, so preempted idle work resumes rather than restarts.
    pub fn checkpoints(&self) -> PathBuf {
        self.state_dir.join("checkpoints")
    }

    /// Installed module manifests.
    pub fn modules(&self) -> PathBuf {
        self.config_dir.join("modules")
    }

    /// Installed data-provider manifests. Staged the same way module manifests are —
    /// read-only, root-owned, absent on a node with none installed, which is a valid
    /// offline node rather than an error.
    pub fn providers(&self) -> PathBuf {
        self.config_dir.join("providers")
    }

    /// The daemon control socket the CLI connects to.
    pub fn control_socket(&self) -> PathBuf {
        self.runtime_dir.join("archied.sock")
    }

    /// Where the control endpoint address is published, for platforms where the CLI
    /// cannot assume a Unix socket path.
    pub fn control_endpoint_file(&self) -> PathBuf {
        self.runtime_dir.join("archied.endpoint")
    }

    /// Process lock held for the entire daemon lifetime.
    ///
    /// The file may survive a crash, but the operating-system lock never does. This
    /// prevents two daemons rooted at the same state tree from both becoming the node
    /// authority while still allowing unattended recovery after a crash.
    pub fn control_lock_file(&self) -> PathBuf {
        self.runtime_dir.join("archied.lock")
    }

    pub fn main_config(&self) -> PathBuf {
        self.config_dir.join("archie.toml")
    }

    /// Create every directory this layout needs.
    ///
    /// Idempotent. Called on every boot rather than only at install, because `/run` is
    /// a tmpfs that is empty again after each reboot.
    ///
    /// # Configuration is read-only to the daemon
    ///
    /// The daemon *owns* its state, log and runtime directories and cannot work without
    /// write access to them, so failing to create one is fatal.
    ///
    /// It does **not** own its configuration. On an installed system `/etc/archie` is
    /// `root:archie 0750` â€” the daemon reads its policy and must not be able to rewrite
    /// it. Trying to create a directory there fails with `EACCES`, and treating that as
    /// a boot failure means a correctly-hardened node refuses to start. So the
    /// configuration paths are created best-effort: useful in a development tree where
    /// the operator owns everything, ignored where the image already provisioned them.
    ///
    /// Only permission and read-only-filesystem errors are tolerated. A missing parent
    /// or an I/O fault still surfaces.
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        // Directories the daemon owns. Failure here is genuinely fatal.
        for dir in [&self.state_dir, &self.log_dir, &self.runtime_dir] {
            std::fs::create_dir_all(dir)?;
        }
        for dir in [
            self.state_dir.join("identity"),
            self.state_dir.join("fabric"),
            self.checkpoints(),
        ] {
            std::fs::create_dir_all(dir)?;
        }

        // Operator-owned. Present on an installed system; created only in a dev tree.
        for dir in [self.config_dir.clone(), self.modules(), self.providers()] {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {}
                    _ if is_read_only_filesystem(&e) => {}
                    _ => return Err(e),
                }
            }
        }
        Ok(())
    }
}

impl Default for StatePaths {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Whether an error is EROFS.
///
/// `ErrorKind::ReadOnlyFilesystem` is not stable, so this matches the raw errno on Unix
/// and falls back to a substring check elsewhere. A read-only `/etc` is a legitimate
/// hardening choice, not a fault.
fn is_read_only_filesystem(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        if error.raw_os_error() == Some(30) {
            return true;
        }
    }
    error.to_string().contains("read-only")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_layout_uses_fhs_paths() {
        let p = StatePaths::system();
        assert_eq!(p.state_dir, PathBuf::from("/var/lib/archie"));
        assert_eq!(p.runtime_dir, PathBuf::from("/run/archie"));
    }

    #[test]
    fn providers_sits_beside_modules_in_the_operator_owned_config_tree() {
        // Same directory an operator already edits `modules/` under, for the same
        // reason: both are read-only, root-owned, declarative inventories the daemon
        // reads and must not be able to write back to.
        let p = StatePaths::system();
        assert_eq!(p.providers(), PathBuf::from("/etc/archie/providers"));
        assert_eq!(p.providers().parent(), p.modules().parent());
    }

    #[test]
    fn rooted_layout_contains_everything_under_the_root() {
        let p = StatePaths::rooted("/tmp/nodeA");
        for path in [
            p.database(),
            p.identity_key(),
            p.control_socket(),
            p.control_lock_file(),
            p.modules(),
            p.providers(),
        ] {
            assert!(
                path.starts_with("/tmp/nodeA"),
                "{path:?} escaped the root, which would let two simulated nodes collide"
            );
        }
    }

    #[test]
    fn state_never_lands_in_a_binary_directory() {
        let p = StatePaths::system();
        for path in [p.identity_key(), p.checkpoints()] {
            assert!(path.starts_with("/var/lib/archie"));
        }
    }

    #[test]
    fn a_read_only_configuration_directory_does_not_stop_the_boot() {
        // On an installed node /etc/archie is root:archie 0750: the daemon reads its
        // policy and must not be able to rewrite it. Treating that as a boot failure
        // means a correctly-hardened node refuses to start â€” which is exactly what
        // happened to the 0.1.0 ISO.
        let temp = std::env::temp_dir().join(format!("archie-ro-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let paths = StatePaths::rooted(&temp);
        paths.ensure_directories().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // r-x for everyone: readable, not writable.
            std::fs::set_permissions(&paths.config_dir, std::fs::Permissions::from_mode(0o555))
                .unwrap();
            let _ = std::fs::remove_dir_all(paths.modules());

            paths
                .ensure_directories()
                .expect("a read-only configuration directory must not fail the boot");

            // Restore so cleanup can proceed.
            std::fs::set_permissions(&paths.config_dir, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        // The directories the daemon owns must still exist on every platform.
        assert!(paths.state_dir.is_dir());
        assert!(paths.checkpoints().is_dir());
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn the_daemon_owned_directories_are_created_strictly() {
        // The daemon cannot work without these, so their absence must surface.
        let temp = std::env::temp_dir().join(format!("archie-owned-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let paths = StatePaths::rooted(&temp);
        paths.ensure_directories().unwrap();

        for dir in [
            paths.state_dir.clone(),
            paths.log_dir.clone(),
            paths.runtime_dir.clone(),
            paths.checkpoints(),
            paths.state_dir.join("identity"),
            paths.state_dir.join("fabric"),
        ] {
            assert!(dir.is_dir(), "{} was not created", dir.display());
        }
        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn ensure_directories_is_idempotent() {
        let temp = std::env::temp_dir().join(format!("archie-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let p = StatePaths::rooted(&temp);
        p.ensure_directories().unwrap();
        p.ensure_directories().unwrap();
        assert!(p.checkpoints().is_dir());
        assert!(p.modules().is_dir());
        assert!(p.providers().is_dir());
        std::fs::remove_dir_all(&temp).unwrap();
    }
}
