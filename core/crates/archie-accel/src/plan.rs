//! Per focus: reuse, track or infer. Across foci and models: in which order.
//!
//! Each tick a focus (a screen, a camera) is decided from its budget, its damage and when it
//! was last inferred: nothing changed → the last results stand (exact); something changed
//! but the focus had its inference recently, or one is running → the boxes are tracked;
//! otherwise → an inference, the open sweep only when enough of the scene is new. Jobs from
//! every focus and model are then ordered by priority class (voice and reflex first,
//! training last), focus strength and age, and admitted while the tick's GPU time lasts by
//! the cost model, which learns each kind's time from measurements.

use crate::damage::Damage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Priority classes, lowest first (`§4.5` of the plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Training,
    Batch,
    Background,
    WandaInteractive,
    ActiveFocus,
    VoiceReflex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    Off,
    /// The task's phrases only.
    Focused,
    /// The everyday vocabulary.
    Open,
    /// The task's phrases, and the open sweep when enough of the scene is new.
    FocusedAndOpen,
}

impl FocusMode {
    pub fn has_open(self) -> bool {
        matches!(self, FocusMode::Open | FocusMode::FocusedAndOpen)
    }
}

/// What a focus may use, after Cortex's advice was clamped into Policy's envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusBudget {
    pub focus: String,
    pub priority: Priority,
    /// Cortex's activation of the focus, in [0, 1]; orders foci within a class.
    pub strength: f32,
    /// Inferences per second at most; tracking fills the frames between.
    pub max_fps: f32,
    pub mode: FocusMode,
    pub read_text: bool,
}

/// What the planner remembers about a focus.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FocusState {
    pub last_inference_ms: Option<u64>,
    pub last_open_ms: Option<u64>,
    /// An inference for this focus is queued or running.
    pub in_flight: bool,
    /// Damage seen since the frame the last inference read.
    pub damaged_since_inference: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlanConfig {
    /// Damage over this fraction of the frame asks for the open sweep.
    pub open_fraction: f32,
    /// The open sweep runs at most this often per focus, damage or not.
    pub open_min_interval_ms: u64,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            open_fraction: 0.25,
            open_min_interval_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "do")]
pub enum Decision {
    /// The focus is off.
    Idle,
    /// Nothing changed since the last inference: its results stand as they are.
    Reuse,
    /// Something moved: carry the last results to this frame.
    Track,
    /// Run the model on this frame.
    Infer { open: bool },
}

/// Decides one focus for one frame, and records what it decided.
pub fn decide(
    budget: &FocusBudget,
    state: &mut FocusState,
    damage: &Damage,
    now_ms: u64,
    config: &PlanConfig,
) -> Decision {
    if budget.mode == FocusMode::Off || budget.max_fps <= 0.0 {
        return Decision::Idle;
    }
    if !damage.is_empty() {
        state.damaged_since_inference = true;
    }
    let Some(last) = state.last_inference_ms else {
        return if state.in_flight {
            Decision::Track
        } else {
            start(budget, state, now_ms, true)
        };
    };
    if !state.damaged_since_inference {
        return Decision::Reuse;
    }
    let interval = (1000.0 / budget.max_fps).ceil() as u64;
    if state.in_flight || now_ms.saturating_sub(last) < interval {
        return Decision::Track;
    }
    let open_due = budget.mode == FocusMode::Open
        || (budget.mode.has_open()
            && damage.fraction >= config.open_fraction
            && state
                .last_open_ms
                .is_none_or(|t| now_ms.saturating_sub(t) >= config.open_min_interval_ms));
    start(budget, state, now_ms, open_due)
}

fn start(budget: &FocusBudget, state: &mut FocusState, now_ms: u64, open: bool) -> Decision {
    let open = open && budget.mode.has_open();
    state.in_flight = true;
    state.damaged_since_inference = false;
    if open {
        state.last_open_ms = Some(now_ms);
    }
    Decision::Infer { open }
}

/// The inference started by [`decide`] finished (or failed) at `now_ms`.
pub fn finished(state: &mut FocusState, now_ms: u64) {
    state.in_flight = false;
    state.last_inference_ms = Some(now_ms);
}

/// A unit of GPU work waiting for a tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub focus: String,
    pub kind: String,
    pub priority: Priority,
    pub strength: f32,
    pub enqueued_ms: u64,
}

/// Priority class first, then the stronger focus, then the older job.
pub fn order(jobs: &mut [Job]) {
    jobs.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(b.strength.total_cmp(&a.strength))
            .then(a.enqueued_ms.cmp(&b.enqueued_ms))
    });
}

/// Each kind's GPU time, learned from measurements (exponential moving average).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    alpha: f32,
    default_ms: f32,
    estimates: HashMap<String, f32>,
}

impl CostModel {
    pub fn new(alpha: f32, default_ms: f32) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 1.0),
            default_ms,
            estimates: HashMap::new(),
        }
    }

    pub fn observe(&mut self, kind: &str, ms: f32) {
        let ms = ms.max(0.0);
        let alpha = self.alpha;
        self.estimates
            .entry(kind.to_owned())
            .and_modify(|e| *e += alpha * (ms - *e))
            .or_insert(ms);
    }

    pub fn estimate(&self, kind: &str) -> f32 {
        self.estimates.get(kind).copied().unwrap_or(self.default_ms)
    }
}

/// Jobs admitted to a tick of `budget_ms`, in order; the rest wait. Voice and reflex are
/// always admitted, and so is the first job, so one long job cannot starve forever.
pub fn admit(jobs: &mut Vec<Job>, costs: &CostModel, budget_ms: f32) -> Vec<Job> {
    order(jobs);
    let mut spent = 0.0;
    let mut admitted = Vec::new();
    let mut waiting = Vec::new();
    for job in jobs.drain(..) {
        let cost = costs.estimate(&job.kind);
        if admitted.is_empty() || job.priority == Priority::VoiceReflex || spent + cost <= budget_ms
        {
            spent += cost;
            admitted.push(job);
        } else {
            waiting.push(job);
        }
    }
    *jobs = waiting;
    admitted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    fn budget(mode: FocusMode, fps: f32) -> FocusBudget {
        FocusBudget {
            focus: "screen:0".into(),
            priority: Priority::ActiveFocus,
            strength: 1.0,
            max_fps: fps,
            mode,
            read_text: false,
        }
    }

    fn damage(fraction: f32) -> Damage {
        Damage {
            regions: vec![Rect::new(0, 0, 10, 10)],
            fraction,
        }
    }

    #[test]
    fn unchanged_frames_reuse_and_changes_track_until_the_next_inference_is_due() {
        let config = PlanConfig::default();
        let b = budget(FocusMode::Focused, 20.0); // one inference per 50 ms
        let mut s = FocusState::default();
        assert_eq!(
            decide(&b, &mut s, &Damage::default(), 0, &config),
            Decision::Infer { open: false }
        );
        assert_eq!(
            decide(&b, &mut s, &damage(0.01), 16, &config),
            Decision::Track,
            "in flight"
        );
        finished(&mut s, 30);
        assert_eq!(
            decide(&b, &mut s, &Damage::default(), 33, &config),
            Decision::Track,
            "damage seen while in flight"
        );
        assert_eq!(
            decide(&b, &mut s, &Damage::default(), 81, &config),
            Decision::Infer { open: false }
        );
        finished(&mut s, 100);
        assert_eq!(
            decide(&b, &mut s, &Damage::default(), 116, &config),
            Decision::Reuse
        );
        assert_eq!(
            decide(&b, &mut s, &Damage::default(), 900, &config),
            Decision::Reuse,
            "a still scene costs nothing"
        );
    }

    #[test]
    fn the_open_sweep_needs_new_scene_and_time() {
        let config = PlanConfig::default();
        let b = budget(FocusMode::FocusedAndOpen, 30.0);
        let mut s = FocusState::default();
        assert_eq!(
            decide(&b, &mut s, &damage(1.0), 0, &config),
            Decision::Infer { open: true }
        );
        finished(&mut s, 10);
        assert_eq!(
            decide(&b, &mut s, &damage(0.05), 100, &config),
            Decision::Infer { open: false }
        );
        finished(&mut s, 110);
        assert_eq!(
            decide(&b, &mut s, &damage(0.9), 500, &config),
            Decision::Infer { open: false },
            "too soon"
        );
        finished(&mut s, 510);
        assert_eq!(
            decide(&b, &mut s, &damage(0.9), 2_100, &config),
            Decision::Infer { open: true }
        );
        let off = budget(FocusMode::Off, 30.0);
        assert_eq!(
            decide(&off, &mut FocusState::default(), &damage(1.0), 0, &config),
            Decision::Idle
        );
    }

    #[test]
    fn voice_goes_first_and_training_last() {
        let job = |focus: &str, priority, strength, t| Job {
            focus: focus.into(),
            kind: "detect".into(),
            priority,
            strength,
            enqueued_ms: t,
        };
        let mut jobs = vec![
            job("train", Priority::Training, 1.0, 0),
            job("cam-weak", Priority::ActiveFocus, 0.2, 1),
            job("voice", Priority::VoiceReflex, 0.0, 9),
            job("cam-strong", Priority::ActiveFocus, 0.9, 5),
            job("cam-strong-old", Priority::ActiveFocus, 0.9, 2),
        ];
        order(&mut jobs);
        let names: Vec<&str> = jobs.iter().map(|j| j.focus.as_str()).collect();
        assert_eq!(
            names,
            ["voice", "cam-strong-old", "cam-strong", "cam-weak", "train"]
        );
    }

    #[test]
    fn admission_follows_measured_costs() {
        let mut costs = CostModel::new(0.5, 10.0);
        costs.observe("open", 500.0);
        costs.observe("focused", 40.0);
        costs.observe("focused", 60.0);
        assert_eq!(costs.estimate("focused"), 50.0);
        let job = |kind: &str, priority| Job {
            focus: kind.into(),
            kind: kind.into(),
            priority,
            strength: 0.5,
            enqueued_ms: 0,
        };
        let mut jobs = vec![
            job("open", Priority::Background),
            job("focused", Priority::ActiveFocus),
            job("wanda", Priority::WandaInteractive),
        ];
        let run = admit(&mut jobs, &costs, 66.0);
        let ran: Vec<&str> = run.iter().map(|j| j.kind.as_str()).collect();
        assert_eq!(ran, ["focused", "wanda"]);
        assert_eq!(jobs.len(), 1, "the open sweep waits");
        let run = admit(&mut jobs, &costs, 66.0);
        assert_eq!(run.len(), 1, "alone, it is admitted rather than starved");
    }
}
