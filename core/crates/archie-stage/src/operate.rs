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
use crate::protocol::{ClientError, StageClient, StageEvent, StageSurface, StageWorld};
use crate::seal::{CommandKey, SealError, Sealer};
use archie_orb::{SignalKind, StageStatus};
use archie_policy::{CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_types::{PrivacyGrade, TaskId, Timestamp, TraceId};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// How long a sealed command stays valid.
const COMMAND_TTL_MS: i64 = 5_000;

fn publish_orb_stage(
    request: &ActRequest,
    surface: &StageSurface,
    operation: u64,
    status: StageStatus,
) {
    archie_orb::publish(&archie_orb::signal(SignalKind::Stage {
        operation_id: operation.to_string(),
        app_id: request.app_id.clone(),
        surface: surface.surface,
        pid: (surface.pid != 0).then_some(surface.pid),
        status,
    }));
}

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
    Receipt {
        sequence: u64,
        outcome: String,
        /// How many lines of a drawing were finished before it stopped.
        finished: usize,
    },
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
    #[error("look takes a snapshot action")]
    NotACapture,
    #[error("the compositor reported a capture it never described")]
    CaptureMissing,
    #[error("the compositor named a capture outside its snapshot directory; it was not read")]
    CaptureOutside,
    #[error("the capture does not match the digest the compositor announced")]
    CaptureInvalid,
    #[error("reading the capture: {0}")]
    CaptureIo(#[from] std::io::Error),
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

/// Applies every event already waiting, and returns without waiting for more. A loop that
/// works in bursts catches up on what the compositor said while nobody was listening.
pub async fn drain<R, W>(
    client: &mut StageClient<R, W>,
    world: &mut StageWorld,
) -> Result<(), OperateError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::ZERO, client.next_event()).await {
            Err(_) => return Ok(()),
            Ok(event) => match event? {
                Some(event) => world.apply(&event),
                None => return Err(OperateError::Closed),
            },
        }
    }
}

/// Reads events into `world` until `settled` holds, or `timeout` passes. Says whether it
/// holds. A control loop waits this way for an application to redraw what it just did,
/// and goes back to work the moment it does.
pub async fn wait_until<R, W>(
    client: &mut StageClient<R, W>,
    world: &mut StageWorld,
    timeout: Duration,
    mut settled: impl FnMut(&StageWorld) -> bool,
) -> Result<bool, OperateError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if settled(world) {
        return Ok(true);
    }
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Ok(false),
            event = client.next_event() => match event? {
                Some(event) => {
                    world.apply(&event);
                    if settled(world) {
                        return Ok(true);
                    }
                }
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
        Ok(self
            .perform(client, world, request, receipt_timeout)
            .await?
            .outcome)
    }

    /// Captures a surface: `request.action` must be [`StageAction::Snapshot`] or
    /// [`StageAction::Frame`]. The capture
    /// is read from `snapshot_dir`, where the compositor beside this control socket keeps
    /// captures, checked against the digest it announced, and removed.
    pub async fn look<R, W>(
        &mut self,
        client: &mut StageClient<R, W>,
        world: &mut StageWorld,
        request: &ActRequest,
        snapshot_dir: &Path,
        receipt_timeout: Duration,
    ) -> Result<LookOutcome, OperateError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        if !matches!(request.action, StageAction::Snapshot { .. } | StageAction::Frame { .. }) {
            return Err(OperateError::NotACapture);
        }
        let performed = self
            .perform(client, world, request, receipt_timeout)
            .await?;
        if !performed.outcome.executed() {
            if let Some(capture) = &performed.capture {
                // A capture written for a command that did not complete is not used.
                let _ = std::fs::remove_file(&capture.path);
            }
            return Ok(LookOutcome::NotCaptured(performed.outcome));
        }
        let (Some(capture), Some(surface)) = (performed.capture, performed.surface) else {
            return Err(OperateError::CaptureMissing);
        };
        let png = take_capture(&capture.path, &capture.sha256, snapshot_dir)?;
        Ok(LookOutcome::Captured(Snapshot {
            surface: surface.surface,
            app_id: surface.app_id,
            geometry_revision: surface.geometry_revision,
            x: capture.x,
            y: capture.y,
            width: capture.width,
            height: capture.height,
            captured_at_ms: performed.sent_at_ms,
            sha256: capture.sha256,
            png,
        }))
    }

    async fn perform<R, W>(
        &mut self,
        client: &mut StageClient<R, W>,
        world: &mut StageWorld,
        request: &ActRequest,
        receipt_timeout: Duration,
    ) -> Result<Performed, OperateError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let unsent = |outcome| Performed {
            outcome,
            capture: None,
            surface: None,
            sent_at_ms: 0,
        };
        let surface = match request.surface {
            Some(id) => world
                .surface(id)
                .filter(|s| s.app_id == request.app_id)
                .cloned(),
            None => world.surfaces_of(&request.app_id).next().cloned(),
        };
        let Some(surface) = surface else {
            return Ok(unsent(ActOutcome::NoSurface));
        };

        // One operation, on this application, for this principal. Policy decides whether
        // there is a lease at all, and a capture asks only to read.
        let now = Timestamp::now();
        let operation = u64::from(surface.surface) << 32 | (now.as_millis() as u64 & 0xffff_ffff);
        let decision = self.engine.decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: request.task_id,
            principal: request.principal.clone(),
            resource: stage_resource(&request.app_id),
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([request.action.kind().policy_action()]),
            justification: request.justification.clone(),
        });
        let Some(lease) = decision.lease else {
            return Ok(unsent(ActOutcome::Refused {
                code: "no_lease",
                message: "policy did not grant a lease for this application".into(),
            }));
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
                return Ok(unsent(ActOutcome::NeedsApproval {
                    reason: reason.to_string(),
                }))
            }
            StageAdmission::Refused(refusal) => {
                return Ok(unsent(ActOutcome::Refused {
                    code: refusal.code(),
                    message: refusal.to_string(),
                }))
            }
        };

        let now = Timestamp::now();
        self.sealer.advance_to(now.as_millis().max(1) as u64);
        let sealed = self.sealer.seal(&command, now, COMMAND_TTL_MS)?;
        publish_orb_stage(request, &surface, operation, StageStatus::Predicted);
        if let Err(error) = client.send(&sealed).await {
            publish_orb_stage(request, &surface, operation, StageStatus::Error);
            return Err(error.into());
        }
        publish_orb_stage(request, &surface, operation, StageStatus::Active);

        let mut capture = None;
        let deadline = tokio::time::sleep(receipt_timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    publish_orb_stage(request, &surface, operation, StageStatus::Error);
                    return Err(OperateError::NoReceipt(receipt_timeout));
                },
                event = client.next_event() => {
                    let event = match event {
                        Err(error) => {
                            publish_orb_stage(request, &surface, operation, StageStatus::Error);
                            return Err(error.into());
                        }
                        Ok(Some(event)) => event,
                        Ok(None) => {
                            publish_orb_stage(request, &surface, operation, StageStatus::Error);
                            return Err(OperateError::Closed);
                        }
                    };
                    match event {
                        StageEvent::Receipt { seq, outcome, finished, .. }
                            if seq == sealed.sequence() || seq == 0 =>
                        {
                            // Sequence 0 answers a line the compositor could not verify, and
                            // it only ever goes back to the connection that sent the line.
                            publish_orb_stage(request, &surface, operation, if outcome == "executed" {
                                StageStatus::Active
                            } else { StageStatus::Error });
                            return Ok(Performed {
                                outcome: ActOutcome::Receipt { sequence: seq, outcome, finished },
                                capture,
                                surface: Some(surface),
                                sent_at_ms: now.as_millis(),
                            });
                        }
                        StageEvent::Snapshot { seq, surface: captured, x, y, width, height, sha256, path }
                            if seq == sealed.sequence() && captured == surface.surface => {
                                capture = Some(CaptureEvent { x, y, width, height, sha256, path });
                            }
                        event => world.apply(&event),
                    }
                },
            }
        }
    }
}

struct Performed {
    outcome: ActOutcome,
    capture: Option<CaptureEvent>,
    surface: Option<StageSurface>,
    sent_at_ms: i64,
}

struct CaptureEvent {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    sha256: String,
    path: String,
}

/// The largest capture read back.
pub const MAX_CAPTURE_BYTES: u64 = 64 * 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// A frame for a live controller is a binary PPM: a text header and raw RGB rows.
const PPM_SIGNATURE: &[u8] = b"P6\n";

/// A captured surface, verified, with its file already removed.
#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub surface: u32,
    pub app_id: String,
    /// The surface's geometry revision the capture was requested against.
    pub geometry_revision: u64,
    /// Where in the surface this capture starts. Zero for a whole window; the corner of
    /// the part asked for otherwise, which is what puts its pixels back in the window's
    /// own coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub captured_at_ms: i64,
    pub sha256: String,
    /// The encoded pixels: PNG for a snapshot, a binary PPM for a frame.
    pub png: Vec<u8>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // What the window showed is not for logs.
        f.debug_struct("Snapshot")
            .field("surface", &self.surface)
            .field("app_id", &self.app_id)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.png.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum LookOutcome {
    Captured(Snapshot),
    /// Not captured, and the reason: a refusal, a missing surface, or the compositor's
    /// receipt, such as `capture_failed` or `operator_has_control`.
    NotCaptured(ActOutcome),
}

/// Reads a capture where only this stage's compositor writes them, checks it is the file
/// that was announced, and removes it whatever the check says.
fn take_capture(path: &str, sha256: &str, snapshot_dir: &Path) -> Result<Vec<u8>, OperateError> {
    let path = Path::new(path);
    let inside = match (
        path.parent().map(std::fs::canonicalize),
        std::fs::canonicalize(snapshot_dir),
    ) {
        (Some(Ok(parent)), Ok(expected)) => parent == expected,
        _ => false,
    };
    if !inside {
        // A message naming a file anywhere else is not followed, and nothing is removed.
        return Err(OperateError::CaptureOutside);
    }
    let read = (|| {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_CAPTURE_BYTES {
            return Err(OperateError::CaptureInvalid);
        }
        let bytes = std::fs::read(path)?;
        let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
        if digest != sha256 || !(bytes.starts_with(PNG_SIGNATURE) || bytes.starts_with(PPM_SIGNATURE)) {
            return Err(OperateError::CaptureInvalid);
        }
        Ok(bytes)
    })();
    let _ = std::fs::remove_file(path);
    read
}
