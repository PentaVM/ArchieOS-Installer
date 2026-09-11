//! The worker control channel: asking a task to save its state and stand down.
//!
//! RFC-007 §7, step 5. This is the cooperative half of preemption; the freezer is the
//! fallback when cooperation does not arrive in time.
//!
//! # Why ask at all, when the kernel can just stop it
//!
//! Because a frozen task's state lives in the memory of a process that is one `SIGKILL`
//! from gone, and a checkpointed task's does not. Idle research that resumes is worth
//! vastly more than idle research that restarts, so it is worth waiting a second to find
//! out which one we have.
//!
//! The deadline is what stops that courtesy from becoming an unbounded wait. A user asked
//! a question; the deep-idle job holding the slot gets a few seconds to be polite about
//! releasing it, and then the kernel handles it.
//!
//! # A late checkpoint is still a checkpoint
//!
//! The case worth getting right: the worker replies at deadline + 1 ms, after the freezer
//! has already stopped it. The reply is not stale — the artifact it names *exists on
//! disk*. The work is durable whether or not we were still listening.
//!
//! So a late reply is accepted for what it proves, and the task is recorded as having a
//! valid checkpoint even though it is frozen. That changes the next decision entirely:
//! killing a frozen task with no checkpoint loses everything since its last one, and
//! killing this one loses nothing at all.

use archie_types::{TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// What the executor sends down a worker's control channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum CheckpointRequest {
    /// Save your state and prepare to stop.
    ///
    /// The deadline is absolute rather than a duration, so a worker that was descheduled
    /// between the message being written and being read does not get the full budget
    /// starting from whenever it woke up.
    Checkpoint {
        task_id: TaskId,
        deadline: Timestamp,
    },
}

/// What a worker answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum CheckpointReply {
    /// State is durable. The reference names where — a path, an object key, a database
    /// row — and the executor does not interpret it, only records it and hands it back on
    /// resume.
    Checkpointed { task_id: TaskId, artifact: String },
    /// The worker tried and could not. Distinct from silence: a worker that says it
    /// cannot checkpoint has told us something useful, and there is no point waiting out
    /// the rest of the deadline for a better answer.
    Failed { task_id: TaskId, reason: String },
}

impl CheckpointReply {
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Checkpointed { task_id, .. } | Self::Failed { task_id, .. } => *task_id,
        }
    }

    /// Whether this reply means durable state exists.
    pub fn is_durable(&self) -> bool {
        matches!(self, Self::Checkpointed { .. })
    }

    /// Whether waiting longer could produce a better answer.
    ///
    /// It cannot: both replies are final. A worker that failed will not succeed by being
    /// waited on, so the deadline is abandoned as soon as either arrives.
    pub fn is_final(&self) -> bool {
        true
    }
}

/// How a checkpoint attempt ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CheckpointOutcome {
    /// The worker answered in time with durable state.
    Durable { artifact: String },
    /// The worker answered in time and could not checkpoint.
    Refused { reason: String },
    /// Nothing arrived before the deadline.
    TimedOut,
    /// The worker answered *after* the deadline, with durable state.
    ///
    /// Kept apart from `Durable` because the two lead to different situations: this task
    /// has already been frozen or stopped, and the useful consequence is that it can now
    /// be killed without losing anything. Recording it as a plain success would hide that
    /// the fallback already fired.
    DurableButLate { artifact: String },
}

impl CheckpointOutcome {
    /// Whether the task's work survives being killed now.
    ///
    /// The question that actually matters at the end of a preemption, and the reason a
    /// late reply is worth accepting: the artifact is on disk whether or not we were
    /// still listening when it landed.
    pub fn work_is_durable(&self) -> bool {
        matches!(self, Self::Durable { .. } | Self::DurableButLate { .. })
    }

    /// The artifact reference, if there is one.
    pub fn artifact(&self) -> Option<&str> {
        match self {
            Self::Durable { artifact } | Self::DurableButLate { artifact } => {
                Some(artifact.as_str())
            }
            Self::Refused { .. } | Self::TimedOut => None,
        }
    }

    /// Whether the kernel had to step in.
    pub fn needed_fallback(&self) -> bool {
        matches!(self, Self::TimedOut | Self::DurableButLate { .. })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Durable { .. } => "durable",
            Self::Refused { .. } => "refused",
            Self::TimedOut => "timed_out",
            Self::DurableButLate { .. } => "durable_but_late",
        }
    }
}

/// Classify a reply against the deadline.
///
/// `arrived_at` is when the executor read it, not when the worker claims to have sent it:
/// a worker's own clock is not evidence, and a worker that would rather not be preempted
/// has an obvious incentive to backdate.
pub fn classify(
    reply: Option<CheckpointReply>,
    deadline: Timestamp,
    arrived_at: Timestamp,
) -> CheckpointOutcome {
    let Some(reply) = reply else {
        return CheckpointOutcome::TimedOut;
    };

    let late = arrived_at > deadline;
    match reply {
        CheckpointReply::Checkpointed { artifact, .. } => {
            if late {
                CheckpointOutcome::DurableButLate { artifact }
            } else {
                CheckpointOutcome::Durable { artifact }
            }
        }
        // A late failure is just a timeout with an explanation attached. There is no
        // durable state either way, so it does not deserve its own outcome — but the
        // reason is worth keeping, because "the disk was full" and silence call for
        // different investigations.
        CheckpointReply::Failed { reason, .. } => CheckpointOutcome::Refused { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms)
    }

    #[test]
    fn an_answer_in_time_with_state_is_durable() {
        let outcome = classify(
            Some(CheckpointReply::Checkpointed {
                task_id: TaskId::new(),
                artifact: "/var/lib/archie/ckpt/abc".into(),
            }),
            at(1_000),
            at(900),
        );
        assert_eq!(
            outcome,
            CheckpointOutcome::Durable {
                artifact: "/var/lib/archie/ckpt/abc".into()
            }
        );
        assert!(outcome.work_is_durable());
        assert!(!outcome.needed_fallback());
        assert_eq!(outcome.artifact(), Some("/var/lib/archie/ckpt/abc"));
    }

    #[test]
    fn silence_before_the_deadline_is_a_timeout() {
        let outcome = classify(None, at(1_000), at(1_001));
        assert_eq!(outcome, CheckpointOutcome::TimedOut);
        assert!(!outcome.work_is_durable());
        assert!(outcome.needed_fallback());
        assert!(outcome.artifact().is_none());
    }

    #[test]
    fn a_late_checkpoint_is_still_a_checkpoint() {
        // The case worth getting right. The worker replied after the freezer already
        // stopped it — but the artifact is on disk whether or not we were still
        // listening, so the work is durable and killing the frozen task now loses
        // nothing.
        let outcome = classify(
            Some(CheckpointReply::Checkpointed {
                task_id: TaskId::new(),
                artifact: "/var/lib/archie/ckpt/late".into(),
            }),
            at(1_000),
            at(1_001),
        );
        assert_eq!(
            outcome,
            CheckpointOutcome::DurableButLate {
                artifact: "/var/lib/archie/ckpt/late".into()
            }
        );
        assert!(
            outcome.work_is_durable(),
            "the artifact exists; our attention span is not what makes it real"
        );
        assert!(
            outcome.needed_fallback(),
            "recording it as a plain success would hide that the kernel already stepped in"
        );
    }

    #[test]
    fn a_worker_that_says_it_cannot_ends_the_wait_immediately() {
        // Waiting out the rest of the deadline for a better answer from a worker that
        // already told us it failed is pure latency on the user request that triggered
        // the preemption.
        let outcome = classify(
            Some(CheckpointReply::Failed {
                task_id: TaskId::new(),
                reason: "checkpoint volume is full".into(),
            }),
            at(5_000),
            at(100),
        );
        assert_eq!(
            outcome,
            CheckpointOutcome::Refused {
                reason: "checkpoint volume is full".into()
            }
        );
        assert!(!outcome.work_is_durable());
        assert!(
            !outcome.needed_fallback(),
            "the worker answered; the kernel did not have to step in"
        );
    }

    #[test]
    fn a_late_failure_keeps_its_reason_rather_than_becoming_silence() {
        // "The disk was full" and "no answer at all" call for different investigations.
        let outcome = classify(
            Some(CheckpointReply::Failed {
                task_id: TaskId::new(),
                reason: "checkpoint volume is full".into(),
            }),
            at(1_000),
            at(9_999),
        );
        match &outcome {
            CheckpointOutcome::Refused { reason } => assert!(reason.contains("full")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!outcome.work_is_durable());
    }

    #[test]
    fn arriving_exactly_on_the_deadline_is_in_time() {
        // The boundary belongs to the worker: it was asked for an answer by this
        // instant, and it produced one.
        let outcome = classify(
            Some(CheckpointReply::Checkpointed {
                task_id: TaskId::new(),
                artifact: "a".into(),
            }),
            at(1_000),
            at(1_000),
        );
        assert!(matches!(outcome, CheckpointOutcome::Durable { .. }));
    }

    #[test]
    fn both_replies_are_final() {
        // Neither improves by waiting, so the deadline is abandoned as soon as either
        // arrives.
        let id = TaskId::new();
        assert!(CheckpointReply::Checkpointed {
            task_id: id,
            artifact: "a".into()
        }
        .is_final());
        assert!(CheckpointReply::Failed {
            task_id: id,
            reason: "no".into()
        }
        .is_final());
        assert!(CheckpointReply::Checkpointed {
            task_id: id,
            artifact: "a".into()
        }
        .is_durable());
        assert!(!CheckpointReply::Failed {
            task_id: id,
            reason: "no".into()
        }
        .is_durable());
    }

    #[test]
    fn the_deadline_travels_as_an_instant_not_a_duration() {
        // A worker descheduled between the message being written and read must not get
        // the full budget starting from whenever it woke up.
        let request = CheckpointRequest::Checkpoint {
            task_id: TaskId::new(),
            deadline: at(5_000),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("deadline"));
        assert!(!json.contains("duration"));
        assert_eq!(
            serde_json::from_str::<CheckpointRequest>(&json).unwrap(),
            request
        );
    }

    #[test]
    fn outcomes_survive_the_wire_and_name_themselves() {
        for outcome in [
            CheckpointOutcome::Durable {
                artifact: "a".into(),
            },
            CheckpointOutcome::Refused { reason: "b".into() },
            CheckpointOutcome::TimedOut,
            CheckpointOutcome::DurableButLate {
                artifact: "c".into(),
            },
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            assert!(json.contains(outcome.as_str()));
            assert_eq!(
                serde_json::from_str::<CheckpointOutcome>(&json).unwrap(),
                outcome
            );
        }
    }
}
