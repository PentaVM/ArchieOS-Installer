#[path = "support/utility.rs"]
mod synthetic;
use archie_memory::utility::{
    AdmittedUtilitySample, ContextUtilityModel, UtilityError, RETENTION_MS,
};
use synthetic::*;

#[test]
fn never_adds_task_unrequested_references() {
    let model = trained(42);
    for requested in [
        vec![],
        vec!["vault:context/unknown".into()],
        vec![
            "vault:context/exposing".into(),
            "vault:context/unknown".into(),
        ],
    ] {
        let indices = model.select_indices(&requested, NOW);
        assert!(indices.iter().all(|i| *i < requested.len()));
        assert!(!indices
            .iter()
            .any(|i| requested[*i] == "vault:context/useful"));
    }
}
#[test]
fn synthetic_comparison_privacy_cost_changes_success_only_retrieval() {
    let requested = vec![
        "vault:context/exposing".into(),
        "vault:context/useful".into(),
    ];
    // Both contexts succeeded in every synthetic observation. Success-only ranking
    // retains both. The privacy-aware learner excludes the costlier reference.
    let success_only_indices: Vec<_> = [true, true]
        .into_iter()
        .enumerate()
        .filter_map(|(i, succeeded)| succeeded.then_some(i))
        .collect();
    let actual = trained(42).select_indices(&requested, NOW);
    assert_eq!(actual, vec![1]);
    assert_eq!(success_only_indices.len(), 2);
    let synthetic_exposure_permille = [1000, 100];
    assert_eq!(
        success_only_indices
            .iter()
            .map(|i| synthetic_exposure_permille[*i])
            .sum::<i32>(),
        1100
    );
    assert_eq!(
        actual
            .iter()
            .map(|i| synthetic_exposure_permille[*i])
            .sum::<i32>(),
        100
    );
    // Same admitted event shape, replacing only exposure, actually changes learning.
    let mut cheap = ContextUtilityModel::new(42, NOW).unwrap();
    for i in 0..4 {
        let (c, e) = example(i, "vault:context/exposing", 0.0);
        cheap
            .train(AdmittedUtilitySample::bind(&admit(c)[0], &e).unwrap(), NOW)
            .unwrap();
    }
    assert_eq!(cheap.select_indices(&requested, NOW), vec![0, 1]);
}
#[test]
fn zero_and_insufficient_training_keep_original_order() {
    let requested = vec![
        "vault:context/exposing".into(),
        "vault:context/useful".into(),
        "vault:context/exposing".into(),
    ];
    let mut model = ContextUtilityModel::new(9, NOW).unwrap();
    assert_eq!(model.select_indices(&requested, NOW), vec![0, 1, 2]);
    let (c, e) = example(0, "vault:context/exposing", 1.0);
    model
        .train(AdmittedUtilitySample::bind(&admit(c)[0], &e).unwrap(), NOW)
        .unwrap();
    assert_eq!(model.select_indices(&requested, NOW), vec![0, 1, 2]);
}
#[test]
fn same_data_seed_and_reordered_training_have_identical_parameters() {
    let a = trained(42);
    assert_eq!(a, trained(42));
    let mut b = ContextUtilityModel::new(42, NOW).unwrap();
    for i in (0..8).rev() {
        let (c, e) = example(
            i,
            if i < 4 {
                "vault:context/exposing"
            } else {
                "vault:context/useful"
            },
            if i < 4 { 1.0 } else { 0.1 },
        );
        b.train(AdmittedUtilitySample::bind(&admit(c)[0], &e).unwrap(), NOW)
            .unwrap();
    }
    assert_eq!(a.snapshot(), b.snapshot());
}
#[test]
fn rejected_candidate_cannot_supply_a_training_record() {
    let (mut c, _) = example(0, "vault:context/one", 0.0);
    c.consent_granted = false;
    assert!(admit(c).is_empty());
}
#[test]
fn swapping_or_modifying_rewards_after_admission_fails() {
    let (c, mut e) = example(0, "vault:context/one", 1.0);
    let records = admit(c);
    e.observed.privacy_exposure = Some(0.0);
    assert!(matches!(
        AdmittedUtilitySample::bind(&records[0], &e),
        Err(UtilityError::NotAdmitted)
    ));
}
#[test]
fn snapshot_roundtrip_retention_and_replay_are_enforced() {
    let model = trained(42);
    let bytes = model.snapshot();
    let mut restored = ContextUtilityModel::restore(&bytes, NOW).unwrap();
    assert_eq!(model, restored);
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("vault:"));
    assert!(!text.contains("subject:"));
    let (c, e) = example(0, "vault:context/exposing", 1.0);
    assert_eq!(
        restored.train(AdmittedUtilitySample::bind(&admit(c)[0], &e).unwrap(), NOW),
        Err(UtilityError::Replay)
    );
    assert_eq!(
        ContextUtilityModel::restore(&model.snapshot(), NOW + RETENTION_MS),
        Err(UtilityError::Expired)
    );
    assert_eq!(
        model.select_indices(&["vault:context/exposing".into()], NOW + RETENTION_MS),
        vec![0]
    );
}
#[test]
fn corrupt_or_unbounded_parameters_are_rejected() {
    let mut v: serde_json::Value = serde_json::from_slice(&trained(0).snapshot()).unwrap();
    v["weights"][0]["total"] = serde_json::json!(i64::MAX);
    assert_eq!(
        ContextUtilityModel::restore(&serde_json::to_vec(&v).unwrap(), NOW),
        Err(UtilityError::Invalid)
    );
    assert_eq!(
        ContextUtilityModel::restore(&vec![0; 512 * 1024 + 1], NOW),
        Err(UtilityError::Capacity)
    );
}
