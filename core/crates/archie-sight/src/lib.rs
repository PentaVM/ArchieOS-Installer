//! # archie-sight
//!
//! Archie looks at a window, finds what a task names in it, acts on it, and checks that
//! the window changed.
//!
//! IRIS perceives and grants no authority; the Archie Stage acts through its gate. This
//! crate is the seam between them and adds no authority of its own:
//!
//! ```text
//! look      archie_stage::Operator::look       a snapshot command: the application's
//!                                               `snapshot` grant, a read lease, the
//!                                               compositor's checks; a digest-verified PNG
//! ground    archie_iris::LocateAnythingClient  a task-scoped Vault capsule naming the
//!                                               target, and that capture, to whichever
//!                                               grounding model was chosen; a box back
//! activate  archie_iris::MotorController       the node's live bounds, a lease on the
//!                                               IRIS source, then StageMotorExecutor:
//!                                               a click through the stage gate again
//! verify    archie_stage::Operator::look       another capture; a window whose pixels did
//!                                               not change is not evidence of anything
//! ```
//!
//! Two gates, deliberately. IRIS's controller authorises acting on what it perceived; the
//! stage authorises operating that application. Either can refuse.
//!
//! # A capture is a disclosure
//!
//! The capture goes to the grounding model's locality. Its grade, `screen_grade`, is the
//! caller's statement about what the window shows, and the grounding client refuses a
//! destination that grade does not permit, exactly as it does for any context. A cloud
//! model whose retention nobody has verified sees public windows only.

#![forbid(unsafe_code)]

use archie_iris::{
    CortexRequest, GroundingProposal, KeyframeReference, LocateAnythingClient, LocateAnythingError,
    MotorController, MotorError, MotorExecutor, MotorOutcome, NodeId, NodeKind, Point,
    PreparedMotorAction, Rect, SceneError, SceneEvent, SceneGraph, SceneNode, SemanticAction,
    SourceId,
};
use archie_model_runtime::{VisionImage, VisionMime};
use archie_policy::{
    Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Resource, Scope,
};
use archie_stage::{
    ActOutcome, ActRequest, LookOutcome, OperateError, Operator, PointerButton, Snapshot,
    StageAction, StageClient, StageWorld,
};
use archie_types::{Locality, PrivacyGrade, Timestamp, TraceId};
use archie_vault::{
    ContextCapsule, ContextGateway, ContextRequest, EntryClass, Vault, VaultKey, VaultRef,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

/// How long an application gets to repaint what a click changed before it is looked at.
const REPAINT_MS: u64 = 350;

#[derive(Debug, thiserror::Error)]
pub enum SightError {
    #[error(transparent)]
    Stage(#[from] OperateError),
    #[error("the window was not captured: {0:?}")]
    NotCaptured(ActOutcome),
    #[error("the target could not be put in a capsule: {0}")]
    Capsule(String),
    #[error(transparent)]
    Grounding(#[from] LocateAnythingError),
    #[error("the grounding does not fit a scene: {0}")]
    Scene(#[from] SceneError),
    #[error("the activation did not complete: {0}")]
    Motor(#[from] MotorError),
    #[error("policy did not grant a lease to act on this source")]
    NoLease,
    #[error("this surface cannot be named as an IRIS source")]
    Identifier,
}

/// What a task wants found and activated.
#[derive(Debug, Clone)]
pub struct Target {
    pub app_id: String,
    /// The scene node grounding will name, such as `button:save`.
    pub node: NodeId,
    pub kind: NodeKind,
    /// What to look for, in words, as it will travel to the grounding model.
    pub description: String,
    pub description_grade: PrivacyGrade,
    /// The caller's statement about what the window shows.
    pub screen_grade: PrivacyGrade,
    pub task_id: String,
}

/// One pass of look, ground, activate and verify.
#[derive(Debug)]
pub struct Sighting {
    pub before: Snapshot,
    pub bbox: Rect,
    pub point: Point,
    pub outcome: MotorOutcome,
    /// The capture taken after the click, ready for the next step.
    pub after: Option<Snapshot>,
}

/// The IRIS source a stage surface is.
pub fn stage_source(surface: u32) -> Result<SourceId, SightError> {
    SourceId::new(format!("stage:{surface}")).map_err(|_| SightError::Identifier)
}

/// A capsule for the grounding model holding exactly one entry, the target's description,
/// for exactly this task. Its vault exists for this call, under a key nobody else holds.
pub fn target_capsule(
    target: &Target,
    destination: Locality,
    now_ms: i64,
) -> Result<ContextCapsule, SightError> {
    let mut key = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| SightError::Capsule("no randomness for the vault key".into()))?;
    let mut vault =
        Vault::open(&VaultKey::from_bytes(key)).map_err(|e| SightError::Capsule(e.to_string()))?;
    let reference =
        VaultRef::parse("vault:stage/target").map_err(|e| SightError::Capsule(e.to_string()))?;
    vault
        .store(
            "archie-sight",
            reference.clone(),
            EntryClass::Document,
            target.description_grade,
            &target.description,
            now_ms,
            None,
        )
        .map_err(|e| SightError::Capsule(e.to_string()))?;
    ContextGateway::new()
        .assemble(
            &mut vault,
            &ContextRequest::new(
                target.task_id.clone(),
                target.task_id.clone(),
                "stage-visual-grounding",
                destination,
                60_000,
            )
            .including(reference),
            now_ms,
        )
        .map_err(|e| SightError::Capsule(e.to_string()))
}

/// The grounding request for one capture of a whole surface: one node, one keyframe whose
/// region is the capture itself, bound to the capture's digest.
pub fn grounding_request(
    snapshot: &Snapshot,
    target: &Target,
) -> Result<CortexRequest, SightError> {
    Ok(CortexRequest {
        task_class: "stage-ui-grounding".into(),
        scene_revision: snapshot.geometry_revision,
        nodes: vec![target.node.clone()],
        keyframes: vec![KeyframeReference {
            source: stage_source(snapshot.surface)?,
            captured_at_ms: snapshot.captured_at_ms,
            roi: Rect::new(0, 0, snapshot.width, snapshot.height)
                .map_err(|_| SightError::Identifier)?,
            content_sha256: snapshot.sha256.clone(),
        }],
        privacy: target.screen_grade,
    })
}

pub fn vision_image(snapshot: &Snapshot) -> VisionImage<'_> {
    VisionImage {
        mime: VisionMime::Png,
        bytes: &snapshot.png,
        width: snapshot.width,
        height: snapshot.height,
    }
}

/// A scene holding the node grounding found, in the surface's own coordinates. The
/// description stays out of it: a scene label is local state, and this one is not needed.
pub fn scene_with(
    proposal: &GroundingProposal,
    snapshot: &Snapshot,
    kind: NodeKind,
    now_ms: i64,
) -> Result<SceneGraph, SightError> {
    let mut scene = SceneGraph::default();
    scene.apply(
        0,
        &[SceneEvent::Upsert(SceneNode {
            id: proposal.referenced_node.clone(),
            source: stage_source(snapshot.surface)?,
            kind,
            bbox: Some(proposal.bbox),
            parent: None,
            track_id: None,
            // A grounding model gives a box, not a calibrated confidence.
            confidence_permille: 500,
            uncertainty_permille: 500,
            last_seen_ms: now_ms,
            label: None,
            revision: 0,
        })],
    )?;
    Ok(scene)
}

/// IRIS's hands on the stage. An activation becomes a click through the stage gate, and it
/// counts as observed only when a capture afterwards differs from the one grounding used.
pub struct StageMotorExecutor<'a, R, W> {
    pub operator: &'a mut Operator,
    pub client: &'a mut StageClient<R, W>,
    pub world: &'a mut StageWorld,
    pub snapshot_dir: PathBuf,
    pub app_id: String,
    pub surface: u32,
    pub principal: Principal,
    pub before_sha256: String,
    pub timeout: Duration,
    pub after: Option<Snapshot>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> MotorExecutor for StageMotorExecutor<'_, R, W> {
    fn execute(&mut self, action: &PreparedMotorAction) -> Result<MotorOutcome, MotorError> {
        let target = action.target.ok_or(MotorError::MissingTarget)?;
        let click = ActRequest {
            app_id: self.app_id.clone(),
            surface: Some(self.surface),
            action: StageAction::Click {
                x: target.x,
                y: target.y,
                button: PointerButton::Left,
            },
            text_grade: PrivacyGrade::Public,
            principal: self.principal.clone(),
            task_id: None,
            user_present: false,
            approved: false,
            justification: "activate a node IRIS grounded".into(),
        };
        let look = ActRequest {
            action: StageAction::Snapshot,
            justification: "see what the activation changed".into(),
            ..click.clone()
        };
        let handle = tokio::runtime::Handle::current();
        // The controller is synchronous; the stage is a socket. This thread lends itself to
        // the runtime for the duration, which needs a multi-threaded runtime.
        let (clicked, after) = tokio::task::block_in_place(|| {
            handle.block_on(async {
                let clicked = self
                    .operator
                    .act(self.client, self.world, &click, self.timeout)
                    .await?;
                if !clicked.executed() {
                    return Ok::<_, OperateError>((clicked, None));
                }
                tokio::time::sleep(Duration::from_millis(REPAINT_MS)).await;
                let after = self
                    .operator
                    .look(
                        self.client,
                        self.world,
                        &look,
                        &self.snapshot_dir,
                        self.timeout,
                    )
                    .await?;
                Ok((clicked, Some(after)))
            })
        })
        .map_err(|_| MotorError::Executor)?;
        if !clicked.executed() {
            return Err(MotorError::Executor);
        }
        let changed = match after {
            Some(LookOutcome::Captured(snapshot)) => {
                let changed = snapshot.sha256 != self.before_sha256;
                self.after = Some(snapshot);
                changed
            }
            // Clicked, but not seen: that is not verification either.
            _ => false,
        };
        Ok(MotorOutcome {
            observed_scene_revision: if changed {
                action.scene_revision + 1
            } else {
                action.scene_revision
            },
            outcome_code: if changed {
                "clicked_and_changed"
            } else {
                "clicked_without_change"
            }
            .into(),
        })
    }
}

/// Everything one pass needs from the caller.
pub struct Sight<'a, R, W> {
    pub operator: &'a mut Operator,
    pub client: &'a mut StageClient<R, W>,
    pub world: &'a mut StageWorld,
    pub snapshot_dir: PathBuf,
    pub principal: Principal,
    pub engine: PolicyEngine,
    pub timeout: Duration,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Sight<'_, R, W> {
    /// Captures the target application's window through the stage gate.
    pub async fn look(
        &mut self,
        target: &Target,
        surface: Option<u32>,
    ) -> Result<Snapshot, SightError> {
        let request = ActRequest {
            app_id: target.app_id.clone(),
            surface,
            action: StageAction::Snapshot,
            text_grade: PrivacyGrade::Public,
            principal: self.principal.clone(),
            task_id: None,
            user_present: false,
            approved: false,
            justification: "look for what the task names".into(),
        };
        match self
            .operator
            .look(
                self.client,
                self.world,
                &request,
                &self.snapshot_dir,
                self.timeout,
            )
            .await?
        {
            LookOutcome::Captured(snapshot) => Ok(snapshot),
            LookOutcome::NotCaptured(outcome) => Err(SightError::NotCaptured(outcome)),
        }
    }

    /// Looks, grounds the target in the capture, activates it through IRIS's controller
    /// and the stage, and verifies with another look. Call from a multi-threaded runtime.
    pub async fn find_and_activate(
        &mut self,
        target: &Target,
        grounding: &LocateAnythingClient,
        grounding_locality: Locality,
        operation_id: u64,
    ) -> Result<Sighting, SightError> {
        let before = self.look(target, None).await?;
        let now_ms = Timestamp::now().as_millis();
        let capsule = target_capsule(target, grounding_locality, now_ms)?;
        let request = grounding_request(&before, target)?;
        let proposal = grounding
            .ground(
                &target.task_id,
                &request,
                &capsule,
                vision_image(&before),
                now_ms,
            )
            .await?;

        let scene = scene_with(&proposal, &before, target.kind, now_ms)?;
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: proposal.referenced_node.clone(),
            },
            operation_id,
        )?;
        let point = prepared.target.ok_or(MotorError::MissingTarget)?;
        let mut lease = self
            .engine
            .decide(&CapabilityRequest {
                trace_id: TraceId::new(),
                task_id: None,
                principal: self.principal.clone(),
                resource: prepared.policy_resource.clone(),
                scope: Scope::OneOperation {
                    operation: operation_id,
                },
                constraints: Constraints::with_actions([Action::Execute]),
                justification: "activate what the task named".into(),
            })
            .lease
            .ok_or(SightError::NoLease)?;
        debug_assert!(matches!(prepared.policy_resource, Resource::Process { .. }));

        let mut executor = StageMotorExecutor {
            operator: &mut *self.operator,
            client: &mut *self.client,
            world: &mut *self.world,
            snapshot_dir: self.snapshot_dir.clone(),
            app_id: target.app_id.clone(),
            surface: before.surface,
            principal: self.principal.clone(),
            before_sha256: before.sha256.clone(),
            timeout: self.timeout,
            after: None,
        };
        let outcome = MotorController::execute(
            &mut lease,
            self.principal.clone(),
            None,
            false,
            &prepared,
            &scene,
            Timestamp::now(),
            &mut executor,
        )?;
        let after = executor.after.take();
        Ok(Sighting {
            before,
            bbox: proposal.bbox,
            point,
            outcome,
            after,
        })
    }
}
