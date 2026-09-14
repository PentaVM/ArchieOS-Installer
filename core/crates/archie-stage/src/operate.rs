//! One action on a stage, the whole way through.
//!
//! Every caller that operates an application goes through the same steps: learn the stage
//! as it is, ask policy for a one-operation lease, pass the gate, seal, send, and wait for
//! the compositor's receipt. [`Operator`] is those steps, written once, so a hand-driven
//! tool and an IDE agent cannot drift into doing them differently.
//!
//! It is generic over the byte stream: a Unix socket on a node, an in-memory pipe in
//! tests.

use crate::action::StageAction;
use crate::gate::{stage_resource, StageAdmission, StageGate, StageProposal};
use crate::policy::StagePolicy;
use crate::protocol::{ClientError, StageClient, StageEvent, StageWorld};
use crate::seal::{CommandKey, SealError, Sealer};
use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_types::{PrivacyGrade, TaskId, Timestamp, TraceId};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// How long a sealed command stays valid.
const COMMAND_TTL_MS: i64 = 5_000;

/// What to do, and on whose behalf.
#[derive(Debug, Clone)]
pub struct ActRequest {
    pub app_id: String,
    /// A specific surface, when the application has several. Otherwise its first.
    pub surface: Option<u32>,
    pub action: StageAction,
    /// The grade of any text the action types.
    pub text_grade: PrivacyGrade,
    pub principal: Principal,
    pub task_id: Option<TaskId>,
    /// Whether a person is at the machine for this action.
    pub user_present: bool,
    /// That person has approved an action the stage policy holds for approval.
    pub approved: bool,
    pub justification: String,
}

/// How an action ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActOutcome {
    /// The compositor answered: `executed`, `cancelled`, or one of its refusal codes.
    /// Sequence 0 means it could not verify the command at all.
    Receipt { sequence: u64, outcome: String },
    /// The stage policy holds this action for a person's approval, and nobody gave it.
    NeedsApproval { reason: String },
    /// Refused before anything was sent: by policy, lease or gate.
    Refused { code: &'static str, message: String },
    /// The application has no surface on the stage.
    NoSurface,
}

impl ActOutcome {
    pub fn executed(&self) -> bool {
        matches!(self, Self::Receipt { outcome, .. } if outcome == "executed")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OperateError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("could not seal the command: {0}")]
    Seal(#[from] SealError),
    #[error("the compositor closed the connection")]
    Closed,
    #[error("no receipt within {0:?}")]
    NoReceipt(Duration),
}

/// Reads what the compositor says for `settle`, into `world`. The compositor greets
/// every connection with its surfaces, focus and authority.
pub async fn learn<R, W>(
    client: &mut StageClient<R, W>,
    world: &mut StageWorld,
    settle: Duration,
) -> Result<(), OperateError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let deadline = tokio::time::sleep(settle);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Ok(()),
            event = client.next_event() => match event? {
                Some(event) => world.apply(&event),
                None => return Err(OperateError::Closed),
            },
        }
    }
}

/// Operates applications on one compositor.
#[derive(Debug)]
pub struct Operator {
    sealer: Sealer,
    policy: StagePolicy,
    engine: PolicyEngine,
}

impl Operator {
    pub fn new(key: CommandKey, policy: StagePolicy, engine: PolicyEngine) -> Self {
        Self {
            sealer: Sealer::new(key, 1),
            policy,
            engine,
        }
    }

    pub fn policy(&self) -> &StagePolicy {
        &self.policy
    }

    /// Never seal below `sequence`, for a caller that knows what the compositor accepted.
    pub fn advance_sequence(&mut self, sequence: u64) {
        self.sealer.advance_to(sequence);
    }

    /// Performs one action against `world` as last learned, and waits for its receipt.
    pub async fn act<R, W>(
        &mut self,
        client: &mut StageClient<R, W>,
        world: &mut StageWorld,
        request: &ActRequest,
        receipt_timeout: Duration,
    ) -> Result<ActOutcome, OperateError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let surface = match request.surface {
            Some(id) => world
                .surface(id)
                .filter(|s| s.app_id == request.app_id)
                .cloned(),
            None => world.surfaces_of(&request.app_id).next().cloned(),
        };
        let Some(surface) = surface else {
            return Ok(ActOutcome::NoSurface);
        };

        // One operation, on this application, for this principal. Policy decides whether
        // there is a lease at all.
        let now = Timestamp::now();
        let operation = u64::from(surface.surface) << 32 | (now.as_millis() as u64 & 0xffff_ffff);
        let decision = self.engine.decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: request.task_id,
            principal: request.principal.clone(),
            resource: stage_resource(&request.app_id),
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([Action::Execute]),
            justification: request.justification.clone(),
        });
        let Some(lease) = decision.lease else {
            return Ok(ActOutcome::Refused {
                code: "no_lease",
                message: "policy did not grant a lease for this application".into(),
            });
        };

        let proposal = StageProposal {
            app_id: request.app_id.clone(),
            surface: surface.surface,
            geometry_revision: surface.geometry_revision,
            action: request.action.clone(),
            text_grade: request.text_grade,
            operation_id: operation,
            task_id: request.task_id,
            trace_id: TraceId::new().to_string(),
        };
        let command = match StageGate::authorize(
            &self.policy,
            &lease,
            &request.principal,
            request.user_present,
            &proposal,
            Timestamp::now(),
        ) {
            StageAdmission::Allowed(command) => command,
            StageAdmission::NeedsApproval { command, .. } if request.approved => command,
            StageAdmission::NeedsApproval { reason, .. } => {
                return Ok(ActOutcome::NeedsApproval {
                    reason: reason.to_string(),
                })
            }
            StageAdmission::Refused(refusal) => {
                return Ok(ActOutcome::Refused {
                    code: refusal.code(),
                    message: refusal.to_string(),
                })
            }
        };

        let now = Timestamp::now();
        self.sealer.advance_to(now.as_millis().max(1) as u64);
        let sealed = self.sealer.seal(&command, now, COMMAND_TTL_MS)?;
        client.send(&sealed).await?;

        let deadline = tokio::time::sleep(receipt_timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return Err(OperateError::NoReceipt(receipt_timeout)),
                event = client.next_event() => match event? {
                    Some(StageEvent::Receipt { seq, outcome, .. })
                        if seq == sealed.sequence() || seq == 0 =>
                    {
                        // Sequence 0 answers a line the compositor could not verify, and
                        // it only ever goes back to the connection that sent the line.
                        return Ok(ActOutcome::Receipt { sequence: seq, outcome });
                    }
                    Some(event) => world.apply(&event),
                    None => return Err(OperateError::Closed),
                },
            }
        }
    }
}
