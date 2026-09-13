//! Explicitly synthetic task outcomes, not recordings or measured accuracy.
use archie_memory::{
    admit_learning,
    cognitive::{LearningSignal, TrainingEvent, UtilityObservation},
    graph::Sensitivity,
    utility::{event_digest, AdmittedUtilitySample, ContextUtilityModel},
    AdmissionPolicy, ContentClass, CuratedTrainingRecord, LearningCandidate, SensorSource,
    Supervision,
};
use std::collections::BTreeSet;
pub const NOW: i64 = 1_800_000_000_000;
pub fn example(i: usize, reference: &str, exposure: f64) -> (LearningCandidate, TrainingEvent) {
    let event = TrainingEvent {
        event_id: format!("event:{i}"),
        subject_id: "subject:synthetic".into(),
        snapshot_id: "snapshot:synthetic".into(),
        focus_id: "focus:synthetic".into(),
        evidence_ref: reference.into(),
        signal: LearningSignal::TaskOutcome,
        observed: UtilityObservation {
            quality: Some(0.8),
            relevance: Some(0.8),
            privacy_exposure: Some(exposure),
            risk: Some(0.0),
            task_success: Some(true),
            ..Default::default()
        },
        recorded_at_ms: NOW,
    };
    let candidate = LearningCandidate {
        candidate_id: format!("candidate:{i}"),
        source: SensorSource::TaskRuntime,
        source_ref: event.event_id.clone(),
        source_root: format!("task:{i}"),
        evidence_ref: reference.into(),
        evidence_sha256: event_digest(&event).unwrap(),
        semantic_key: format!("semantic:{i}"),
        content_class: ContentClass::TaskOutcome,
        sensitivity: Sensitivity::LocalRestricted,
        machine_analysis_allowed: true,
        consent_required: true,
        consent_granted: true,
        supervision: Supervision::VerifiedTaskOutcome {
            outcome_receipt_ref: "receipt:synthetic".into(),
        },
        quality_permille: 900,
        relevance_permille: 900,
        novelty_permille: 900,
        observed_at_ms: NOW,
        expires_at_ms: None,
    };
    (candidate, event)
}
pub fn admit(candidate: LearningCandidate) -> Vec<CuratedTrainingRecord> {
    admit_learning(
        &[candidate],
        &AdmissionPolicy::default(),
        NOW,
        &BTreeSet::from(["receipt:synthetic".into()]),
        &BTreeSet::new(),
    )
    .unwrap()
    .into_training_records()
}
pub fn trained(seed: u64) -> ContextUtilityModel {
    let mut model = ContextUtilityModel::new(seed, NOW).unwrap();
    for i in 0..8 {
        let (candidate, event) = example(
            i,
            if i < 4 {
                "vault:context/exposing"
            } else {
                "vault:context/useful"
            },
            if i < 4 { 1.0 } else { 0.1 },
        );
        let records = admit(candidate);
        model
            .train(
                AdmittedUtilitySample::bind(&records[0], &event).unwrap(),
                NOW,
            )
            .unwrap();
    }
    model
}
