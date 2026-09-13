//! Generated transient numeric inputs only; no audio fixture or labelled corpus.
use archie_memory::floor::{decide_floor, FloorOutput};
use archie_speech::estimator::{
    AudioFeatures, EstimatorError, FloorContext, FloorEstimator, FloorTargets, FRAME_SAMPLES,
    RETENTION_MS,
};
use archie_types::Locality;
const NOW: i64 = 1_800_000_000_000;
fn model() -> FloorEstimator {
    FloorEstimator::new(Locality::Local, NOW).unwrap()
}
fn context() -> FloorContext {
    FloorContext {
        response_utility: 0.8,
        context_sufficiency: 0.9,
        explicit_question: true,
        ..Default::default()
    }
}
fn features(value: f32, silence: u32) -> AudioFeatures {
    let mut buffer = [value; FRAME_SAMPLES];
    let features = AudioFeatures::extract(&mut buffer, silence);
    assert!(buffer.iter().all(|x| *x == 0.0));
    features
}
#[test]
fn no_cloud_or_fabric_creation_or_checkpoint_route() {
    assert_eq!(model().locality(), Locality::Local);
    for locality in [Locality::Cloud, Locality::Fabric] {
        assert_eq!(
            FloorEstimator::new(locality, NOW),
            Err(EstimatorError::NonLocal)
        );
        assert_eq!(
            FloorEstimator::restore(&model().snapshot(), locality, NOW),
            Err(EstimatorError::NonLocal)
        );
    }
}
#[test]
fn extreme_audio_timing_and_nan_produce_valid_conservative_outputs() {
    let model = model();
    for amplitude in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
        -f32::MAX,
        -1.0,
        -0.01,
        0.0,
        0.01,
        1.0,
    ] {
        for silence in [0, 20, 400, 1200, 3000, u32::MAX] {
            let state = model.estimate(&features(amplitude, silence), &context(), NOW);
            state.validate().unwrap();
            if !amplitude.is_finite() || amplitude.abs() > 1.0 {
                assert_eq!(
                    decide_floor(&state, true, false, NOW).output,
                    FloorOutput::Silent
                );
            }
        }
    }
}
#[test]
fn invalid_task_estimates_also_fail_silent() {
    for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 2.0, -2.0] {
        let context = FloorContext {
            response_utility: bad,
            ..context()
        };
        let state = model().estimate(&features(0.0, 3000), &context, NOW);
        state.validate().unwrap();
        assert_eq!(
            decide_floor(&state, true, false, NOW).output,
            FloorOutput::Silent
        );
    }
}
#[test]
fn baseline_supplies_inputs_to_the_existing_policy() {
    let model = model();
    let busy = model.estimate(&features(0.5, 20), &context(), NOW);
    assert_eq!(
        decide_floor(&busy, true, false, NOW).output,
        FloorOutput::Silent
    );
    let complete = model.estimate(&features(0.0, 3000), &context(), NOW);
    assert_eq!(
        decide_floor(&complete, true, false, NOW).output,
        FloorOutput::ShortResponse
    );
    assert_eq!(
        decide_floor(&complete, false, false, NOW).output,
        FloorOutput::Silent
    );
    let unsolicited = model.estimate(&features(0.0, 3000), &FloorContext::default(), NOW);
    assert_eq!(
        decide_floor(&unsolicited, true, false, NOW).output,
        FloorOutput::Silent
    );
}
#[test]
fn injected_invalid_estimator_result_is_silenced_by_unchanged_kernel() {
    let valid = model().estimate(&features(0.0, 3000), &context(), NOW);
    for bad in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let mut state = valid.clone();
        state.speaker_hold_probability = bad;
        assert_eq!(
            decide_floor(&state, true, false, NOW).output,
            FloorOutput::Silent
        );
    }
    for bad in [f64::NAN, -1.01, 1.01] {
        let mut state = valid.clone();
        state.interruption_utility = bad;
        assert_eq!(
            decide_floor(&state, true, false, NOW).output,
            FloorOutput::Silent
        );
    }
}
#[test]
fn online_updates_change_only_weights_reproducibly_without_retaining_examples() {
    let mut a = model();
    let mut b = model();
    let before = a.snapshot();
    for m in [&mut a, &mut b] {
        for _ in 0..32 {
            m.learn(
                features(0.2, 400),
                FloorTargets::new([100, 900, 100, 800, -900]).unwrap(),
                NOW,
            )
            .unwrap();
        }
    }
    assert_eq!(a, b);
    assert_ne!(a.snapshot(), before);
    assert_eq!(a.updates(), 32);
    let state = a.estimate(&features(0.2, 400), &context(), NOW);
    state.validate().unwrap();
    assert!(
        state.thought_completion_probability
            > model()
                .estimate(&features(0.2, 400), &context(), NOW)
                .thought_completion_probability
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&a.snapshot()).unwrap();
    let keys: Vec<_> = snapshot
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "created_at_ms",
            "expires_at_ms",
            "updates",
            "version",
            "weights"
        ]
    );
    assert_eq!(
        FloorEstimator::restore(&a.snapshot(), Locality::Local, NOW).unwrap(),
        a
    );
}
#[test]
fn extreme_online_targets_keep_every_head_bounded() {
    let mut model = model();
    for i in 0..256 {
        model
            .learn(
                features(if i % 2 == 0 { 1.0 } else { -1.0 }, u32::MAX),
                FloorTargets::new(if i % 2 == 0 {
                    [1000, 1000, 1000, 1000, 1000]
                } else {
                    [0, 0, 0, 0, -1000]
                })
                .unwrap(),
                NOW,
            )
            .unwrap();
        model
            .estimate(&features(1.0, u32::MAX), &context(), NOW)
            .validate()
            .unwrap();
    }
}
#[test]
fn invalid_labels_features_weights_and_expired_state_are_refused() {
    assert!(FloorTargets::new([1001, 0, 0, 0, 0]).is_err());
    let mut model = model();
    let before = model.snapshot();
    assert_eq!(
        model.learn(
            features(f32::NAN, 0),
            FloorTargets::new([0; 5]).unwrap(),
            NOW
        ),
        Err(EstimatorError::Invalid)
    );
    assert_eq!(model.snapshot(), before);
    let mut corrupt: serde_json::Value = serde_json::from_slice(&before).unwrap();
    corrupt["weights"][0][0] = serde_json::json!(i32::MAX);
    assert_eq!(
        FloorEstimator::restore(&serde_json::to_vec(&corrupt).unwrap(), Locality::Local, NOW),
        Err(EstimatorError::Invalid)
    );
    assert_eq!(
        FloorEstimator::restore(&before, Locality::Local, NOW + RETENTION_MS),
        Err(EstimatorError::Expired)
    );
    let state = model.estimate(&features(0.0, 3000), &context(), NOW + RETENTION_MS);
    state.validate().unwrap();
    assert_eq!(
        decide_floor(&state, true, false, NOW + RETENTION_MS).output,
        FloorOutput::Silent
    );
}
