//! The shared board: which skills exist, who is training which, the candidates waiting for a
//! second node's check, the package in use for each skill, and the credits.
//!
//! It is one JSON file in the shared git repository. Every change is a commit pushed as the
//! person whose node made it, on top of the board it read, so two changes
//! never silently overwrite each other and the history says who did what.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const BOARD_FORMAT: &str = "archie-iris-skill-board-v1";
/// The longest a skill may be held by one node before anyone else may take it.
pub const MAX_CLAIM_HOURS: u64 = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub format: String,
    /// The IRIS base every package on this board was trained on: its fingerprint, the 64
    /// hex characters the trainer reports and IRIS checks before loading a package.
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub skills: BTreeMap<String, Skill>,
    /// What people asked for and nobody has trained yet, with why.
    #[serde(default)]
    pub wanted: Vec<Wanted>,
    #[serde(default)]
    pub credits: BTreeMap<String, Credit>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// The data a node trains it from: "csgo", or a folder name nodes share by convention.
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Package>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<Claim>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
    pub trained_by: String,
    pub verified_by: String,
    pub gpu: String,
    pub at: u64,
    #[serde(default)]
    pub sealed_after: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
    pub trained_by: String,
    pub gpu: String,
    pub at: u64,
    /// The trainer's own sealed measures, before and after; a check does not trust them.
    #[serde(default)]
    pub sealed_before: BTreeMap<String, f64>,
    #[serde(default)]
    pub sealed_after: BTreeMap<String, f64>,
    /// "waiting", "rejected:<reason>" once a check said no.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub by: String,
    pub until: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wanted {
    pub skill: String,
    pub why: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Credit {
    /// Packages this person trained that a second node verified and that went into use.
    pub promoted: u64,
    /// Packages this person checked for someone else.
    pub verified: u64,
    /// Packages this person trained that a check turned down.
    pub rejected: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardError {
    ClaimedByOther { by: String, until: u64 },
    NotClaimedByYou,
    UnknownSkill,
    UnknownCandidate,
    OwnCandidate,
    AnotherBase,
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaimedByOther { by, until } => write!(f, "skill_claimed_by:{by}:until:{until}"),
            Self::NotClaimedByYou => f.write_str("skill_not_claimed_by_this_node: claim it first"),
            Self::UnknownSkill => f.write_str("skill_unknown"),
            Self::UnknownCandidate => f.write_str("candidate_unknown"),
            Self::OwnCandidate => {
                f.write_str("a candidate is checked by a node other than the one that trained it")
            }
            Self::AnotherBase => f.write_str("package_trained_on_another_base"),
        }
    }
}

impl Board {
    pub fn empty(base: &str) -> Self {
        Self {
            format: BOARD_FORMAT.into(),
            base: base.into(),
            skills: BTreeMap::new(),
            wanted: Vec::new(),
            credits: BTreeMap::new(),
        }
    }

    /// Takes a skill for `hours`, unless someone else holds it now.
    pub fn claim(
        &mut self,
        skill: &str,
        source: &str,
        by: &str,
        hours: u64,
        now: u64,
    ) -> Result<(), BoardError> {
        let entry = self.skills.entry(skill.to_string()).or_default();
        if let Some(claim) = &entry.claim {
            if claim.by != by && claim.until > now {
                return Err(BoardError::ClaimedByOther {
                    by: claim.by.clone(),
                    until: claim.until,
                });
            }
        }
        if entry.source.is_empty() {
            entry.source = source.to_string();
        }
        entry.claim = Some(Claim {
            by: by.to_string(),
            until: now + hours.clamp(1, MAX_CLAIM_HOURS) * 3600,
        });
        self.wanted.retain(|wanted| wanted.skill != skill);
        Ok(())
    }

    pub fn holds(&self, skill: &str, by: &str, now: u64) -> bool {
        self.skills
            .get(skill)
            .and_then(|s| s.claim.as_ref())
            .is_some_and(|c| c.by == by && c.until > now)
    }

    /// A trained package goes up as a candidate and the trainer lets the skill go.
    pub fn add_candidate(
        &mut self,
        skill: &str,
        candidate: Candidate,
        now: u64,
    ) -> Result<(), BoardError> {
        if !self.holds(skill, &candidate.trained_by, now) {
            return Err(BoardError::NotClaimedByYou);
        }
        let entry = self.skills.get_mut(skill).ok_or(BoardError::UnknownSkill)?;
        entry.claim = None;
        entry.candidates.push(candidate);
        Ok(())
    }

    /// A second node's verdict on a candidate. Verified: it becomes the package in use and
    /// both people are credited. Not verified: it is marked rejected with the reason.
    #[allow(clippy::too_many_arguments)]
    pub fn record_check(
        &mut self,
        skill: &str,
        path: &str,
        checker: &str,
        verified: bool,
        reason: &str,
        base: &str,
        now: u64,
    ) -> Result<(), BoardError> {
        if !self.base.is_empty() && base != self.base {
            return Err(BoardError::AnotherBase);
        }
        let entry = self.skills.get_mut(skill).ok_or(BoardError::UnknownSkill)?;
        let index = entry
            .candidates
            .iter()
            .position(|c| c.path == path && c.state == "waiting")
            .ok_or(BoardError::UnknownCandidate)?;
        if entry.candidates[index].trained_by == checker {
            return Err(BoardError::OwnCandidate);
        }
        let candidate = entry.candidates.remove(index);
        self.credits
            .entry(checker.to_string())
            .or_default()
            .verified += 1;
        if verified {
            self.credits
                .entry(candidate.trained_by.clone())
                .or_default()
                .promoted += 1;
            entry.current = Some(Package {
                path: candidate.path,
                sha256: candidate.sha256,
                bytes: candidate.bytes,
                trained_by: candidate.trained_by,
                verified_by: checker.to_string(),
                gpu: candidate.gpu,
                at: now,
                sealed_after: candidate.sealed_after,
            });
        } else {
            self.credits
                .entry(candidate.trained_by.clone())
                .or_default()
                .rejected += 1;
            let mut rejected = candidate;
            rejected.state = format!("rejected:{reason}");
            entry.candidates.push(rejected);
        }
        Ok(())
    }

    /// The catalog IRIS serving downloads from (`archie-iris-skill-catalog-v1`): the package
    /// in use for each skill, with its base, digest, size and address under `public_url`.
    pub fn catalog(&self, public_url: &str) -> serde_json::Value {
        let skills: serde_json::Map<String, serde_json::Value> = self
            .skills
            .iter()
            .filter_map(|(name, skill)| {
                skill.current.as_ref().map(|current| {
                    (
                        name.clone(),
                        serde_json::json!({
                "base": self.base, "sha256": current.sha256, "bytes": current.bytes,
                "url": format!("{}/{}", public_url.trim_end_matches('/'), current.path),
                "trained_by": current.trained_by, "verified_by": current.verified_by}),
                    )
                })
            })
            .collect();
        serde_json::json!({"format": "archie-iris-skill-catalog-v1", "skills": skills})
    }

    /// The work there is, most needed first, for an AI choosing what its node does next:
    /// candidates others trained that wait for a check, then wanted skills nobody has, then
    /// skills with no package in use, then the rest (retraining can still gain). Skills
    /// someone else holds right now are left out.
    pub fn needs(&self, me: &str, now: u64) -> Vec<Need> {
        let mut out = Vec::new();
        for (name, skill) in &self.skills {
            for candidate in skill
                .candidates
                .iter()
                .filter(|c| c.state == "waiting" && c.trained_by != me)
            {
                out.push(Need {
                    skill: name.clone(),
                    kind: "check".into(),
                    priority: 100,
                    why: format!(
                        "{} trained a candidate that waits for a second node",
                        candidate.trained_by
                    ),
                    source: skill.source.clone(),
                    candidate: Some(candidate.path.clone()),
                });
            }
        }
        for wanted in &self.wanted {
            out.push(Need {
                skill: wanted.skill.clone(),
                kind: "train".into(),
                priority: 80,
                why: wanted.why.clone(),
                source: wanted.source.clone(),
                candidate: None,
            });
        }
        for (name, skill) in &self.skills {
            let held = skill
                .claim
                .as_ref()
                .is_some_and(|c| c.by != me && c.until > now);
            let waiting = skill.candidates.iter().any(|c| c.state == "waiting");
            if held || waiting {
                continue;
            }
            let (priority, why) = match &skill.current {
                None => (60, "no package in use yet".to_string()),
                Some(current) => (
                    20,
                    format!("in use since {}; a retrain may still gain", current.at),
                ),
            };
            out.push(Need {
                skill: name.clone(),
                kind: "train".into(),
                priority,
                why,
                source: skill.source.clone(),
                candidate: None,
            });
        }
        out.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.skill.cmp(&b.skill)));
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Need {
    pub skill: String,
    /// "train" or "check".
    pub kind: String,
    pub priority: u32,
    pub why: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(by: &str, path: &str) -> Candidate {
        Candidate {
            path: path.into(),
            sha256: "ab".into(),
            bytes: 13,
            trained_by: by.into(),
            gpu: "RTX 3060 Ti".into(),
            at: 10,
            sealed_before: BTreeMap::new(),
            sealed_after: BTreeMap::from([("key_f1".into(), 0.8)]),
            state: "waiting".into(),
        }
    }

    #[test]
    fn a_skill_held_by_one_node_is_not_taken_by_another_until_the_claim_runs_out() {
        let mut board = Board::empty("base");
        board.claim("csgo", "csgo", "sheriff", 2, 1_000).unwrap();
        assert_eq!(
            board.claim("csgo", "csgo", "xavi", 2, 2_000),
            Err(BoardError::ClaimedByOther {
                by: "sheriff".into(),
                until: 1_000 + 7_200
            })
        );
        assert!(
            board.claim("csgo", "csgo", "sheriff", 1, 2_000).is_ok(),
            "the holder renews it"
        );
        assert!(
            board
                .claim("csgo", "csgo", "xavi", 99, 1_000 + 7_200 + 3_600)
                .is_ok(),
            "an old claim lapses"
        );
        assert_eq!(
            board.skills["csgo"].claim.as_ref().unwrap().until,
            1_000 + 7_200 + 3_600 + MAX_CLAIM_HOURS * 3600
        );
    }

    #[test]
    fn a_package_is_used_only_after_someone_else_checked_it_and_both_get_the_credit() {
        let mut board = Board::empty("base");
        assert_eq!(
            board.add_candidate("csgo", candidate("nelson", "a"), 0),
            Err(BoardError::NotClaimedByYou)
        );
        board.claim("csgo", "csgo", "nelson", 2, 0).unwrap();
        board
            .add_candidate("csgo", candidate("nelson", "skills/csgo/a.skill"), 10)
            .unwrap();
        assert!(
            board.skills["csgo"].claim.is_none(),
            "publishing lets the skill go"
        );
        assert_eq!(
            board.record_check(
                "csgo",
                "skills/csgo/a.skill",
                "nelson",
                true,
                "",
                "base",
                20
            ),
            Err(BoardError::OwnCandidate)
        );
        assert_eq!(
            board.record_check("csgo", "skills/csgo/a.skill", "xavi", true, "", "other", 20),
            Err(BoardError::AnotherBase)
        );
        board
            .record_check(
                "csgo",
                "skills/csgo/a.skill",
                "xavi",
                true,
                "gain:0.1",
                "base",
                20,
            )
            .unwrap();
        let current = board.skills["csgo"].current.as_ref().unwrap();
        assert_eq!(
            (current.trained_by.as_str(), current.verified_by.as_str()),
            ("nelson", "xavi")
        );
        assert_eq!(board.credits["nelson"].promoted, 1);
        assert_eq!(board.credits["xavi"].verified, 1);
        let catalog = board.catalog("https://skills.example/main/");
        assert_eq!(catalog["format"], "archie-iris-skill-catalog-v1");
        assert_eq!(
            catalog["skills"]["csgo"]["url"],
            "https://skills.example/main/skills/csgo/a.skill"
        );
        assert_eq!(
            (
                catalog["skills"]["csgo"]["bytes"].as_u64(),
                catalog["skills"]["csgo"]["base"].as_str()
            ),
            (Some(13), Some("base"))
        );
    }

    #[test]
    fn a_turned_down_candidate_stays_on_the_board_with_its_reason() {
        let mut board = Board::empty("base");
        board
            .claim("kdenlive", "recorded", "sheriff", 2, 0)
            .unwrap();
        board
            .add_candidate("kdenlive", candidate("sheriff", "k"), 5)
            .unwrap();
        board
            .record_check(
                "kdenlive",
                "k",
                "nelson",
                false,
                "worse_on:pointer_within_5pct",
                "base",
                9,
            )
            .unwrap();
        let skill = &board.skills["kdenlive"];
        assert!(skill.current.is_none());
        assert_eq!(
            skill.candidates[0].state,
            "rejected:worse_on:pointer_within_5pct"
        );
        assert_eq!(board.credits["sheriff"].rejected, 1);
        assert_eq!(
            board.record_check("kdenlive", "k", "xavi", true, "", "base", 10),
            Err(BoardError::UnknownCandidate)
        );
    }

    #[test]
    fn the_work_list_puts_checks_for_others_first_and_leaves_out_what_others_hold() {
        let mut board = Board::empty("base");
        board.claim("csgo", "csgo", "sheriff", 2, 0).unwrap();
        board
            .add_candidate("csgo", candidate("sheriff", "c"), 1)
            .unwrap();
        board.claim("photopea", "recorded", "xavi", 2, 0).unwrap();
        board.skills.insert(
            "blender".into(),
            Skill {
                source: "recorded".into(),
                ..Skill::default()
            },
        );
        board.wanted.push(Wanted {
            skill: "rocket-league".into(),
            why: "Nelson plays it".into(),
            source: "recorded".into(),
        });
        let mine = board.needs("nelson", 100);
        let order: Vec<(&str, &str)> = mine
            .iter()
            .map(|n| (n.skill.as_str(), n.kind.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("csgo", "check"),
                ("rocket-league", "train"),
                ("blender", "train")
            ]
        );
        assert!(
            board
                .needs("sheriff", 100)
                .iter()
                .all(|n| n.kind != "check"),
            "nobody checks their own work"
        );
    }
}
