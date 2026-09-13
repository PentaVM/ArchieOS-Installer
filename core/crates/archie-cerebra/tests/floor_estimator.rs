use archie_cerebra::{
    parameters::{load_floor, save_floor},
    Cerebra, CerebraConfig, Observation, Trigger,
};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_speech::estimator::{
    AudioFeatures, FloorContext, FloorEstimator, FloorTargets, FRAME_SAMPLES, RETENTION_MS,
};
use archie_types::{Locality, TraceId};
use archie_vault::{ContextGateway, Vault, VaultKey, VaultRef};
const NOW: i64 = 1_800_000_000_000;
#[test]
fn estimated_silence_stops_before_context_even_with_utility_learning() {
    let mut buffer = [0.3; FRAME_SAMPLES];
    let features = AudioFeatures::extract(&mut buffer, 30);
    let state = FloorEstimator::new(Locality::Local, NOW).unwrap().estimate(
        &features,
        &FloorContext::default(),
        NOW,
    );
    let observation = Observation::new(Trigger::Utterance, "task:one", "floor-test", NOW)
        .with_floor(state)
        .needing(VaultRef::parse("vault:absent/one").unwrap());
    let mut vault = Vault::open(&VaultKey::from_bytes([3; 32])).unwrap();
    let cerebra = Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
    .with_context_utility(archie_memory::utility::ContextUtilityModel::new(1, NOW).unwrap());
    let pass = cerebra
        .consider(
            &mut vault,
            &ModelRegistry::new(),
            &observation,
            TraceId::new(),
            NOW,
        )
        .unwrap();
    assert!(pass.intent.is_silent());
    assert!(vault.audit_log().is_empty());
    let trace = serde_json::to_string(&pass.trace).unwrap();
    assert!(!trace.contains("weights"));
    assert!(!trace.contains("values"));
}
#[test]
fn floor_checkpoint_contains_only_encrypted_parameters_and_restores_locally() {
    let mut model = FloorEstimator::new(Locality::Local, NOW).unwrap();
    let mut buffer = [0.2; FRAME_SAMPLES];
    model
        .learn(
            AudioFeatures::extract(&mut buffer, 100),
            FloorTargets::new([500, 500, 500, 500, -500]).unwrap(),
            NOW,
        )
        .unwrap();
    let key = VaultKey::from_bytes([4; 32]);
    let path = std::env::temp_dir().join(format!("archie-floor-{}.json", TraceId::new()));
    save_floor(&path, &key, &model, NOW).unwrap();
    assert_eq!(load_floor(&path, &key, NOW).unwrap(), model);
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(!disk.contains("weights"));
    assert!(disk.contains("restricted"));
    assert!(load_floor(&path, &key, NOW + RETENTION_MS).is_err());
    std::fs::remove_file(path).unwrap();
}
