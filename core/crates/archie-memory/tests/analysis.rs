use archie_memory::{
    assess_hypothesis, gate_model_proposal, AnalysisError, AnalysisPolicy, AnalyticalSignal,
    AssessmentState, EvidenceStance, ModelProposal, ProposalDisposition, ProposalError,
    ProposalReason, SignalReason,
};

const NOW: i64 = 1_800_000_000_000;
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn signal(id: &str, source: &str, stance: EvidenceStance) -> AnalyticalSignal {
    AnalyticalSignal {
        signal_id: format!("signal:{id}"),
        hypothesis_id: "hypothesis:port-event".into(),
        evidence_ref: format!("evidence:{id}"),
        evidence_sha256: DIGEST.into(),
        source_root: format!("source:{source}"),
        semantic_key: "event:port:activity".into(),
        stance,
        reliability_permille: 900,
        relevance_permille: 900,
        observed_at_ms: NOW - 100,
        expires_at_ms: Some(NOW + 10_000),
        machine_analysis_allowed: true,
        consent_required: false,
        consent_granted: false,
    }
}

fn assess(signals: &[AnalyticalSignal]) -> archie_memory::AnalyticalAssessment {
    assess_hypothesis(
        "hypothesis:port-event",
        signals,
        &AnalysisPolicy::default(),
        NOW,
    )
    .unwrap()
}

fn proposal(assessment: &archie_memory::AnalyticalAssessment) -> ModelProposal {
    ModelProposal {
        proposal_id: "proposal:one".into(),
        hypothesis_id: assessment.hypothesis_id().to_string(),
        model_id: "model:analyst-v1".into(),
        model_sha256: DIGEST.into(),
        stance: EvidenceStance::Supports,
        score_permille: 950,
        evidence_refs: assessment.evidence_refs().to_vec(),
        produced_at_ms: NOW,
    }
}

#[test]
fn repetition_from_one_source_never_becomes_corroboration() {
    let inputs: Vec<_> = (0..100)
        .map(|index| {
            signal(
                &format!("repeat-{index}"),
                "camera-one",
                EvidenceStance::Supports,
            )
        })
        .collect();
    let result = assess(&inputs);
    assert_eq!(result.state(), AssessmentState::InsufficientEvidence);
    assert_eq!(result.independent_support_sources(), 1);
    assert_eq!(
        result
            .decisions()
            .iter()
            .filter(|decision| decision.reason == SignalReason::DuplicateSource)
            .count(),
        99
    );
}

#[test]
fn independent_sources_can_support_or_refute_a_hypothesis() {
    let supported = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    assert_eq!(supported.state(), AssessmentState::Supported);
    assert_eq!(supported.independent_sources(), 2);

    let refuted = assess(&[
        signal("a", "camera", EvidenceStance::Contradicts),
        signal("b", "operator", EvidenceStance::Contradicts),
    ]);
    assert_eq!(refuted.state(), AssessmentState::Refuted);
    assert_eq!(refuted.independent_contradiction_sources(), 2);
}

#[test]
fn disagreement_is_preserved_instead_of_averaged_away() {
    let result = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
        signal("c", "public-record", EvidenceStance::Contradicts),
    ]);
    assert_eq!(result.state(), AssessmentState::Disputed);
    assert!(result.requires_human_review());
    assert_eq!(result.independent_support_sources(), 2);
    assert_eq!(result.independent_contradiction_sources(), 1);
}

#[test]
fn a_source_that_contradicts_itself_is_quarantined() {
    let result = assess(&[
        signal("yes", "same-feed", EvidenceStance::Supports),
        signal("no", "same-feed", EvidenceStance::Contradicts),
    ]);
    assert_eq!(result.state(), AssessmentState::Disputed);
    assert_eq!(result.conflicted_sources(), 1);
    assert_eq!(result.independent_sources(), 0);
    assert!(result.evidence_refs().is_empty());
    assert!(result
        .decisions()
        .iter()
        .all(|decision| decision.reason == SignalReason::SourceContradiction));
}

#[test]
fn permissions_quality_and_age_fail_closed() {
    let mut display_only = signal("display", "a", EvidenceStance::Supports);
    display_only.machine_analysis_allowed = false;
    let mut no_consent = signal("private", "b", EvidenceStance::Supports);
    no_consent.consent_required = true;
    let mut stale = signal("stale", "c", EvidenceStance::Supports);
    stale.observed_at_ms = NOW - AnalysisPolicy::default().maximum_age_ms - 1;
    stale.expires_at_ms = None;
    let mut weak = signal("weak", "d", EvidenceStance::Supports);
    weak.reliability_permille = 499;
    let unknown = signal("unknown", "e", EvidenceStance::Unknown);

    let result = assess(&[display_only, no_consent, stale, weak, unknown]);
    assert_eq!(result.state(), AssessmentState::Unavailable);
    for reason in [
        SignalReason::MachineAnalysisForbidden,
        SignalReason::ConsentAbsent,
        SignalReason::Stale,
        SignalReason::LowReliability,
        SignalReason::NoStance,
    ] {
        assert!(result.decisions().iter().any(|item| item.reason == reason));
    }
}

#[test]
fn recency_reduces_strength_without_changing_source_count() {
    let recent = assess(&[
        signal("recent-a", "a", EvidenceStance::Supports),
        signal("recent-b", "b", EvidenceStance::Supports),
    ]);
    let mut old_a = signal("old-a", "a", EvidenceStance::Supports);
    let mut old_b = signal("old-b", "b", EvidenceStance::Supports);
    old_a.observed_at_ms = NOW - AnalysisPolicy::default().maximum_age_ms + 1;
    old_b.observed_at_ms = old_a.observed_at_ms;
    old_a.expires_at_ms = None;
    old_b.expires_at_ms = None;
    let old = assess(&[old_a, old_b]);
    assert_eq!(old.state(), AssessmentState::Supported);
    assert!(old.support_strength_permille() < recent.support_strength_permille());
}

#[test]
fn model_output_is_advisory_only_when_it_matches_admitted_evidence() {
    let assessment = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let decision = gate_model_proposal(&proposal(&assessment), &assessment).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Advisory);
    assert_eq!(decision.reason, ProposalReason::EvidenceAligned);
}

#[test]
fn model_must_abstain_on_dispute_and_cannot_invent_evidence() {
    let disputed = assess(&[
        signal("yes", "a", EvidenceStance::Supports),
        signal("no", "b", EvidenceStance::Contradicts),
    ]);
    let decision = gate_model_proposal(&proposal(&disputed), &disputed).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Abstain);
    assert_eq!(decision.reason, ProposalReason::EvidenceDisputed);

    let supported = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let mut fabricated = proposal(&supported);
    fabricated.evidence_refs.push("evidence:invented".into());
    let decision = gate_model_proposal(&fabricated, &supported).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Rejected);
    assert_eq!(
        decision.reason,
        ProposalReason::UnsupportedEvidenceReference
    );
}

#[test]
fn a_model_cannot_override_the_evidence_direction() {
    let assessment = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let mut contrary = proposal(&assessment);
    contrary.stance = EvidenceStance::Contradicts;
    let decision = gate_model_proposal(&contrary, &assessment).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Rejected);
    assert_eq!(decision.reason, ProposalReason::ContradictsAssessment);
}

#[test]
fn a_model_cannot_cherry_pick_one_citation_from_a_corroborated_result() {
    let assessment = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let mut cherry_picked = proposal(&assessment);
    cherry_picked.evidence_refs.truncate(1);
    let decision = gate_model_proposal(&cherry_picked, &assessment).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Abstain);
    assert_eq!(decision.reason, ProposalReason::InsufficientCitations);
}

#[test]
fn an_uncertain_model_abstains_even_when_the_evidence_is_supported() {
    let assessment = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let mut uncertain = proposal(&assessment);
    uncertain.score_permille = AnalysisPolicy::default().minimum_model_score_permille - 1;
    let decision = gate_model_proposal(&uncertain, &assessment).unwrap();
    assert_eq!(decision.disposition, ProposalDisposition::Abstain);
    assert_eq!(decision.reason, ProposalReason::ModelUncertain);
}

#[test]
fn invalid_or_cross_hypothesis_inputs_are_refused_before_analysis() {
    let unbounded = AnalysisPolicy {
        maximum_signals: 4097,
        minimum_independent_sources: 2,
        ..AnalysisPolicy::default()
    };
    assert_eq!(
        assess_hypothesis("hypothesis:port-event", &[], &unbounded, NOW),
        Err(AnalysisError::InvalidPolicy)
    );

    let policy = AnalysisPolicy {
        maximum_signals: 2,
        ..AnalysisPolicy::default()
    };
    assert!(matches!(
        assess_hypothesis(
            "hypothesis:port-event",
            &[
                signal("a", "a", EvidenceStance::Supports),
                signal("b", "b", EvidenceStance::Supports),
                signal("c", "c", EvidenceStance::Supports)
            ],
            &policy,
            NOW
        ),
        Err(AnalysisError::TooManySignals { .. })
    ));

    let assessment = assess(&[
        signal("a", "camera", EvidenceStance::Supports),
        signal("b", "operator", EvidenceStance::Supports),
    ]);
    let mut mismatched = proposal(&assessment);
    mismatched.hypothesis_id = "hypothesis:other".into();
    assert_eq!(
        gate_model_proposal(&mismatched, &assessment),
        Err(ProposalError::HypothesisMismatch)
    );
}
