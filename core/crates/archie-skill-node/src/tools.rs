//! The tools an AI agent gets when it connects to a node: see the node and the board,
//! choose work, claim it, train or check, publish, and install what is in use.

use crate::board::{Board, BoardError, Candidate, Wanted};
use crate::hub::{Hub, HubError};
use crate::node::{self, Job, NodeConfig};
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub const BOARD_PATH: &str = "board.json";
pub const CATALOG_PATH: &str = "catalog.json";

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

pub fn tools() -> Vec<Tool> {
    let skill = json!({"type": "string", "pattern": "^[A-Za-z0-9_-]{1,32}$"});
    let job = json!({"type": "object", "properties": {"job": {"type": "string"}}, "required": ["job"], "additionalProperties": false});
    vec![
        Tool { name: "node_status",
               description: "This node: who it credits, its GPU now (name, memory, use), whether the IRIS base is here, \
                             the skills installed and the jobs running.",
               schema: json!({"type": "object", "properties": {}, "additionalProperties": false}) },
        Tool { name: "skill_board",
               description: "The shared board and the work there is, most needed first: candidates others trained that \
                             wait for a second node's check (do these first; nobody checks their own), skills people want, \
                             skills with no package yet, then retrains. Choose one that fits a source this node has, claim \
                             it, then train or check it.",
               schema: json!({"type": "object", "properties": {}, "additionalProperties": false}) },
        Tool { name: "skill_claim",
               description: "Takes a skill for up to 12 hours so no other node trains it meanwhile. Refused if another \
                             node holds it.",
               schema: json!({"type": "object", "properties": {"skill": skill, "source": {"type": "string"},
                              "hours": {"type": "integer", "minimum": 1, "maximum": 12}},
                              "required": ["skill", "source"], "additionalProperties": false}) },
        Tool { name: "skill_train",
               description: "Trains the claimed skill's adapter on this node's GPU with the IRIS base frozen, then \
                             measures it on sealed windows. Runs in the background; follow it with job_status.",
               schema: json!({"type": "object", "properties": {"skill": skill, "steps": {"type": "integer", "minimum": 20,
                              "maximum": 3000}}, "required": ["skill"], "additionalProperties": false}) },
        Tool { name: "skill_check",
               description: "Checks a candidate someone else trained on this node's own sealed windows. Runs in the \
                             background; record the verdict with skill_record_check when job_status says done.",
               schema: json!({"type": "object", "properties": {"skill": skill, "candidate": {"type": "string"}},
                              "required": ["skill", "candidate"], "additionalProperties": false}) },
        Tool { name: "job_list", description: "This node's jobs, newest first, with their state.",
               schema: json!({"type": "object", "properties": {}, "additionalProperties": false}) },
        Tool { name: "job_status", description: "One job: its state, the end of its log, and its report once done.",
               schema: job.clone() },
        Tool { name: "skill_publish",
               description: "Uploads a finished training job's package as a candidate, credited to this node's person, \
                             and lets the skill go. Only a package the node's own sealed check promoted is uploaded.",
               schema: job.clone() },
        Tool { name: "skill_record_check",
               description: "Writes a finished check's verdict to the board: verified, the candidate becomes the package \
                             in use and both people are credited; not verified, it is marked rejected with the reason.",
               schema: job },
        Tool { name: "skill_install",
               description: "Downloads the package in use for a skill into this node's skills folder, where IRIS serving \
                             reads it.",
               schema: json!({"type": "object", "properties": {"skill": skill}, "required": ["skill"],
                              "additionalProperties": false}) },
        Tool { name: "skill_want",
               description: "Adds a skill to the wanted list with why (an app or game someone uses) and the source to \
                             train it from.",
               schema: json!({"type": "object", "properties": {"skill": skill, "why": {"type": "string", "maxLength": 200},
                              "source": {"type": "string"}}, "required": ["skill", "why"], "additionalProperties": false}) },
    ]
}

pub struct Node<H: Hub> {
    pub config: NodeConfig,
    pub hub: H,
    pub clock: fn() -> u64,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

type Outcome = Result<Value, String>;

fn text(arguments: &Map<String, Value>, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{name} is required"))
}

fn name_arg(arguments: &Map<String, Value>, name: &str) -> Result<String, String> {
    let value = text(arguments, name)?;
    if node::valid_name(&value) {
        Ok(value)
    } else {
        Err(format!("{name} must be letters, digits, '-' or '_'"))
    }
}

impl<H: Hub> Node<H> {
    pub fn call(&mut self, name: &str, arguments: &Map<String, Value>) -> Outcome {
        match name {
            "node_status" => self.status(),
            "skill_board" => self.board_view(),
            "skill_claim" => {
                let skill = name_arg(arguments, "skill")?;
                let source = text(arguments, "source")?;
                node::source_arg(&self.config, &source)?;
                let hours = arguments.get("hours").and_then(Value::as_u64).unwrap_or(4);
                let (me, now) = (self.config.contributor.clone(), (self.clock)());
                self.change(&format!("{me} claims {skill}"), Vec::new(), |board| {
                    board
                        .claim(&skill, &source, &me, hours, now)
                        .map_err(|e| e.to_string())
                })?;
                Ok(json!({"claimed": skill, "by": me, "hours": hours}))
            }
            "skill_train" => {
                let skill = name_arg(arguments, "skill")?;
                let steps = arguments
                    .get("steps")
                    .and_then(Value::as_u64)
                    .unwrap_or(300)
                    .clamp(20, 3000) as u32;
                let board = self.read_board()?.0;
                if !board.holds(&skill, &self.config.contributor, (self.clock)()) {
                    return Err(BoardError::NotClaimedByYou.to_string());
                }
                let source = node::source_arg(&self.config, &board.skills[&skill].source)?;
                self.start("train", &skill, &source, "", steps, None)
            }
            "skill_check" => {
                let skill = name_arg(arguments, "skill")?;
                let candidate = text(arguments, "candidate")?;
                let board = self.read_board()?.0;
                let entry = board.skills.get(&skill).ok_or("skill_unknown")?;
                let found = entry
                    .candidates
                    .iter()
                    .find(|c| c.path == candidate && c.state == "waiting")
                    .ok_or("candidate_unknown: skill_board lists the ones waiting")?;
                if found.trained_by == self.config.contributor {
                    return Err(BoardError::OwnCandidate.to_string());
                }
                let bytes = self.hub.read(&candidate).map_err(|e| e.to_string())?;
                if node::sha256_hex(&bytes) != found.sha256 {
                    return Err("the candidate's bytes do not match the digest on the board".into());
                }
                let source = node::source_arg(&self.config, &entry.source)?;
                let folder = self.config.work_dir.join("candidates");
                std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                let local = folder.join(format!("{}-{}.skill", skill, &found.sha256[..12]));
                std::fs::write(&local, &bytes).map_err(|e| e.to_string())?;
                self.start("verify", &skill, &source, &candidate, 0, Some(&local))
            }
            "job_list" => self.jobs(),
            "job_status" => {
                let job = Job::load(&self.config, &text(arguments, "job")?)?;
                Ok(
                    json!({"job": job, "state": job.state(&self.config), "log": job.log_tail(&self.config, 20),
                          "report": job.report(&self.config)}),
                )
            }
            "skill_publish" => self.publish(&text(arguments, "job")?),
            "skill_record_check" => self.record_check(&text(arguments, "job")?),
            "skill_install" => {
                let skill = name_arg(arguments, "skill")?;
                let board = self.read_board()?.0;
                let current = board
                    .skills
                    .get(&skill)
                    .and_then(|s| s.current.clone())
                    .ok_or("that skill has no package in use yet")?;
                let bytes = self.hub.read(&current.path).map_err(|e| e.to_string())?;
                if node::sha256_hex(&bytes) != current.sha256 {
                    return Err("the package's bytes do not match the digest on the board".into());
                }
                std::fs::create_dir_all(&self.config.skills_dir).map_err(|e| e.to_string())?;
                std::fs::write(
                    self.config.skills_dir.join(format!("{skill}.skill")),
                    &bytes,
                )
                .map_err(|e| e.to_string())?;
                Ok(
                    json!({"installed": skill, "trained_by": current.trained_by, "verified_by": current.verified_by}),
                )
            }
            "skill_want" => {
                let skill = name_arg(arguments, "skill")?;
                let why: String = text(arguments, "why")?.chars().take(200).collect();
                let source = arguments
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let me = self.config.contributor.clone();
                self.change(&format!("{me} wants {skill}"), Vec::new(), |board| {
                    if !board.wanted.iter().any(|w| w.skill == skill)
                        && !board.skills.contains_key(&skill)
                    {
                        board.wanted.push(Wanted {
                            skill: skill.clone(),
                            why: why.clone(),
                            source: source.clone(),
                        });
                    }
                    Ok(())
                })?;
                Ok(json!({"wanted": skill}))
            }
            _ => Err(format!("unknown tool {name}")),
        }
    }

    fn status(&self) -> Outcome {
        let installed: Vec<String> = std::fs::read_dir(&self.config.skills_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .filter(|n| n.ends_with(".skill"))
                    .collect()
            })
            .unwrap_or_default();
        let running: Vec<String> = self
            .job_records()
            .into_iter()
            .filter(|job| job.state(&self.config) == "running")
            .map(|job| job.id)
            .collect();
        Ok(json!({
            "contributor": self.config.contributor, "gpu": self.config.gpu, "gpu_now": node::gpu_now(),
            "base_weights": self.config.weights.is_file(),
            "skills_installed": installed, "jobs_running": running,
            "board_repo": self.config.board_repo.is_dir(), "catalog_published": !self.config.public_url.is_empty(),
        }))
    }

    fn board_view(&self) -> Outcome {
        let (board, _) = self.read_board()?;
        let now = (self.clock)();
        // Skills trainable from data the trainer fetches by itself and nobody has started.
        let ready: Vec<Value> = node::READY_SOURCES
            .iter()
            .filter(|(skill, _)| !board.skills.contains_key(*skill))
            .map(|(skill, source)| json!({"skill": skill, "source": source}))
            .collect();
        Ok(
            json!({"needs": board.needs(&self.config.contributor, now), "ready_to_start": ready, "skills": board.skills,
                  "wanted": board.wanted, "credits": board.credits, "base": board.base,
                  "this_node": self.config.contributor}),
        )
    }

    fn start(
        &self,
        kind: &str,
        skill: &str,
        source: &str,
        candidate: &str,
        steps: u32,
        package: Option<&std::path::Path>,
    ) -> Outcome {
        if self
            .job_records()
            .iter()
            .any(|job| job.state(&self.config) == "running")
        {
            return Err("a job is already running on this GPU: follow it with job_status".into());
        }
        let started = (self.clock)();
        let mut job = Job {
            id: format!("{started}-{kind}-{skill}"),
            kind: kind.into(),
            skill: skill.into(),
            source: source.into(),
            candidate: candidate.into(),
            pid: 0,
            started,
        };
        let args = node::trainer_args(&self.config, &job, steps, package);
        job.pid = node::spawn(&self.config, &job, &args)?;
        std::fs::write(
            job.dir(&self.config).join("job.json"),
            serde_json::to_vec_pretty(&job).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        Ok(json!({"job": job.id, "kind": kind, "skill": skill, "follow": "job_status"}))
    }

    fn job_records(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = std::fs::read_dir(self.config.jobs_dir())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .filter_map(|id| Job::load(&self.config, &id).ok())
                    .collect()
            })
            .unwrap_or_default();
        jobs.sort_by(|a, b| b.started.cmp(&a.started));
        jobs
    }

    fn jobs(&self) -> Outcome {
        Ok(Value::Array(
            self.job_records()
                .iter()
                .map(|job| {
                    json!({
            "job": job.id, "kind": job.kind, "skill": job.skill, "state": job.state(&self.config)})
                })
                .collect(),
        ))
    }

    fn publish(&mut self, id: &str) -> Outcome {
        let job = Job::load(&self.config, id)?;
        if job.kind != "train" {
            return Err("only a training job publishes a package".into());
        }
        let report = job
            .report(&self.config)
            .ok_or("the job has not finished: job_status")?;
        if report["promoted"] != Value::Bool(true) {
            return Err(format!(
                "this node's own sealed check did not promote it ({}); nothing to publish",
                report["reason"].as_str().unwrap_or("?")
            ));
        }
        let package = job
            .dir(&self.config)
            .join("skills")
            .join(format!("{}.skill", job.skill));
        let bytes = std::fs::read(&package).map_err(|e| format!("{}: {e}", package.display()))?;
        let sha256 = node::sha256_hex(&bytes);
        let me = self.config.contributor.clone();
        let now = (self.clock)();
        let path = format!(
            "skills/{}/{}-{}-{}.skill",
            job.skill,
            now,
            me,
            &sha256[..12]
        );
        let measures = |key: &str| -> std::collections::BTreeMap<String, f64> {
            report[key]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_f64().map(|v| (k.clone(), v)))
                        .collect()
                })
                .unwrap_or_default()
        };
        let candidate = Candidate {
            path: path.clone(),
            sha256,
            bytes: bytes.len() as u64,
            trained_by: me.clone(),
            gpu: self.config.gpu.clone(),
            at: now,
            sealed_before: measures("before"),
            sealed_after: measures("after"),
            state: "waiting".into(),
        };
        let skill = job.skill.clone();
        let base = report["base_fingerprint"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if base.len() != 64 {
            return Err("the trainer's report has no base fingerprint: update archie-iris".into());
        }
        self.change(
            &format!("{me} publishes a {skill} candidate"),
            vec![(path.clone(), bytes)],
            |board| {
                // The first package names the base every later one must share.
                if board.base.is_empty() {
                    board.base = base.clone();
                } else if board.base != base {
                    return Err(BoardError::AnotherBase.to_string());
                }
                board
                    .add_candidate(&skill, candidate.clone(), now)
                    .map_err(|e| e.to_string())
            },
        )?;
        Ok(
            json!({"published": path, "trained_by": me, "next": "another node checks it with skill_check"}),
        )
    }

    fn record_check(&mut self, id: &str) -> Outcome {
        let job = Job::load(&self.config, id)?;
        if job.kind != "verify" {
            return Err("only a checking job records a verdict".into());
        }
        let report = job
            .report(&self.config)
            .ok_or("the check has not finished: job_status")?;
        let verified = report["verified"] == Value::Bool(true);
        let reason = report["reason"].as_str().unwrap_or("").to_string();
        let base = report["base_fingerprint"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let (me, now, skill, candidate) = (
            self.config.contributor.clone(),
            (self.clock)(),
            job.skill.clone(),
            job.candidate.clone(),
        );
        self.change(
            &format!(
                "{me} checks {skill}: {}",
                if verified { "verified" } else { "rejected" }
            ),
            Vec::new(),
            |board| {
                board
                    .record_check(&skill, &candidate, &me, verified, &reason, &base, now)
                    .map_err(|e| e.to_string())
            },
        )?;
        Ok(json!({"skill": skill, "verified": verified, "reason": reason, "checked_by": me}))
    }

    fn read_board(&self) -> Result<(Board, String), String> {
        let head = self.hub.head().map_err(|e| e.to_string())?;
        let board = match self.hub.read(BOARD_PATH) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("board.json: {e}"))?,
            Err(HubError::NotFound) => Board::empty(""),
            Err(error) => return Err(error.to_string()),
        };
        Ok((board, head))
    }

    /// Read, change, commit on top of what was read; if another node committed first, read
    /// again and redo the change (three tries).
    fn change<F>(
        &self,
        summary: &str,
        extra: Vec<(String, Vec<u8>)>,
        mut edit: F,
    ) -> Result<(), String>
    where
        F: FnMut(&mut Board) -> Result<(), String>,
    {
        for _ in 0..3 {
            let (mut board, head) = self.read_board()?;
            edit(&mut board)?;
            let mut files = extra.clone();
            files.push((
                BOARD_PATH.into(),
                serde_json::to_vec_pretty(&board).map_err(|e| e.to_string())?,
            ));
            if !self.config.public_url.is_empty() {
                // What IRIS serving reads: the board's packages in use, in the same commit.
                files.push((
                    CATALOG_PATH.into(),
                    serde_json::to_vec_pretty(&board.catalog(&self.config.public_url))
                        .map_err(|e| e.to_string())?,
                ));
            }
            match self.hub.commit(summary, &files, &head) {
                Ok(_) => return Ok(()),
                Err(HubError::Moved) => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("the board kept changing under this node; try again".into())
    }
}
