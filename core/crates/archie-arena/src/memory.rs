//! What Archie carries from one run of a task to the next: the RSI loop's memory.
//!
//! After every run a reflection turns what happened — the score, the critic's words, the
//! actions that were refused or did nothing, Archie's own notes — into short lessons. The
//! next run is given the lessons of the runs that scored best, and those of the latest,
//! so what worked is kept and what was just learned is tried. The file is the whole of
//! it: a run can be read, and a lesson that misleads can be struck out by hand.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    pub runs: Vec<Remembered>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remembered {
    pub run: String,
    pub tier: u32,
    pub score: f32,
    pub lessons: Vec<String>,
}

/// The most lessons a run is given.
pub const MOST_LESSONS: usize = 12;

impl Memory {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    }

    /// The lessons for the next run of `tier`: the best-scoring runs of any tier first (what
    /// is known to work transfers across time limits), then the latest run's, without
    /// repeats, at most [`MOST_LESSONS`].
    pub fn lessons(&self) -> Vec<String> {
        let mut ranked: Vec<&Remembered> = self.runs.iter().collect();
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut out: Vec<String> = Vec::new();
        let latest = self.runs.last();
        for run in ranked.iter().take(3).copied().chain(latest) {
            for lesson in &run.lessons {
                let plain = lesson.trim();
                if !plain.is_empty() && !out.iter().any(|l| l.eq_ignore_ascii_case(plain)) {
                    out.push(plain.to_string());
                }
            }
        }
        out.truncate(MOST_LESSONS);
        out
    }

    /// The best score so far at `tier`, if it was ever run.
    pub fn best(&self, tier: u32) -> Option<f32> {
        self.runs.iter().filter(|r| r.tier == tier).map(|r| r.score).max_by(f32::total_cmp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, tier: u32, score: f32, lessons: &[&str]) -> Remembered {
        Remembered { run: name.into(), tier, score, lessons: lessons.iter().map(|l| l.to_string()).collect() }
    }

    #[test]
    fn the_best_runs_lessons_come_first_then_the_latest_without_repeats() {
        let memory = Memory {
            runs: vec![
                run("a", 15, 30.0, &["brush is B"]),
                run("b", 60, 55.0, &["colour: click the foreground swatch", "brush is B"]),
                run("c", 15, 20.0, &["zooming wastes time"]),
            ],
        };
        assert_eq!(
            memory.lessons(),
            vec!["colour: click the foreground swatch", "brush is B", "zooming wastes time"]
        );
        assert_eq!(memory.best(15), Some(30.0));
        assert_eq!(memory.best(300), None);
    }
}
