//! One person's GPU machine: who it credits, where the IRIS base and the trainer are, and
//! the training and checking jobs it runs. A job is the IRIS Foundry trainer in a process
//! of its own that outlives the SSH session that started it; it writes a log and, when it
//! finishes, a report the node reads.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Deserialize)]
pub struct NodeConfig {
    /// The name credits go to. Also what the other nodes see on the board.
    pub contributor: String,
    /// "RTX 3070 8GB": recorded with every package this node trains.
    pub gpu: String,
    /// The Python of the IRIS Foundry environment.
    pub python: PathBuf,
    /// An archie-iris checkout; its `src` is the trainer's import path.
    pub foundry: PathBuf,
    /// The IRIS base the skills are trained on (a Beta 2 weights file or checkpoint).
    pub weights: PathBuf,
    /// Packages in use on this machine (IRIS serving reads them).
    pub skills_dir: PathBuf,
    /// Jobs and downloaded candidates.
    pub work_dir: PathBuf,
    /// This node's clone of the shared git repository (the board, the catalog, the packages).
    /// Pushes go out with this person's own git identity and credentials.
    pub board_repo: PathBuf,
    #[serde(default = "default_branch")]
    pub board_branch: String,
    /// Where the shared repository's files are served over HTTPS, for the catalog IRIS
    /// downloads from (e.g. "https://raw.githubusercontent.com/PentaVM/iris-skills/main").
    /// Empty: the board is kept, no catalog is written.
    #[serde(default)]
    pub public_url: String,
}

fn default_branch() -> String {
    "main".into()
}

impl NodeConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let config: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if !valid_name(&config.contributor) {
            return Err("contributor must be 1-32 letters, digits, '-' or '_'".into());
        }
        Ok(config)
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.work_dir.join("jobs")
    }
}

/// Skill and contributor names: short, and safe as a path component on every system.
pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Job ids: a start time, the kind and a skill name, so a little longer than a name.
pub fn valid_job_id(id: &str) -> bool {
    (1..=80).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Skills every node can train from data the trainer fetches by itself, and their sources
/// (archie_iris_foundry.skill_training.SKILL_SOURCES): a game, and families of apps.
pub const READY_SOURCES: [(&str, &str); 5] = [
    ("csgo", "csgo"),
    ("android", "gui:aitw,coat"),
    ("web", "gui:mind2web"),
    ("desktop", "gui:guide"),
    ("miniwob", "gui:miniwob"),
];

/// Where a job's data comes from: a set the trainer knows (CS:GO, or the GUI steps of some
/// families of apps), or a folder of recorded windows under this node's `work_dir/data`.
pub fn source_arg(config: &NodeConfig, source: &str) -> Result<String, String> {
    if source == "csgo" {
        return Ok("csgo".into());
    }
    if let Some(subsets) = source.strip_prefix("gui:") {
        let known = ["aitw", "coat", "mind2web", "miniwob", "guide"];
        if !subsets.is_empty() && subsets.split(',').all(|subset| known.contains(&subset)) {
            return Ok(source.to_string());
        }
        return Err(
            "gui: sources are aitw, coat, mind2web, miniwob and guide, comma-separated".into(),
        );
    }
    if !valid_name(source) {
        return Err(
            "source is 'csgo', 'gui:<subsets>' or the name of a folder in work_dir/data".into(),
        );
    }
    let folder = config.work_dir.join("data").join(source);
    if !folder.is_dir() {
        return Err(format!("no recorded windows at {}", folder.display()));
    }
    Ok(folder.display().to_string())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// "train" or "verify".
    pub kind: String,
    pub skill: String,
    pub source: String,
    /// For a check: the candidate's path in the repository.
    #[serde(default)]
    pub candidate: String,
    pub pid: u32,
    pub started: u64,
}

impl Job {
    pub fn dir(&self, config: &NodeConfig) -> PathBuf {
        config.jobs_dir().join(&self.id)
    }

    pub fn load(config: &NodeConfig, id: &str) -> Result<Self, String> {
        if !valid_job_id(id) {
            return Err("job ids are the ones job_list gives".into());
        }
        let text = std::fs::read_to_string(config.jobs_dir().join(id).join("job.json"))
            .map_err(|_| format!("no job {id}"))?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn report(&self, config: &NodeConfig) -> Option<serde_json::Value> {
        let text = std::fs::read_to_string(self.dir(config).join("report.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn log_tail(&self, config: &NodeConfig, lines: usize) -> String {
        let text = std::fs::read_to_string(self.dir(config).join("log.txt")).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    /// "done" once the report is written, "running" while the process lives, else "failed".
    pub fn state(&self, config: &NodeConfig) -> &'static str {
        if self.report(config).is_some() {
            "done"
        } else if process_alive(self.pid) {
            "running"
        } else {
            "failed"
        }
    }
}

/// The trainer's arguments for a job; the package goes to the job's own folder, never
/// straight into `skills_dir`.
pub fn trainer_args(
    config: &NodeConfig,
    job: &Job,
    steps: u32,
    package: Option<&Path>,
) -> Vec<String> {
    let dir = job.dir(config);
    let mut args = vec![
        "-u".into(),
        "-m".into(),
        "archie_iris_foundry.skill_training".into(),
        "--weights".into(),
        config.weights.display().to_string(),
        "--skill".into(),
        job.skill.clone(),
        "--source".into(),
        job.source.clone(),
        "--skills-dir".into(),
        dir.join("skills").display().to_string(),
        "--contributor".into(),
        config.contributor.clone(),
        "--gpu".into(),
        config.gpu.clone(),
        "--report".into(),
        dir.join("report.json").display().to_string(),
    ];
    match package {
        Some(package) => args.extend(["--verify".into(), package.display().to_string()]),
        None => args.extend(["--steps".into(), steps.to_string()]),
    }
    args
}

/// Starts the trainer detached from this process and from the SSH session: it keeps
/// running when the agent disconnects.
pub fn spawn(config: &NodeConfig, job: &Job, args: &[String]) -> Result<u32, String> {
    let dir = job.dir(config);
    std::fs::create_dir_all(dir.join("skills")).map_err(|e| e.to_string())?;
    // What this node uses now is the baseline a new package has to beat.
    let current = config.skills_dir.join(format!("{}.skill", job.skill));
    if current.is_file() {
        let _ = std::fs::copy(
            &current,
            dir.join("skills").join(format!("{}.skill", job.skill)),
        );
    }
    let start = |breakaway: bool| -> std::io::Result<u32> {
        let log = std::fs::File::create(dir.join("log.txt"))?;
        let log_err = log.try_clone()?;
        let mut command = detached(&config.python, breakaway);
        command
            .args(args)
            .current_dir(&config.foundry)
            .env("PYTHONPATH", config.foundry.join("src"))
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err);
        Ok(command.spawn()?.id())
    };
    // A session whose job object forbids breaking away refuses the first form; the trainer
    // then runs inside it, and a disconnect may end it (docs/SKILL-NODES.md).
    start(true)
        .or_else(|_| start(false))
        .map_err(|e| format!("could not start the trainer: {e}"))
}

#[cfg(windows)]
fn detached(program: &Path, breakaway: bool) -> Command {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP, and CREATE_BREAKAWAY_FROM_JOB to leave the
    // SSH session's job object, so closing the session does not end the training.
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
    const BREAKAWAY: u32 = 0x0100_0000;
    let mut command = Command::new(program);
    command.creation_flags(if breakaway {
        DETACHED | BREAKAWAY
    } else {
        DETACHED
    });
    command
}

#[cfg(not(windows))]
fn detached(program: &Path, _breakaway: bool) -> Command {
    // A session of its own: the hangup at logout does not reach it.
    let mut command = Command::new("setsid");
    command.arg(program);
    command
}

pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    #[cfg(not(windows))]
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output();
    output
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

/// The GPU as `nvidia-smi` reports it now: name, memory used and total, use.
pub fn gpu_now() -> Option<String> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.used,memory.total,utilization.gpu",
            "--format=csv,noheader",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(work: &Path) -> NodeConfig {
        NodeConfig {
            contributor: "nelson".into(),
            gpu: "RTX 3070 8GB".into(),
            python: "python".into(),
            foundry: work.join("archie-iris"),
            weights: work.join("base.pt"),
            skills_dir: work.join("skills"),
            work_dir: work.to_path_buf(),
            board_repo: work.join("iris-skills"),
            board_branch: "main".into(),
            public_url: String::new(),
        }
    }

    #[test]
    fn names_are_path_safe_and_sources_stay_inside_the_node() {
        assert!(valid_name("rocket-league") && valid_name("csgo_2"));
        for bad in ["", "../etc", "a/b", "C:", "x".repeat(33).as_str()] {
            assert!(!valid_name(bad), "{bad}");
        }
        let work =
            std::env::temp_dir().join(format!("archie-skill-node-test-{}", std::process::id()));
        std::fs::create_dir_all(work.join("data").join("kdenlive")).unwrap();
        let config = config(&work);
        assert_eq!(source_arg(&config, "csgo").unwrap(), "csgo");
        assert_eq!(
            source_arg(&config, "gui:aitw,coat").unwrap(),
            "gui:aitw,coat"
        );
        assert!(
            source_arg(&config, "gui:").is_err() && source_arg(&config, "gui:aitw,../x").is_err()
        );
        assert!(READY_SOURCES
            .iter()
            .all(|(_, source)| source_arg(&config, source).is_ok()));
        assert!(source_arg(&config, "kdenlive")
            .unwrap()
            .ends_with("kdenlive"));
        assert!(source_arg(&config, "missing").is_err());
        assert!(source_arg(&config, "../../secrets").is_err());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn a_job_trains_into_its_own_folder_and_says_who_trained_it() {
        let work = PathBuf::from("/work");
        let config = config(&work);
        let job = Job {
            id: "20260930-csgo".into(),
            kind: "train".into(),
            skill: "csgo".into(),
            source: "csgo".into(),
            candidate: String::new(),
            pid: 0,
            started: 0,
        };
        let args = trainer_args(&config, &job, 300, None);
        let at = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(at("--contributor"), "nelson");
        assert_eq!(at("--steps"), "300");
        assert!(
            at("--skills-dir").contains("20260930-csgo"),
            "never straight into the skills in use"
        );
        let check = trainer_args(&config, &job, 300, Some(Path::new("/work/candidate.skill")));
        assert!(check.contains(&"--verify".to_string()) && !check.contains(&"--steps".to_string()));
    }
}
