//! Boxes carried from the last inference to the current frame.
//!
//! Full IRIS inference runs at the rate the GPU allows; a camera sends 30 or 60 frames a
//! second. Between inferences each box is followed by block matching on luma: the box's
//! patch at the keyframe is searched for where its last motion predicts it, by root mean
//! square difference after removing each side's mean (so a camera's exposure change does
//! not move the box). Every whole-pixel position within `search` is tried with a sparse
//! 8 x 8 sample of the patch, then the best and its neighbours with a sample of at most
//! `max_samples` per side; a box not found there is searched again within `recover`.
//! Trying positions on a coarser grid, or on a downscaled frame, aliases on fine texture
//! such as text; every position tried here is a real pixel. The keyframe's patch is kept,
//! not the last match, so the box does not drift. A box whose best match is too different
//! is reported lost and stays where it was.
//!
//! Translation only: a box that grows or turns keeps its size until the next inference.
//! Every result says it was tracked and how many frames ago it was last inferred.

use crate::damage::Luma;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Samples per side of the sparse patch the whole search window is tried with.
const COARSE_SAMPLES: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackConfig {
    /// How far from its predicted place a box is looked for first, in pixels, each way.
    pub search: u32,
    /// How far it is looked for when the first search does not find it.
    pub recover: u32,
    /// Samples per side of a box's patch, at most.
    pub max_samples: u32,
    /// Root mean square difference per sample (0..255, means removed) above which the box
    /// is lost.
    pub accept: f32,
    /// IoU with a box of the same key above which a new inference keeps the track's id.
    pub keep_iou: f32,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            search: 16,
            recover: 48,
            max_samples: 24,
            accept: 18.0,
            keep_iou: 0.3,
        }
    }
}

/// A box as tracked now, in the frame's pixels, `x0 y0 x1 y1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tracked {
    /// Stable across frames and across inferences that find the same thing.
    pub id: u64,
    /// The caller's key for what the box is (a phrase index, a label hash).
    pub key: u64,
    pub bbox: [f32; 4],
    /// The last match's root mean square difference per sample, means removed.
    pub error: f32,
    pub lost: bool,
    /// Frames since the box was last inferred.
    pub frames: u32,
}

fn pixel(frame: &Luma, x: i64, y: i64) -> Option<f32> {
    (x >= 0 && y >= 0 && x < i64::from(frame.width) && y < i64::from(frame.height))
        .then(|| f32::from(frame.at(x as u32, y as u32)))
}

/// A patch sampled on a grid: where the grid starts, its step and size, and the values.
#[derive(Debug, Clone)]
struct Patch {
    x: i64,
    y: i64,
    step: u32,
    columns: u32,
    rows: u32,
    values: Vec<f32>,
}

impl Patch {
    fn take(frame: &Luma, bbox: &[f32; 4], max_samples: u32) -> Self {
        let width = (bbox[2] - bbox[0]).max(1.0);
        let height = (bbox[3] - bbox[1]).max(1.0);
        let (x, y) = (bbox[0].round() as i64, bbox[1].round() as i64);
        let step = (width.max(height) / max_samples.max(1) as f32)
            .ceil()
            .max(1.0) as u32;
        let columns = ((width / step as f32).floor() as u32).max(1);
        let rows = ((height / step as f32).floor() as u32).max(1);
        let mut values = Vec::with_capacity((columns * rows) as usize);
        for j in 0..rows {
            for i in 0..columns {
                let (px, py) = (x + i64::from(i * step), y + i64::from(j * step));
                values.push(pixel(frame, px, py).unwrap_or(0.0));
            }
        }
        Self {
            x,
            y,
            step,
            columns,
            rows,
            values,
        }
    }

    /// Root mean square difference between the patch and `frame` displaced by
    /// `(dx, dy)`, each side's mean removed, over the samples inside the frame; `None` when
    /// fewer than half are inside. One pass: `sum (t - c)^2 - n (mean_t - mean_c)^2`.
    fn score(&self, frame: &Luma, dx: i64, dy: i64) -> Option<f32> {
        let (mut n, mut sum_t, mut sum_c, mut squares) = (0u32, 0f32, 0f32, 0f32);
        for j in 0..self.rows {
            let py = self.y + dy + i64::from(j * self.step);
            if py < 0 || py >= i64::from(frame.height) {
                continue;
            }
            let row =
                &frame.pixels[(py as usize) * (frame.width as usize)..][..frame.width as usize];
            let values = &self.values[(j * self.columns) as usize..][..self.columns as usize];
            for (i, &t) in values.iter().enumerate() {
                let px = self.x + dx + (i as i64) * i64::from(self.step);
                if px < 0 || px >= i64::from(frame.width) {
                    continue;
                }
                let c = f32::from(row[px as usize]);
                n += 1;
                sum_t += t;
                sum_c += c;
                squares += (t - c) * (t - c);
            }
        }
        if n == 0 || (n as usize) * 2 < self.values.len() {
            return None;
        }
        let n = n as f32;
        let gap = (sum_t - sum_c) / n;
        Some(((squares / n) - gap * gap).max(0.0).sqrt())
    }
}

#[derive(Debug, Clone)]
struct Track {
    id: u64,
    key: u64,
    /// The box at the keyframe.
    origin: [f32; 4],
    /// Offset of the box now from `origin`, whole pixels.
    shift: (i64, i64),
    /// The last frame's move, to predict the next.
    velocity: (i64, i64),
    full: Patch,
    coarse: Patch,
    error: f32,
    lost: bool,
    frames: u32,
}

impl Track {
    fn state(&self) -> Tracked {
        let (dx, dy) = (self.shift.0 as f32, self.shift.1 as f32);
        Tracked {
            id: self.id,
            key: self.key,
            bbox: [
                self.origin[0] + dx,
                self.origin[1] + dy,
                self.origin[2] + dx,
                self.origin[3] + dy,
            ],
            error: self.error,
            lost: self.lost,
            frames: self.frames,
        }
    }
}

/// Whether `(value, dx, dy)` beats `best`: a lower difference, then closer to `near`.
fn better(best: Option<(f32, i64, i64)>, value: f32, dx: i64, dy: i64, near: (i64, i64)) -> bool {
    match best {
        None => true,
        Some((b, bx, by)) => {
            let distance = |x: i64, y: i64| (x - near.0).abs() + (y - near.1).abs();
            value < b || (value == b && distance(dx, dy) < distance(bx, by))
        }
    }
}

/// The best whole-pixel shift within `reach` of `center`: every position with the sparse
/// patch, then the best and its neighbours with the full one.
fn search(track: &Track, frame: &Luma, center: (i64, i64), reach: i64) -> Option<(f32, i64, i64)> {
    let mut coarse: Option<(f32, i64, i64)> = None;
    for dy in center.1 - reach..=center.1 + reach {
        for dx in center.0 - reach..=center.0 + reach {
            if let Some(value) = track.coarse.score(frame, dx, dy) {
                if better(coarse, value, dx, dy, center) {
                    coarse = Some((value, dx, dy));
                }
            }
        }
    }
    let (_, bx, by) = coarse?;
    let mut best: Option<(f32, i64, i64)> = None;
    for dy in by - 1..=by + 1 {
        for dx in bx - 1..=bx + 1 {
            if let Some(value) = track.full.score(frame, dx, dy) {
                if better(best, value, dx, dy, (bx, by)) {
                    best = Some((value, dx, dy));
                }
            }
        }
    }
    best
}

fn follow(track: &mut Track, frame: &Luma, config: &TrackConfig) {
    track.frames += 1;
    let predicted = (
        track.shift.0 + track.velocity.0,
        track.shift.1 + track.velocity.1,
    );
    let mut best = search(track, frame, predicted, i64::from(config.search));
    if best.is_none_or(|(error, _, _)| error > config.accept) && config.recover > config.search {
        let wide = search(track, frame, track.shift, i64::from(config.recover));
        if wide.is_some_and(|(e, _, _)| best.is_none_or(|(b, _, _)| e < b)) {
            best = wide;
        }
    }
    match best {
        Some((error, dx, dy)) if error <= config.accept => {
            track.velocity = (dx - track.shift.0, dy - track.shift.1);
            track.shift = (dx, dy);
            track.error = error;
            track.lost = false;
        }
        Some((error, _, _)) => {
            track.velocity = (0, 0);
            track.error = error;
            track.lost = true;
        }
        None => {
            track.velocity = (0, 0);
            track.lost = true;
        }
    }
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let area = |r: &[f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let union = area(a) + area(b) - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Tracker {
    config: TrackConfig,
    tracks: Vec<Track>,
    next_id: u64,
}

impl Tracker {
    pub fn new(config: TrackConfig) -> Self {
        Self {
            config,
            tracks: Vec::new(),
            next_id: 1,
        }
    }

    pub fn tracks(&self) -> Vec<Tracked> {
        self.tracks.iter().map(Track::state).collect()
    }

    /// A new inference on `frame`: its boxes replace the tracks. A box keeps the id of the
    /// current track with the same key it overlaps most (IoU above `keep_iou`), so a thing
    /// followed across inferences keeps one id.
    pub fn anchor(&mut self, frame: &Luma, boxes: &[(u64, [f32; 4])]) -> Vec<Tracked> {
        let previous = self.tracks();
        let mut taken = vec![false; previous.len()];
        let mut tracks = Vec::with_capacity(boxes.len());
        for (key, bbox) in boxes {
            let best = previous
                .iter()
                .enumerate()
                .filter(|(i, t)| !taken[*i] && t.key == *key)
                .map(|(i, t)| (i, iou(&t.bbox, bbox)))
                .filter(|(_, overlap)| *overlap > self.config.keep_iou)
                .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
            let id = match best {
                Some((index, _)) => {
                    taken[index] = true;
                    previous[index].id
                }
                None => {
                    self.next_id += 1;
                    self.next_id - 1
                }
            };
            tracks.push(Track {
                id,
                key: *key,
                origin: *bbox,
                shift: (0, 0),
                velocity: (0, 0),
                full: Patch::take(frame, bbox, self.config.max_samples),
                coarse: Patch::take(frame, bbox, COARSE_SAMPLES),
                error: 0.0,
                lost: false,
                frames: 0,
            });
        }
        self.tracks = tracks;
        self.tracks()
    }

    /// Follows every box into `frame`, the boxes in parallel.
    pub fn advance(&mut self, frame: &Luma) -> Vec<Tracked> {
        let config = self.config;
        self.tracks
            .par_iter_mut()
            .for_each(|track| follow(track, frame, &config));
        self.tracks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noise-textured square of `side` at (x, y) on a smooth background.
    fn scene(width: u32, height: u32, x: u32, y: u32, side: u32, gain: i32) -> Luma {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for py in 0..height {
            for px in 0..width {
                let inside = px >= x && px < x + side && py >= y && py < y + side;
                let base = if inside {
                    let (u, v) = (px - x, py - y);
                    ((u * 37 + v * 91 + (u * v) % 13 * 17) % 200) as i32 + 30
                } else {
                    ((px / 8 + py / 8) % 2 * 20 + 60) as i32
                };
                pixels.push((base + gain).clamp(0, 255) as u8);
            }
        }
        Luma::new(width, height, pixels).unwrap()
    }

    #[test]
    fn a_moving_patch_is_followed_to_the_pixel() {
        let mut tracker = Tracker::new(TrackConfig::default());
        let first = scene(320, 240, 100, 80, 40, 0);
        let anchored = tracker.anchor(&first, &[(7, [100.0, 80.0, 140.0, 120.0])]);
        assert_eq!(anchored[0].frames, 0);
        let id = anchored[0].id;
        let mut position = (100u32, 80u32);
        for (dx, dy) in [(5, 3), (9, -4), (12, 7), (-6, 10)] {
            position = (
                (position.0 as i32 + dx) as u32,
                (position.1 as i32 + dy) as u32,
            );
            let frame = scene(320, 240, position.0, position.1, 40, 0);
            let now = tracker.advance(&frame);
            assert_eq!(now.len(), 1);
            assert!(!now[0].lost, "{now:?}");
            assert_eq!(now[0].id, id);
            assert_eq!(
                now[0].bbox,
                [
                    position.0 as f32,
                    position.1 as f32,
                    position.0 as f32 + 40.0,
                    position.1 as f32 + 40.0
                ]
            );
        }
        assert_eq!(tracker.tracks()[0].frames, 4);
    }

    #[test]
    fn an_exposure_change_does_not_move_the_box() {
        let mut tracker = Tracker::new(TrackConfig::default());
        tracker.anchor(
            &scene(200, 200, 50, 50, 32, 0),
            &[(1, [50.0, 50.0, 82.0, 82.0])],
        );
        let now = tracker.advance(&scene(200, 200, 50, 50, 32, 25));
        assert!(!now[0].lost);
        assert_eq!(now[0].bbox, [50.0, 50.0, 82.0, 82.0]);
    }

    #[test]
    fn a_vanished_patch_is_lost_where_it_was() {
        let mut tracker = Tracker::new(TrackConfig::default());
        tracker.anchor(
            &scene(200, 200, 60, 60, 32, 0),
            &[(1, [60.0, 60.0, 92.0, 92.0])],
        );
        let empty = scene(200, 200, 0, 0, 0, 0);
        let now = tracker.advance(&empty);
        assert!(now[0].lost);
        assert_eq!(now[0].bbox, [60.0, 60.0, 92.0, 92.0]);
    }

    #[test]
    fn a_new_inference_keeps_ids_of_what_it_finds_again() {
        let mut tracker = Tracker::new(TrackConfig::default());
        let frame = scene(300, 300, 40, 40, 30, 0);
        let first = tracker.anchor(
            &frame,
            &[
                (1, [40.0, 40.0, 70.0, 70.0]),
                (2, [200.0, 200.0, 240.0, 240.0]),
            ],
        );
        let second = tracker.anchor(
            &frame,
            &[
                (1, [44.0, 42.0, 74.0, 72.0]),
                (2, [10.0, 250.0, 40.0, 290.0]),
                (3, [200.0, 200.0, 240.0, 240.0]),
            ],
        );
        assert_eq!(second[0].id, first[0].id);
        assert_ne!(
            second[1].id, first[1].id,
            "same key, no overlap: a new thing"
        );
        assert_ne!(second[2].id, first[1].id, "overlap, other key: a new thing");
        let ids: std::collections::HashSet<u64> = second.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 3);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release -p archie-accel bench -- --ignored --nocapture`
    #[test]
    #[ignore = "timing, run in release"]
    fn twenty_boxes_on_a_vga_frame() {
        let (w, h) = (640u32, 480u32);
        let frame = |shift: u32| {
            let pixels = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w + shift, i / w);
                    ((x * 13 + y * 7 + (x * y) % 17 * 11) % 251) as u8
                })
                .collect();
            Luma::new(w, h, pixels).unwrap()
        };
        let mut tracker = Tracker::new(TrackConfig::default());
        let boxes: Vec<(u64, [f32; 4])> = (0..20)
            .map(|i| {
                let (x, y) = (40.0 + (i % 5) as f32 * 110.0, 40.0 + (i / 5) as f32 * 100.0);
                (i, [x, y, x + 60.0, y + 60.0])
            })
            .collect();
        tracker.anchor(&frame(0), &boxes);
        let started = std::time::Instant::now();
        let rounds = 20;
        for round in 1..=rounds {
            let now = tracker.advance(&frame(round * 2));
            assert!(now.iter().all(|t| !t.lost));
        }
        let per = started.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        println!(
            "20 boxes, 640x480, search 16 (recover 48): {per:.2} ms per frame on {} threads",
            rayon::current_num_threads()
        );
    }
}
