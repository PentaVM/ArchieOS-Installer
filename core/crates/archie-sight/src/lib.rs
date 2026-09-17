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
pub use archie_a11y::{control_named, Control, DeclareError};

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
use std::sync::Arc;
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
    #[error("an ask that was already under way did not finish")]
    Interrupted,
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
    /// It had moved, and the local model found it again in a fresh capture without
    /// anybody being asked.
    Refound,
    /// The application itself said where it is, through the accessibility bus. No
    /// capture, no model: the fastest and the most exact of the four.
    Declared,
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
            Self::Declared => "declared",
            Self::Refound => "refound",
            Self::Grounded => "grounded",
            Self::Trusted => "trusted",
            Self::Confirmed => "confirmed",
        }
    }
}

/// Where the time went, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timings {
    /// Asking the local model what there is to click in a capture.
    pub recognise_ms: u64,
    /// Asking the application what it declares, when there is a bus to ask.
    pub declare_ms: u64,
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

/// Something that can say what an application declares about its own controls, such as
/// `archie_a11y::Accessibility`.
///
/// This is perception and grants nothing. What it returns is a proposal like a grounding
/// model's: the stage gate still has to allow the click, and the window still has to react
/// to it.
pub trait Declares: Send + Sync {
    fn controls<'a>(
        &'a self,
        pid: u32,
        window: (i32, i32),
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<Control>, DeclareError>> + Send + 'a>,
    >;
}

#[cfg(unix)]
impl Declares for archie_a11y::Accessibility {
    fn controls<'a>(
        &'a self,
        pid: u32,
        window: (i32, i32),
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<Control>, DeclareError>> + Send + 'a>,
    > {
        Box::pin(self.controls_of(pid, Some(window)))
    }
}


/// Something that can find controls in a picture of a window, such as the local model in
/// `archie-reflex`.
///
/// It says where things that look like controls are, never what they say. Naming is what
/// an application's declarations are for, and what a grounding model is asked for when
/// nothing declares. Like every other seam here, it advises: the stage gate still has to
/// allow the click and the window still has to react.
pub trait Sees: Send + Sync {
    /// Every control it finds in that capture, in the window's own coordinates.
    fn controls_in(&self, png: &[u8]) -> Vec<Rect>;
}

impl Sees for archie_reflex::Deployed {
    fn controls_in(&self, png: &[u8]) -> Vec<Rect> {
        let Ok(tile) = archie_reflex::Tile::from_png(png) else {
            return Vec::new();
        };
        self.detect(&tile, RECOGNISED_CONFIDENCE)
            .into_iter()
            .filter_map(|found| {
                let (x, y, width, height) =
                    found.control.in_window(tile.source_width, tile.source_height);
                Rect::new(x.max(0), y.max(0), width, height).ok()
            })
            .collect()
    }
}

/// How sure the local model has to be before its box is worth using. Swept on the windows
/// the trained model was never taught (`archie-reflex check`): at 600 it finds 784 of every
/// 1000 declared controls and says 453 things per 1000 that nothing declared, at 800 it
/// finds 740 and says 344. This moves a mouse, so it takes the surer end of that.
pub const RECOGNISED_CONFIDENCE: u16 = 800;

/// The share of the smaller box two boxes share.
fn shared(a: Rect, b: Rect) -> f32 {
    let x = (a.right().min(b.right()) - i64::from(a.x).max(i64::from(b.x))) as f32;
    let y = (a.bottom().min(b.bottom()) - i64::from(a.y).max(i64::from(b.y))) as f32;
    if x <= 0.0 || y <= 0.0 {
        return 0.0;
    }
    let smaller = ((a.width * a.height).min(b.width * b.height)).max(1) as f32;
    (x * y) / smaller
}

/// A grounding model points near a control; the local model knows where the control
/// actually is. When the two are about the same thing, the control wins. It is the same
/// rule wherever an answer arrives, in the foreground or from a background ask.
fn snapped_to(sees: Option<&Arc<dyn Sees>>, png: &[u8], bbox: Rect) -> Rect {
    nearest_control(sees, png, bbox).unwrap_or(bbox)
}

/// The control the local model sees nearest to where something else pointed, when it is
/// near enough to be about the same thing.
fn nearest_control(sees: Option<&Arc<dyn Sees>>, png: &[u8], bbox: Rect) -> Option<Rect> {
    sees?
        .controls_in(png)
        .into_iter()
        .filter(|control| shared(*control, bbox) >= 0.3)
        .max_by(|a, b| {
            shared(*a, bbox)
                .partial_cmp(&shared(*b, bbox))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

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
    /// The local model that finds controls in a picture, when one is loaded.
    sees: Option<Arc<dyn Sees>>,
    /// Targets a grounding model is being asked about right now. Asking takes seconds; the
    /// loop goes on working and waits for an answer only when it needs that one.
    pending: BTreeMap<(String, String), tokio::task::JoinHandle<Result<Found, SightError>>>,
    /// What applications say about themselves, when a session has somewhere to ask.
    declares: Option<Box<dyn Declares>>,
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
            sees: None,
            pending: BTreeMap::new(),
            declares: None,
            uses: 0,
        }
    }


    /// Uses a local model to see what there is to click in a window, which puts a
    /// grounding model's answer on a real control and finds a control that has moved.
    pub fn recognising(mut self, sees: Arc<dyn Sees>) -> Self {
        self.sees = Some(sees);
        self
    }


    /// The same control somewhere else: what the local model sees that is about the size
    /// of the box that was there, and still looks like what was there.
    fn refind(&self, known: &Known, snapshot: &Snapshot) -> Option<Rect> {
        let sees = self.sees.as_ref()?;
        let mut best: Option<(Rect, f32)> = None;
        let (was_x, was_y) = (
            known.bbox.x as f32 + known.bbox.width as f32 / 2.0,
            known.bbox.y as f32 + known.bbox.height as f32 / 2.0,
        );
        for control in sees.controls_in(&snapshot.png) {
            let alike = |a: u32, b: u32| a as f32 >= b as f32 * 0.5 && a as f32 <= b as f32 * 1.8;
            if !alike(control.width, known.bbox.width) || !alike(control.height, known.bbox.height) {
                continue;
            }
            let Ok(now) = appearance_of(&snapshot.png, control) else {
                continue;
            };
            if !now.matches(&known.appearance) {
                continue;
            }
            let distance = ((control.x as f32 + control.width as f32 / 2.0 - was_x).powi(2)
                + (control.y as f32 + control.height as f32 / 2.0 - was_y).powi(2))
            .sqrt();
            if best.is_none() || best.is_some_and(|(_, so_far)| distance < so_far) {
                best = Some((control, distance));
            }
        }
        best.map(|(control, _)| control)
    }

    /// Remembers a control that moved, keeping what it looked like when it was found.
    fn moved_to(&mut self, key: &(String, String), known: &Known, bbox: Rect, mark: u64) {
        self.uses += 1;
        self.known.insert(
            key.clone(),
            Known {
                bbox,
                mark,
                used: self.uses,
                ..known.clone()
            },
        );
    }


    /// The same rule as `snapped_to`, for a session that has a capture in hand.
    fn snapped(&self, snapshot: &Snapshot, bbox: Rect) -> Rect {
        snapped_to(self.sees.as_ref(), &snapshot.png, bbox)
    }

    /// Reads applications' own accounts of their controls before looking at any pixels.
    pub fn asking(mut self, declares: Box<dyn Declares>) -> Self {
        self.declares = Some(declares);
        self
    }

    /// Where the application says the target is, if it says anything unambiguous about it.
    /// Nothing is remembered from this: it is read again each time, so it cannot go stale.
    async fn declared(
        &self,
        target: &Target,
        surface: &archie_stage::StageSurface,
    ) -> Option<Rect> {
        let declares = self.declares.as_ref()?;
        if surface.pid == 0 {
            return None;
        }
        let controls = declares
            .controls(surface.pid, (surface.width, surface.height))
            .await
            .ok()?;
        let control = control_named(&controls, &target.description, target.kind)?;
        Some(control.bounds)
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
        for (_, handle) in std::mem::take(&mut self.pending) {
            handle.abort();
        }
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

    /// Starts finding targets nothing else can answer for, without acting on any of them:
    /// one capture now, and a grounding model asked about each of them in the background.
    /// Returns how many are being asked about.
    ///
    /// This is what keeps a slow model out of the way. While it thinks, Archie goes on
    /// clicking the controls it already knows, and a target whose answer has arrived by the
    /// time it is wanted costs nothing at all.
    pub async fn start_finding(
        &mut self,
        targets: &[Target],
        grounding: &Arc<LocateAnythingClient>,
        destination: Locality,
    ) -> Result<usize, SightError> {
        drain(&mut self.client, &mut self.world).await?;
        self.collect_found().await;
        self.collect_found().await;
        let mut asking = Vec::new();
        for target in targets {
            let key = (target.app_id.clone(), target.node.as_str().to_string());
            if self.pending.contains_key(&key)
                || self.known.contains_key(&key)
                || asking.iter().any(|(asked, _, _)| asked == &key)
            {
                continue;
            }
            let Some(surface) = self.world.surfaces_of(&target.app_id).next().cloned() else {
                continue;
            };
            // A control the application itself names needs no model, now or later.
            if self.declared(target, &surface).await.is_some() {
                continue;
            }
            asking.push((key, target.clone(), surface));
        }
        if asking.is_empty() {
            return Ok(0);
        }

        let mut started = 0;
        // One capture answers for every target in the same window.
        let mut captured: BTreeMap<u32, (Arc<Snapshot>, u64)> = BTreeMap::new();
        for (key, target, surface) in asking {
            let (snapshot, mark) = match captured.get(&surface.surface) {
                Some(taken) => taken.clone(),
                None => {
                    let mark = self.world.damage_mark();
                    let snapshot =
                        Arc::new(self.look(&target.app_id, Some(surface.surface)).await?);
                    self.confirm_known(&snapshot, mark);
                    captured.insert(surface.surface, (snapshot.clone(), mark));
                    (snapshot, mark)
                }
            };
            self.pending.insert(
                key,
                tokio::spawn(find_in_background(
                    target.clone(),
                    snapshot,
                    grounding.clone(),
                    destination,
                    mark,
                    self.sees.clone(),
                )),
            );
            started += 1;
        }
        Ok(started)
    }

    /// Takes in every answer that has arrived, and leaves the rest still being asked.
    async fn collect_found(&mut self) {
        let arrived: Vec<(String, String)> = self
            .pending
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(key, _)| key.clone())
            .collect();
        for key in arrived {
            if let Some(handle) = self.pending.remove(&key) {
                if let Ok(Ok(found)) = handle.await {
                    self.keep(&key, found);
                }
            }
        }
    }

    /// Remembers what a background ask found, as if this session had just seen it.
    fn keep(&mut self, key: &(String, String), found: Found) {
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
                surface: found.surface,
                geometry_revision: found.geometry_revision,
                description: found.description,
                kind: found.kind,
                bbox: found.bbox,
                appearance: found.appearance,
                mark: found.mark,
                used: self.uses,
            },
        );
    }

    async fn ground(
        &self,
        target: &Target,
        snapshot: &Snapshot,
        grounding: &LocateAnythingClient,
        destination: Locality,
    ) -> Result<Rect, SightError> {
        ground(target, snapshot, grounding, destination).await
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

        // What the application says about itself comes first: it is exact, it is live,
        // and it costs neither a capture nor a model.
        let at = Instant::now();
        let declared = self.declared(target, &surface).await;
        timings.declare_ms = at.elapsed().as_millis() as u64;
        // An answer already on its way is the next cheapest thing after a declaration.
        let waiting = if declared.is_none() && remembered.is_none() {
            self.pending.remove(&key)
        } else {
            None
        };

        let (bbox, knowledge, before) = if let Some(bbox) = declared {
            (bbox, Knowledge::Declared, None)
        } else if let Some(handle) = waiting {
            // Asked already, while Archie was doing something else.
            let at = Instant::now();
            let found = handle.await.map_err(|_| SightError::Interrupted)??;
            timings.ground_ms = at.elapsed().as_millis() as u64;
            let bbox = found.bbox;
            self.keep(&key, found);
            (bbox, Knowledge::Grounded, None)
        } else {
            match remembered {
                Some(known)
                    if self.world.repainted_since(
                        known.mark,
                        known.surface,
                        &watched(known.bbox),
                    ) == Repainted::No =>
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
                            // Not where it was. Before asking a model again, the local one
                            // is asked whether the same control is somewhere near.
                            let at = Instant::now();
                            let moved = self.refind(&known, &snapshot);
                            timings.recognise_ms = at.elapsed().as_millis() as u64;
                            if let Some(bbox) = moved {
                                self.moved_to(&key, &known, bbox, mark);
                                (bbox, Knowledge::Refound, Some(snapshot))
                            } else {
                                let at = Instant::now();
                                let bbox = self
                                    .ground(target, &snapshot, grounding, grounding_locality)
                                    .await?;
                                timings.ground_ms = at.elapsed().as_millis() as u64;
                                let bbox = self.snapped(&snapshot, bbox);
                                self.remember(&key, target, &snapshot, bbox, mark)?;
                                (bbox, Knowledge::Grounded, Some(snapshot))
                            }
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
                    let at = Instant::now();
                    let bbox = self.snapped(&snapshot, bbox);
                    timings.recognise_ms = at.elapsed().as_millis() as u64;
                    self.remember(&key, target, &snapshot, bbox, mark)?;
                    (bbox, Knowledge::Grounded, Some(snapshot))
                }
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

/// Asks a grounding model where one target is in one capture.
async fn ground(
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

/// A position a grounding model found, ready to be remembered.
struct Found {
    bbox: Rect,
    appearance: Appearance,
    mark: u64,
    surface: u32,
    geometry_revision: u64,
    description: String,
    kind: NodeKind,
}

/// One background ask: the model, then the shape of what it pointed at, so the session can
/// recognise that control again without asking anything.
async fn find_in_background(
    target: Target,
    snapshot: Arc<Snapshot>,
    grounding: Arc<LocateAnythingClient>,
    destination: Locality,
    mark: u64,
    sees: Option<Arc<dyn Sees>>,
) -> Result<Found, SightError> {
    let bbox = ground(&target, &snapshot, &grounding, destination).await?;
    let bbox = snapped_to(sees.as_ref(), &snapshot.png, bbox);
    Ok(Found {
        bbox,
        appearance: appearance_of(&snapshot.png, bbox)?,
        mark,
        surface: snapshot.surface,
        geometry_revision: snapshot.geometry_revision,
        description: target.description,
        kind: target.kind,
    })
}
