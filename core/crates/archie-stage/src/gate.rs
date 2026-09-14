//! The admission gate every stage action passes through.
//!
//! [`AuthorizedStageCommand`] has private fields and no public constructor. The only way
//! to obtain one is [`StageGate::authorize`], and the only thing that seals a command for
//! the compositor, [`crate::Sealer::seal`], takes one. A caller holding an action a model
//! proposed has a [`StageProposal`], which nothing seals.

use crate::action::{
    ActionKind, Key, StageAction, MAX_COORDINATE, MAX_SCROLL_STEPS, MAX_TEXT_CHARS,
};
use crate::policy::StagePolicy;
use archie_policy::{Action, Lease, LeaseRejection, Operation, Principal, Resource};
use archie_types::{PrivacyGrade, TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// An action somebody — usually a model, through Cerebra — has proposed. Carries no
/// authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageProposal {
    pub app_id: String,
    /// The compositor's surface id, from its most recent `surface` event.
    pub surface: u32,
    /// The geometry revision the action was prepared against. The compositor refuses the
    /// command if the surface has moved or resized since.
    pub geometry_revision: u64,
    pub action: StageAction,
    /// The sensitivity of typed text, as assessed by whoever composed it. Ignored for
    /// actions that type nothing.
    pub text_grade: PrivacyGrade,
    /// The prepared-operation id the lease was granted for.
    pub operation_id: u64,
    pub task_id: Option<TaskId>,
    pub trace_id: String,
}

/// The resource a lease must name for actions on an application.
pub fn stage_resource(app_id: &str) -> Resource {
    Resource::Process {
        executable: format!("stage:{app_id}"),
    }
}

/// An action that has passed every gate. Constructible only by [`StageGate::authorize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedStageCommand {
    app_id: String,
    surface: u32,
    geometry_revision: u64,
    action: StageAction,
    text_grade: PrivacyGrade,
    operation_id: u64,
    trace_id: String,
}

impl AuthorizedStageCommand {
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn surface(&self) -> u32 {
        self.surface
    }

    pub fn geometry_revision(&self) -> u64 {
        self.geometry_revision
    }

    pub fn action(&self) -> &StageAction {
        &self.action
    }

    pub fn operation_id(&self) -> u64 {
        self.operation_id
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// What may be written down about this command. Text is counted, never included.
    pub fn receipt(&self) -> StageReceipt {
        let (text_chars, key_count) = match &self.action {
            StageAction::Text { text } => (text.chars(), 0),
            StageAction::Key { chord } => (0, chord.key_count()),
            _ => (0, 0),
        };
        StageReceipt {
            app_id: self.app_id.clone(),
            surface: self.surface,
            action: self.action.kind(),
            text_chars,
            key_count,
            grade: if matches!(self.action, StageAction::Text { .. }) {
                self.text_grade
            } else {
                PrivacyGrade::Public
            },
            operation_id: self.operation_id,
            trace_id: self.trace_id.clone(),
        }
    }
}

/// What may be written down about a stage action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReceipt {
    pub app_id: String,
    pub surface: u32,
    pub action: ActionKind,
    pub text_chars: usize,
    pub key_count: usize,
    pub grade: PrivacyGrade,
    pub operation_id: u64,
    pub trace_id: String,
}

/// The gate's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageAdmission {
    Allowed(Box<AuthorizedStageCommand>),
    /// Legitimate, and the policy says a person approves it first.
    NeedsApproval {
        command: Box<AuthorizedStageCommand>,
        reason: &'static str,
    },
    Refused(StageRefusal),
}

impl StageAdmission {
    pub fn allowed(&self) -> Option<&AuthorizedStageCommand> {
        match self {
            Self::Allowed(command) => Some(command),
            _ => None,
        }
    }

    pub fn refusal(&self) -> Option<&StageRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }
}

/// Why an action did not pass. None of these repeat typed text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StageRefusal {
    #[error("an action must name the prepared operation its lease was granted for")]
    NoOperation,
    #[error("{0} is not in the stage policy")]
    AppNotInPolicy(String),
    #[error("{kind:?} is not allowed for {app_id}")]
    ActionNotAllowed { app_id: String, kind: ActionKind },
    #[error("the lease does not authorise this action: {0:?}")]
    Lease(LeaseRejection),
    #[error("a coordinate is outside any surface")]
    CoordinateOutOfRange,
    #[error("scroll steps are out of range")]
    ScrollOutOfRange,
    #[error("typed text may never be graded secret")]
    SecretText,
    #[error("text graded {grade} exceeds what {app_id} may receive")]
    TextExceedsAppGrade { app_id: String, grade: PrivacyGrade },
    #[error("text is empty or longer than {app_id} permits")]
    TextOutOfBounds { app_id: String },
    #[error("text contains control or direction-override characters")]
    TextNotPlain,
    #[error("that chord belongs to the system, not to an application")]
    ChordReserved,
    #[error("chords with modifiers are not allowed for {0}")]
    ModifiedKeysNotAllowed(String),
}

impl StageRefusal {
    /// A stable code for traces. Never a sentence.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoOperation => "stage_operation_missing",
            Self::AppNotInPolicy(_) => "stage_app_not_in_policy",
            Self::ActionNotAllowed { .. } => "stage_action_not_allowed",
            Self::Lease(rejection) => match rejection {
                LeaseRejection::Revoked => "lease_revoked",
                LeaseRejection::Expired => "lease_expired",
                LeaseRejection::Exhausted => "lease_exhausted",
                LeaseRejection::ActionNotPermitted => "lease_action_not_permitted",
                LeaseRejection::ResourceMismatch => "lease_resource_mismatch",
                LeaseRejection::PrincipalMismatch => "lease_principal_mismatch",
                LeaseRejection::OperationMismatch => "lease_operation_mismatch",
                LeaseRejection::OperationIdentifierRequired => "lease_operation_required",
                LeaseRejection::TaskMismatch => "lease_task_mismatch",
                LeaseRejection::UserPresenceRequired => "lease_user_presence_required",
                LeaseRejection::ClockRolledBack => "lease_clock_rolled_back",
            },
            Self::CoordinateOutOfRange => "stage_coordinate_out_of_range",
            Self::ScrollOutOfRange => "stage_scroll_out_of_range",
            Self::SecretText => "secret_in_text",
            Self::TextExceedsAppGrade { .. } => "text_exceeds_app_grade",
            Self::TextOutOfBounds { .. } => "text_out_of_bounds",
            Self::TextNotPlain => "text_not_plain",
            Self::ChordReserved => "chord_reserved",
            Self::ModifiedKeysNotAllowed(_) => "modified_keys_not_allowed",
        }
    }
}

/// Admits stage actions, or does not.
#[derive(Debug, Default, Clone)]
pub struct StageGate;

impl StageGate {
    /// The only way to obtain an [`AuthorizedStageCommand`].
    ///
    /// Checks the lease but does not consume it: an action is only spent once the
    /// compositor's receipt says it ran.
    pub fn authorize(
        policy: &StagePolicy,
        lease: &Lease,
        principal: &Principal,
        user_present: bool,
        proposal: &StageProposal,
        now: Timestamp,
    ) -> StageAdmission {
        let refuse = StageAdmission::Refused;
        if proposal.operation_id == 0 {
            return refuse(StageRefusal::NoOperation);
        }
        let Some(app) = policy.app(&proposal.app_id) else {
            return refuse(StageRefusal::AppNotInPolicy(proposal.app_id.clone()));
        };
        let kind = proposal.action.kind();
        if !app.allow.contains(&kind) {
            return refuse(StageRefusal::ActionNotAllowed {
                app_id: app.app_id.clone(),
                kind,
            });
        }
        let operation = Operation {
            principal: principal.clone(),
            resource: stage_resource(&app.app_id),
            action: Action::Execute,
            task_id: proposal.task_id,
            user_present,
        };
        if let Err(rejection) =
            lease.authorises_one_operation(&operation, proposal.operation_id, now)
        {
            return refuse(StageRefusal::Lease(rejection));
        }

        if proposal
            .action
            .points()
            .iter()
            .any(|(x, y)| !(0..=MAX_COORDINATE).contains(x) || !(0..=MAX_COORDINATE).contains(y))
        {
            return refuse(StageRefusal::CoordinateOutOfRange);
        }

        let mut approval: Option<&'static str> = None;
        match &proposal.action {
            StageAction::Scroll {
                steps_x, steps_y, ..
            } => {
                if (*steps_x == 0 && *steps_y == 0)
                    || steps_x.abs() > MAX_SCROLL_STEPS
                    || steps_y.abs() > MAX_SCROLL_STEPS
                {
                    return refuse(StageRefusal::ScrollOutOfRange);
                }
            }
            StageAction::Text { text } => {
                if proposal.text_grade == PrivacyGrade::Secret {
                    return refuse(StageRefusal::SecretText);
                }
                if proposal.text_grade > app.max_text_grade {
                    return refuse(StageRefusal::TextExceedsAppGrade {
                        app_id: app.app_id.clone(),
                        grade: proposal.text_grade,
                    });
                }
                let chars = text.chars();
                if chars == 0 || chars > app.max_text_chars.min(MAX_TEXT_CHARS) {
                    return refuse(StageRefusal::TextOutOfBounds {
                        app_id: app.app_id.clone(),
                    });
                }
                if !text.is_plain() {
                    return refuse(StageRefusal::TextNotPlain);
                }
            }
            StageAction::Key { chord } => {
                if chord.is_reserved() {
                    return refuse(StageRefusal::ChordReserved);
                }
                if !chord.modifiers.is_empty() && !app.allow_modified_keys {
                    return refuse(StageRefusal::ModifiedKeysNotAllowed(app.app_id.clone()));
                }
                if let Key::Named(named) = chord.key {
                    if app.approval_keys.contains(&named) {
                        approval = Some("policy requires approval for this key");
                    }
                }
            }
            StageAction::Move { .. }
            | StageAction::Click { .. }
            | StageAction::DoubleClick { .. }
            | StageAction::Drag { .. }
            | StageAction::Focus => {}
        }
        if app.requires_approval.contains(&kind) {
            approval = Some("policy requires approval for this action");
        }

        let command = Box::new(AuthorizedStageCommand {
            app_id: app.app_id.clone(),
            surface: proposal.surface,
            geometry_revision: proposal.geometry_revision,
            action: proposal.action.clone(),
            text_grade: proposal.text_grade,
            operation_id: proposal.operation_id,
            trace_id: proposal.trace_id.clone(),
        });
        match approval {
            Some(reason) => StageAdmission::NeedsApproval { command, reason },
            None => StageAdmission::Allowed(command),
        }
    }
}
