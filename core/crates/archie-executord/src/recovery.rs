//! Reconciling records against the machine after either daemon restarts.
//!
//! RFC-007 §7, step 8.
//!
//! # Two sources of truth, and neither is allowed to win by default
//!
//! After a crash the executor holds task records, and the delegated cgroup subtree holds
//! the actual processes. They can disagree in both directions, and the two disagreements
//! call for opposite responses:
//!
//! ```text
//! record says running, no process     the task died unobserved       -> Lost
//! process exists, no live record      something is running untracked -> Orphaned
//! ```
//!
//! The second is the dangerous one. A supervisor that resolves it by starting the task
//! "again" runs it twice, and a task that ran twice has done its side effects twice. So
//! nothing here concludes an action: reconciliation reports what it found and hands the
//! decision to policy.
//!
//! # The cgroup outranks the pid
//!
//! A pid can be reused. A worker that died and had its number handed to an unrelated
//! process would look alive to a pid check, and the executor would go on believing a task
//! was running while its slot was actually free.
//!
//! The cgroup cannot be confused this way: it was created for exactly one task, it is
//! named after that task, and `cgroup.events`'s `populated` counts descendants — so a
//! worker that forked is not mistaken for gone either. Where the two disagree, the cgroup
//! is believed.

use archie_execution::lifecycle::ExecutionState;
use archie_types::TaskId;
use std::collections::{BTreeMap, BTreeSet};

/// One task cgroup found in the delegated subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCgroup {
    pub task_id: TaskId,
    /// Whether any process is in it, including descendants.
    pub populated: bool,
    pub frozen: bool,
}

/// What the executor believed about a task before reconciling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedTask {
    pub task_id: TaskId,
    pub state: ExecutionState,
    /// The pid we last saw. Kept for the audit record and for signalling, never used to
    /// decide whether the task is alive.
    pub pid: Option<u32>,
}

/// What reconciliation concluded about one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Record and machine agree. Nothing to do.
    Consistent {
        task_id: TaskId,
        state: ExecutionState,
    },

    /// The worker outlived the executor. Adopt it rather than restarting it — this is the
    /// duplicate-execution bug in its most plausible disguise, because "the supervisor
    /// restarted, so surely nothing is running" feels obviously true and is not.
    Survived { task_id: TaskId, frozen: bool },

    /// We expected a process and the cgroup is empty. The task died unobserved and we do
    /// not know how — deliberately not `Failed`, which would erase the difference between
    /// a task that failed and a task nobody was watching.
    Vanished {
        task_id: TaskId,
        was: ExecutionState,
    },

    /// A populated cgroup with no live record. Something is running that we are not
    /// supervising, and starting the task again would run it twice.
    Untracked { task_id: TaskId },

    /// The record says finished and the cgroup still has processes in it. The executor
    /// died between the worker exiting and the record being written, or the worker
    /// outlived its own exit report.
    OutlivedItsRecord {
        task_id: TaskId,
        recorded: ExecutionState,
    },

    /// An empty cgroup for a task that finished. Harmless, but it is litter: every
    /// leftover cgroup is a directory the kernel keeps and a name that cannot be reused.
    Reclaimable { task_id: TaskId },
}

impl Finding {
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Consistent { task_id, .. }
            | Self::Survived { task_id, .. }
            | Self::Vanished { task_id, .. }
            | Self::Untracked { task_id }
            | Self::OutlivedItsRecord { task_id, .. }
            | Self::Reclaimable { task_id } => *task_id,
        }
    }

    /// Whether a human or a policy has to decide what happens next.
    ///
    /// `Survived` does not: adopting a running worker is the only correct answer, and
    /// making an operator confirm it would mean a routine restart leaves work in limbo.
    pub fn needs_decision(&self) -> bool {
        matches!(
            self,
            Self::Vanished { .. } | Self::Untracked { .. } | Self::OutlivedItsRecord { .. }
        )
    }

    /// Whether it is safe to start this task now.
    ///
    /// False for everything except a task with no trace of a process anywhere. This is
    /// the predicate that stands between a restart and running someone's job twice, so it
    /// is deliberately conservative: `Vanished` says the process is gone, and it still
    /// answers false, because *why* it vanished decides whether restarting is correct and
    /// reconciliation does not know that.
    pub fn safe_to_start(&self) -> bool {
        false
    }

    /// The state this finding implies. Never terminal — reconciliation records what it
    /// saw and does not decide that a task failed.
    pub fn implied_state(&self) -> Option<ExecutionState> {
        match self {
            Self::Consistent { state, .. } => Some(*state),
            Self::Survived { frozen, .. } => Some(if *frozen {
                ExecutionState::Frozen
            } else {
                ExecutionState::Running
            }),
            Self::Vanished { .. } => Some(ExecutionState::Lost),
            Self::Untracked { .. } | Self::OutlivedItsRecord { .. } => {
                Some(ExecutionState::Orphaned)
            }
            Self::Reclaimable { .. } => None,
        }
    }
}

/// Reconcile every record against every cgroup found in the subtree.
///
/// Both directions are walked deliberately. Iterating only over records would miss a
/// populated cgroup whose record was lost — which is precisely the case where starting
/// the task again would run it twice.
pub fn reconcile(records: &[RecordedTask], discovered: &[DiscoveredCgroup]) -> Vec<Finding> {
    let by_task: BTreeMap<TaskId, &DiscoveredCgroup> =
        discovered.iter().map(|c| (c.task_id, c)).collect();
    let mut seen: BTreeSet<TaskId> = BTreeSet::new();
    let mut findings = Vec::new();

    for record in records {
        seen.insert(record.task_id);
        let cgroup = by_task.get(&record.task_id);

        // The cgroup is the evidence. A pid can be reused; a cgroup created for one task
        // and named after it cannot be confused with another.
        let populated = cgroup.is_some_and(|c| c.populated);
        let frozen = cgroup.is_some_and(|c| c.frozen);

        let finding = if record.state.is_terminal() {
            if populated {
                Finding::OutlivedItsRecord {
                    task_id: record.task_id,
                    recorded: record.state,
                }
            } else if cgroup.is_some() {
                Finding::Reclaimable {
                    task_id: record.task_id,
                }
            } else {
                Finding::Consistent {
                    task_id: record.task_id,
                    state: record.state,
                }
            }
        } else if record.state.expects_process() {
            if populated {
                Finding::Survived {
                    task_id: record.task_id,
                    frozen,
                }
            } else {
                Finding::Vanished {
                    task_id: record.task_id,
                    was: record.state,
                }
            }
        } else if populated {
            // Queued or checkpointed, and yet something is running in its cgroup.
            Finding::Untracked {
                task_id: record.task_id,
            }
        } else {
            Finding::Consistent {
                task_id: record.task_id,
                state: record.state,
            }
        };

        findings.push(finding);
    }

    // The other direction: cgroups we hold no record for at all. This is what a lost or
    // truncated record store looks like, and it is the case that must never be resolved
    // by assuming nothing is running.
    for cgroup in discovered {
        if seen.contains(&cgroup.task_id) {
            continue;
        }
        findings.push(if cgroup.populated {
            Finding::Untracked {
                task_id: cgroup.task_id,
            }
        } else {
            Finding::Reclaimable {
                task_id: cgroup.task_id,
            }
        });
    }

    findings
}

/// A summary an operator can read at a glance after a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    pub consistent: usize,
    pub survived: usize,
    pub vanished: usize,
    pub untracked: usize,
    pub reclaimable: usize,
    pub outlived_record: usize,
}

impl Summary {
    pub fn of(findings: &[Finding]) -> Self {
        let mut s = Self::default();
        for finding in findings {
            match finding {
                Finding::Consistent { .. } => s.consistent += 1,
                Finding::Survived { .. } => s.survived += 1,
                Finding::Vanished { .. } => s.vanished += 1,
                Finding::Untracked { .. } => s.untracked += 1,
                Finding::OutlivedItsRecord { .. } => s.outlived_record += 1,
                Finding::Reclaimable { .. } => s.reclaimable += 1,
            }
        }
        s
    }

    /// Whether anything needs a decision.
    pub fn needs_attention(&self) -> bool {
        self.vanished > 0 || self.untracked > 0 || self.outlived_record > 0
    }

    pub fn describe(&self) -> String {
        if !self.needs_attention() {
            return format!(
                "{} tasks consistent, {} workers adopted, {} cgroups reclaimable",
                self.consistent, self.survived, self.reclaimable
            );
        }
        format!(
            "{} consistent, {} adopted, {} vanished, {} untracked, {} outlived their \
             record — untracked and outlived mean something is running that this executor \
             is not supervising, and must not be started again",
            self.consistent, self.survived, self.vanished, self.untracked, self.outlived_record
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: ExecutionState) -> RecordedTask {
        RecordedTask {
            task_id: TaskId::new(),
            state,
            pid: Some(4242),
        }
    }

    fn cgroup(task_id: TaskId, populated: bool) -> DiscoveredCgroup {
        DiscoveredCgroup {
            task_id,
            populated,
            frozen: false,
        }
    }

    #[test]
    fn a_worker_that_outlived_the_executor_is_adopted_not_restarted() {
        // The duplicate-execution bug in its most plausible disguise: "the supervisor
        // restarted, so surely nothing is running" feels obviously true and is not.
        let r = record(ExecutionState::Running);
        let findings = reconcile(std::slice::from_ref(&r), &[cgroup(r.task_id, true)]);

        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0],
            Finding::Survived {
                task_id: r.task_id,
                frozen: false
            }
        );
        assert_eq!(findings[0].implied_state(), Some(ExecutionState::Running));
        assert!(
            !findings[0].needs_decision(),
            "adopting a running worker is the only correct answer; asking an operator \
             would leave work in limbo after every routine restart"
        );
        assert!(!findings[0].safe_to_start());
    }

    #[test]
    fn a_frozen_survivor_is_recognised_as_frozen() {
        // Freezing survives the executor dying: the cgroup and its frozen process are
        // still there. Adopting it as Running would have the scheduler wait forever for
        // output from a task that cannot produce any.
        let r = record(ExecutionState::Frozen);
        let findings = reconcile(
            std::slice::from_ref(&r),
            &[DiscoveredCgroup {
                task_id: r.task_id,
                populated: true,
                frozen: true,
            }],
        );
        assert_eq!(
            findings[0],
            Finding::Survived {
                task_id: r.task_id,
                frozen: true
            }
        );
        assert_eq!(findings[0].implied_state(), Some(ExecutionState::Frozen));
    }

    #[test]
    fn a_record_expecting_a_process_with_an_empty_cgroup_is_lost_not_failed() {
        // Concluding failure would erase the difference between a task that failed and a
        // task nobody was watching, and the second is a bug in us.
        let r = record(ExecutionState::Running);
        let findings = reconcile(std::slice::from_ref(&r), &[cgroup(r.task_id, false)]);

        assert_eq!(
            findings[0],
            Finding::Vanished {
                task_id: r.task_id,
                was: ExecutionState::Running
            }
        );
        assert_eq!(findings[0].implied_state(), Some(ExecutionState::Lost));
        assert!(findings[0].needs_decision());
        assert!(
            !findings[0].safe_to_start(),
            "why it vanished decides whether restarting is correct, and reconciliation \
             does not know that"
        );
    }

    #[test]
    fn a_populated_cgroup_with_no_record_is_never_assumed_idle() {
        // The lost-record-store case. Iterating only over records would miss this
        // entirely — which is precisely when starting the task again runs it twice.
        let orphan = TaskId::new();
        let findings = reconcile(&[], &[cgroup(orphan, true)]);

        assert_eq!(findings, vec![Finding::Untracked { task_id: orphan }]);
        assert_eq!(findings[0].implied_state(), Some(ExecutionState::Orphaned));
        assert!(findings[0].needs_decision());
        assert!(!findings[0].safe_to_start());
    }

    #[test]
    fn a_task_that_outlived_its_own_terminal_record_is_flagged() {
        // The executor died between the worker exiting and the record being written.
        let r = record(ExecutionState::Completed);
        let findings = reconcile(std::slice::from_ref(&r), &[cgroup(r.task_id, true)]);

        assert_eq!(
            findings[0],
            Finding::OutlivedItsRecord {
                task_id: r.task_id,
                recorded: ExecutionState::Completed
            }
        );
        assert_eq!(findings[0].implied_state(), Some(ExecutionState::Orphaned));
        assert!(findings[0].needs_decision());
    }

    #[test]
    fn a_queued_task_with_something_running_in_its_cgroup_is_untracked() {
        // A start that got as far as spawning before the record was updated. Treating it
        // as merely queued and starting it would be the duplicate.
        let r = record(ExecutionState::Queued);
        let findings = reconcile(std::slice::from_ref(&r), &[cgroup(r.task_id, true)]);
        assert_eq!(findings[0], Finding::Untracked { task_id: r.task_id });
    }

    #[test]
    fn a_finished_task_leaving_an_empty_cgroup_is_only_litter() {
        // Harmless, but every leftover cgroup is a directory the kernel keeps and a name
        // that cannot be reused.
        let r = record(ExecutionState::Completed);
        let findings = reconcile(std::slice::from_ref(&r), &[cgroup(r.task_id, false)]);

        assert_eq!(findings[0], Finding::Reclaimable { task_id: r.task_id });
        assert!(!findings[0].needs_decision());
        assert_eq!(findings[0].implied_state(), None);
    }

    #[test]
    fn agreement_needs_nothing_doing() {
        let finished = record(ExecutionState::Completed);
        let queued = record(ExecutionState::Queued);
        let findings = reconcile(&[finished.clone(), queued.clone()], &[]);

        assert_eq!(
            findings[0],
            Finding::Consistent {
                task_id: finished.task_id,
                state: ExecutionState::Completed
            }
        );
        assert_eq!(
            findings[1],
            Finding::Consistent {
                task_id: queued.task_id,
                state: ExecutionState::Queued
            }
        );
        assert!(!Summary::of(&findings).needs_attention());
    }

    #[test]
    fn nothing_reconciliation_finds_is_ever_safe_to_start() {
        // The single predicate standing between a restart and running someone's job
        // twice. Deliberately conservative in every case.
        let r = record(ExecutionState::Running);
        let all = vec![
            Finding::Consistent {
                task_id: r.task_id,
                state: ExecutionState::Running,
            },
            Finding::Survived {
                task_id: r.task_id,
                frozen: false,
            },
            Finding::Vanished {
                task_id: r.task_id,
                was: ExecutionState::Running,
            },
            Finding::Untracked { task_id: r.task_id },
            Finding::OutlivedItsRecord {
                task_id: r.task_id,
                recorded: ExecutionState::Completed,
            },
            Finding::Reclaimable { task_id: r.task_id },
        ];
        for finding in &all {
            assert!(!finding.safe_to_start(), "{finding:?}");
            assert_eq!(finding.task_id(), r.task_id);
        }
    }

    #[test]
    fn no_finding_concludes_a_terminal_state() {
        // Reconciliation reports what it saw. Deciding a task failed from a comparison
        // would be a guess wearing the clothes of a fact.
        let r = record(ExecutionState::Running);
        for (records, cgroups) in [
            (vec![r.clone()], vec![cgroup(r.task_id, false)]),
            (vec![r.clone()], vec![cgroup(r.task_id, true)]),
            (vec![], vec![cgroup(TaskId::new(), true)]),
        ] {
            for finding in reconcile(&records, &cgroups) {
                if let Some(state) = finding.implied_state() {
                    assert!(!state.is_terminal(), "{finding:?} implied {state}");
                }
            }
        }
    }

    #[test]
    fn a_mixed_restart_is_summarised_for_a_human() {
        let running = record(ExecutionState::Running);
        let dead = record(ExecutionState::Running);
        let done = record(ExecutionState::Completed);
        let orphan = TaskId::new();

        let findings = reconcile(
            &[running.clone(), dead.clone(), done.clone()],
            &[
                cgroup(running.task_id, true),
                cgroup(dead.task_id, false),
                cgroup(done.task_id, false),
                cgroup(orphan, true),
            ],
        );

        let summary = Summary::of(&findings);
        assert_eq!(summary.survived, 1);
        assert_eq!(summary.vanished, 1);
        assert_eq!(summary.reclaimable, 1);
        assert_eq!(summary.untracked, 1);
        assert!(summary.needs_attention());
        assert!(summary.describe().contains("must not be started again"));
    }

    #[test]
    fn a_clean_restart_reads_as_clean() {
        let running = record(ExecutionState::Running);
        let findings = reconcile(
            std::slice::from_ref(&running),
            &[cgroup(running.task_id, true)],
        );
        let summary = Summary::of(&findings);

        assert!(!summary.needs_attention());
        assert_eq!(summary.survived, 1);
        assert!(summary.describe().contains("adopted"));
    }

    #[test]
    fn every_record_and_every_cgroup_appears_exactly_once() {
        // A finding that got lost would be a task nobody looked at, and a duplicated one
        // would be counted twice in the summary an operator reads.
        let a = record(ExecutionState::Running);
        let b = record(ExecutionState::Completed);
        let orphan = TaskId::new();

        let findings = reconcile(
            &[a.clone(), b.clone()],
            &[cgroup(a.task_id, true), cgroup(orphan, false)],
        );
        assert_eq!(findings.len(), 3);

        let ids: BTreeSet<TaskId> = findings.iter().map(Finding::task_id).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&a.task_id) && ids.contains(&b.task_id) && ids.contains(&orphan));
    }
}
