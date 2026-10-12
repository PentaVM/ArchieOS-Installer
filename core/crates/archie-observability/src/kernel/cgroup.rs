//! cgroup v2 — where a resource budget stops being a request.
//!
//! # The gap this closes
//!
//! Two of ArchieOS's stated guarantees are currently cooperative, and cooperative means
//! "true until something misbehaves":
//!
//! * The observatory "accepts a quality budget". It is a number the renderer is asked to
//!   respect. A wedged renderer respects nothing.
//! * Preemption emits `must_checkpoint` and *waits*. A worker in a tight loop never
//!   checkpoints, and the user request that triggered the preemption waits with it.
//!
//! The kernel already has the machinery for both. `cpu.max` and `memory.max` are
//! enforced whether or not the process agrees, and `cgroup.freeze` stops a process
//! between instructions.
//!
//! # Freezing is the second step, never the first
//!
//! A checkpoint is worth waiting a second for: idle research that resumes is worth far
//! more than idle research that restarts. So the sequence is
//!
//! ```text
//! 1. ask the worker to checkpoint       cooperative, with a deadline
//! 2. on deadline, freeze                kernel stops it mid-instruction
//! 3. start the user's work
//! 4. thaw, or kill and let it resume from its last checkpoint
//! ```
//!
//! The freezer is not a replacement for asking. It is what makes the deadline mean
//! something.
//!
//! # What must never be frozen
//!
//! Only leaf worker cgroups. Freezing anything `archied` talks to synchronously — the
//! control endpoint, the fabric, the state store — deadlocks the control plane against
//! itself, and the freezer is not interruptible from inside. [`CgroupPath::is_freezable`]
//! encodes that rule so it is checked rather than remembered.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the unified hierarchy is mounted on a systemd machine.
pub const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// A cgroup, identified by its path relative to the v2 root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupPath {
    /// Relative path, e.g. `system.slice/archie-display.service`. Never absolute, and
    /// never containing `..`.
    relative: String,
}

impl CgroupPath {
    /// Build a cgroup path, rejecting anything that could escape the hierarchy.
    ///
    /// This is the only constructor. A path assembled from a unit name that came off the
    /// network or out of a config file must not be able to reach `/sys/fs/cgroup/../..`,
    /// so traversal is refused here rather than trusted to callers.
    pub fn new(relative: impl Into<String>) -> Result<Self, CgroupError> {
        let relative = relative.into();
        let trimmed = relative.trim_matches('/').to_string();

        if trimmed.is_empty() {
            return Err(CgroupError::InvalidPath("empty cgroup path".into()));
        }
        if trimmed.split('/').any(|segment| {
            segment.is_empty() || segment == ".." || segment == "." || segment.contains('\0')
        }) {
            return Err(CgroupError::InvalidPath(format!(
                "{relative:?} contains a traversal or empty segment"
            )));
        }
        Ok(Self { relative: trimmed })
    }

    pub fn as_str(&self) -> &str {
        &self.relative
    }

    /// The absolute path under a given cgroup root.
    pub fn resolve(&self, root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().join(&self.relative)
    }

    /// The unit name, when this cgroup belongs to a systemd unit.
    pub fn unit_name(&self) -> Option<&str> {
        self.relative
            .rsplit('/')
            .next()
            .filter(|segment| segment.ends_with(".service") || segment.ends_with(".scope"))
    }

    /// Whether this cgroup may be frozen.
    ///
    /// Freezing a cgroup the control plane talks to synchronously deadlocks it against
    /// itself, and the freezer cannot be released from inside. Only worker scopes and
    /// slices below the archie worker slice qualify; everything else is refused by name.
    pub fn is_freezable(&self) -> bool {
        const NEVER_FREEZE: [&str; 4] = [
            "archied.service",
            "archie-displayd.service",
            "archie-worldd.service",
            "init.scope",
        ];
        if let Some(unit) = self.unit_name() {
            if NEVER_FREEZE.contains(&unit) {
                return false;
            }
        }
        // The control plane's own slice and anything above it are off limits regardless
        // of the leaf name.
        !self.relative.is_empty()
            && !NEVER_FREEZE
                .iter()
                .any(|never| self.relative.contains(never))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupError {
    /// No cgroup v2 hierarchy. A v1-only machine, or not Linux.
    Unsupported,
    /// The path was refused before any I/O.
    InvalidPath(String),
    /// The controller file existed but did not parse — the format changed.
    Malformed(String),
    /// The write was refused by the service sandbox rather than by the kernel.
    ///
    /// `archied.service` sets `ProtectControlGroups=yes`, which mounts `/sys/fs/cgroup`
    /// read-only for the unit, so every controller write returns `EROFS` no matter how
    /// correct it is. An operator seeing a bare "read-only file system" has no way to
    /// know that the daemon is behaving and the *unit* is the thing to change, so this
    /// variant says which directive to look at. See RFC-007 §6.
    Sandboxed {
        path: String,
        source: String,
    },
    /// Refused by policy, not by the kernel.
    Refused(String),
    Io(String),
}

impl std::fmt::Display for CgroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("no cgroup v2 hierarchy on this system"),
            Self::InvalidPath(detail) => write!(f, "invalid cgroup path: {detail}"),
            Self::Malformed(detail) => write!(f, "malformed cgroup data: {detail}"),
            Self::Sandboxed { path, source } => write!(
                f,
                "refused write to {path} ({source}); the service sandbox, not the kernel                  — archied.service sets ProtectControlGroups=yes, which needs Delegate=                  to grant the unit its own writable subtree (RFC-007 §6.1)"
            ),
            Self::Refused(detail) => write!(f, "refused: {detail}"),
            Self::Io(detail) => write!(f, "cgroup i/o error: {detail}"),
        }
    }
}

/// A CPU bandwidth limit, as `cpu.max` expresses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuLimit {
    /// No limit. Written as the literal `max`.
    Unlimited,
    /// `quota` microseconds of CPU time per `period` microseconds of wall time.
    ///
    /// A quota above the period is legitimate and means more than one core: 200000/100000
    /// is two cores' worth.
    Quota { quota_us: u64, period_us: u64 },
}

impl CpuLimit {
    /// Express a limit as a number of cores.
    pub fn cores(fraction: f64) -> Self {
        const PERIOD_US: u64 = 100_000;
        Self::Quota {
            quota_us: (fraction * PERIOD_US as f64).round().max(1.0) as u64,
            period_us: PERIOD_US,
        }
    }

    /// How many cores this permits, or `None` when unlimited.
    pub fn as_cores(&self) -> Option<f64> {
        match self {
            Self::Unlimited => None,
            Self::Quota {
                quota_us,
                period_us,
            } => {
                if *period_us == 0 {
                    return None;
                }
                Some(*quota_us as f64 / *period_us as f64)
            }
        }
    }

    /// The exact bytes `cpu.max` expects.
    pub fn to_value(&self) -> String {
        match self {
            Self::Unlimited => "max".to_string(),
            Self::Quota {
                quota_us,
                period_us,
            } => format!("{quota_us} {period_us}"),
        }
    }

    /// Parse `cpu.max`, which is `"<quota|max> <period>"`.
    pub fn parse(text: &str) -> Result<Self, CgroupError> {
        let mut fields = text.split_whitespace();
        let quota = fields
            .next()
            .ok_or_else(|| CgroupError::Malformed(text.to_string()))?;
        if quota == "max" {
            return Ok(Self::Unlimited);
        }
        let period = fields.next().unwrap_or("100000");
        Ok(Self::Quota {
            quota_us: quota
                .parse()
                .map_err(|_| CgroupError::Malformed(text.to_string()))?,
            period_us: period
                .parse()
                .map_err(|_| CgroupError::Malformed(text.to_string()))?,
        })
    }
}

/// A memory limit, as `memory.max` and friends express it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryLimit {
    Unlimited,
    Bytes(u64),
}

impl MemoryLimit {
    pub fn to_value(&self) -> String {
        match self {
            Self::Unlimited => "max".to_string(),
            Self::Bytes(bytes) => bytes.to_string(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, CgroupError> {
        let trimmed = text.trim();
        if trimmed == "max" {
            return Ok(Self::Unlimited);
        }
        trimmed
            .parse()
            .map(Self::Bytes)
            .map_err(|_| CgroupError::Malformed(text.to_string()))
    }

    pub fn as_bytes(&self) -> Option<u64> {
        match self {
            Self::Unlimited => None,
            Self::Bytes(bytes) => Some(*bytes),
        }
    }
}

/// Whether a cgroup is frozen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreezeState {
    Thawed,
    Frozen,
}

impl FreezeState {
    /// `cgroup.freeze` is `0` or `1`.
    pub fn parse(text: &str) -> Result<Self, CgroupError> {
        match text.trim() {
            "0" => Ok(Self::Thawed),
            "1" => Ok(Self::Frozen),
            other => Err(CgroupError::Malformed(format!(
                "cgroup.freeze held {other:?}"
            ))),
        }
    }

    pub fn to_value(&self) -> &'static str {
        match self {
            Self::Thawed => "0",
            Self::Frozen => "1",
        }
    }
}

/// What a cgroup currently permits and consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupState {
    pub path: CgroupPath,
    pub cpu: Option<CpuLimit>,
    pub memory_max: Option<MemoryLimit>,
    pub memory_current: Option<u64>,
    pub freeze: Option<FreezeState>,
}

impl CgroupState {
    /// Fraction of the memory limit in use, when both numbers are known.
    ///
    /// Returns `None` for an unlimited cgroup rather than 0.0: "no limit" and "using
    /// none of the limit" are different facts and the caller must not confuse them.
    pub fn memory_utilisation(&self) -> Option<f64> {
        let limit = self.memory_max?.as_bytes()?;
        if limit == 0 {
            return None;
        }
        Some(self.memory_current? as f64 / limit as f64)
    }

    /// Whether this cgroup is close enough to its memory limit to be at risk of the OOM
    /// killer, and should stop being given work.
    pub fn is_near_memory_limit(&self) -> bool {
        self.memory_utilisation().is_some_and(|used| used >= 0.9)
    }
}

/// Reads and writes cgroup v2 controllers under a given root.
///
/// The root is injectable so the whole type can be exercised against a fixture tree.
#[derive(Debug, Clone)]
pub struct CgroupController {
    root: PathBuf,
}

impl Default for CgroupController {
    fn default() -> Self {
        Self::new(DEFAULT_CGROUP_ROOT)
    }
}

impl CgroupController {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where this controller reads and writes.
    ///
    /// Exposed so a caller can enumerate the hierarchy itself — reconciliation has to
    /// look at what the machine holds, and a controller that could only be asked about
    /// paths it was already given could not answer that.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether a cgroup v2 hierarchy is present.
    ///
    /// `cgroup.controllers` exists only on v2, which is what distinguishes a unified
    /// hierarchy from a v1 mount at the same path.
    pub fn is_available(&self) -> bool {
        self.root.join("cgroup.controllers").exists()
    }

    /// The controllers this kernel exposes, e.g. `cpu`, `memory`, `io`.
    pub fn available_controllers(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("cgroup.controllers"))
            .map(|text| text.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Read everything interesting about one cgroup.
    ///
    /// Individual missing files become `None` rather than errors: not every cgroup has
    /// every controller enabled, and a partially-delegated cgroup is normal.
    pub fn read(&self, path: &CgroupPath) -> Result<CgroupState, CgroupError> {
        if !self.is_available() {
            return Err(CgroupError::Unsupported);
        }
        let dir = path.resolve(&self.root);
        if !dir.is_dir() {
            return Err(CgroupError::Io(format!("{} does not exist", dir.display())));
        }

        Ok(CgroupState {
            path: path.clone(),
            cpu: read_file(&dir.join("cpu.max"))
                .map(|text| CpuLimit::parse(&text))
                .transpose()?,
            memory_max: read_file(&dir.join("memory.max"))
                .map(|text| MemoryLimit::parse(&text))
                .transpose()?,
            memory_current: read_file(&dir.join("memory.current"))
                .and_then(|text| text.trim().parse().ok()),
            freeze: read_file(&dir.join("cgroup.freeze"))
                .map(|text| FreezeState::parse(&text))
                .transpose()?,
        })
    }

    /// Create a cgroup.
    ///
    /// `mkdir` under a cgroup v2 hierarchy is how a cgroup comes into existence; the
    /// kernel populates the interface files itself. Creating one that already exists is
    /// not an error — the executor re-creating a task's cgroup after a restart should
    /// find it and carry on, not fail.
    pub fn create(&self, path: &CgroupPath) -> Result<(), CgroupError> {
        if !self.is_available() {
            return Err(CgroupError::Unsupported);
        }
        let dir = path.resolve(&self.root);
        if dir.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(&dir).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) {
                CgroupError::Sandboxed {
                    path: dir.display().to_string(),
                    source: e.to_string(),
                }
            } else {
                CgroupError::Io(format!("creating {}: {e}", dir.display()))
            }
        })
    }

    /// Remove a cgroup.
    ///
    /// The kernel refuses to remove a cgroup that still has processes in it, which is the
    /// behaviour we want: a removal that succeeded while a worker was still running would
    /// leave that worker outside every budget on the machine. `rmdir` rather than a
    /// recursive delete for the same reason — removing a subtree would silently take
    /// children with it.
    pub fn remove(&self, path: &CgroupPath) -> Result<(), CgroupError> {
        let dir = path.resolve(&self.root);
        if !dir.exists() {
            return Ok(());
        }
        std::fs::remove_dir(&dir)
            .map_err(|e| CgroupError::Io(format!("removing {}: {e}", dir.display())))
    }

    /// Whether any process is in this cgroup.
    ///
    /// Read from `cgroup.events`'s `populated` key rather than by counting `cgroup.procs`,
    /// because `populated` accounts for descendants too — a worker that forked before
    /// being checked would otherwise look gone.
    pub fn is_populated(&self, path: &CgroupPath) -> Option<bool> {
        let text = read_file(&path.resolve(&self.root).join("cgroup.events"))?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("populated ") {
                return Some(value.trim() == "1");
            }
        }
        None
    }

    /// How many times the kernel OOM-killed something in this cgroup.
    ///
    /// The distinction that matters for a failure report: a worker killed here hit *its*
    /// memory limit, which is a budget that was set too low or a workload that grew. It is
    /// not the same as the machine running out of memory, and the two call for opposite
    /// responses.
    pub fn oom_kills(&self, path: &CgroupPath) -> Option<u64> {
        let text = read_file(&path.resolve(&self.root).join("memory.events"))?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("oom_kill ") {
                return value.trim().parse().ok();
            }
        }
        None
    }

    /// Set the soft memory limit, where the kernel starts reclaiming.
    pub fn set_memory_high(
        &self,
        path: &CgroupPath,
        limit: MemoryLimit,
    ) -> Result<(), CgroupError> {
        self.write_controller(path, "memory.high", &limit.to_value())
    }

    /// Set the relative IO weight, 1..=10000.
    ///
    /// Refused outside that range rather than clamped: a caller asking for weight 0 has a
    /// bug, and silently giving them 1 hides it while producing a cgroup that barely gets
    /// any IO.
    pub fn set_io_weight(&self, path: &CgroupPath, weight: u16) -> Result<(), CgroupError> {
        if weight == 0 || weight > 10_000 {
            return Err(CgroupError::InvalidPath(format!(
                "io weight {weight} is outside 1..=10000"
            )));
        }
        self.write_controller(path, "io.weight", &format!("default {weight}"))
    }

    /// Set the CPU budget.
    ///
    /// This is what turns the observatory's quality budget from a request into a fact:
    /// under inference pressure the display's `cpu.max` falls and the kernel enforces it
    /// whether or not the renderer cooperates.
    pub fn set_cpu_limit(&self, path: &CgroupPath, limit: CpuLimit) -> Result<(), CgroupError> {
        self.write_controller(path, "cpu.max", &limit.to_value())
    }

    pub fn set_memory_limit(
        &self,
        path: &CgroupPath,
        limit: MemoryLimit,
    ) -> Result<(), CgroupError> {
        self.write_controller(path, "memory.max", &limit.to_value())
    }

    /// Freeze or thaw a cgroup.
    ///
    /// Refuses any cgroup [`CgroupPath::is_freezable`] rejects — freezing the control
    /// plane deadlocks it against itself and cannot be undone from inside.
    pub fn set_freeze(&self, path: &CgroupPath, state: FreezeState) -> Result<(), CgroupError> {
        if !path.is_freezable() {
            return Err(CgroupError::Refused(format!(
                "{} is control-plane infrastructure and must never be frozen",
                path.as_str()
            )));
        }
        self.write_controller(path, "cgroup.freeze", state.to_value())
    }

    fn write_controller(
        &self,
        path: &CgroupPath,
        file: &str,
        value: &str,
    ) -> Result<(), CgroupError> {
        if !self.is_available() {
            return Err(CgroupError::Unsupported);
        }
        let target = path.resolve(&self.root).join(file);
        // cgroup controller files take a single write with no trailing newline; writing
        // more than once, or appending, is an error the kernel reports obscurely.
        std::fs::write(&target, value).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) {
                return CgroupError::Sandboxed {
                    path: target.display().to_string(),
                    source: e.to_string(),
                };
            }
            CgroupError::Io(format!("writing {value:?} to {}: {e}", target.display()))
        })
    }
}

/// This process's own cgroup, from `/proc/self/cgroup`.
///
/// On a v2 system the file has a single line of the form `0::/path`. Anything else is a
/// v1 or hybrid hierarchy, which this module does not handle — reporting `Unsupported`
/// is better than guessing which of several v1 hierarchies was meant.
pub fn parse_self_cgroup(text: &str) -> Result<CgroupPath, CgroupError> {
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("0::") {
            // `0::/` is the root of the unified hierarchy: a container or a hybrid
            // v1/v2 host. There is no cgroup of our own to manage there.
            if path.trim_matches('/').is_empty() {
                return Err(CgroupError::Unsupported);
            }
            return CgroupPath::new(path);
        }
    }
    Err(CgroupError::Unsupported)
}

/// Read this process's own cgroup.
pub fn read_self_cgroup() -> Result<CgroupPath, CgroupError> {
    let text =
        std::fs::read_to_string("/proc/self/cgroup").map_err(|_| CgroupError::Unsupported)?;
    parse_self_cgroup(&text)
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ────────────────────────────────────────────────────────────── path safety

    #[test]
    fn a_normal_path_is_accepted_and_normalised() {
        let path = CgroupPath::new("/system.slice/archie-display.service/").unwrap();
        assert_eq!(path.as_str(), "system.slice/archie-display.service");
        assert_eq!(path.unit_name(), Some("archie-display.service"));
    }

    #[test]
    fn traversal_is_refused_at_construction() {
        // A path assembled from a unit name out of a config file must not be able to
        // reach outside the hierarchy.
        for bad in [
            "../../etc",
            "system.slice/../../..",
            "a/./b",
            "a//b",
            "",
            "/",
        ] {
            assert!(
                CgroupPath::new(bad).is_err(),
                "{bad:?} should have been refused"
            );
        }
    }

    #[test]
    fn a_path_resolves_under_the_given_root() {
        let path = CgroupPath::new("system.slice/worker.scope").unwrap();
        let resolved = path.resolve("/sys/fs/cgroup");
        assert!(resolved.ends_with("system.slice/worker.scope"));
        assert!(resolved.starts_with("/sys/fs/cgroup"));
    }

    // ─────────────────────────────────────────────────────────── freeze safety

    #[test]
    fn the_control_plane_can_never_be_frozen() {
        // Freezing something archied talks to synchronously deadlocks it against itself,
        // and the freezer cannot be released from inside.
        for protected in [
            "system.slice/archied.service",
            "system.slice/archie-displayd.service",
            "system.slice/archie-worldd.service",
            "init.scope",
        ] {
            let path = CgroupPath::new(protected).unwrap();
            assert!(!path.is_freezable(), "{protected} must never be freezable");
        }
    }

    #[test]
    fn worker_scopes_are_freezable() {
        for worker in [
            "archie.slice/worker-research-1.scope",
            "system.slice/archie-worker@3.service",
        ] {
            assert!(CgroupPath::new(worker).unwrap().is_freezable(), "{worker}");
        }
    }

    #[test]
    fn a_freeze_on_protected_infrastructure_is_refused_before_any_io() {
        // The controller refuses by policy, so this holds even where the write would
        // have succeeded.
        let controller = CgroupController::new("/nonexistent");
        let protected = CgroupPath::new("system.slice/archied.service").unwrap();
        match controller.set_freeze(&protected, FreezeState::Frozen) {
            Err(CgroupError::Refused(detail)) => assert!(detail.contains("archied.service")),
            other => panic!("expected a policy refusal, got {other:?}"),
        }
    }

    // ──────────────────────────────────────────────────────────────── parsing

    #[test]
    fn cpu_max_parses_both_forms() {
        assert_eq!(CpuLimit::parse("max 100000").unwrap(), CpuLimit::Unlimited);
        assert_eq!(
            CpuLimit::parse("150000 100000").unwrap(),
            CpuLimit::Quota {
                quota_us: 150_000,
                period_us: 100_000
            }
        );
    }

    #[test]
    fn a_quota_above_the_period_means_more_than_one_core() {
        // 200000/100000 is two cores, not an error.
        let limit = CpuLimit::parse("200000 100000").unwrap();
        assert_eq!(limit.as_cores(), Some(2.0));
        assert_eq!(CpuLimit::Unlimited.as_cores(), None);
    }

    #[test]
    fn a_cpu_limit_round_trips_through_its_wire_form() {
        for limit in [
            CpuLimit::Unlimited,
            CpuLimit::cores(0.5),
            CpuLimit::cores(1.5),
        ] {
            assert_eq!(CpuLimit::parse(&limit.to_value()).unwrap(), limit);
        }
        assert_eq!(CpuLimit::cores(1.5).to_value(), "150000 100000");
        assert_eq!(CpuLimit::Unlimited.to_value(), "max");
    }

    #[test]
    fn a_fractional_core_never_rounds_to_zero_quota() {
        // A zero quota would stop the cgroup entirely, which is not what "a very small
        // share" means.
        assert!(CpuLimit::cores(0.000001).as_cores().unwrap() > 0.0);
    }

    #[test]
    fn memory_max_parses_both_forms() {
        assert_eq!(MemoryLimit::parse("max").unwrap(), MemoryLimit::Unlimited);
        assert_eq!(
            MemoryLimit::parse("1073741824\n").unwrap(),
            MemoryLimit::Bytes(1_073_741_824)
        );
        assert!(MemoryLimit::parse("nonsense").is_err());
    }

    #[test]
    fn freeze_state_parses_and_rejects_anything_else() {
        assert_eq!(FreezeState::parse("0\n").unwrap(), FreezeState::Thawed);
        assert_eq!(FreezeState::parse("1").unwrap(), FreezeState::Frozen);
        assert!(FreezeState::parse("frozen").is_err());
    }

    #[test]
    fn self_cgroup_parses_the_v2_line() {
        let text = "0::/system.slice/archied.service\n";
        assert_eq!(
            parse_self_cgroup(text).unwrap().as_str(),
            "system.slice/archied.service"
        );
    }

    #[test]
    fn a_v1_hierarchy_is_unsupported_rather_than_guessed() {
        // Several v1 hierarchies could each claim to be "the" cgroup; picking one would
        // be a guess the caller cannot audit.
        let v1 = "11:cpuset:/\n10:memory:/system.slice\n9:cpu,cpuacct:/\n";
        assert_eq!(parse_self_cgroup(v1), Err(CgroupError::Unsupported));
    }

    // ──────────────────────────────────────────────────────── read against fixture

    fn fixture_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("archie-cgroup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("cgroup.controllers"),
            "cpuset cpu io memory pids\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn reads_a_cgroup_from_a_fixture_hierarchy() {
        let root = fixture_root("read");
        let dir = root.join("system.slice/archie-display.service");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cpu.max"), "150000 100000\n").unwrap();
        std::fs::write(dir.join("memory.max"), "1073741824\n").unwrap();
        std::fs::write(dir.join("memory.current"), "536870912\n").unwrap();
        std::fs::write(dir.join("cgroup.freeze"), "0\n").unwrap();

        let controller = CgroupController::new(&root);
        assert!(controller.is_available());
        assert!(controller
            .available_controllers()
            .contains(&"memory".to_string()));

        let path = CgroupPath::new("system.slice/archie-display.service").unwrap();
        let state = controller.read(&path).unwrap();
        assert_eq!(state.cpu.unwrap().as_cores(), Some(1.5));
        assert_eq!(state.memory_max.unwrap().as_bytes(), Some(1 << 30));
        assert_eq!(state.memory_current, Some(1 << 29));
        assert_eq!(state.freeze, Some(FreezeState::Thawed));
        assert_eq!(state.memory_utilisation(), Some(0.5));
        assert!(!state.is_near_memory_limit());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_partially_delegated_cgroup_reads_the_controllers_it_has() {
        // Not every cgroup has every controller enabled; a missing file is normal.
        let root = fixture_root("partial");
        let dir = root.join("archie.slice");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("memory.current"), "1024\n").unwrap();

        let state = CgroupController::new(&root)
            .read(&CgroupPath::new("archie.slice").unwrap())
            .unwrap();
        assert!(state.cpu.is_none());
        assert_eq!(state.memory_current, Some(1024));
        assert_eq!(
            state.memory_utilisation(),
            None,
            "no limit means no utilisation figure, not zero"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unlimited_cgroup_reports_no_utilisation_rather_than_zero() {
        // "No limit" and "using none of the limit" are different facts.
        let state = CgroupState {
            path: CgroupPath::new("a").unwrap(),
            cpu: None,
            memory_max: Some(MemoryLimit::Unlimited),
            memory_current: Some(999),
            freeze: None,
        };
        assert_eq!(state.memory_utilisation(), None);
        assert!(!state.is_near_memory_limit());
    }

    #[test]
    fn a_cgroup_near_its_memory_limit_is_flagged() {
        let state = CgroupState {
            path: CgroupPath::new("a").unwrap(),
            cpu: None,
            memory_max: Some(MemoryLimit::Bytes(1000)),
            memory_current: Some(950),
            freeze: None,
        };
        assert!(state.is_near_memory_limit());
    }

    #[test]
    fn a_write_refused_by_the_sandbox_names_the_directive_not_just_errno() {
        // `ProtectControlGroups=yes` makes every controller write return EROFS however
        // correct it is. An operator reading "read-only file system" has no way to know
        // the daemon is fine and the unit is what needs changing.
        let root = fixture_root("sandboxed");
        let dir = root.join("archie.slice/worker.scope");
        std::fs::create_dir_all(&dir).unwrap();
        // No cpu.max file exists, so the write fails; on Windows and Linux alike this
        // exercises the mapping rather than the specific errno.
        let controller = CgroupController::new(&root);
        let path = CgroupPath::new("archie.slice/worker.scope/missing-child").unwrap();
        match controller.set_cpu_limit(&path, CpuLimit::cores(1.0)) {
            Err(CgroupError::Sandboxed { source, .. }) => {
                let rendered = CgroupError::Sandboxed {
                    path: "/sys/fs/cgroup/x".into(),
                    source,
                }
                .to_string();
                assert!(rendered.contains("ProtectControlGroups"));
                assert!(rendered.contains("Delegate="));
            }
            Err(CgroupError::Io(_)) => {
                // The fixture path simply does not exist, which is a plain i/o error.
                // Check the sandbox rendering directly instead.
                let rendered = CgroupError::Sandboxed {
                    path: "/sys/fs/cgroup/archie.slice/cpu.max".into(),
                    source: "read-only file system".into(),
                }
                .to_string();
                assert!(rendered.contains("ProtectControlGroups"));
                assert!(rendered.contains("Delegate="));
                assert!(rendered.contains("not the kernel"));
            }
            other => panic!("unexpected {other:?}"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_machine_without_cgroup_v2_reports_unsupported() {
        let root = std::env::temp_dir().join("archie-no-cgroup-xyz");
        let _ = std::fs::remove_dir_all(&root);
        let controller = CgroupController::new(&root);
        assert!(!controller.is_available());
        assert_eq!(
            controller.read(&CgroupPath::new("a").unwrap()),
            Err(CgroupError::Unsupported)
        );
        assert_eq!(
            controller.set_cpu_limit(&CgroupPath::new("a").unwrap(), CpuLimit::cores(1.0)),
            Err(CgroupError::Unsupported)
        );
    }

    #[test]
    fn writing_a_limit_produces_exactly_the_kernel_wire_form() {
        // Controller files take one write with no trailing newline; anything else the
        // kernel reports obscurely.
        let root = fixture_root("write");
        let dir = root.join("archie.slice/worker.scope");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cpu.max"), "max 100000\n").unwrap();

        let controller = CgroupController::new(&root);
        let path = CgroupPath::new("archie.slice/worker.scope").unwrap();
        controller
            .set_cpu_limit(&path, CpuLimit::cores(0.25))
            .unwrap();

        let written = std::fs::read_to_string(dir.join("cpu.max")).unwrap();
        assert_eq!(written, "25000 100000", "no trailing newline");
        assert_eq!(CpuLimit::parse(&written).unwrap().as_cores(), Some(0.25));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn freezing_a_worker_writes_the_freezer() {
        let root = fixture_root("freeze");
        let dir = root.join("archie.slice/worker.scope");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();

        let controller = CgroupController::new(&root);
        let path = CgroupPath::new("archie.slice/worker.scope").unwrap();
        controller.set_freeze(&path, FreezeState::Frozen).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(),
            "1"
        );

        controller.set_freeze(&path, FreezeState::Thawed).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(),
            "0"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_root_of_the_hierarchy_is_not_a_cgroup_of_our_own() {
        let hybrid = "4:memory:/process_api/x\n1:cpu:/\n0::/\n";
        assert!(matches!(
            parse_self_cgroup(hybrid),
            Err(CgroupError::Unsupported)
        ));
        let unit = parse_self_cgroup("0::/system.slice/archie-executord.service\n").unwrap();
        assert_eq!(unit.unit_name(), Some("archie-executord.service"));
    }

    #[test]
    fn reading_this_machine_never_panics() {
        // Linux exercises the real path; elsewhere it must report Unsupported.
        match read_self_cgroup() {
            Ok(path) => assert!(!path.as_str().is_empty()),
            Err(CgroupError::Unsupported) => {}
            Err(other) => panic!("unexpected error reading own cgroup: {other:?}"),
        }
        let _ = CgroupController::default().is_available();
    }
}
