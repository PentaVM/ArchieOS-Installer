#[path = "../../archie-memory/tests/support/utility.rs"]
mod synthetic;
use archie_cerebra::{
    parameters::{load_utility, save_utility},
    Cerebra, CerebraConfig, Observation, Trigger,
};
use archie_memory::utility::{ContextUtilityModel, RETENTION_MS};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::{PrivacyGrade, TraceId};
use archie_vault::{ContextGateway, EntryClass, Vault, VaultKey, VaultRef};
use synthetic::*;

#[test]
fn gateway_reads_only_the_learned_subset_and_empty_training_is_unchanged() {
    for train in [false, true] {
        let key = VaultKey::from_bytes([7; 32]);
        let mut vault = Vault::open(&key).unwrap();
        let mut observation = Observation::new(
            Trigger::SystemEvent {
                source: "synthetic".into(),
            },
            "task:test",
            "utility-test",
            NOW,
        )
        .graded(PrivacyGrade::Restricted);
        for r in ["vault:context/exposing", "vault:context/useful"] {
            let reference = VaultRef::parse(r).unwrap();
            vault
                .store(
                    "test",
                    reference.clone(),
                    EntryClass::Preference,
                    PrivacyGrade::Restricted,
                    "{\"synthetic\":true}",
                    NOW,
                    None,
                )
                .unwrap();
            observation.context_refs.push(reference);
        }
        let model = if train {
            trained(42)
        } else {
            ContextUtilityModel::new(42, NOW).unwrap()
        };
        let cerebra = Cerebra::new(
            Router::default(),
            ContextGateway::new(),
            CerebraConfig::default(),
        )
        .with_context_utility(model);
        // Routing can refuse after context; the vault's own log still witnesses retrieval.
        cerebra
            .consider(
                &mut vault,
                &ModelRegistry::new(),
                &observation,
                TraceId::new(),
                NOW,
            )
            .unwrap();
        assert_eq!(vault.audit_log().len(), if train { 3 } else { 4 });
    }
}
#[test]
fn encrypted_disk_checkpoint_restores_weights_and_rejects_wrong_key_or_expiry() {
    let path = std::env::temp_dir().join(format!("archie-utility-{}.json", TraceId::new()));
    let key = VaultKey::from_bytes([8; 32]);
    let model = trained(42);
    save_utility(&path, &key, &model, NOW).unwrap();
    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(!stored.contains("reference_hash"));
    assert!(!stored.contains("task_success"));
    assert!(stored.contains("restricted"));
    assert_eq!(load_utility(&path, &key, NOW).unwrap(), model);
    assert!(save_utility(&path, &key, &model, NOW).is_err());
    assert!(load_utility(&path, &VaultKey::from_bytes([9; 32]), NOW).is_err());
    assert!(load_utility(&path, &key, NOW + RETENTION_MS).is_err());
    std::fs::remove_file(path).unwrap();
}
