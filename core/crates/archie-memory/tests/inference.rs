use archie_memory::{
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus, RetainedTaskState},
    inference::{
        sha256, InferenceBinding, InferenceReceipt, InferenceStatus, ObservedUsage,
        OpaqueSessionId, ScopedReference,
    },
};
use archie_vault::ContextDomain;

fn binding() -> InferenceBinding {
    InferenceBinding {
        schema_version: 1,
        domain: ContextDomain::work("studio", "project").unwrap(),
        focus_id: "focus:one".into(),
        task_id: "task:one".into(),
        objective_ref: "objective:one".into(),
        runner_id: "runner:api".into(),
        provider_id: "provider:local".into(),
        account_id: "account:one".into(),
    }
}
fn scoped(binding: &InferenceBinding, suffix: &str) -> ScopedReference {
    ScopedReference {
        reference: binding.domain.reference(suffix).unwrap(),
        account_id: binding.account_id.clone(),
    }
}
fn receipt() -> InferenceReceipt {
    let binding = binding();
    InferenceReceipt {
        schema_version: 1,
        receipt_id: "receipt:one".into(),
        checkpoint_id: "checkpoint:one".into(),
        session_id: Some(OpaqueSessionId::new("thread-one").unwrap()),
        model_id: "local:model".into(),
        request_sha256: sha256(b"admitted rendered request"),
        output_sha256: Some(sha256(b"attributed fixture answer")),
        admitted_evidence_refs: vec![scoped(&binding, "evidence/one")],
        result_ref: Some(scoped(&binding, "inference/result-one")),
        binding,
        status: InferenceStatus::Completed,
        error_code: None,
        observed_at_ms: 30,
        usage: ObservedUsage {
            input_tokens: Some(12),
            output_tokens: Some(4),
            duration_ms: Some(10),
        },
    }
}
fn checkpoint(receipt: &InferenceReceipt) -> ExecutionCheckpoint {
    ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: receipt.checkpoint_id.clone(),
        focus_id: receipt.binding.focus_id.clone(),
        branch_id: "branch:one".into(),
        parent_checkpoint_id: None,
        objective_ref: receipt.binding.objective_ref.clone(),
        evidence_ref: receipt.provenance_ref(),
        recorded_at_ms: receipt.observed_at_ms,
        kind: ExecutionKind::Result,
        status: ExecutionStatus::Current,
        state: RetainedTaskState {
            artifact_refs: vec![receipt.result_ref.as_ref().unwrap().reference.to_string()],
            ..Default::default()
        },
    }
}

#[test]
fn attributed_receipt_is_bound_to_every_identity_and_has_no_raw_content() {
    let receipt = receipt();
    receipt.validate().unwrap();
    receipt.validate_checkpoint(&checkpoint(&receipt)).unwrap();
    let encoded = serde_json::to_string(&receipt).unwrap();
    for forbidden in [
        "attributed fixture answer",
        "admitted rendered request",
        "credential",
        "prompt",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    let mut changed = receipt.binding.clone();
    changed.account_id = "account:two".into();
    assert_ne!(receipt.binding, changed);
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn every_evidence_and_result_reference_requires_exact_domain_and_account() {
    for result in [false, true] {
        for foreign in [
            "personal/owner/item",
            "work/studio/project-other/item",
            "work/other/project/item",
        ] {
            let mut receipt = receipt();
            let reference = ScopedReference {
                reference: archie_vault::VaultRef::from_path(foreign).unwrap(),
                account_id: receipt.binding.account_id.clone(),
            };
            if result {
                receipt.result_ref = Some(reference);
            } else {
                receipt.admitted_evidence_refs = vec![reference];
            }
            assert!(receipt.validate().is_err(), "foreign ref {foreign}");
        }
        let mut receipt = receipt();
        if result {
            receipt.result_ref.as_mut().unwrap().account_id = "account:two".into();
        } else {
            receipt.admitted_evidence_refs[0].account_id = "account:two".into();
        }
        assert!(receipt.validate().is_err());
    }
}

#[test]
fn a_checkpoint_cannot_promote_a_model_result_to_a_fact_or_retain_unadmitted_refs() {
    let receipt = receipt();
    let mut checkpoint = checkpoint(&receipt);
    checkpoint.state.verified_fact_refs = checkpoint.state.artifact_refs.clone();
    assert!(receipt.validate_checkpoint(&checkpoint).is_err());
    checkpoint.state.verified_fact_refs.clear();
    for foreign in [
        "vault:personal/owner/secret",
        "vault:work/studio/project/not-admitted",
        "decision:invented",
    ] {
        checkpoint.state.decision_refs = vec![foreign.into()];
        assert!(receipt.validate_checkpoint(&checkpoint).is_err());
    }
    checkpoint.state.decision_refs.clear();
    checkpoint.focus_id = "focus:other".into();
    assert!(receipt.validate_checkpoint(&checkpoint).is_err());
}

#[test]
fn failed_and_cancelled_receipts_never_claim_completed_output_or_terminal_parent() {
    for status in [InferenceStatus::Failed, InferenceStatus::Cancelled] {
        let mut receipt = receipt();
        let mut checkpoint = checkpoint(&receipt);
        receipt.status = status;
        receipt.error_code = Some("runner_timeout".into());
        assert!(receipt.validate().is_err());
        receipt.output_sha256 = None;
        receipt.result_ref = None;
        receipt.validate().unwrap();
        checkpoint.state.artifact_refs.clear();
        checkpoint.kind = ExecutionKind::Attempt;
        receipt.validate_checkpoint(&checkpoint).unwrap();
        checkpoint.status = ExecutionStatus::Completed;
        assert!(receipt.validate_checkpoint(&checkpoint).is_err());
    }
}

#[test]
fn digests_sessions_unknown_fields_and_observation_bounds_fail_closed() {
    for bad in [
        "",
        "../thread",
        "Bearer secret",
        "thread\nother",
        "thread?token=value",
    ] {
        assert!(OpaqueSessionId::new(bad).is_err());
        assert!(serde_json::from_value::<OpaqueSessionId>(serde_json::json!(bad)).is_err());
    }
    let mut receipt = receipt();
    receipt.request_sha256 = "A".repeat(64);
    assert!(receipt.validate().is_err());
    receipt.request_sha256 = sha256(b"request");
    receipt.observed_at_ms = 0;
    assert!(receipt.validate().is_err());
    receipt.observed_at_ms = 30;
    receipt.usage.output_tokens = Some(u64::MAX);
    assert!(receipt.validate().is_err());
    let mut encoded = serde_json::to_value(binding()).unwrap();
    encoded["secret"] = serde_json::json!("not accepted");
    assert!(serde_json::from_value::<InferenceBinding>(encoded).is_err());
}
