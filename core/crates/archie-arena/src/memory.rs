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
    /// The task it was a run of ("" for runs remembered before tasks were told apart).
    #[serde(default)]
    pub task: String,
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

    /// The lessons for the next run of `task` at `tier`. One memory serves every task in an
    /// application, and a lesson is often about one picture ("the face is at 370,450") or one
    /// time limit ("one decision, no second round" is right at 15 s and ruinous at 60 s). So
    /// the same task at the same tier comes first — its two best runs and its latest — then
    /// the best run of the same task at another tier, then the best run of any other task,
    /// for what it learnt about the application itself. No repeats, at most [`MOST_LESSONS`].
    pub fn lessons(&self, task: &str, tier: u32) -> Vec<String> {
        let best = |keep: &dyn Fn(&Remembered) -> bool, n: usize| {
            let mut ranked: Vec<&Remembered> = self.runs.iter().filter(|r| keep(r)).collect();
            ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
            ranked.truncate(n);
            ranked
        };
        let mut chosen = best(&|r| r.task == task && r.tier == tier, 2);
        chosen.extend(self.runs.iter().rev().find(|r| r.task == task && r.tier == tier));
        chosen.extend(best(&|r| r.task == task && r.tier != tier, 1));
        chosen.extend(best(&|r| r.task != task, 1));
        let mut out: Vec<String> = Vec::new();
        for run in chosen {
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

    /// The best score so far of `task` at `tier`, if it was ever run.
    pub fn best(&self, task: &str, tier: u32) -> Option<f32> {
        self.runs.iter().filter(|r| r.task == task && r.tier == tier).map(|r| r.score).max_by(f32::total_cmp)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, task: &str, tier: u32, score: f32, lessons: &[&str]) -> Remembered {
        Remembered {
            run: name.into(),
            task: task.into(),
            tier,
            score,
            lessons: lessons.iter().map(|l| l.to_string()).collect(),
        }
    }

    #[test]
    fn a_task_learns_first_from_itself_at_its_own_time_limit() {
        let memory = Memory {
            runs: vec![
                run("a", "mona", 15, 30.0, &["one decision, no second round"]),
                run("b", "dama", 60, 55.0, &["the face is at 370,450", "brush is B"]),
                run("c", "mona", 60, 20.0, &["paint the face early"]),
                run("d", "mona", 60, 10.0, &["brush is B", "close the colour dialog"]),
                run("e", "noche", 60, 5.0, &["swirls"]),
            ],
        };
        assert_eq!(
            memory.lessons("mona", 60),
            vec![
                "paint the face early",
                "brush is B",
                "close the colour dialog",
                "one decision, no second round",
                "the face is at 370,450",
            ]
        );
        assert_eq!(memory.best("mona", 15), Some(30.0));
        assert_eq!(memory.best("dama", 15), None);
    }
}
