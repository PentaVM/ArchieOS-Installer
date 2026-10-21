//! `archie-accel`: what the process that owns the GPU decides each tick.
//!
//! The plan (`_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`, §4) calls this the OS-level
//! "megakernel": not one kernel, but one owner for the candle models that decides what runs,
//! in which batch, at which priority, and what lives in VRAM. This crate holds those
//! decisions as plain data and functions, with no tensor library, so IRIS and Wanda's LNCM
//! share them and they can be tested without a GPU:
//!
//! - [`damage`]: which part of a frame changed, from pixels (cameras) or the compositor's
//!   rectangles (screens). An unchanged frame produces no work; caret-sized changes neither.
//! - [`track`]: boxes carried from the last inference to the current frame by block
//!   matching, so results come out at the camera's rate while full inference runs at the
//!   rate the GPU allows. Tracked boxes say so; they are not inferences.
//! - [`plan`]: per focus, reuse, track or infer; across foci and models, the order by
//!   priority class; a cost model learned from measured job times.
//! - [`memory`]: the VRAM ledger and which MoE experts stay resident under pressure.
//! - [`advice`]: Cortex's `ResourceAdvice`, clamped into the envelope Policy approved.
//!
//! Authority does not change here. Cortex advises and Policy sets the envelope; this crate
//! applies the advice inside it and never raises a priority class or a memory limit. Every
//! action IRIS proposes still goes Cerebra → Policy → lease → Stage.

pub mod advice;
pub mod damage;
pub mod memory;
pub mod plan;
pub mod track;

/// An axis-aligned rectangle in pixels, `x0 <= x1`, `y0 <= y1`, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    pub fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        Self {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("frame: {0}")]
    Frame(String),
    #[error("memory: {0}")]
    Memory(String),
    #[error("advice: {0}")]
    Advice(String),
}
