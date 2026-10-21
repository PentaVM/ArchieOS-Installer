//! Live foci: results at the camera's rate, inference at the GPU's.
//!
//! A focus (a camera, a screen) posts every frame. Each frame is answered at once from the
//! CPU: its damage against the previous frame is measured, the planner decides
//! (`archie_accel::plan`), and the last inference's boxes are carried to this frame by
//! block matching (`archie_accel::track`). When the planner asks for an inference, the
//! frame goes to a queue where the newest frame of a focus replaces an older one, and a
//! worker runs the model on it. Its boxes re-anchor the tracker; the next frames are
//! answered from them.
//!
//! So a still scene costs nothing after its first inference, a moving one is answered at
//! the frame rate with tracked boxes, and inference runs only as often as the focus's budget
//! and the GPU allow. Every box says where it came from: `inferred` (this frame was read by
//! the model), `reused` (nothing changed since it was), `tracked` (carried by block
//! matching), or `lost` (the match failed; the box stays where it was last seen).

use crate::detect::{round_to, Detection};
use crate::preprocess::Frame;
use crate::Error;
use archie_accel::advice::{Envelope, ResourceAdvice};
use archie_accel::damage::{Damage, DamageConfig, Luma};
use archie_accel::plan::{
    decide, finished, CostModel, Decision, FocusBudget, FocusState, PlanConfig,
};
use archie_accel::track::{TrackConfig, Tracked, Tracker};
use archie_slink::link::Link;
use archie_slink::links::{SceneState, SceneTrack};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// Longest side of the luma image damage and tracking work on.
pub const LUMA_SIDE: usize = 640;

#[derive(Debug, Clone, Copy, Default)]
pub struct LiveConfig {
    pub damage: DamageConfig,
    pub track: TrackConfig,
    pub plan: PlanConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Inferred,
    Reused,
    Tracked,
    Lost,
}

/// A detection as a live focus reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveDetection {
    #[serde(flatten)]
    pub detection: Detection,
    /// Stable while the thing is followed, across inferences.
    pub track: u64,
    pub source: Source,
    /// Frames since the model last read this box.
    pub frames: u32,
}

/// A frame the model should read for a focus.
#[derive(Debug, Clone)]
pub struct InferRequest {
    pub focus: String,
    pub frame: Frame,
    pub phrases: Vec<String>,
    pub open: bool,
    pub threshold: f32,
    pub budget: FocusBudget,
    pub enqueued_ms: u64,
    luma: Luma,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveReply {
    pub focus: String,
    pub decision: Decision,
    /// Changed fraction of this frame against the previous one.
    pub damage: f32,
    pub detections: Vec<LiveDetection>,
    /// Since the model last read a frame of this focus.
    pub inference_age_ms: Option<u64>,
    pub inference_pending: bool,
}

struct Focus {
    budget: FocusBudget,
    phrases: Vec<String>,
    threshold: f32,
    state: FocusState,
    previous: Option<Luma>,
    tracker: Tracker,
    /// What each track is, from the inference that found it.
    labels: HashMap<u64, Detection>,
    /// Image pixels per luma pixel, for the frames now arriving.
    scale: f32,
    /// The tracker was re-anchored on an older frame: carry it to the next one.
    behind: bool,
    inferred_at: Option<u64>,
}

/// Every live focus's state. Holds no model; [`LiveHub::observe`] returns the inference
/// to run and [`LiveHub::complete`] takes its result.
#[derive(Default)]
pub struct LiveHub {
    config: LiveConfig,
    foci: HashMap<String, Focus>,
}

/// FNV-1a: a phrase's key for the tracker, so a thing keeps its track across inferences.
fn phrase_key(phrase: &str) -> u64 {
    phrase.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The frame's luma, shrunk by a whole factor so its longest side is at most
/// [`LUMA_SIDE`]; returns it with the factor.
fn luma_of(frame: &Frame) -> Result<(Luma, f32), Error> {
    let image = &frame.image;
    let factor = image.width.max(image.height).div_ceil(LUMA_SIDE).max(1);
    let (width, height) = (image.width / factor, image.height / factor);
    let (width, height) = (width.max(1), height.max(1));
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            let mut count = 0u32;
            for dy in 0..factor {
                for dx in 0..factor {
                    let (px, py) = (x * factor + dx, y * factor + dy);
                    if px < image.width && py < image.height {
                        let p = &image.pixels[(py * image.width + px) * 3..][..3];
                        sum +=
                            (77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]))
                                >> 8;
                        count += 1;
                    }
                }
            }
            pixels.push((sum / count.max(1)) as u8);
        }
    }
    let luma = Luma::new(width as u32, height as u32, pixels)
        .map_err(|error| Error::Image(error.to_string()))?;
    Ok((luma, factor as f32))
}

impl LiveHub {
    pub fn new(config: LiveConfig) -> Self {
        Self {
            config,
            foci: HashMap::new(),
        }
    }

    /// Sets (or changes) what a focus reads and how often.
    pub fn set_focus(&mut self, budget: FocusBudget, phrases: Vec<String>, threshold: f32) {
        let track = self.config.track;
        let focus = self
            .foci
            .entry(budget.focus.clone())
            .or_insert_with(|| Focus {
                budget: budget.clone(),
                phrases: Vec::new(),
                threshold,
                state: FocusState::default(),
                previous: None,
                tracker: Tracker::new(track),
                labels: HashMap::new(),
                scale: 1.0,
                behind: false,
                inferred_at: None,
            });
        if focus.phrases != phrases || focus.budget.mode != budget.mode {
            // Other things to look for: the next frame is read again.
            focus.state.last_inference_ms = None;
        }
        focus.budget = budget;
        focus.phrases = phrases;
        focus.threshold = threshold;
    }

    pub fn has_focus(&self, focus: &str) -> bool {
        self.foci.contains_key(focus)
    }

    pub fn foci(&self) -> Vec<(FocusBudget, Vec<String>, FocusState)> {
        self.foci
            .values()
            .map(|f| (f.budget.clone(), f.phrases.clone(), f.state.clone()))
            .collect()
    }

    /// Answers a frame of `focus` now, and says whether the model should read it.
    pub fn observe(
        &mut self,
        focus: &str,
        frame: &Frame,
        now_ms: u64,
    ) -> Result<(LiveReply, Option<InferRequest>), Error> {
        let config = self.config;
        let entry = self
            .foci
            .get_mut(focus)
            .ok_or_else(|| Error::Refused(format!("no live focus {focus}")))?;
        let (luma, scale) = luma_of(frame)?;
        if (scale - entry.scale).abs() > f32::EPSILON && entry.previous.is_some() {
            // A new frame size: what was tracked is in other coordinates.
            entry.state.last_inference_ms = None;
            entry.tracker = Tracker::new(config.track);
            entry.labels.clear();
        }
        entry.scale = scale;
        let damage = Damage::between(entry.previous.as_ref(), &luma, &config.damage);
        let decision = decide(
            &entry.budget,
            &mut entry.state,
            &damage,
            now_ms,
            &config.plan,
        );
        let moved = !damage.is_empty() || entry.behind;
        let tracked: Vec<Tracked> = match decision {
            Decision::Idle => Vec::new(),
            _ if moved => {
                entry.behind = false;
                entry.tracker.advance(&luma)
            }
            _ => entry.tracker.tracks(),
        };
        let source_of = |t: &Tracked| {
            if t.lost {
                Source::Lost
            } else if t.frames == 0 {
                Source::Inferred
            } else if matches!(decision, Decision::Reuse) && !moved {
                Source::Reused
            } else {
                Source::Tracked
            }
        };
        let detections = tracked
            .iter()
            .filter_map(|t| {
                let known = entry.labels.get(&t.id)?;
                let b = t.bbox.map(|v| f64::from(v * scale));
                let clip = |v: f64, limit: usize| v.clamp(0.0, limit as f64);
                let (w, h) = (frame.image.width, frame.image.height);
                let bbox = [
                    round_to(clip(b[0], w), 1),
                    round_to(clip(b[1], h), 1),
                    round_to(clip(b[2], w), 1),
                    round_to(clip(b[3], h), 1),
                ];
                Some(LiveDetection {
                    detection: Detection {
                        bbox,
                        point: [
                            round_to((bbox[0] + bbox[2]) / 2.0, 1),
                            round_to((bbox[1] + bbox[3]) / 2.0, 1),
                        ],
                        ..known.clone()
                    },
                    track: t.id,
                    source: source_of(t),
                    frames: t.frames,
                })
            })
            .collect();
        let request = match decision {
            Decision::Infer { open } => Some(InferRequest {
                focus: focus.to_owned(),
                frame: frame.clone(),
                phrases: entry.phrases.clone(),
                open,
                threshold: entry.threshold,
                budget: entry.budget.clone(),
                enqueued_ms: now_ms,
                luma: luma.clone(),
            }),
            _ => None,
        };
        entry.previous = Some(luma);
        Ok((
            LiveReply {
                focus: focus.to_owned(),
                decision,
                damage: damage.fraction,
                detections,
                inference_age_ms: entry.inferred_at.map(|t| now_ms.saturating_sub(t)),
                inference_pending: entry.state.in_flight,
            },
            request,
        ))
    }

    /// The model read `request`'s frame and found `detections` (pixels of that frame).
    pub fn complete(&mut self, request: &InferRequest, detections: Vec<Detection>, now_ms: u64) {
        let Some(entry) = self.foci.get_mut(&request.focus) else {
            return;
        };
        finished(&mut entry.state, now_ms);
        entry.inferred_at = Some(request.enqueued_ms);
        let scale = entry.scale;
        let boxes: Vec<(u64, [f32; 4])> = detections
            .iter()
            .map(|d| (phrase_key(&d.phrase), d.bbox.map(|v| v as f32 / scale)))
            .collect();
        let anchored = entry.tracker.anchor(&request.luma, &boxes);
        entry.labels = anchored
            .iter()
            .zip(detections)
            .map(|(t, d)| (t.id, d))
            .collect();
        entry.behind = entry.previous.as_ref() != Some(&request.luma);
    }

    /// The model could not read `request`'s frame; the focus may ask again.
    pub fn failed(&mut self, request: &InferRequest, now_ms: u64) {
        if let Some(entry) = self.foci.get_mut(&request.focus) {
            finished(&mut entry.state, now_ms);
            entry.state.damaged_since_inference = true;
        }
    }
}

/// Inference requests waiting for the model: the newest per focus, taken in priority order.
#[derive(Default)]
pub struct InferQueue {
    pending: HashMap<String, InferRequest>,
}

impl InferQueue {
    /// Queues `request`, replacing an older one of the same focus (returned, so its focus
    /// can be told it will not run).
    pub fn push(&mut self, request: InferRequest) -> Option<InferRequest> {
        self.pending.insert(request.focus.clone(), request)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Everything waiting, voice and reflex first, then stronger foci, then older frames.
    pub fn take(&mut self) -> Vec<InferRequest> {
        let mut all: Vec<InferRequest> = self.pending.drain().map(|(_, r)| r).collect();
        all.sort_by(|a, b| {
            b.budget
                .priority
                .cmp(&a.budget.priority)
                .then(b.budget.strength.total_cmp(&a.budget.strength))
                .then(a.enqueued_ms.cmp(&b.enqueued_ms))
        });
        all
    }
}

/// What the live endpoints and the inference worker share.
pub struct LiveShared {
    pub hub: Mutex<LiveHub>,
    queue: Mutex<InferQueue>,
    wake: Condvar,
    /// Measured inference times, by kind (`focused`, `open`), per request.
    pub costs: Mutex<CostModel>,
    /// What Policy approved for live foci on this node.
    pub envelope: Envelope,
    /// `scene.state`: every live focus's tracks, for the action head, Cortex and Wanda.
    pub scene: Link<SceneState>,
    /// The same link for other processes, when exported.
    #[cfg(target_os = "linux")]
    export: Mutex<Option<Arc<archie_slink::shm::Exporter>>>,
    clock: Instant,
    stop: AtomicBool,
}

impl LiveShared {
    pub fn new(config: LiveConfig, envelope: Envelope) -> Arc<Self> {
        Arc::new(Self {
            hub: Mutex::new(LiveHub::new(config)),
            queue: Mutex::new(InferQueue::default()),
            wake: Condvar::new(),
            costs: Mutex::new(CostModel::new(0.2, 100.0)),
            envelope,
            scene: Link::new(64),
            #[cfg(target_os = "linux")]
            export: Mutex::new(None),
            clock: Instant::now(),
            stop: AtomicBool::new(false),
        })
    }

    /// Exports `scene.state` to other processes through a socket in `dir`.
    #[cfg(target_os = "linux")]
    pub fn export_scene(&self, dir: &std::path::Path) -> Result<std::path::PathBuf, Error> {
        let path = dir.join("scene.state.sock");
        let exporter = archie_slink::shm::Exporter::serve(&path, "scene.state", 512 * 1024)
            .map_err(|error| Error::Refused(error.to_string()))?;
        if let Ok(mut export) = self.export.lock() {
            *export = Some(exporter);
        }
        Ok(path)
    }

    /// Publishes a live reply as `scene.state` (and to other processes when exported).
    pub fn publish_scene(&self, reply: &LiveReply, observed_at_ms: u64) -> Result<u64, Error> {
        let state = SceneState {
            focus: reply.focus.clone(),
            slots: Vec::new(),
            tracks: reply
                .detections
                .iter()
                .take(256)
                .map(|d| SceneTrack {
                    id: d.track,
                    label: d.detection.phrase.chars().take(64).collect(),
                    bbox: d.detection.bbox.map(|v| v as f32),
                    score: d.detection.score as f32,
                    source: match d.source {
                        Source::Inferred => "inferred",
                        Source::Reused => "reused",
                        Source::Tracked => "tracked",
                        Source::Lost => "lost",
                    }
                    .into(),
                })
                .collect(),
            damage: reply.damage,
        };
        let revision = self
            .scene
            .publish(observed_at_ms, state)
            .map_err(|error| Error::Refused(error.to_string()))?;
        #[cfg(target_os = "linux")]
        if let Ok(export) = self.export.lock() {
            if let (Some(exporter), Some(message)) = (export.as_ref(), self.scene.latest()) {
                exporter
                    .publish(&message)
                    .map_err(|error| Error::Refused(error.to_string()))?;
            }
        }
        Ok(revision)
    }

    pub fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    /// Queues a read; a newer frame of the same focus replaces an older one.
    pub fn submit(&self, request: InferRequest) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(request);
        }
        self.wake.notify_one();
    }

    pub fn queued(&self) -> usize {
        self.queue.lock().map_or(0, |q| q.pending.len())
    }

    /// Applies Cortex's advice (or the same fields from a request) inside the envelope.
    pub fn advise(&self, advice: &ResourceAdvice, threshold: f32) -> Result<(), Error> {
        let (budget, phrases) = advice
            .apply(&self.envelope)
            .map_err(|error| Error::Refused(error.to_string()))?;
        self.hub
            .lock()
            .map_err(|_| Error::Refused("live state unavailable".into()))?
            .set_focus(budget, phrases, threshold.clamp(0.01, 0.99));
        Ok(())
    }

    /// Ends the worker after the batch it is running.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

/// Runs queued reads until [`LiveShared::shutdown`]. `infer` gets every waiting request in
/// priority order and returns one result per request, in order; it decides how to batch
/// them (the server reads focused requests of several foci in one pass).
pub fn spawn_worker<F>(shared: Arc<LiveShared>, mut infer: F) -> std::thread::JoinHandle<()>
where
    F: FnMut(&[InferRequest]) -> Vec<Result<Vec<Detection>, String>> + Send + 'static,
{
    std::thread::spawn(move || loop {
        let batch = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            while queue.is_empty() && !shared.stop.load(Ordering::SeqCst) {
                queue = match shared.wake.wait(queue) {
                    Ok(queue) => queue,
                    Err(_) => return,
                };
            }
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            queue.take()
        };
        let started = Instant::now();
        let results = infer(&batch);
        let spent = started.elapsed().as_secs_f32() * 1000.0;
        if let Ok(mut costs) = shared.costs.lock() {
            for request in &batch {
                let kind = if request.open { "open" } else { "focused" };
                costs.observe(kind, spent / batch.len() as f32);
            }
        }
        let now = shared.now_ms();
        if let Ok(mut hub) = shared.hub.lock() {
            for (index, request) in batch.iter().enumerate() {
                match results.get(index) {
                    Some(Ok(found)) => hub.complete(request, found.clone(), now),
                    _ => hub.failed(request, now),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preprocess::Rgb;
    use archie_accel::plan::{FocusMode, Priority};

    /// A grey frame with a textured square at (x, y).
    fn frame(width: usize, height: usize, x: usize, y: usize, side: usize) -> Frame {
        let mut pixels = vec![90u8; width * height * 3];
        for py in y..(y + side).min(height) {
            for px in x..(x + side).min(width) {
                let (u, v) = (px - x, py - y);
                let value = ((u * 37 + v * 91 + (u * v) % 13 * 17) % 200 + 30) as u8;
                pixels[(py * width + px) * 3..][..3].copy_from_slice(&[
                    value,
                    value / 2,
                    255 - value,
                ]);
            }
        }
        Frame::new(Rgb::new(width, height, pixels).unwrap(), 64)
    }

    fn budget(fps: f32) -> FocusBudget {
        FocusBudget {
            focus: "camera:desk".into(),
            priority: Priority::ActiveFocus,
            strength: 1.0,
            max_fps: fps,
            mode: FocusMode::Focused,
            read_text: false,
        }
    }

    fn found(x: f64, y: f64, side: f64) -> Detection {
        Detection {
            phrase: "taza".into(),
            score: 0.9,
            bbox: [x, y, x + side, y + side],
            point: [x + side / 2.0, y + side / 2.0],
            embedding: None,
        }
    }

    #[test]
    fn a_still_scene_is_read_once_and_a_moving_one_is_tracked_between_reads() {
        let mut hub = LiveHub::new(LiveConfig::default());
        hub.set_focus(budget(10.0), vec!["taza".into()], 0.35);
        let first = frame(320, 240, 100, 80, 40);
        let (reply, request) = hub.observe("camera:desk", &first, 0).unwrap();
        assert_eq!(reply.decision, Decision::Infer { open: false });
        let request = request.unwrap();
        hub.complete(&request, vec![found(100.0, 80.0, 40.0)], 40);

        // Nothing moved: the inference stands, no new request.
        let (reply, request) = hub.observe("camera:desk", &first, 50).unwrap();
        assert!(request.is_none());
        assert_eq!(reply.decision, Decision::Reuse);
        assert_eq!(reply.detections.len(), 1);
        assert_eq!(
            reply.detections[0].detection.bbox,
            [100.0, 80.0, 140.0, 120.0]
        );

        // It moves: tracked at once, read again when the budget allows (10 per second).
        let moved = frame(320, 240, 106, 83, 40);
        let (reply, request) = hub.observe("camera:desk", &moved, 66).unwrap();
        assert_eq!(reply.decision, Decision::Track);
        assert!(request.is_none());
        assert_eq!(reply.detections[0].source, Source::Tracked);
        assert_eq!(
            reply.detections[0].detection.bbox,
            [106.0, 83.0, 146.0, 123.0]
        );
        let track = reply.detections[0].track;

        let further = frame(320, 240, 112, 86, 40);
        let (reply, request) = hub.observe("camera:desk", &further, 150).unwrap();
        assert_eq!(reply.decision, Decision::Infer { open: false });
        assert_eq!(
            reply.detections[0].detection.bbox,
            [112.0, 86.0, 152.0, 126.0]
        );
        // The model reads that frame while the scene moves on; the next frame is carried
        // from the new inference and the thing keeps its track.
        hub.complete(&request.unwrap(), vec![found(112.0, 86.0, 40.0)], 200);
        let (reply, _) = hub
            .observe("camera:desk", &frame(320, 240, 116, 88, 40), 216)
            .unwrap();
        assert_eq!(reply.detections[0].track, track);
        assert_eq!(
            reply.detections[0].detection.bbox,
            [116.0, 88.0, 156.0, 128.0]
        );
        assert_eq!(reply.inference_age_ms, Some(66));
    }

    #[test]
    fn large_frames_are_tracked_on_a_reduced_luma() {
        let mut hub = LiveHub::new(LiveConfig::default());
        hub.set_focus(budget(5.0), vec!["taza".into()], 0.35);
        let first = frame(1280, 720, 400, 300, 120);
        let (_, request) = hub.observe("camera:desk", &first, 0).unwrap();
        hub.complete(&request.unwrap(), vec![found(400.0, 300.0, 120.0)], 30);
        let (reply, _) = hub
            .observe("camera:desk", &frame(1280, 720, 420, 310, 120), 33)
            .unwrap();
        assert_eq!(
            reply.detections[0].detection.bbox,
            [420.0, 310.0, 540.0, 430.0]
        );
    }

    #[test]
    fn unknown_foci_are_refused_and_new_phrases_ask_for_a_read() {
        let mut hub = LiveHub::new(LiveConfig::default());
        let f = frame(64, 64, 0, 0, 8);
        assert!(hub.observe("camera:none", &f, 0).is_err());
        hub.set_focus(budget(10.0), vec!["taza".into()], 0.35);
        let (_, request) = hub.observe("camera:desk", &f, 0).unwrap();
        hub.complete(&request.unwrap(), Vec::new(), 10);
        assert!(hub.observe("camera:desk", &f, 20).unwrap().1.is_none());
        hub.set_focus(budget(10.0), vec!["llaves".into()], 0.35);
        assert!(hub.observe("camera:desk", &f, 30).unwrap().1.is_some());
    }

    #[test]
    fn the_queue_keeps_the_newest_frame_per_focus_in_priority_order() {
        let mut hub = LiveHub::new(LiveConfig::default());
        let mut queue = InferQueue::default();
        let mut first_screen = None;
        for (name, priority, strength) in [
            ("screen", Priority::ActiveFocus, 0.4),
            ("voice", Priority::VoiceReflex, 0.1),
            ("camera", Priority::ActiveFocus, 0.9),
        ] {
            let mut b = budget(10.0);
            b.focus = name.into();
            b.priority = priority;
            b.strength = strength;
            hub.set_focus(b, vec!["x".into()], 0.35);
            let request = hub
                .observe(name, &frame(64, 64, 0, 0, 8), 0)
                .unwrap()
                .1
                .unwrap();
            if name == "screen" {
                first_screen = Some(request.clone());
            }
            assert!(queue.push(request).is_none());
        }
        // While a read is pending the focus tracks; once it fails, the focus asks again
        // and the newer frame replaces the older one in the queue.
        let pending = hub.observe("screen", &frame(64, 64, 9, 9, 30), 50).unwrap();
        assert!(pending.1.is_none());
        hub.failed(&first_screen.unwrap(), 60);
        let again = hub
            .observe("screen", &frame(64, 64, 12, 9, 30), 200)
            .unwrap()
            .1
            .unwrap();
        let replaced = queue.push(again).unwrap();
        assert_eq!(replaced.enqueued_ms, 0);
        let order: Vec<(String, u64)> = queue
            .take()
            .into_iter()
            .map(|r| (r.focus, r.enqueued_ms))
            .collect();
        assert_eq!(
            order,
            [
                ("voice".into(), 0),
                ("camera".into(), 0),
                ("screen".into(), 200)
            ]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn the_worker_reads_queued_frames_and_re_anchors_the_focus() {
        let envelope = Envelope {
            max_priority: Priority::ActiveFocus,
            max_fps: 30.0,
            max_vram_bytes: 0,
            allow_partial_experts: false,
            expert_floor: 0.5,
        };
        let shared = LiveShared::new(LiveConfig::default(), envelope);
        let advice: ResourceAdvice = serde_json::from_str(
            r#"{"focus":"camera:desk","share":1,"mode":"focused","phrases":["taza"],
                "priority":"voice_reflex","max_fps":120}"#,
        )
        .unwrap();
        shared.advise(&advice, 0.35).unwrap();
        let (budget, _, _) = shared.hub.lock().unwrap().foci()[0].clone();
        assert_eq!(budget.priority, Priority::ActiveFocus, "clamped");
        assert_eq!(budget.max_fps, 30.0, "clamped");
        let worker = spawn_worker(shared.clone(), |batch| {
            batch
                .iter()
                .map(|r| {
                    assert_eq!(r.phrases, ["taza"]);
                    Ok(vec![found(100.0, 80.0, 40.0)])
                })
                .collect()
        });
        let first = frame(320, 240, 100, 80, 40);
        let (_, request) = shared
            .hub
            .lock()
            .unwrap()
            .observe("camera:desk", &first, shared.now_ms())
            .unwrap();
        shared.submit(request.unwrap());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let done = shared.hub.lock().unwrap().foci()[0]
                .2
                .last_inference_ms
                .is_some();
            if done {
                break;
            }
            assert!(Instant::now() < deadline, "the worker did not finish");
            std::thread::yield_now();
        }
        let (reply, _) = shared
            .hub
            .lock()
            .unwrap()
            .observe(
                "camera:desk",
                &frame(320, 240, 104, 82, 40),
                shared.now_ms(),
            )
            .unwrap();
        assert_eq!(
            reply.detections[0].detection.bbox,
            [104.0, 82.0, 144.0, 122.0]
        );
        assert!(shared.costs.lock().unwrap().estimate("focused") < 100.0);
        assert_eq!(shared.publish_scene(&reply, 5).unwrap(), 1);
        let scene = shared.scene.latest().unwrap();
        assert_eq!(scene.payload.tracks[0].bbox, [104.0, 82.0, 144.0, 122.0]);
        assert_eq!(scene.payload.tracks[0].source, "tracked");
        assert!(!scene.may_reach(archie_slink::Locality::Cloud));
        #[cfg(target_os = "linux")]
        {
            let dir = std::env::temp_dir().join(format!("iris-slink-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = shared.export_scene(&dir).unwrap();
            let reader = archie_slink::shm::connect(&path).unwrap();
            shared.publish_scene(&reply, 6).unwrap();
            let seen = reader.latest::<SceneState>().unwrap().unwrap();
            assert_eq!(seen.header.revision, 2);
            assert_eq!(seen.payload.focus, "camera:desk");
            std::fs::remove_dir_all(&dir).unwrap();
        }
        shared.shutdown();
        worker.join().unwrap();
    }
}
