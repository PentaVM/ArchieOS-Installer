//! # archie-sight
//!
//! Archie looks at a window, finds what a task names in it, acts on it, and checks that
//! the window reacted.
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
//! verify    the compositor's damage events     the control redrew itself, or the click
//!                                               did nothing and is reported as such
//! ```
//!
//! Two gates, deliberately. IRIS's controller authorises acting on what it perceived; the
//! stage authorises operating that application. Either can refuse.
//!
//! # Grounding is not in the loop
//!
//! A grounding model answers in seconds; a hand works in tenths of one. So a [`Sight`]
//! lives as long as the work does and remembers where it found each target. It asks the
//! model again only when it must:
//!
//! - nothing has been drawn over the box since it was found: use it ([`Knowledge::Trusted`]);
//! - something has: look once, and if the box still holds the same control, use it
//!   ([`Knowledge::Confirmed`]);
//! - it does not, or the window resized: ask the model ([`Knowledge::Grounded`]).
//!
//! What confirms a box is [`Appearance`], standing in for the local model IRIS's fast path
//! is meant to have. It decides nothing: it only says whether to look again or ask again.
//!
//! # A capture is a disclosure
//!
//! The capture goes to the grounding model's locality. Its grade, `screen_grade`, is the
//! caller's statement about what the window shows, and the grounding client refuses a
//! destination that grade does not permit, exactly as it does for any context. A cloud
//! model whose retention nobody has verified sees public windows only.

#![forbid(unsafe_code)]

mod appearance;

pub use appearance::{appearance_of, Appearance, AppearanceError};

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
    drain, wait_until, ActOutcome, ActRequest, DamageRect, LookOutcome, OperateError, Operator,
    PointerButton, Repainted, Snapshot, StageAction, StageClient, StageWorld,
};
use archie_types::{Locality, PrivacyGrade, Timestamp, TraceId};
use archie_vault::{
    ContextCapsule, ContextGateway, ContextRequest, EntryClass, Vault, VaultKey, VaultRef,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};

/// How long an application has to redraw what a click changed before the click counts as
/// having changed nothing.
pub const REPAINT_TIMEOUT: Duration = Duration::from_millis(300);
/// How long the compositor has to answer a command.
pub const RECEIPT_TIMEOUT: Duration = Duration::from_secs(30);
/// A control is redrawn a little outside its own box: a focus ring, a shadow.
const REPAINT_MARGIN_PX: i32 = 8;
/// How many targets one session remembers before it forgets the least used.
const REMEMBERED_TARGETS: usize = 256;

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
    #[error("the capture says nothing about that box: {0}")]
    Appearance(#[from] AppearanceError),
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

/// How a target's position was known when it was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knowledge {
    /// A grounding model was asked, because nothing usable was known.
    Grounded,
    /// Known already, and nothing had been drawn over it since.
    Trusted,
    /// Known already, something had been drawn there, and a fresh capture still shows the
    /// same control in the same place.
    Confirmed,
}

impl Knowledge {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grounded => "grounded",
            Self::Trusted => "trusted",
            Self::Confirmed => "confirmed",
        }
    }
}

/// Where the time went, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timings {
    /// Capturing the window, when this pass needed a capture.
    pub look_ms: u64,
    /// Waiting for the grounding model, when this pass had to ask it.
    pub ground_ms: u64,
    /// The click itself, from its lease to the compositor's receipt.
    pub act_ms: u64,
    /// Waiting for the control to redraw.
    pub verify_ms: u64,
    pub total_ms: u64,
}

/// One pass of look, ground, activate and verify.
#[derive(Debug)]
pub struct Sighting {
    /// The capture this pass used, when it needed one.
    pub before: Option<Snapshot>,
    pub bbox: Rect,
    pub point: Point,
    pub knowledge: Knowledge,
    pub outcome: MotorOutcome,
    pub timings: Timings,
}

/// What a session remembers about a target it has already found.
#[derive(Debug, Clone)]
struct Known {
    surface: u32,
    geometry_revision: u64,
    description: String,
    kind: NodeKind,
    bbox: Rect,
    appearance: Appearance,
    /// Where the record of repaints stood when that appearance was last true.
    mark: u64,
    /// When it was last used, for forgetting the least used first.
    used: u64,
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

/// A scene holding one node where it is on screen, in the surface's own coordinates. The
/// description stays out of it: a scene label is local state, and this one is not needed.
pub fn scene_with(
    node: &NodeId,
    bbox: Rect,
    surface: u32,
    kind: NodeKind,
    now_ms: i64,
) -> Result<SceneGraph, SightError> {
    let mut scene = SceneGraph::default();
    scene.apply(
        0,
        &[SceneEvent::Upsert(SceneNode {
            id: node.clone(),
            source: stage_source(surface)?,
            kind,
            bbox: Some(bbox),
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

/// Where a control is expected to redraw itself when it reacts.
fn watched(bbox: Rect) -> DamageRect {
    DamageRect {
        x: bbox.x,
        y: bbox.y,
        width: i32::try_from(bbox.width).unwrap_or(i32::MAX),
        height: i32::try_from(bbox.height).unwrap_or(i32::MAX),
    }
    .grown(REPAINT_MARGIN_PX)
}

/// IRIS's hands on the stage. An activation becomes a click through the stage gate, and it
/// counts as observed only when the control redraws itself afterwards.
pub struct StageMotorExecutor<'a, R, W> {
    pub operator: &'a mut Operator,
    pub client: &'a mut StageClient<R, W>,
    pub world: &'a mut StageWorld,
    pub app_id: String,
    pub surface: u32,
    pub principal: Principal,
    /// Where a repaint has to land for the click to count as having done anything.
    pub watch: DamageRect,
    pub receipt_timeout: Duration,
    pub repaint_timeout: Duration,
    /// Filled in by the click: how long it took, and how long its repaint took.
    pub act_ms: u64,
    pub verify_ms: u64,
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
        let handle = tokio::runtime::Handle::current();
        // The controller is synchronous; the stage is a socket. This thread lends itself to
        // the runtime for the duration, which needs a multi-threaded runtime.
        let (clicked, repainted, act_ms, verify_ms) = tokio::task::block_in_place(|| {
            handle.block_on(async {
                let mark = self.world.damage_mark();
                let at = Instant::now();
                let clicked = self
                    .operator
                    .act(self.client, self.world, &click, self.receipt_timeout)
                    .await?;
                let act_ms = at.elapsed().as_millis() as u64;
                if !clicked.executed() {
                    return Ok::<_, OperateError>((clicked, false, act_ms, 0));
                }
                let at = Instant::now();
                let (surface, watch) = (self.surface, self.watch);
                let repainted =
                    wait_until(self.client, self.world, self.repaint_timeout, |world| {
                        world.repainted_since(mark, surface, &watch) == Repainted::Yes
                    })
                    .await?;
                Ok((clicked, repainted, act_ms, at.elapsed().as_millis() as u64))
            })
        })
        .map_err(|_| MotorError::Executor)?;
        self.act_ms = act_ms;
        self.verify_ms = verify_ms;
        if !clicked.executed() {
            return Err(MotorError::Executor);
        }
        Ok(MotorOutcome {
            observed_scene_revision: if repainted {
                action.scene_revision + 1
            } else {
                action.scene_revision
            },
            outcome_code: if repainted {
                "clicked_and_repainted"
            } else {
                "clicked_without_repaint"
            }
            .into(),
        })
    }
}

/// One session of looking and acting on one stage, holding what it has learned.
pub struct Sight<R, W> {
    operator: Operator,
    client: StageClient<R, W>,
    world: StageWorld,
    snapshot_dir: PathBuf,
    principal: Principal,
    engine: PolicyEngine,
    receipt_timeout: Duration,
    repaint_timeout: Duration,
    known: BTreeMap<(String, String), Known>,
    uses: u64,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Sight<R, W> {
    pub fn new(
        operator: Operator,
        client: StageClient<R, W>,
        world: StageWorld,
        snapshot_dir: PathBuf,
        principal: Principal,
    ) -> Self {
        Self {
            operator,
            client,
            world,
            snapshot_dir,
            principal,
            engine: PolicyEngine::default(),
            receipt_timeout: RECEIPT_TIMEOUT,
            repaint_timeout: REPAINT_TIMEOUT,
            known: BTreeMap::new(),
            uses: 0,
        }
    }

    pub fn with_timeouts(mut self, receipt: Duration, repaint: Duration) -> Self {
        self.receipt_timeout = receipt;
        self.repaint_timeout = repaint;
        self
    }

    /// The stage as this session last heard it described.
    pub fn world(&self) -> &StageWorld {
        &self.world
    }

    /// Forgets every position, so the next pass asks the grounding model again.
    pub fn forget_all(&mut self) {
        self.known.clear();
    }

    /// Captures the application's window through the stage gate.
    pub async fn look(
        &mut self,
        app_id: &str,
        surface: Option<u32>,
    ) -> Result<Snapshot, SightError> {
        let request = ActRequest {
            app_id: app_id.to_string(),
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
                &mut self.client,
                &mut self.world,
                &request,
                &self.snapshot_dir,
                self.receipt_timeout,
            )
            .await?
        {
            LookOutcome::Captured(snapshot) => Ok(snapshot),
            LookOutcome::NotCaptured(outcome) => Err(SightError::NotCaptured(outcome)),
        }
    }

    async fn ground(
        &self,
        target: &Target,
        snapshot: &Snapshot,
        grounding: &LocateAnythingClient,
        destination: Locality,
    ) -> Result<Rect, SightError> {
        let now_ms = Timestamp::now().as_millis();
        let capsule = target_capsule(target, destination, now_ms)?;
        let request = grounding_request(snapshot, target)?;
        let proposal: GroundingProposal = grounding
            .ground(
                &target.task_id,
                &request,
                &capsule,
                vision_image(snapshot),
                now_ms,
            )
            .await?;
        Ok(proposal.bbox)
    }

    /// One capture says something about every control this session knows in that window.
    /// The ones still where they were stay usable without another look; the ones that are
    /// not are forgotten rather than clicked on faith.
    fn confirm_known(&mut self, snapshot: &Snapshot, mark: u64) {
        let mut forget = Vec::new();
        for (key, known) in self.known.iter_mut() {
            if known.surface != snapshot.surface
                || known.geometry_revision != snapshot.geometry_revision
            {
                continue;
            }
            match appearance_of(&snapshot.png, known.bbox) {
                Ok(now) if now.matches(&known.appearance) => known.mark = mark,
                _ => forget.push(key.clone()),
            }
        }
        for key in forget {
            self.known.remove(&key);
        }
    }

    fn remember(
        &mut self,
        key: &(String, String),
        target: &Target,
        snapshot: &Snapshot,
        bbox: Rect,
        mark: u64,
    ) -> Result<(), SightError> {
        let appearance = appearance_of(&snapshot.png, bbox)?;
        self.uses += 1;
        if self.known.len() >= REMEMBERED_TARGETS && !self.known.contains_key(key) {
            if let Some(stale) = self
                .known
                .iter()
                .min_by_key(|(_, known)| known.used)
                .map(|(key, _)| key.clone())
            {
                self.known.remove(&stale);
            }
        }
        self.known.insert(
            key.clone(),
            Known {
                surface: snapshot.surface,
                geometry_revision: snapshot.geometry_revision,
                description: target.description.clone(),
                kind: target.kind,
                bbox,
                appearance,
                mark,
                used: self.uses,
            },
        );
        Ok(())
    }

    /// Finds the target when its position is not known or no longer trustworthy, activates
    /// it through IRIS's controller and the stage gate, and checks that it reacted.
    /// Call from a multi-threaded runtime.
    pub async fn find_and_activate(
        &mut self,
        target: &Target,
        grounding: &LocateAnythingClient,
        grounding_locality: Locality,
        operation_id: u64,
    ) -> Result<Sighting, SightError> {
        let started = Instant::now();
        let mut timings = Timings::default();
        // What the compositor said while nobody was listening counts too.
        drain(&mut self.client, &mut self.world).await?;
        let surface = self
            .world
            .surfaces_of(&target.app_id)
            .next()
            .cloned()
            .ok_or(SightError::NotCaptured(ActOutcome::NoSurface))?;
        let key = (target.app_id.clone(), target.node.as_str().to_string());
        let remembered = self.known.get(&key).cloned().filter(|known| {
            known.surface == surface.surface
                && known.geometry_revision == surface.geometry_revision
                && known.description == target.description
                && known.kind == target.kind
        });

        let (bbox, knowledge, before) = match remembered {
            Some(known)
                if self
                    .world
                    .repainted_since(known.mark, known.surface, &watched(known.bbox))
                    == Repainted::No =>
            {
                (known.bbox, Knowledge::Trusted, None)
            }
            Some(known) => {
                // Something was drawn where it was: look once before believing the old box.
                let mark = self.world.damage_mark();
                let at = Instant::now();
                let snapshot = self.look(&target.app_id, Some(known.surface)).await?;
                timings.look_ms = at.elapsed().as_millis() as u64;
                self.confirm_known(&snapshot, mark);
                match self.known.get(&key).cloned() {
                    Some(fresh) if fresh.mark == mark => {
                        (fresh.bbox, Knowledge::Confirmed, Some(snapshot))
                    }
                    _ => {
                        let at = Instant::now();
                        let bbox = self
                            .ground(target, &snapshot, grounding, grounding_locality)
                            .await?;
                        timings.ground_ms = at.elapsed().as_millis() as u64;
                        self.remember(&key, target, &snapshot, bbox, mark)?;
                        (bbox, Knowledge::Grounded, Some(snapshot))
                    }
                }
            }
            None => {
                let mark = self.world.damage_mark();
                let at = Instant::now();
                let snapshot = self.look(&target.app_id, Some(surface.surface)).await?;
                timings.look_ms = at.elapsed().as_millis() as u64;
                self.confirm_known(&snapshot, mark);
                let at = Instant::now();
                let bbox = self
                    .ground(target, &snapshot, grounding, grounding_locality)
                    .await?;
                timings.ground_ms = at.elapsed().as_millis() as u64;
                self.remember(&key, target, &snapshot, bbox, mark)?;
                (bbox, Knowledge::Grounded, Some(snapshot))
            }
        };

        let now_ms = Timestamp::now().as_millis();
        let scene = scene_with(&target.node, bbox, surface.surface, target.kind, now_ms)?;
        let prepared = MotorController::prepare(
            &scene,
            &SemanticAction::Activate {
                node: target.node.clone(),
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
            operator: &mut self.operator,
            client: &mut self.client,
            world: &mut self.world,
            app_id: target.app_id.clone(),
            surface: surface.surface,
            principal: self.principal.clone(),
            watch: watched(bbox),
            receipt_timeout: self.receipt_timeout,
            repaint_timeout: self.repaint_timeout,
            act_ms: 0,
            verify_ms: 0,
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
        );
        timings.act_ms = executor.act_ms;
        timings.verify_ms = executor.verify_ms;
        timings.total_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(outcome) => Ok(Sighting {
                before,
                bbox,
                point,
                knowledge,
                outcome,
                timings,
            }),
            Err(error) => {
                // A control that did not react is not where this thought it was.
                self.known.remove(&key);
                Err(error.into())
            }
        }
    }
}
