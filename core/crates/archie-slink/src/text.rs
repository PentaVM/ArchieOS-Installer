//! Seen text to Wanda: stage A of the bridge from IRIS's reading to Wanda's LNCM (§7.2).
//!
//! Text sources, in order: the accessibility tree and a page's DOM (exact, free), IRIS's
//! reading head on damaged regions with no structured text, a specialist OCR. Each tick:
//!
//! 1. [`TextBridge::needs_reading`] says which damaged regions to read: those whose pixels
//!    changed (by digest) and that no structured source covers. Unchanged regions are not
//!    read again.
//! 2. [`TextBridge::tick`] takes every source's lines and keeps, per region, the most
//!    preferred source's line; only lines that are new or changed go out, as one
//!    `scene.text` delta.
//! 3. [`Batcher`] gathers deltas and hands them to Wanda's state update in one call every
//!    `max_lines` lines or `max_wait_ms`, so the LNCM's reader updates the social state in
//!    place instead of a transcript growing until it has to be compacted.
//!
//! IRIS's reading head (`read_decoder`) is in the release but not yet ported to Rust; until
//! it is, regions without structured text are listed by `needs_reading` and wait for a
//! reader.

use crate::links::{SceneTextDelta, SceneTextLine, TextOrigin};
use std::collections::{HashMap, HashSet};

/// A line one source read for a region (a track) this tick.
#[derive(Debug, Clone, PartialEq)]
pub struct LineCandidate {
    pub track: u64,
    /// Digest of the region's pixels when it was read.
    pub region_digest: [u8; 32],
    pub text: String,
    pub origin: TextOrigin,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
struct Seen {
    digest: [u8; 32],
    text: String,
    origin: TextOrigin,
}

#[derive(Debug, Clone)]
pub struct TextBridge {
    focus: String,
    seen: HashMap<u64, Seen>,
}

impl TextBridge {
    pub fn new(focus: &str) -> Self {
        Self {
            focus: focus.to_owned(),
            seen: HashMap::new(),
        }
    }

    /// Damaged regions `(track, digest)` worth reading: changed since last read, and not
    /// covered by a structured source this tick.
    pub fn needs_reading(
        &self,
        regions: &[(u64, [u8; 32])],
        structured: &HashSet<u64>,
    ) -> Vec<u64> {
        regions
            .iter()
            .filter(|(track, digest)| {
                !structured.contains(track)
                    && self.seen.get(track).is_none_or(|s| s.digest != *digest)
            })
            .map(|(track, _)| *track)
            .collect()
    }

    /// The tick's lines: the most preferred source per region; new or changed only.
    pub fn tick(&mut self, candidates: Vec<LineCandidate>, now_ms: u64) -> Option<SceneTextDelta> {
        let mut best: HashMap<u64, LineCandidate> = HashMap::new();
        for candidate in candidates {
            let text = candidate.text.trim();
            if text.is_empty() {
                continue;
            }
            let candidate = LineCandidate {
                text: text.to_owned(),
                ..candidate
            };
            match best.get(&candidate.track) {
                Some(current)
                    if (
                        current.origin,
                        current.confidence.total_cmp(&candidate.confidence),
                    ) >= (candidate.origin, std::cmp::Ordering::Equal) => {}
                _ => {
                    best.insert(candidate.track, candidate);
                }
            }
        }
        let mut tracks: Vec<u64> = best.keys().copied().collect();
        tracks.sort_unstable();
        let mut lines = Vec::new();
        for track in tracks {
            let candidate = best.remove(&track).expect("listed");
            let changed = match self.seen.get(&track) {
                None => true,
                Some(seen) => seen.text != candidate.text,
            };
            let keep_older = self.seen.get(&track).is_some_and(|seen| {
                seen.origin > candidate.origin && seen.digest == candidate.region_digest
            });
            if keep_older {
                continue;
            }
            self.seen.insert(
                track,
                Seen {
                    digest: candidate.region_digest,
                    text: candidate.text.clone(),
                    origin: candidate.origin,
                },
            );
            if changed {
                lines.push(SceneTextLine {
                    track,
                    text: candidate.text,
                    origin: candidate.origin,
                    confidence: candidate.confidence,
                    observed_at_ms: now_ms,
                });
            }
        }
        (!lines.is_empty()).then(|| SceneTextDelta {
            focus: self.focus.clone(),
            lines,
        })
    }

    /// A region is gone (the window closed, the track was lost).
    pub fn forget(&mut self, track: u64) {
        self.seen.remove(&track);
    }
}

/// Where seen text ends up: Wanda's state update (the LNCM reader's `update` with the lines
/// as a chunk). Implemented by Wanda's side.
pub trait StateUpdate {
    fn update(&mut self, focus: &str, lines: &[SceneTextLine]) -> Result<(), String>;
}

/// Gathers lines and hands them over in batches.
#[derive(Debug, Clone)]
pub struct Batcher {
    pending: Vec<SceneTextLine>,
    focus: String,
    first_at: Option<u64>,
    pub max_lines: usize,
    pub max_wait_ms: u64,
}

impl Batcher {
    pub fn new(focus: &str, max_lines: usize, max_wait_ms: u64) -> Self {
        Self {
            pending: Vec::new(),
            focus: focus.to_owned(),
            first_at: None,
            max_lines: max_lines.max(1),
            max_wait_ms,
        }
    }

    pub fn push(&mut self, delta: SceneTextDelta, now_ms: u64) {
        if delta.lines.is_empty() {
            return;
        }
        self.first_at.get_or_insert(now_ms);
        // A newer line for a region replaces the older one still waiting.
        for line in delta.lines {
            self.pending.retain(|l| l.track != line.track);
            self.pending.push(line);
        }
    }

    /// Hands the batch to `sink` when it is full or old enough; returns lines handed over.
    pub fn flush_due(&mut self, now_ms: u64, sink: &mut dyn StateUpdate) -> Result<usize, String> {
        let due = self.pending.len() >= self.max_lines
            || self
                .first_at
                .is_some_and(|t| now_ms.saturating_sub(t) >= self.max_wait_ms);
        if !due || self.pending.is_empty() {
            return Ok(0);
        }
        sink.update(&self.focus, &self.pending)?;
        let count = self.pending.len();
        self.pending.clear();
        self.first_at = None;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(track: u64, digest: u8, text: &str, origin: TextOrigin) -> LineCandidate {
        LineCandidate {
            track,
            region_digest: [digest; 32],
            text: text.into(),
            origin,
            confidence: 0.9,
        }
    }

    #[test]
    fn only_changed_regions_without_structured_text_are_read() {
        let mut bridge = TextBridge::new("screen:0");
        bridge.tick(vec![line(1, 1, "Asunto: viernes", TextOrigin::IrisRead)], 0);
        let structured: HashSet<u64> = [3].into();
        let regions = [(1, [1; 32]), (2, [7; 32]), (3, [9; 32]), (1, [1; 32])];
        assert_eq!(bridge.needs_reading(&regions[..3], &structured), vec![2]);
        assert_eq!(bridge.needs_reading(&[(1, [2; 32])], &structured), vec![1]);
    }

    #[test]
    fn the_preferred_source_wins_and_unchanged_lines_are_not_sent_again() {
        let mut bridge = TextBridge::new("screen:0");
        let delta = bridge
            .tick(
                vec![
                    line(1, 1, "Reunion con Ana", TextOrigin::IrisRead),
                    line(1, 1, "Reunión con Ana", TextOrigin::Accessibility),
                    line(2, 2, "  ", TextOrigin::Dom),
                ],
                10,
            )
            .unwrap();
        assert_eq!(delta.lines.len(), 1);
        assert_eq!(delta.lines[0].text, "Reunión con Ana");
        assert_eq!(delta.lines[0].origin, TextOrigin::Accessibility);
        // The same text again: nothing to send.
        assert!(bridge
            .tick(
                vec![line(1, 1, "Reunión con Ana", TextOrigin::Accessibility)],
                20
            )
            .is_none());
        // A worse source on unchanged pixels does not replace the better reading.
        assert!(bridge
            .tick(
                vec![line(1, 1, "Reunlon con Ana", TextOrigin::ExternalOcr)],
                30
            )
            .is_none());
        // The region changed: the new text goes out.
        let delta = bridge
            .tick(
                vec![line(1, 4, "Reunión con Ana, 10:00", TextOrigin::IrisRead)],
                40,
            )
            .unwrap();
        assert_eq!(delta.lines[0].text, "Reunión con Ana, 10:00");
        bridge.forget(1);
        assert_eq!(
            bridge.needs_reading(&[(1, [4; 32])], &HashSet::new()),
            vec![1]
        );
    }

    struct Sink(Vec<(String, Vec<String>)>);

    impl StateUpdate for Sink {
        fn update(&mut self, focus: &str, lines: &[SceneTextLine]) -> Result<(), String> {
            self.0
                .push((focus.into(), lines.iter().map(|l| l.text.clone()).collect()));
            Ok(())
        }
    }

    #[test]
    fn batches_go_to_the_state_update_by_size_or_age() {
        let mut bridge = TextBridge::new("screen:0");
        let mut batcher = Batcher::new("screen:0", 3, 500);
        let mut sink = Sink(Vec::new());
        batcher.push(
            bridge
                .tick(
                    vec![
                        line(1, 1, "uno", TextOrigin::Dom),
                        line(2, 2, "dos", TextOrigin::Dom),
                    ],
                    0,
                )
                .unwrap(),
            0,
        );
        assert_eq!(batcher.flush_due(100, &mut sink).unwrap(), 0);
        batcher.push(
            bridge
                .tick(vec![line(1, 3, "uno bis", TextOrigin::Dom)], 150)
                .unwrap(),
            150,
        );
        assert_eq!(
            batcher.flush_due(200, &mut sink).unwrap(),
            0,
            "two regions, one replaced"
        );
        assert_eq!(batcher.flush_due(600, &mut sink).unwrap(), 2, "old enough");
        assert_eq!(
            sink.0,
            vec![("screen:0".into(), vec!["dos".into(), "uno bis".into()])]
        );
        for (track, text) in [(5, "a"), (6, "b"), (7, "c")] {
            batcher.push(
                bridge
                    .tick(vec![line(track, 9, text, TextOrigin::Dom)], 700)
                    .unwrap(),
                700,
            );
        }
        assert_eq!(batcher.flush_due(701, &mut sink).unwrap(), 3, "full");
    }
}
