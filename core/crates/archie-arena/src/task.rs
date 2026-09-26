//! A task, as a file: what to ask, in which application, with what, for how long.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    /// What Archie is asked, as a person would ask it.
    pub prompt: String,
    /// The application it is done in, by the app id the Stage policy names.
    pub app_id: String,
    /// A picture Archie is shown beside the task (and a picture task is judged against).
    #[serde(default)]
    pub reference: Option<PathBuf>,
    /// Text Archie is given to read: an application's manual, a device's documentation.
    #[serde(default)]
    pub docs: Option<PathBuf>,
    /// The time limits it is run under, in seconds; looking and deciding count.
    pub tiers: Vec<u32>,
    /// How the result is judged: "picture" compares the canvas with the reference.
    pub judge: String,
    /// The command that gives the application back its first state before the clock starts
    /// (run from the repository root), when the arena is not given one.
    #[serde(default)]
    pub reset: Option<String>,
    /// Whether Archie's "done" ends the run. A picture is judged at the end of its time, so a
    /// painting task says no: a done with 40 s left is only a belief, and the time is still
    /// there to use.
    #[serde(default = "ends_early")]
    pub ends_early: bool,
    /// Models that write the brief before the clock starts, tried in order: slow and
    /// thorough, since their time is not counted.
    #[serde(default = "default_planners")]
    pub planners: Vec<String>,
    /// Models that decide the actions while the clock runs, tried in order: fast, since every
    /// second they think is a second not painting. IRIS is to take this part over.
    #[serde(default = "default_actors")]
    pub actors: Vec<String>,
    /// Models that judge and reflect, tried in order.
    #[serde(default = "default_critics")]
    pub critics: Vec<String>,
}

fn ends_early() -> bool {
    true
}

fn default_planners() -> Vec<String> {
    vec!["moonshotai/Kimi-K3".into()]
}

/// MiniCPM-V 4.5 answers in about a second where Kimi K3 took 4 to 11 s, and 75 s when
/// Nebius was busy (2026-09-26): a timed run cannot wait on the planner. Alone, MiniCPM
/// mostly planned in words; with Kimi's brief in front of it, it has the plan to act on.
fn default_actors() -> Vec<String> {
    vec!["openbmb/MiniCPM-V-4_5".into()]
}

fn default_critics() -> Vec<String> {
    vec!["moonshotai/Kimi-K3".into(), "openbmb/MiniCPM-V-4_5".into()]
}

impl Task {
    /// Reads a task; its files are named relative to it.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut task: Task = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        task.reference = task.reference.map(|p| if p.is_absolute() { p } else { base.join(p) });
        task.docs = task.docs.map(|p| if p.is_absolute() { p } else { base.join(p) });
        if task.tiers.is_empty() || task.tiers.iter().any(|t| *t == 0) {
            return Err(format!("{}: tiers are positive numbers of seconds", path.display()));
        }
        if task.judge == "picture" && task.reference.is_none() {
            return Err(format!("{}: a picture is judged against a reference", path.display()));
        }
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_reads_with_its_files_beside_it() {
        let dir = std::env::temp_dir().join("archie-arena-task-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.toml");
        std::fs::write(
            &path,
            "id = \"x\"\nprompt = \"copy it\"\napp_id = \"firefox\"\nreference = \"ref.png\"\ntiers = [15, 60]\njudge = \"picture\"\n",
        )
        .unwrap();
        let task = Task::load(&path).unwrap();
        assert_eq!(task.reference.unwrap(), dir.join("ref.png"));
        assert_eq!(task.planners[0], "moonshotai/Kimi-K3", "the slow, thorough model plans");
        assert_eq!(task.actors[0], "openbmb/MiniCPM-V-4_5", "the fast one acts on the clock");
        std::fs::write(&path, "id = \"x\"\nprompt = \"p\"\napp_id = \"a\"\ntiers = [15]\njudge = \"picture\"\n").unwrap();
        assert!(Task::load(&path).is_err(), "a picture task needs its reference");
    }
}
