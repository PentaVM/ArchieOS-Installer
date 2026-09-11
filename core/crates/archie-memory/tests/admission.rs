use archie_memory::graph::Sensitivity;
use archie_memory::{
    admit_learning, AdmissionBatch, AdmissionError, AdmissionPolicy, AdmissionReason, ContentClass,
    LearningCandidate, MemoryDisposition, SensorSource, Supervision,
};
use std::collections::BTreeSet;

const NOW: i64 = 1_800_000_000_000;
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn candidate(id: &str) -> LearningCandidate {
    LearningCandidate {
        candidate_id: format!("candidate:{id}"),
        source: SensorSource::OperatorCorrection,
        source_ref: "cortex:correction/one".into(),
        source_root: "owner:local".into(),
        evidence_ref: format!("vault:evidence/{id}"),
        evidence_sha256: DIGEST.into(),
        semantic_key: format!("semantic:{id}"),
        content_class: ContentClass::CorrectionTrajectory,
        sensitivity: Sensitivity::LocalRestricted,
        machine_analysis_allowed: true,
        consent_required: true,
        consent_granted: true,
        supervision: Supervision::OperatorCorrection {
            graph_writer_receipt_ref: format!("graph:receipt/{id}"),
        },
        quality_permille: 900,
        relevance_permille: 900,
        novelty_permille: 900,
        observed_at_ms: NOW - 100,
        expires_at_ms: None,
    }
}

fn verified_receipts(candidates: &[LearningCandidate]) -> BTreeSet<String> {
    candidates
        .iter()
        .filter_map(|candidate| match &candidate.supervision {
            Supervision::None => None,
            Supervision::OperatorCorrection {
                graph_writer_receipt_ref,
            } => Some(graph_writer_receipt_ref.clone()),
            Supervision::VerifiedTaskOutcome {
                outcome_receipt_ref,
            } => Some(outcome_receipt_ref.clone()),
        })
        .collect()
}

fn admit(
    candidates: &[LearningCandidate],
    policy: &AdmissionPolicy,
) -> Result<AdmissionBatch, AdmissionError> {
    admit_learning(
        candidates,
        policy,
        NOW,
        &verified_receipts(candidates),
        &BTreeSet::new(),
    )
}

#[test]
fn only_explicit_verified_supervision_crosses_the_training_boundary() {
    let mut sentry = candidate("sentry");
    sentry.source = SensorSource::Sentry;
    sentry.content_class = ContentClass::StructuredObservation;
    sentry.supervision = Supervision::None;
    let mut wanda = candidate("wanda");
    wanda.source = SensorSource::Wanda;
    wanda.content_class = ContentClass::SocialHypothesis;
    wanda.supervision = Supervision::None;
    wanda.expires_at_ms = Some(NOW + 1000);
    let correction = candidate("correction");

    let batch = admit(&[sentry, wanda, correction], &AdmissionPolicy::default()).unwrap();
    assert_eq!(batch.training_records().len(), 1);
    assert_eq!(
        batch.training_records()[0].candidate_id(),
        "candidate:correction"
    );
    assert_eq!(
        batch.decisions()[0].disposition,
        MemoryDisposition::WorkingSet
    );
    assert_eq!(
        batch.decisions()[1].disposition,
        MemoryDisposition::WorkingSet
    );
    assert_eq!(
        batch.decisions()[2].disposition,
        MemoryDisposition::Training
    );
}

#[test]
fn raw_audio_images_video_and_transcripts_are_never_training_data() {
    let classes = [
        ContentClass::RawAudio,
        ContentClass::RawVideo,
        ContentClass::RawImage,
        ContentClass::Transcript,
    ];
    let candidates: Vec<_> = classes
        .into_iter()
        .enumerate()
        .map(|(index, class)| {
            let mut item = candidate(&format!("raw-{index}"));
            item.content_class = class;
            item
        })
        .collect();
    let batch = admit(&candidates, &AdmissionPolicy::default()).unwrap();
    assert!(batch.training_records().is_empty());
    assert_eq!(batch.count(AdmissionReason::RawSensoryPayload), 4);
    assert!(batch
        .decisions()
        .iter()
        .all(|item| item.disposition == MemoryDisposition::Discard));
}

#[test]
fn a_sentry_flood_cannot_become_training_by_repetition() {
    let candidates: Vec<_> = (0..4000)
        .map(|index| {
            let mut item = candidate(&format!("sensor-{index}"));
            item.source = SensorSource::Sentry;
            item.source_root = "camera:entry".into();
            item.content_class = ContentClass::SanitizedAggregate;
            item.supervision = Supervision::None;
            item.semantic_key = "occupancy:entry:minute".into();
            item
        })
        .collect();
    let batch = admit(&candidates, &AdmissionPolicy::default()).unwrap();
    assert!(batch.training_records().is_empty());
    assert_eq!(
        batch.count(AdmissionReason::ObservationIsNotSupervision),
        4000
    );
}

#[test]
fn forged_supervision_from_sentry_or_wanda_is_rejected() {
    for source in [SensorSource::Sentry, SensorSource::Wanda] {
        let mut item = candidate("forged");
        item.source = source;
        item.content_class = ContentClass::SocialHypothesis;
        let batch = admit(&[item], &AdmissionPolicy::default()).unwrap();
        assert!(batch.training_records().is_empty());
    }
}

#[test]
fn consent_and_provider_analysis_policy_fail_closed() {
    let mut no_consent = candidate("no-consent");
    no_consent.consent_granted = false;
    let mut display_only = candidate("display-only");
    display_only.machine_analysis_allowed = false;
    let batch = admit(&[no_consent, display_only], &AdmissionPolicy::default()).unwrap();
    assert_eq!(batch.count(AdmissionReason::ConsentAbsent), 1);
    assert_eq!(batch.count(AdmissionReason::MachineAnalysisForbidden), 1);
    assert!(batch.training_records().is_empty());
}

#[test]
fn naming_a_receipt_does_not_make_it_verified() {
    let input = [candidate("unverified")];
    let batch = admit_learning(
        &input,
        &AdmissionPolicy::default(),
        NOW,
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    assert!(batch.training_records().is_empty());
    assert_eq!(batch.count(AdmissionReason::UnverifiedReceipt), 1);
    assert_eq!(
        batch.decisions()[0].disposition,
        MemoryDisposition::Episodic
    );
}

#[test]
fn an_admitted_digest_cannot_reenter_a_later_training_batch() {
    let input = [candidate("replayed")];
    let known = BTreeSet::from([DIGEST.to_string()]);
    let batch = admit_learning(
        &input,
        &AdmissionPolicy::default(),
        NOW,
        &verified_receipts(&input),
        &known,
    )
    .unwrap();
    assert!(batch.training_records().is_empty());
    assert_eq!(batch.count(AdmissionReason::PreviouslyAdmitted), 1);
    assert_eq!(
        batch.decisions()[0].disposition,
        MemoryDisposition::Episodic
    );
}

#[test]
fn biometric_secret_and_location_data_do_not_enter_training() {
    let mut biometric = candidate("bio");
    biometric.sensitivity = Sensitivity::Biometric;
    let mut secret = candidate("secret");
    secret.sensitivity = Sensitivity::Secret;
    let mut location = candidate("location");
    location.sensitivity = Sensitivity::Location;
    let batch = admit(&[biometric, secret, location], &AdmissionPolicy::default()).unwrap();
    assert!(batch.training_records().is_empty());
    assert_eq!(batch.decisions()[0].disposition, MemoryDisposition::Discard);
    assert_eq!(batch.decisions()[1].disposition, MemoryDisposition::Discard);
    assert_eq!(
        batch.decisions()[2].disposition,
        MemoryDisposition::Episodic
    );
}

#[test]
fn semantic_and_source_quotas_keep_the_dataset_balanced() {
    let policy = AdmissionPolicy {
        max_training_records: 3,
        max_training_records_per_source: 2,
        ..AdmissionPolicy::default()
    };
    let mut duplicate_low = candidate("duplicate-low");
    duplicate_low.semantic_key = "semantic:shared".into();
    duplicate_low.relevance_permille = 600;
    let mut duplicate_high = candidate("duplicate-high");
    duplicate_high.semantic_key = "semantic:shared".into();
    duplicate_high.relevance_permille = 950;
    let one = candidate("one");
    let two = candidate("two");
    let mut other_source = candidate("other-source");
    other_source.source_root = "owner:second".into();

    let batch = admit(
        &[duplicate_low, one, duplicate_high, two, other_source],
        &policy,
    )
    .unwrap();
    assert_eq!(batch.training_records().len(), 3);
    assert!(batch
        .training_records()
        .iter()
        .any(|item| item.candidate_id() == "candidate:duplicate-high"));
    assert!(!batch
        .training_records()
        .iter()
        .any(|item| item.candidate_id() == "candidate:duplicate-low"));
    assert_eq!(batch.count(AdmissionReason::SemanticDuplicate), 1);
    assert_eq!(batch.count(AdmissionReason::SourceQuota), 1);
}

#[test]
fn selection_is_deterministic_and_training_records_have_no_payload_fields() {
    let input = vec![candidate("b"), candidate("a")];
    let first = admit(&input, &AdmissionPolicy::default()).unwrap();
    for _ in 0..10 {
        assert_eq!(admit(&input, &AdmissionPolicy::default()).unwrap(), first);
    }
    let json = serde_json::to_string(first.training_records()).unwrap();
    for forbidden in [
        "audio",
        "video",
        "image",
        "transcript",
        "payload",
        "message",
    ] {
        assert!(
            !json.contains(forbidden),
            "training export leaked field {forbidden}"
        );
    }
}

#[test]
fn oversized_or_malformed_batches_are_refused_before_partial_output() {
    let unbounded = AdmissionPolicy {
        max_batch_candidates: 4097,
        ..AdmissionPolicy::default()
    };
    assert_eq!(admit(&[], &unbounded), Err(AdmissionError::InvalidPolicy));

    let policy = AdmissionPolicy {
        max_batch_candidates: 1,
        max_training_records: 1,
        max_training_records_per_source: 1,
        ..AdmissionPolicy::default()
    };
    assert!(matches!(
        admit(&[candidate("a"), candidate("b")], &policy),
        Err(AdmissionError::BatchTooLarge { .. })
    ));
    let mut malformed = candidate("bad");
    malformed.evidence_sha256 = "not-a-digest".into();
    assert!(matches!(
        admit(&[malformed], &policy),
        Err(AdmissionError::InvalidCandidate { .. })
    ));
}
