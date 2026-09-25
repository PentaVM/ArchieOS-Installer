//! What IRIS proposes, and what the stage does about it.
//!
//! IRIS — the vision and control model trained in `archie-iris` — answers a goal with an
//! `archie-iris-stage-proposal-v1`: one action on one surface, the state it saw, the
//! model that produced it, and when it stops being worth doing. It carries
//! `"authority": "proposal_only"` because that is all it is. This crate is the ArchieOS
//! half of that contract (the model half is `archieos_adapter.py` and
//! `integrations/archieos/iris-stage-proposal.schema.json` in `archie-iris`):
//!
//! 1. [`IrisStageProposal::parse`] reads the JSON exactly as the schema says — the constant
//!    fields, the bounds, no field the schema does not name.
//! 2. [`Bridge::admit`] checks it against the moment it is acted on: a model that was
//!    admitted by hash, a proposal not already acted on, not expired, about the state that
//!    is current, about a surface that is still where and what it was, inside it, while
//!    the stage is Archie's to act on. Any failure is a [`Refusal`] with a code IRIS can
//!    learn from.
//! 3. What passes becomes an [`ActRequest`] for an agent principal — never a person — which
//!    the stage gate, a single-use policy lease and the compositor still check again.
//! 4. [`receipt`] turns the stage's outcome into the `ExecutionReceipt` IRIS expects. An
//!    action is `verified` only when a later observation says so; a receipt is not proof.

use archie_policy::Principal;
use archie_stage::{ActOutcome, ActRequest, PointerButton, StageAction, StageWorld};
use archie_types::{AgentId, PrivacyGrade};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

pub const FORMAT: &str = "archie-iris-stage-proposal-v1";
pub const AUTHORITY: &str = "proposal_only";
/// How many proposals are remembered to refuse a replay.
const REMEMBERED: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceRef {
    pub app_id: String,
    pub surface: u32,
    pub width: u32,
    pub height: u32,
    pub geometry_revision: u64,
    pub observed_state_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedAction {
    pub kind: String,
    pub x: u32,
    pub y: u32,
    pub button: String,
}

/// One proposal, as IRIS sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisStageProposal {
    pub format: String,
    pub request_id: String,
    pub objective: String,
    pub surface: SurfaceRef,
    pub action: ProposedAction,
    pub confidence_permille: u32,
    pub model_sha256: String,
    pub created_monotonic_ms: u64,
    pub expires_monotonic_ms: u64,
    pub authority: String,
}

/// Why a proposal was not acted on. `code` is stable, for IRIS to learn from.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct Refusal {
    pub code: &'static str,
    pub detail: String,
}

fn refuse(code: &'static str, detail: impl Into<String>) -> Refusal {
    Refusal {
        code,
        detail: detail.into(),
    }
}

fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl IrisStageProposal {
    /// Reads and checks a proposal against the schema. Nothing about the moment yet.
    pub fn parse(json: &str) -> Result<Self, Refusal> {
        let proposal: Self =
            serde_json::from_str(json).map_err(|e| refuse("proposal_malformed", e.to_string()))?;
        proposal.check_shape()?;
        Ok(proposal)
    }

    fn check_shape(&self) -> Result<(), Refusal> {
        if self.format != FORMAT {
            return Err(refuse("proposal_format_invalid", self.format.clone()));
        }
        if self.authority != AUTHORITY {
            // A proposal that claims any authority is not a proposal this contract knows.
            return Err(refuse("proposal_claims_authority", self.authority.clone()));
        }
        let bounded = |text: &str, most: usize| {
            !text.is_empty() && text.chars().count() <= most && !text.contains('\0')
        };
        if !bounded(&self.request_id, 128)
            || !bounded(&self.objective, 4096)
            || !bounded(&self.surface.app_id, 255)
        {
            return Err(refuse(
                "proposal_identity_invalid",
                "request, objective or app id out of bounds",
            ));
        }
        if !is_hash(&self.model_sha256) {
            return Err(refuse(
                "proposal_model_hash_invalid",
                "model_sha256 is 64 lowercase hex digits",
            ));
        }
        let surface = &self.surface;
        if surface.surface == 0
            || surface.width == 0
            || surface.height == 0
            || surface.geometry_revision == 0
            || surface.observed_state_version == 0
        {
            return Err(refuse(
                "proposal_surface_invalid",
                "surface fields start at 1",
            ));
        }
        if self.confidence_permille > 1000 {
            return Err(refuse(
                "proposal_confidence_invalid",
                self.confidence_permille.to_string(),
            ));
        }
        if self.expires_monotonic_ms == 0 || self.expires_monotonic_ms <= self.created_monotonic_ms
        {
            return Err(refuse(
                "proposal_expiry_invalid",
                "expires before it was created",
            ));
        }
        if self.action.kind != "click" || self.action.button != "left" {
            return Err(refuse(
                "proposal_action_unsupported",
                format!("{} {}", self.action.kind, self.action.button),
            ));
        }
        if self.action.x >= surface.width || self.action.y >= surface.height {
            return Err(refuse(
                "proposal_out_of_bounds",
                "the point is outside the surface it names",
            ));
        }
        Ok(())
    }
}

/// What ArchieOS knows at the moment a proposal is weighed.
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    /// The monotonic clock both sides read.
    pub now_monotonic_ms: u64,
    /// The observation version the scene is at now.
    pub state_version: u64,
}

/// Admits IRIS proposals for one agent.
#[derive(Debug)]
pub struct Bridge {
    models: BTreeSet<String>,
    /// Below this, a proposal is not worth a lease.
    min_confidence_permille: u32,
    principal: Principal,
    seen: VecDeque<(String, u64)>,
}

impl Bridge {
    /// A bridge that accepts proposals from the models whose checkpoints hash to
    /// `models`, and no other.
    pub fn new(models: impl IntoIterator<Item = String>, min_confidence_permille: u32) -> Self {
        Self {
            models: models.into_iter().filter(|hash| is_hash(hash)).collect(),
            min_confidence_permille: min_confidence_permille.min(1000),
            principal: Principal::Agent {
                agent: AgentId::new(),
            },
            seen: VecDeque::new(),
        }
    }

    /// Checks `proposal` against this moment and the stage as last seen, and turns it into
    /// a request the stage gate will judge again. A proposal is acted on at most once.
    pub fn admit(
        &mut self,
        proposal: &IrisStageProposal,
        world: &StageWorld,
        moment: Moment,
    ) -> Result<ActRequest, Refusal> {
        proposal.check_shape()?;
        if !self.models.contains(&proposal.model_sha256) {
            return Err(refuse("model_not_admitted", proposal.model_sha256.clone()));
        }
        let key = (proposal.request_id.clone(), proposal.created_monotonic_ms);
        if self.seen.contains(&key) {
            return Err(refuse("proposal_replayed", proposal.request_id.clone()));
        }
        if !(proposal.created_monotonic_ms <= moment.now_monotonic_ms
            && moment.now_monotonic_ms < proposal.expires_monotonic_ms)
        {
            return Err(refuse("proposal_expired", "outside its window"));
        }
        if proposal.surface.observed_state_version != moment.state_version {
            return Err(refuse(
                "proposal_stale_state",
                format!(
                    "saw {}, now {}",
                    proposal.surface.observed_state_version, moment.state_version
                ),
            ));
        }
        if proposal.confidence_permille < self.min_confidence_permille {
            return Err(refuse(
                "proposal_not_confident",
                proposal.confidence_permille.to_string(),
            ));
        }
        if !world.agent_may_act() {
            return Err(refuse(
                "stage_not_autonomous",
                "a person holds the stage, or it is paused",
            ));
        }
        let Some(live) = world.surface(proposal.surface.surface) else {
            return Err(refuse("surface_gone", proposal.surface.surface.to_string()));
        };
        let wanted = &proposal.surface;
        if live.app_id != wanted.app_id
            || live.geometry_revision != wanted.geometry_revision
            || live.width != wanted.width as i32
            || live.height != wanted.height as i32
        {
            return Err(refuse(
                "surface_changed",
                "the window is not the one IRIS looked at",
            ));
        }
        self.seen.push_back(key);
        while self.seen.len() > REMEMBERED {
            self.seen.pop_front();
        }
        Ok(ActRequest {
            app_id: wanted.app_id.clone(),
            surface: Some(wanted.surface),
            action: StageAction::Click {
                x: proposal.action.x as i32,
                y: proposal.action.y as i32,
                button: PointerButton::Left,
            },
            text_grade: PrivacyGrade::Public,
            principal: self.principal.clone(),
            task_id: None,
            user_present: true,
            approved: false,
            justification: format!(
                "IRIS proposal {} from model {}",
                proposal.request_id,
                &proposal.model_sha256[..12]
            ),
        })
    }
}

/// The `ExecutionReceipt` IRIS expects, for a proposal and what became of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionReceipt {
    pub request_id: String,
    pub sequence: u64,
    pub proposal_model_sha256: String,
    pub observed_state_version: u64,
    pub outcome: String,
    pub verified: bool,
    pub completed_actions: u32,
    pub detail: String,
}

/// What became of `proposal`: refused here, or the stage's own outcome. `verified` says
/// whether a later observation confirmed the effect — the caller's to know, never assumed.
pub fn receipt(
    proposal: &IrisStageProposal,
    result: &Result<ActOutcome, Refusal>,
    verified: bool,
) -> ExecutionReceipt {
    let (outcome, detail, sequence, done) = match result {
        Err(refusal) => (refusal.code.to_string(), refusal.detail.clone(), 0, false),
        Ok(ActOutcome::Receipt {
            sequence, outcome, ..
        }) => (
            outcome.clone(),
            format!("stage receipt {sequence}"),
            *sequence,
            outcome == "executed",
        ),
        Ok(ActOutcome::NeedsApproval { reason }) => {
            ("needs_approval".into(), reason.clone(), 0, false)
        }
        Ok(ActOutcome::Refused { code, message }) => (code.to_string(), message.clone(), 0, false),
        Ok(ActOutcome::NoSurface) => (
            "surface_gone".into(),
            "no surface on the stage".into(),
            0,
            false,
        ),
    };
    let mut detail = detail;
    if detail.is_empty() {
        detail = outcome.clone();
    }
    ExecutionReceipt {
        request_id: proposal.request_id.clone(),
        sequence,
        proposal_model_sha256: proposal.model_sha256.clone(),
        observed_state_version: proposal.surface.observed_state_version,
        outcome,
        verified: done && verified,
        completed_actions: u32::from(done),
        detail: detail.chars().take(512).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_stage::{AuthorityMode, StageEvent};

    const MODEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// The proposal IRIS's own adapter test builds: surface 7 of lab.app, 128x96, revision
    /// 12, state 44, a click at (30, 40), created at 1000 ms with a 250 ms life.
    fn proposal() -> IrisStageProposal {
        IrisStageProposal::parse(
            &serde_json::json!({
                "format": FORMAT, "request_id": "r1", "objective": "activate",
                "surface": {"app_id": "lab.app", "surface": 7, "width": 128, "height": 96,
                            "geometry_revision": 12, "observed_state_version": 44},
                "action": {"kind": "click", "x": 30, "y": 40, "button": "left"},
                "confidence_permille": 900, "model_sha256": MODEL,
                "created_monotonic_ms": 1000, "expires_monotonic_ms": 1250, "authority": AUTHORITY
            })
            .to_string(),
        )
        .unwrap()
    }

    fn world() -> StageWorld {
        let mut world = StageWorld::default();
        world.apply(&StageEvent::Surface {
            surface: 7,
            app_id: "lab.app".into(),
            x: 0,
            y: 0,
            width: 128,
            height: 96,
            geometry_revision: 12,
            pid: 0,
            parent: 0,
            popup: false,
        });
        world.apply(&StageEvent::Authority {
            mode: AuthorityMode::Autonomous,
            reason: String::new(),
        });
        world
    }

    fn now(ms: u64, state: u64) -> Moment {
        Moment {
            now_monotonic_ms: ms,
            state_version: state,
        }
    }

    #[test]
    fn a_fresh_proposal_from_an_admitted_model_becomes_an_agents_click_once() {
        let mut bridge = Bridge::new([MODEL.to_string()], 500);
        let request = bridge.admit(&proposal(), &world(), now(1100, 44)).unwrap();
        assert_eq!(request.app_id, "lab.app");
        assert_eq!(request.surface, Some(7));
        assert!(matches!(
            request.action,
            StageAction::Click {
                x: 30,
                y: 40,
                button: PointerButton::Left
            }
        ));
        assert!(
            matches!(request.principal, Principal::Agent { .. }),
            "never a person"
        );
        assert!(!request.approved, "IRIS approves nothing");
        let again = bridge.admit(&proposal(), &world(), now(1100, 44));
        assert_eq!(again.unwrap_err().code, "proposal_replayed");
    }

    #[test]
    fn stale_expired_or_unadmitted_proposals_are_refused_with_codes() {
        let mut bridge = Bridge::new([MODEL.to_string()], 500);
        let code = |bridge: &mut Bridge, p: &IrisStageProposal, w: &StageWorld, m: Moment| {
            bridge.admit(p, w, m).unwrap_err().code
        };
        assert_eq!(
            code(&mut bridge, &proposal(), &world(), now(1100, 45)),
            "proposal_stale_state"
        );
        assert_eq!(
            code(&mut bridge, &proposal(), &world(), now(1250, 44)),
            "proposal_expired"
        );
        assert_eq!(
            code(&mut bridge, &proposal(), &world(), now(999, 44)),
            "proposal_expired"
        );

        let mut stranger = Bridge::new(["b".repeat(64)], 500);
        assert_eq!(
            code(&mut stranger, &proposal(), &world(), now(1100, 44)),
            "model_not_admitted"
        );

        let mut shy = Bridge::new([MODEL.to_string()], 950);
        assert_eq!(
            code(&mut shy, &proposal(), &world(), now(1100, 44)),
            "proposal_not_confident"
        );
    }

    #[test]
    fn a_window_that_moved_or_a_person_at_the_controls_stops_it() {
        let mut bridge = Bridge::new([MODEL.to_string()], 0);
        let mut moved = world();
        moved.apply(&StageEvent::Surface {
            surface: 7,
            app_id: "lab.app".into(),
            x: 5,
            y: 5,
            width: 128,
            height: 96,
            geometry_revision: 13,
            pid: 0,
            parent: 0,
            popup: false,
        });
        assert_eq!(
            bridge
                .admit(&proposal(), &moved, now(1100, 44))
                .unwrap_err()
                .code,
            "surface_changed"
        );

        let mut held = world();
        held.apply(&StageEvent::Authority {
            mode: AuthorityMode::Operator,
            reason: "mouse".into(),
        });
        assert_eq!(
            bridge
                .admit(&proposal(), &held, now(1100, 44))
                .unwrap_err()
                .code,
            "stage_not_autonomous"
        );

        let empty = StageWorld::default();
        let mut free = empty.clone();
        free.apply(&StageEvent::Authority {
            mode: AuthorityMode::Autonomous,
            reason: String::new(),
        });
        assert_eq!(
            bridge
                .admit(&proposal(), &free, now(1100, 44))
                .unwrap_err()
                .code,
            "surface_gone"
        );
    }

    #[test]
    fn the_schema_is_read_strictly() {
        let base = serde_json::to_value(proposal()).unwrap();
        let broken = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut value = base.clone();
            edit(&mut value);
            IrisStageProposal::parse(&value.to_string())
                .unwrap_err()
                .code
        };
        assert_eq!(
            broken(&|v| v["authority"] = "execute".into()),
            "proposal_claims_authority"
        );
        assert_eq!(
            broken(&|v| v["format"] = "v2".into()),
            "proposal_format_invalid"
        );
        assert_eq!(
            broken(&|v| v["action"]["kind"] = "type".into()),
            "proposal_action_unsupported"
        );
        assert_eq!(
            broken(&|v| v["action"]["x"] = 128.into()),
            "proposal_out_of_bounds"
        );
        assert_eq!(
            broken(&|v| v["model_sha256"] = "ABC".into()),
            "proposal_model_hash_invalid"
        );
        assert_eq!(
            broken(&|v| v["expires_monotonic_ms"] = 900.into()),
            "proposal_expiry_invalid"
        );
        assert_eq!(broken(&|v| v["extra"] = 1.into()), "proposal_malformed");
        assert_eq!(
            broken(&|v| v["action"]["x"] = (-1).into()),
            "proposal_malformed"
        );
    }

    #[test]
    fn a_receipt_says_what_happened_and_verifies_only_what_was_seen() {
        let p = proposal();
        let executed = Ok(ActOutcome::Receipt {
            sequence: 9,
            outcome: "executed".into(),
            finished: 0,
        });
        let seen = receipt(&p, &executed, true);
        assert_eq!(
            (seen.sequence, seen.completed_actions, seen.verified),
            (9, 1, true)
        );
        assert_eq!(seen.observed_state_version, 44);
        let unseen = receipt(&p, &executed, false);
        assert!(!unseen.verified, "a receipt alone is not proof");
        let taken = Ok(ActOutcome::Receipt {
            sequence: 10,
            outcome: "operator_has_control".into(),
            finished: 0,
        });
        let refused = receipt(&p, &taken, true);
        assert_eq!((refused.completed_actions, refused.verified), (0, false));
        let stale = receipt(
            &p,
            &Err(refuse("proposal_stale_state", "saw 44, now 45")),
            true,
        );
        assert_eq!(stale.outcome, "proposal_stale_state");
        assert!(!stale.verified);
    }
}
