//! Per-task cgroups, inside the executor's own delegated subtree and nowhere else.
//!
//! RFC-007 §7, step 4. This is where the cgroup writer from §3.2 stops being inert.
//!
//! # The delegated subtree is the whole security argument
//!
//! `archied` is hardened with an empty capability bounding set and `ProtectControlGroups=`,
//! and it stays that way. The executor gets `Delegate=` instead, which gives it write
//! access to *its own* cgroup and everything below it — and to nothing above it.
//!
//! That boundary is only as good as the code that respects it. A path assembled from a
//! task id, a config value, or anything that ever touched a network is one `..` away from
//! `/sys/fs/cgroup/system.slice/archied.service`, and writing `cgroup.freeze` there would
//! stop the control plane with no way to restart it.
//!
//! So every path this module produces is built from the delegated root and a validated
//! leaf name, and [`TaskCgroups::contains`] is checked before any write. Belt and braces
//! on purpose: the construction should make escape impossible, and the check catches the
//! day it does not.
//!
//! # Only leaves are frozen
//!
//! The freezer applies to a cgroup and all its descendants. Freezing the delegated root
//! would stop every task at once, including any the control plane is synchronously
//! waiting on — and the freezer cannot be released from inside. Task cgroups are leaves,
//! and freezing anything else is refused by construction.

use archie_execution::spec::ResourceBudget;
use archie_observability::kernel::cgroup::{
    CgroupController, CgroupError, CgroupPath, CpuLimit, FreezeState, MemoryLimit,
};
use archie_types::TaskId;

use crate::recovery::DiscoveredCgroup;

/// Why a cgroup operation was refused before it reached the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupRefusal {
    /// The path is not inside the executor's delegated subtree.
    OutsideDelegatedSubtree { path: String, root: String },
    /// The delegated root itself, or an ancestor, was targeted.
    NotALeaf(String),
    /// This node has no delegated subtree, so nothing here can be enforced.
    NoDelegation,
    /// The kernel refused.
    Kernel(CgroupError),
}

impl std::fmt::Display for CgroupRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideDelegatedSubtree { path, root } => write!(
                f,
                "{path} is outside the executor's delegated subtree {root}; the executor \
                 may only manage cgroups it owns"
            ),
            Self::NotALeaf(path) => write!(
                f,
                "{path} is the delegated root or an ancestor; freezing or removing it \
                 would take every task with it, and the freezer cannot be released from \
                 inside"
            ),
            Self::NoDelegation => f.write_str(
                "this node has no delegated cgroup subtree, so resource limits would be \
                 advisory; configure Delegate= on archie-executord.service (RFC-007 §6.1)",
            ),
            Self::Kernel(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CgroupRefusal {}

impl From<CgroupError> for CgroupRefusal {
    fn from(e: CgroupError) -> Self {
        Self::Kernel(e)
    }
}

/// What the kernel says about a task's cgroup right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskCgroupState {
    /// Whether any process is in it, including descendants.
    pub populated: bool,
    pub frozen: bool,
    /// How many times the kernel OOM-killed something here. Distinct from the machine
    /// running out of memory: this one means the task's own budget was too small.
    pub oom_kills: u64,
}

/// Creates and manages one cgroup per task, under a delegated root.
#[derive(Debug, Clone)]
pub struct TaskCgroups {
    controller: CgroupController,
    /// The executor's own cgroup, delegated to it by systemd. Every task cgroup is a
    /// direct child of this and of nothing else.
    delegated_root: Option<CgroupPath>,
}

impl TaskCgroups {
    /// Build with an explicit delegated root.
    pub fn new(controller: CgroupController, delegated_root: CgroupPath) -> Self {
        Self {
            controller,
            delegated_root: Some(delegated_root),
        }
    }

    /// A manager with no delegation — every operation refuses.
    ///
    /// This is what a development host and an unconfigured node both produce, and they
    /// are deliberately indistinguishable: neither should be mistaken for one where
    /// limits are enforced.
    pub fn undelegated(controller: CgroupController) -> Self {
        Self {
            controller,
            delegated_root: None,
        }
    }

    /// Whether this node can actually enforce anything.
    pub fn is_delegated(&self) -> bool {
        self.delegated_root.is_some() && self.controller.is_available()
    }

    pub fn delegated_root(&self) -> Option<&CgroupPath> {
        self.delegated_root.as_ref()
    }

    /// The cgroup path for one task.
    ///
    /// Built from the root and a name derived from the task id, never from caller input.
    /// The id is a UUID, so the leaf name cannot contain a separator or a traversal — but
    /// [`Self::check_containment`] verifies the result anyway, because the day someone
    /// changes what a task id is should be the day a test fails, not the day the control
    /// plane freezes.
    pub fn path_for(&self, task_id: &TaskId) -> Result<CgroupPath, CgroupRefusal> {
        let root = self
            .delegated_root
            .as_ref()
            .ok_or(CgroupRefusal::NoDelegation)?;
        let leaf = format!("{}/task-{}", root.as_str(), task_id.as_uuid());
        let path = CgroupPath::new(leaf).map_err(CgroupRefusal::Kernel)?;
        self.check_containment(&path)?;
        Ok(path)
    }

    /// Whether a path is strictly inside the delegated subtree.
    ///
    /// Strictly: the root itself is not contained in itself for these purposes, because
    /// every operation here either writes limits or freezes, and doing either to the root
    /// affects every task at once.
    pub fn contains(&self, path: &CgroupPath) -> bool {
        let Some(root) = &self.delegated_root else {
            return false;
        };
        let root = root.as_str();
        let candidate = path.as_str();
        candidate.len() > root.len()
            && candidate.starts_with(root)
            // The separator check is what stops `archie.slice-evil` matching the root
            // `archie.slice` by prefix alone.
            && candidate.as_bytes()[root.len()] == b'/'
    }

    fn check_containment(&self, path: &CgroupPath) -> Result<(), CgroupRefusal> {
        let Some(root) = &self.delegated_root else {
            return Err(CgroupRefusal::NoDelegation);
        };
        if path.as_str() == root.as_str() {
            return Err(CgroupRefusal::NotALeaf(path.as_str().to_string()));
        }
        if !self.contains(path) {
            return Err(CgroupRefusal::OutsideDelegatedSubtree {
                path: path.as_str().to_string(),
                root: root.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// Create a task's cgroup and write its budget.
    ///
    /// The budget is written *before* the worker is placed in the cgroup. The other order
    /// leaves a window in which the worker is running with no limit at all, and a worker
    /// that allocates hard on startup is exactly the one whose limit matters.
    pub fn create(
        &self,
        task_id: &TaskId,
        budget: &ResourceBudget,
    ) -> Result<CgroupPath, CgroupRefusal> {
        let path = self.path_for(task_id)?;
        self.controller.create(&path)?;
        self.apply_budget(&path, budget)?;
        Ok(path)
    }

    /// Write a budget into an existing task cgroup.
    ///
    /// Also the resource governor's lever: a running task's budget can change without
    /// restarting it.
    pub fn apply_budget(
        &self,
        path: &CgroupPath,
        budget: &ResourceBudget,
    ) -> Result<(), CgroupRefusal> {
        self.check_containment(path)?;

        self.controller
            .set_cpu_limit(path, CpuLimit::cores(budget.cpu_cores))?;
        // High before max. Written the other way round, a shrinking budget would briefly
        // have `memory.high` above `memory.max`, and the kernel would kill before reclaim
        // was ever attempted.
        self.controller
            .set_memory_high(path, MemoryLimit::Bytes(budget.memory_high_bytes))?;
        self.controller
            .set_memory_limit(path, MemoryLimit::Bytes(budget.memory_max_bytes))?;
        self.controller.set_io_weight(path, budget.io_weight)?;
        Ok(())
    }

    /// Freeze a task. Only ever a leaf inside the delegated subtree.
    pub fn freeze(&self, path: &CgroupPath) -> Result<(), CgroupRefusal> {
        self.check_containment(path)?;
        self.controller
            .set_freeze(path, FreezeState::Frozen)
            .map_err(CgroupRefusal::Kernel)
    }

    pub fn thaw(&self, path: &CgroupPath) -> Result<(), CgroupRefusal> {
        self.check_containment(path)?;
        self.controller
            .set_freeze(path, FreezeState::Thawed)
            .map_err(CgroupRefusal::Kernel)
    }

    /// What the kernel says about this task's cgroup.
    pub fn state(&self, path: &CgroupPath) -> Result<TaskCgroupState, CgroupRefusal> {
        self.check_containment(path)?;
        let read = self.controller.read(path)?;
        Ok(TaskCgroupState {
            populated: self.controller.is_populated(path).unwrap_or(false),
            frozen: read.freeze == Some(FreezeState::Frozen),
            oom_kills: self.controller.oom_kills(path).unwrap_or(0),
        })
    }

    /// Every task cgroup that exists in the delegated subtree.
    ///
    /// This is the other half of reconciliation. The executor's records say what it
    /// believes; this says what the machine actually has, and the two are compared
    /// without either being assumed correct.
    ///
    /// Scanned rather than remembered on purpose: after a crash the records are exactly
    /// what cannot be trusted, and a list rebuilt from them would agree with itself no
    /// matter what the kernel held.
    pub fn discover(&self) -> Result<Vec<DiscoveredCgroup>, CgroupRefusal> {
        let root = self
            .delegated_root
            .as_ref()
            .ok_or(CgroupRefusal::NoDelegation)?;

        let dir = root.resolve(self.controller.root());
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // The subtree not existing is not a failure to discover: it means no task has
            // ever been started here, and reporting nothing found is the truthful answer.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(CgroupRefusal::Kernel(CgroupError::Io(format!(
                    "reading {}: {e}",
                    dir.display()
                ))))
            }
        };

        let mut found = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            // Only cgroups this executor named. A directory it did not create is not a
            // task, and treating one as an orphaned task would raise an alarm about
            // something that was never ours.
            let Some(task_id) = task_id_from(&name) else {
                continue;
            };

            let path = match CgroupPath::new(format!("{}/{name}", root.as_str())) {
                Ok(path) => path,
                Err(_) => continue,
            };
            let state = self.state(&path)?;

            found.push(DiscoveredCgroup {
                task_id,
                populated: state.populated,
                frozen: state.frozen,
            });
        }

        // Sorted so a reconciliation report reads the same twice. Directory order is a
        // filesystem detail, and an operator comparing two runs should not have to work
        // out whether the list changed or merely moved.
        found.sort_by_key(|c| c.task_id);
        Ok(found)
    }

    /// Remove a finished task's cgroup.
    ///
    /// The kernel refuses while processes remain, which is the behaviour we want: a
    /// removal that succeeded with a worker still inside would leave that worker outside
    /// every budget on the machine.
    pub fn remove(&self, path: &CgroupPath) -> Result<(), CgroupRefusal> {
        self.check_containment(path)?;
        self.controller.remove(path).map_err(CgroupRefusal::Kernel)
    }
}

/// Recover a task id from a cgroup name this executor created.
///
/// The inverse of `path_for`, and deliberately strict: anything that is not exactly the
/// shape we write is not ours. A loose parse would turn a stray directory into an
/// orphaned task and raise an alarm about something nobody here created.
fn task_id_from(name: &str) -> Option<TaskId> {
    let uuid = name.strip_prefix("task-")?;
    uuid.parse().ok().map(TaskId::from_uuid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("archie-taskcg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("cgroup.controllers"), "cpu io memory pids\n").unwrap();
        root
    }

    fn delegated(tag: &str) -> (PathBuf, TaskCgroups) {
        let root = fixture_root(tag);
        let delegated_path = CgroupPath::new("system.slice/archie-executord.service").unwrap();
        std::fs::create_dir_all(delegated_path.resolve(&root)).unwrap();
        let manager = TaskCgroups::new(CgroupController::new(&root), delegated_path);
        (root, manager)
    }

    #[test]
    fn a_task_cgroup_is_a_child_of_the_delegated_root() {
        let (root, manager) = delegated("child");
        let task = TaskId::new();
        let path = manager.path_for(&task).unwrap();

        assert!(path
            .as_str()
            .starts_with("system.slice/archie-executord.service/"));
        assert!(path.as_str().contains(&task.as_uuid().to_string()));
        assert!(manager.contains(&path));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_delegated_root_itself_is_never_a_valid_target() {
        // Every operation here writes limits or freezes, and doing either to the root
        // hits every task at once — including any the control plane is waiting on.
        let (root, manager) = delegated("rootself");
        let root_path = CgroupPath::new("system.slice/archie-executord.service").unwrap();

        assert!(!manager.contains(&root_path));
        match manager.freeze(&root_path) {
            Err(CgroupRefusal::NotALeaf(p)) => assert!(p.contains("executord")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(manager
            .freeze(&root_path)
            .unwrap_err()
            .to_string()
            .contains("cannot be released from inside"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nothing_outside_the_subtree_can_be_touched() {
        // The failure this module exists to prevent: writing cgroup.freeze to the
        // control plane's own cgroup, which stops it with no way to restart it.
        let (root, manager) = delegated("outside");
        let archied = CgroupPath::new("system.slice/archied.service").unwrap();

        assert!(!manager.contains(&archied));
        for result in [
            manager.freeze(&archied),
            manager.thaw(&archied),
            manager.remove(&archied),
            manager.apply_budget(&archied, &ResourceBudget::default()),
        ] {
            match result {
                Err(CgroupRefusal::OutsideDelegatedSubtree { path, .. }) => {
                    assert!(path.contains("archied.service"))
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_sibling_sharing_a_prefix_is_not_inside() {
        // `archie-executord.service-evil` starts with the root's characters but is a
        // different cgroup entirely. Prefix matching alone would have let it through.
        let (root, manager) = delegated("prefix");
        let lookalike =
            CgroupPath::new("system.slice/archie-executord.service-evil/task-1").unwrap();
        assert!(
            !manager.contains(&lookalike),
            "containment needs the separator, not just a matching prefix"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_ancestor_is_not_inside_either() {
        let (root, manager) = delegated("ancestor");
        for ancestor in ["system.slice", "system.slice/other.service"] {
            let path = CgroupPath::new(ancestor).unwrap();
            assert!(!manager.contains(&path), "{ancestor}");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_undelegated_node_refuses_everything_and_says_why() {
        // A development host and a misconfigured node produce the same answer on
        // purpose: neither should be mistaken for one where limits are enforced.
        let root = fixture_root("undelegated");
        let manager = TaskCgroups::undelegated(CgroupController::new(&root));

        assert!(!manager.is_delegated());
        assert_eq!(
            manager.path_for(&TaskId::new()).unwrap_err(),
            CgroupRefusal::NoDelegation
        );
        assert!(manager
            .create(&TaskId::new(), &ResourceBudget::default())
            .unwrap_err()
            .to_string()
            .contains("Delegate="));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn creating_a_task_cgroup_writes_its_whole_budget() {
        let (root, manager) = delegated("budget");
        let task = TaskId::new();
        let budget = ResourceBudget {
            cpu_cores: 1.5,
            memory_high_bytes: 256 * 1024 * 1024,
            memory_max_bytes: 512 * 1024 * 1024,
            io_weight: 50,
        };

        let path = manager.create(&task, &budget).unwrap();
        let dir = path.resolve(&root);
        assert!(dir.is_dir());

        assert_eq!(
            std::fs::read_to_string(dir.join("cpu.max")).unwrap(),
            "150000 100000"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("memory.high")).unwrap(),
            (256 * 1024 * 1024).to_string()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("memory.max")).unwrap(),
            (512 * 1024 * 1024).to_string()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("io.weight")).unwrap(),
            "default 50"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn creating_an_existing_cgroup_is_not_an_error() {
        // The executor re-creating a task's cgroup after a restart should find it and
        // carry on, not fail.
        let (root, manager) = delegated("recreate");
        let task = TaskId::new();
        let budget = ResourceBudget::default();

        let first = manager.create(&task, &budget).unwrap();
        let second = manager.create(&task, &budget).unwrap();
        assert_eq!(first, second);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_budget_can_be_changed_without_restarting_the_task() {
        // The resource governor's lever.
        let (root, manager) = delegated("rebudget");
        let task = TaskId::new();
        let path = manager.create(&task, &ResourceBudget::default()).unwrap();

        manager
            .apply_budget(
                &path,
                &ResourceBudget {
                    cpu_cores: 0.25,
                    memory_high_bytes: 64 * 1024 * 1024,
                    memory_max_bytes: 128 * 1024 * 1024,
                    io_weight: 10,
                },
            )
            .unwrap();

        let dir = path.resolve(&root);
        assert_eq!(
            std::fs::read_to_string(dir.join("cpu.max")).unwrap(),
            "25000 100000"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_out_of_range_io_weight_is_refused_rather_than_clamped() {
        // Silently giving a caller weight 1 hides their bug and produces a cgroup that
        // barely gets any IO.
        let (root, manager) = delegated("ioweight");
        let task = TaskId::new();
        let budget = ResourceBudget {
            io_weight: 0,
            ..ResourceBudget::default()
        };

        assert!(manager.create(&task, &budget).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn freezing_and_thawing_a_task_writes_the_freezer() {
        let (root, manager) = delegated("freeze");
        let task = TaskId::new();
        let path = manager.create(&task, &ResourceBudget::default()).unwrap();
        std::fs::write(path.resolve(&root).join("cgroup.freeze"), "0").unwrap();

        manager.freeze(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "1"
        );

        manager.thaw(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "0"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_kernels_view_distinguishes_an_oom_from_an_empty_cgroup() {
        let (root, manager) = delegated("state");
        let task = TaskId::new();
        let path = manager.create(&task, &ResourceBudget::default()).unwrap();
        let dir = path.resolve(&root);

        std::fs::write(dir.join("cgroup.events"), "populated 1\nfrozen 0\n").unwrap();
        std::fs::write(
            dir.join("memory.events"),
            "low 0\nhigh 3\nmax 1\noom 1\noom_kill 1\n",
        )
        .unwrap();
        std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();

        let state = manager.state(&path).unwrap();
        assert!(state.populated);
        assert!(!state.frozen);
        assert_eq!(
            state.oom_kills, 1,
            "a worker killed here hit its own budget, which is not the machine running \
             out of memory"
        );

        std::fs::write(dir.join("cgroup.events"), "populated 0\nfrozen 0\n").unwrap();
        assert!(!manager.state(&path).unwrap().populated);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_finds_the_tasks_the_machine_actually_holds() {
        // The other half of reconciliation. Scanned rather than remembered, because after
        // a crash the records are exactly what cannot be trusted — a list rebuilt from
        // them would agree with itself whatever the kernel held.
        let (root, manager) = delegated("discover");
        let running = TaskId::new();
        let finished = TaskId::new();

        for task in [running, finished] {
            let path = manager.create(&task, &ResourceBudget::default()).unwrap();
            let dir = path.resolve(&root);
            std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();
            let populated = if task == running { "1" } else { "0" };
            std::fs::write(
                dir.join("cgroup.events"),
                format!("populated {populated}\nfrozen 0\n"),
            )
            .unwrap();
        }

        let found = manager.discover().unwrap();
        assert_eq!(found.len(), 2);

        let alive = found.iter().find(|c| c.task_id == running).unwrap();
        assert!(alive.populated);
        assert!(!alive.frozen);

        let gone = found.iter().find(|c| c.task_id == finished).unwrap();
        assert!(!gone.populated);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_directory_this_executor_did_not_name_is_not_a_task() {
        // Treating a stray directory as an orphaned task would raise an alarm about
        // something nobody here created, and orphans are the finding that most demands a
        // human.
        let (root, manager) = delegated("strays");
        let real = TaskId::new();
        manager.create(&real, &ResourceBudget::default()).unwrap();

        let base = manager.delegated_root().unwrap().resolve(&root);
        for stray in [
            "task-not-a-uuid",
            "notatask",
            "task-",
            "systemd-thing.scope",
        ] {
            std::fs::create_dir_all(base.join(stray)).unwrap();
        }

        let found = manager.discover().unwrap();
        assert_eq!(found.len(), 1, "only the cgroup we named is ours");
        assert_eq!(found[0].task_id, real);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_frozen_task_is_discovered_as_frozen() {
        // Adopting it as running would have the scheduler wait forever for output from a
        // task that cannot produce any.
        let (root, manager) = delegated("frozendisc");
        let task = TaskId::new();
        let path = manager.create(&task, &ResourceBudget::default()).unwrap();
        let dir = path.resolve(&root);
        std::fs::write(dir.join("cgroup.freeze"), "1").unwrap();
        std::fs::write(dir.join("cgroup.events"), "populated 1\nfrozen 1\n").unwrap();

        let found = manager.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].frozen);
        assert!(found[0].populated);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_empty_subtree_reports_nothing_found_rather_than_failing() {
        // No task has ever started here. That is a truthful answer, not an error.
        let (root, manager) = delegated("emptysubtree");
        assert!(manager.discover().unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_is_stable_so_two_reports_can_be_compared() {
        // Directory order is a filesystem detail. An operator comparing two runs should
        // not have to work out whether the list changed or merely moved.
        let (root, manager) = delegated("stable");
        for _ in 0..5 {
            manager
                .create(&TaskId::new(), &ResourceBudget::default())
                .unwrap();
        }

        let first = manager.discover().unwrap();
        let second = manager.discover().unwrap();
        assert_eq!(first, second);
        assert!(first.windows(2).all(|w| w[0].task_id <= w[1].task_id));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_undelegated_node_cannot_discover_anything() {
        let root = fixture_root("nodelegdisc");
        let manager = TaskCgroups::undelegated(CgroupController::new(&root));
        assert_eq!(manager.discover().unwrap_err(), CgroupRefusal::NoDelegation);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn removal_refuses_a_non_empty_cgroup_and_tolerates_a_missing_one() {
        // A removal that succeeded with a worker still inside would leave that worker
        // outside every budget on the machine, so `remove` uses `rmdir` and lets the
        // kernel refuse.
        //
        // The fixture can only go so far here: on a real cgroup filesystem the interface
        // files are synthesized by the kernel and `rmdir` succeeds despite them, while in
        // a plain directory they are ordinary files that block it. So this asserts the
        // refusal — which is the safety-relevant half — and leaves the success path to
        // the Linux integration gate rather than faking it.
        let (root, manager) = delegated("remove");
        let task = TaskId::new();
        let path = manager.create(&task, &ResourceBudget::default()).unwrap();

        assert!(
            manager.remove(&path).is_err(),
            "a cgroup with contents must not be removed out from under them"
        );

        // Removing something already gone is not an error: the executor cleaning up
        // after a crash should not fail on work someone else already tidied.
        let absent = CgroupPath::new(format!(
            "system.slice/archie-executord.service/task-{}",
            TaskId::new().as_uuid()
        ))
        .unwrap();
        assert!(manager.remove(&absent).is_ok());

        std::fs::remove_dir_all(&root).ok();
    }
}
