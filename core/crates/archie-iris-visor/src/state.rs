//! What the window shows, written by the workers and read every frame by the UI.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Events per second over the last two seconds.
#[derive(Default)]
pub struct Rate {
    stamps: VecDeque<Instant>,
}

impl Rate {
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.stamps.push_back(now);
        while self
            .stamps
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(2))
        {
            self.stamps.pop_front();
        }
    }

    pub fn per_second(&self) -> f32 {
        let now = Instant::now();
        let recent = self
            .stamps
            .iter()
            .filter(|t| now.duration_since(**t) <= Duration::from_secs(2))
            .count();
        recent as f32 / 2.0
    }
}

/// The newest camera frame, decoded.
pub struct Frame {
    pub id: u64,
    pub width: usize,
    pub height: usize,
    pub rgb: Arc<Vec<u8>>,
    pub jpeg: Arc<Vec<u8>>,
}

/// One box IRIS answered for a frame.
#[derive(Clone, Debug)]
pub struct Seen {
    pub phrase: String,
    pub score: f32,
    /// Pixels of the frame, x0 y0 x1 y1.
    pub bbox: [f32; 4],
    pub track: u64,
    /// `inferred`, `reused`, `tracked` or `lost`.
    pub source: String,
}

#[derive(Default)]
pub struct Iris {
    pub online: bool,
    pub frame_id: u64,
    pub seen: Vec<Seen>,
    /// `infer`, `track`, `reuse`, `idle`.
    pub decision: String,
    pub open_sweep: bool,
    pub damage: f32,
    pub answer_ms: f32,
    pub inference_age_ms: Option<u64>,
    pub answers: Rate,
    pub reads: Rate,
    pub error: Option<String>,
}

/// One OCR call and what Wanda made of it.
#[derive(Clone)]
pub struct OcrCall {
    pub at: Instant,
    /// Who read the boxes: `Qwen3-VL` or `IRIS read_decoder`.
    pub reader: &'static str,
    pub boxes: usize,
    pub read_ms: f32,
    pub lines: Vec<String>,
    /// New or changed lines sent to Wanda.
    pub sent: usize,
    pub wanda: Result<String, String>,
}

#[derive(Default)]
pub struct Wanda {
    pub online: bool,
    pub domain: String,
    pub calls: u64,
    pub applied: u64,
    pub unchanged: u64,
    pub rejected: u64,
    pub error: Option<String>,
}

pub struct Shared {
    pub frame: Option<Frame>,
    pub camera_connected: bool,
    pub camera_from: String,
    pub camera: Rate,
    pub iris: Iris,
    pub ocr: VecDeque<OcrCall>,
    pub ocr_enabled: bool,
    pub wanda: Wanda,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            frame: None,
            camera_connected: false,
            camera_from: String::new(),
            camera: Rate::default(),
            iris: Iris::default(),
            ocr: VecDeque::new(),
            ocr_enabled: true,
            wanda: Wanda::default(),
        }
    }
}

pub type State = Arc<Mutex<Shared>>;

pub const OCR_LOG: usize = 40;
