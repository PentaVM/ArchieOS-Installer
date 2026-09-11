//! Claims: what Archie believes, and how much it has earned that belief.
//!
//! Archie's source of truth is this store, not a model's weights. That only works if the
//! store is honest about how well established each thing is, which means two orthogonal
//! axes rather than one confidence number:
//!
//! * [`VerificationStage`] — how far a claim has come along the verification path. It
//!   only ever moves forward, one step at a time.
//! * [`ClaimFlag`] — qualifications that can attach at *any* stage. A canonical claim
//!   can be an approximation. A reproduced claim can be domain-limited. Folding these
//!   into the stage would force a choice between "well established" and "only valid
//!   below Mach 1", which are not alternatives.
//!
//! # User overrides never destroy provenance
//!
//! An operator working on a project frequently needs to assume something the canonical
//! record contradicts — a supplier's disputed figure, a hypothesis being explored, a
//! deliberately counterfactual design premise. Archie must honour that *within the
//! project* while the canonical record stays exactly as it was.
//!
//! So an override is an overlay, never an edit:
//!
//! ```text
//! canonical            X
//! project override     assume not-X
//! effective premise    not-X       (inside that project only)
//! canonical record     X           (unchanged, still cited, still verifiable)
//! ```
//!
//! [`Claim::apply_overrides`] takes `&self` and returns a view. There is no method on
//! this type that lets an override mutate a claim, and that omission is the design.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How far a claim has travelled along the verification path.
///
/// Ordered. Each variant is strictly stronger evidence than the one before, and
/// [`VerificationStage::can_advance_to`] permits only single forward steps: skipping
/// from ingestion to canonical is how an unexamined assertion acquires authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStage {
    /// Retrieved, nothing more.
    Ingested,
    /// Structure extracted from the raw source.
    Parsed,
    /// The source says this. Archie is not yet claiming anything itself.
    SourceAsserted,
    /// Internally coherent: units balance, no self-contradiction.
    InternallyConsistent,
    /// Independent sources support it.
    CrossSourceSupported,
    /// Someone other than the original reproduced the result.
    IndependentlyReproduced,
    /// Checked symbolically or by proof.
    FormallyVerified,
    /// Reproduced against measurement.
    EmpiricallyReproduced,
    /// Archie treats this as settled.
    Canonical,
}

impl VerificationStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ingested => "ingested",
            Self::Parsed => "parsed",
            Self::SourceAsserted => "source_asserted",
            Self::InternallyConsistent => "internally_consistent",
            Self::CrossSourceSupported => "cross_source_supported",
            Self::IndependentlyReproduced => "independently_reproduced",
            Self::FormallyVerified => "formally_verified",
            Self::EmpiricallyReproduced => "empirically_reproduced",
            Self::Canonical => "canonical",
        }
    }

    /// The ordered path.
    pub fn all() -> [VerificationStage; 9] {
        [
            Self::Ingested,
            Self::Parsed,
            Self::SourceAsserted,
            Self::InternallyConsistent,
            Self::CrossSourceSupported,
            Self::IndependentlyReproduced,
            Self::FormallyVerified,
            Self::EmpiricallyReproduced,
            Self::Canonical,
        ]
    }

    /// The next stage, if any.
    pub fn next(&self) -> Option<VerificationStage> {
        let all = Self::all();
        let index = all.iter().position(|s| s == self)?;
        all.get(index + 1).copied()
    }

    /// Whether a claim may move from here to `target`.
    ///
    /// Exactly one step forward. Verification is not something a claim can be granted in
    /// bulk: each stage requires evidence the previous one did not have, and jumping
    /// stages means that evidence was never gathered.
    pub fn can_advance_to(&self, target: VerificationStage) -> bool {
        self.next() == Some(target)
    }

    /// Whether this stage is strong enough to act on without qualification.
    pub fn is_established(&self) -> bool {
        *self >= Self::IndependentlyReproduced
    }

    /// Whether the claim is still only what a source said.
    pub fn is_merely_reported(&self) -> bool {
        *self <= Self::SourceAsserted
    }
}

/// Qualifications orthogonal to the verification stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimFlag {
    /// Credible sources disagree. Attaches at any stage, including canonical.
    Disputed,
    /// A later claim replaces this one. The claim is kept: a correction is only legible
    /// next to what it corrected.
    Superseded,
    /// The originator withdrew it.
    Retracted,
    /// Proposed, not established.
    Conjectural,
    /// True within a stated error bound, not exactly.
    Approximation,
    /// Valid only inside a stated domain.
    DomainLimited,
    /// An operator asserted this against the record.
    UserOverride,
    /// Archie generated it. Never enters the canonical record without verification.
    ArchieHypothesis,
}

impl ClaimFlag {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disputed => "disputed",
            Self::Superseded => "superseded",
            Self::Retracted => "retracted",
            Self::Conjectural => "conjectural",
            Self::Approximation => "approximation",
            Self::DomainLimited => "domain_limited",
            Self::UserOverride => "user_override",
            Self::ArchieHypothesis => "archie_hypothesis",
        }
    }

    /// Whether this flag means the claim must not be relied on without a human looking.
    pub fn requires_caution(&self) -> bool {
        matches!(
            self,
            Self::Disputed | Self::Retracted | Self::Conjectural | Self::ArchieHypothesis
        )
    }
}

/// How widely an override applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverrideScope {
    /// Applies only inside one project. The common and safe case.
    Project { project_id: String },
    /// Applies only for one working session.
    Session { session_id: String },
    /// Applies everywhere. Requires an authority and a rationale, and never rewrites the
    /// canonical claim.
    Global,
}

impl OverrideScope {
    pub fn applies_to(&self, context: &OverrideContext) -> bool {
        match self {
            Self::Project { project_id } => context.project_id.as_deref() == Some(project_id),
            Self::Session { session_id } => context.session_id.as_deref() == Some(session_id),
            Self::Global => true,
        }
    }
}

/// Where a premise is being evaluated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverrideContext {
    pub project_id: Option<String>,
    pub session_id: Option<String>,
}

impl OverrideContext {
    pub fn project(id: impl Into<String>) -> Self {
        Self {
            project_id: Some(id.into()),
            session_id: None,
        }
    }

    pub fn session(id: impl Into<String>) -> Self {
        Self {
            project_id: None,
            session_id: Some(id.into()),
        }
    }
}

/// An operator asserting something against the record, within a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserOverride {
    pub claim_id: String,
    pub scope: OverrideScope,
    /// What the operator wants assumed instead.
    pub asserts: String,
    /// Who decided. An override with no author cannot be reviewed later.
    pub authority: String,
    /// Why. Recorded because the reason is usually the only thing that makes the
    /// override intelligible six months on.
    pub rationale: String,
    pub created_at_ms: i64,
}

impl UserOverride {
    pub fn validate(&self) -> Result<(), String> {
        if self.claim_id.trim().is_empty() {
            return Err("an override must name the claim it overrides".into());
        }
        if self.asserts.trim().is_empty() {
            return Err("an override must state what to assume instead".into());
        }
        if self.authority.trim().is_empty() {
            return Err("an override must record who decided it".into());
        }
        // A global override changes what Archie assumes everywhere. Requiring a reason is
        // the cheapest possible guard against it being done casually.
        if matches!(self.scope, OverrideScope::Global) && self.rationale.trim().is_empty() {
            return Err("a global override must record why".into());
        }
        Ok(())
    }
}

/// Something Archie believes, with the evidence and the qualifications attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    /// The assertion itself, in plain language.
    pub statement: String,
    pub stage: VerificationStage,
    pub flags: BTreeSet<ClaimFlag>,
    /// Observations, events, simulation runs and sources supporting it.
    pub evidence_refs: Vec<String>,
    /// Independent sources behind it. Separate from the evidence count, for the same
    /// reason events track it separately: twelve republications are one source.
    pub independent_sources: usize,
    /// The claim that replaced this one, when superseded.
    pub superseded_by: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Claim {
    pub fn new(id: impl Into<String>, statement: impl Into<String>, now_ms: i64) -> Self {
        Self {
            id: id.into(),
            statement: statement.into(),
            stage: VerificationStage::Ingested,
            flags: BTreeSet::new(),
            evidence_refs: Vec::new(),
            independent_sources: 0,
            superseded_by: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    /// Move one step along the verification path.
    ///
    /// Refuses a skip, and refuses to reach `CrossSourceSupported` without the
    /// independent sources that stage is named for — otherwise the stage becomes a label
    /// somebody applied rather than a fact about the evidence.
    pub fn advance_to(&mut self, target: VerificationStage, now_ms: i64) -> Result<(), String> {
        if !self.stage.can_advance_to(target) {
            return Err(format!(
                "cannot advance {} from {} to {}: verification proceeds one stage at a time",
                self.id,
                self.stage.as_str(),
                target.as_str()
            ));
        }
        if target == VerificationStage::CrossSourceSupported && self.independent_sources < 2 {
            return Err(format!(
                "claim {} cannot be cross-source supported with {} independent source(s)",
                self.id, self.independent_sources
            ));
        }
        if self.flags.contains(&ClaimFlag::Retracted) {
            return Err(format!(
                "claim {} was retracted by its originator and cannot be advanced",
                self.id
            ));
        }
        self.stage = target;
        self.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn flag(&mut self, flag: ClaimFlag, now_ms: i64) {
        self.flags.insert(flag);
        self.updated_at_ms = now_ms;
    }

    /// Mark this claim replaced, keeping it in the store.
    pub fn supersede(&mut self, replacement: impl Into<String>, now_ms: i64) {
        self.superseded_by = Some(replacement.into());
        self.flags.insert(ClaimFlag::Superseded);
        self.updated_at_ms = now_ms;
    }

    /// Whether this may be treated as settled.
    ///
    /// Stage alone is not enough: a canonical claim that is disputed or retracted is not
    /// something to act on without a human looking at it.
    pub fn is_actionable(&self) -> bool {
        self.stage.is_established() && !self.flags.iter().any(ClaimFlag::requires_caution)
    }

    /// Why this claim is not actionable, for display.
    pub fn caution_reasons(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if !self.stage.is_established() {
            reasons.push("not independently reproduced");
        }
        for flag in &self.flags {
            if flag.requires_caution() {
                reasons.push(flag.as_str());
            }
        }
        reasons
    }

    /// Resolve what to assume here, given any overrides.
    ///
    /// Takes `&self`. An override produces a *view*; it never edits the claim, so the
    /// canonical record survives intact and can still be cited against the override.
    pub fn apply_overrides<'a>(
        &self,
        overrides: impl IntoIterator<Item = &'a UserOverride>,
        context: &OverrideContext,
    ) -> PremiseView {
        // Narrowest applicable scope wins: a project override beats a global one inside
        // that project, because the person who set it was closer to the problem.
        let mut chosen: Option<&UserOverride> = None;
        for candidate in overrides {
            if candidate.claim_id != self.id || !candidate.scope.applies_to(context) {
                continue;
            }
            let better = match chosen {
                None => true,
                Some(current) => scope_rank(&candidate.scope) < scope_rank(&current.scope),
            };
            if better {
                chosen = Some(candidate);
            }
        }

        match chosen {
            Some(applied) => PremiseView {
                claim_id: self.id.clone(),
                canonical: self.statement.clone(),
                effective: applied.asserts.clone(),
                canonical_stage: self.stage,
                overridden: true,
                override_authority: Some(applied.authority.clone()),
                override_rationale: Some(applied.rationale.clone()),
            },
            None => PremiseView {
                claim_id: self.id.clone(),
                canonical: self.statement.clone(),
                effective: self.statement.clone(),
                canonical_stage: self.stage,
                overridden: false,
                override_authority: None,
                override_rationale: None,
            },
        }
    }
}

fn scope_rank(scope: &OverrideScope) -> u8 {
    match scope {
        OverrideScope::Session { .. } => 0,
        OverrideScope::Project { .. } => 1,
        OverrideScope::Global => 2,
    }
}

/// What to assume in a given context, and what the record actually says.
///
/// Both are always present. A view that showed only the effective premise would let an
/// override quietly become indistinguishable from the truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremiseView {
    pub claim_id: String,
    /// What the canonical record says, always.
    pub canonical: String,
    /// What to work from here.
    pub effective: String,
    pub canonical_stage: VerificationStage,
    pub overridden: bool,
    pub override_authority: Option<String>,
    pub override_rationale: Option<String>,
}

impl PremiseView {
    /// Whether the effective premise departs from the record.
    pub fn contradicts_record(&self) -> bool {
        self.overridden && self.canonical != self.effective
    }

    /// A line for the display, which must never hide that an override is in force.
    pub fn explain(&self) -> String {
        if !self.overridden {
            return format!("{} ({})", self.canonical, self.canonical_stage.as_str());
        }
        format!(
            "working premise: {} — OVERRIDE by {}; canonical record still says: {} ({})",
            self.effective,
            self.override_authority.as_deref().unwrap_or("unknown"),
            self.canonical,
            self.canonical_stage.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn claim() -> Claim {
        Claim::new("claim-1", "the drag coefficient is 0.31", NOW)
    }

    // ────────────────────────────────────────────────────────── the stage path

    #[test]
    fn the_path_is_ordered_from_weakest_to_strongest() {
        let all = VerificationStage::all();
        for pair in all.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} should precede {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(all[0], VerificationStage::Ingested);
        assert_eq!(all[8], VerificationStage::Canonical);
    }

    #[test]
    fn verification_advances_one_stage_at_a_time() {
        let mut c = claim();
        assert!(c.advance_to(VerificationStage::Parsed, NOW).is_ok());
        assert!(c.advance_to(VerificationStage::SourceAsserted, NOW).is_ok());
        assert_eq!(c.stage, VerificationStage::SourceAsserted);
    }

    #[test]
    fn a_claim_cannot_skip_to_canonical() {
        // The failure this guards: an unexamined assertion acquiring authority because
        // someone set a field.
        let mut c = claim();
        let error = c.advance_to(VerificationStage::Canonical, NOW).unwrap_err();
        assert!(error.contains("one stage at a time"));
        assert_eq!(c.stage, VerificationStage::Ingested, "unchanged");
    }

    #[test]
    fn verification_never_runs_backwards() {
        let mut c = claim();
        c.advance_to(VerificationStage::Parsed, NOW).unwrap();
        assert!(c.advance_to(VerificationStage::Ingested, NOW).is_err());
    }

    #[test]
    fn cross_source_support_requires_actual_independent_sources() {
        // Otherwise the stage is a label somebody applied, not a fact about evidence.
        let mut c = claim();
        c.advance_to(VerificationStage::Parsed, NOW).unwrap();
        c.advance_to(VerificationStage::SourceAsserted, NOW)
            .unwrap();
        c.advance_to(VerificationStage::InternallyConsistent, NOW)
            .unwrap();

        let error = c
            .advance_to(VerificationStage::CrossSourceSupported, NOW)
            .unwrap_err();
        assert!(error.contains("independent source"));

        c.independent_sources = 2;
        assert!(c
            .advance_to(VerificationStage::CrossSourceSupported, NOW)
            .is_ok());
    }

    #[test]
    fn a_retracted_claim_cannot_be_advanced() {
        let mut c = claim();
        c.flag(ClaimFlag::Retracted, NOW);
        assert!(c.advance_to(VerificationStage::Parsed, NOW).is_err());
    }

    // ──────────────────────────────────────────────────────── orthogonal flags

    #[test]
    fn a_canonical_claim_can_still_be_an_approximation() {
        // The reason flags are orthogonal to stages: "well established" and "true within
        // a stated error bound" are not alternatives.
        let mut c = claim();
        c.stage = VerificationStage::Canonical;
        c.flag(ClaimFlag::Approximation, NOW);
        c.flag(ClaimFlag::DomainLimited, NOW);

        assert!(c.stage.is_established());
        assert!(
            c.is_actionable(),
            "an approximation with a stated domain is still usable"
        );
    }

    #[test]
    fn a_canonical_claim_that_is_disputed_is_not_actionable() {
        let mut c = claim();
        c.stage = VerificationStage::Canonical;
        c.flag(ClaimFlag::Disputed, NOW);
        assert!(!c.is_actionable());
        assert!(c.caution_reasons().contains(&"disputed"));
    }

    #[test]
    fn an_archie_hypothesis_is_never_actionable_on_its_own() {
        let mut c = claim();
        c.stage = VerificationStage::Canonical;
        c.flag(ClaimFlag::ArchieHypothesis, NOW);
        assert!(!c.is_actionable());
    }

    #[test]
    fn an_unestablished_claim_says_so() {
        let c = claim();
        assert!(!c.is_actionable());
        assert!(c
            .caution_reasons()
            .contains(&"not independently reproduced"));
        assert!(c.stage.is_merely_reported());
    }

    #[test]
    fn superseding_keeps_the_original() {
        // A correction is only legible next to what it corrected.
        let mut c = claim();
        c.supersede("claim-2", NOW + 1);
        assert_eq!(c.superseded_by.as_deref(), Some("claim-2"));
        assert!(c.flags.contains(&ClaimFlag::Superseded));
        assert_eq!(
            c.statement, "the drag coefficient is 0.31",
            "the original statement survives"
        );
    }

    // ─────────────────────────────────────────────────────────────── overrides

    fn override_for(scope: OverrideScope, asserts: &str) -> UserOverride {
        UserOverride {
            claim_id: "claim-1".into(),
            scope,
            asserts: asserts.into(),
            authority: "operator".into(),
            rationale: "supplier data sheet disagrees with the literature".into(),
            created_at_ms: NOW,
        }
    }

    #[test]
    fn with_no_override_the_effective_premise_is_the_record() {
        let view = claim().apply_overrides([], &OverrideContext::default());
        assert!(!view.overridden);
        assert_eq!(view.effective, view.canonical);
        assert!(!view.contradicts_record());
    }

    #[test]
    fn a_project_override_changes_the_premise_but_not_the_record() {
        // THE case this module exists for.
        let c = claim();
        let o = override_for(
            OverrideScope::Project {
                project_id: "suit-v2".into(),
            },
            "assume the drag coefficient is 0.28",
        );

        let view = c.apply_overrides([&o], &OverrideContext::project("suit-v2"));
        assert!(view.overridden);
        assert_eq!(view.effective, "assume the drag coefficient is 0.28");
        assert_eq!(
            view.canonical, "the drag coefficient is 0.31",
            "the canonical record is untouched"
        );
        assert!(view.contradicts_record());

        // And the claim itself was not mutated.
        assert_eq!(c.statement, "the drag coefficient is 0.31");
        assert!(!c.flags.contains(&ClaimFlag::UserOverride));
    }

    #[test]
    fn an_override_does_not_leak_into_another_project() {
        let c = claim();
        let o = override_for(
            OverrideScope::Project {
                project_id: "suit-v2".into(),
            },
            "assume 0.28",
        );
        let view = c.apply_overrides([&o], &OverrideContext::project("drone-v1"));
        assert!(!view.overridden);
        assert_eq!(view.effective, "the drag coefficient is 0.31");
    }

    #[test]
    fn a_session_override_is_narrower_than_a_project_one() {
        let c = claim();
        let project = override_for(
            OverrideScope::Project {
                project_id: "p".into(),
            },
            "project premise",
        );
        let session = override_for(
            OverrideScope::Session {
                session_id: "s".into(),
            },
            "session premise",
        );
        let context = OverrideContext {
            project_id: Some("p".into()),
            session_id: Some("s".into()),
        };
        let view = c.apply_overrides([&project, &session], &context);
        assert_eq!(
            view.effective, "session premise",
            "the narrowest applicable scope wins"
        );
    }

    #[test]
    fn a_project_override_beats_a_global_one_inside_that_project() {
        let c = claim();
        let global = override_for(OverrideScope::Global, "global premise");
        let project = override_for(
            OverrideScope::Project {
                project_id: "p".into(),
            },
            "project premise",
        );
        let view = c.apply_overrides([&global, &project], &OverrideContext::project("p"));
        assert_eq!(view.effective, "project premise");
    }

    #[test]
    fn a_global_override_applies_where_nothing_narrower_does() {
        let c = claim();
        let global = override_for(OverrideScope::Global, "global premise");
        let view = c.apply_overrides([&global], &OverrideContext::project("anything"));
        assert!(view.overridden);
        assert_eq!(view.effective, "global premise");
    }

    #[test]
    fn an_override_for_a_different_claim_is_ignored() {
        let c = claim();
        let mut other = override_for(OverrideScope::Global, "irrelevant");
        other.claim_id = "claim-999".into();
        assert!(
            !c.apply_overrides([&other], &OverrideContext::default())
                .overridden
        );
    }

    #[test]
    fn the_display_line_never_hides_an_override() {
        let c = claim();
        let o = override_for(OverrideScope::Global, "assume 0.28");
        let explanation = c
            .apply_overrides([&o], &OverrideContext::default())
            .explain();
        assert!(explanation.contains("OVERRIDE"));
        assert!(explanation.contains("operator"));
        assert!(
            explanation.contains("0.31"),
            "the canonical value must remain visible: {explanation}"
        );
    }

    #[test]
    fn an_override_must_record_who_decided_and_what_to_assume() {
        let mut o = override_for(OverrideScope::Global, "x");
        assert!(o.validate().is_ok());

        o.authority = "  ".into();
        assert!(o.validate().is_err(), "an override needs an author");

        let mut no_assertion = override_for(OverrideScope::Global, "");
        assert!(no_assertion.validate().is_err());
        no_assertion.asserts = "something".into();
        assert!(no_assertion.validate().is_ok());
    }

    #[test]
    fn a_global_override_must_record_why() {
        let mut o = override_for(OverrideScope::Global, "x");
        o.rationale = String::new();
        assert!(
            o.validate().is_err(),
            "changing what Archie assumes everywhere needs a reason"
        );

        // A project override is a narrower act and does not.
        let mut scoped = override_for(
            OverrideScope::Project {
                project_id: "p".into(),
            },
            "x",
        );
        scoped.rationale = String::new();
        assert!(scoped.validate().is_ok());
    }

    #[test]
    fn a_claim_round_trips_through_json() {
        let mut c = claim();
        c.flag(ClaimFlag::Approximation, NOW);
        c.evidence_refs.push("obs-1".into());
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Claim>(&json).unwrap(), c);
    }

    #[test]
    fn a_premise_view_round_trips_through_json() {
        let c = claim();
        let o = override_for(OverrideScope::Global, "assume 0.28");
        let view = c.apply_overrides([&o], &OverrideContext::default());
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<PremiseView>(&json).unwrap(), view);
    }
}
