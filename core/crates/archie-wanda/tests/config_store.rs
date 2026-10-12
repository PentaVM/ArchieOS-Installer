use archie_vault::ContextDomain;
use archie_wanda::config::{DomainBinding, SourceBinding, WandaConfig};
use archie_wanda::store::WandaStore;
use archie_wanda::types::{Actor, Author, Coverage, EventSource, WandaEvent};
use archie_wanda::WandaError;
use std::collections::BTreeMap;
use std::path::Path;

fn domain() -> ContextDomain {
    ContextDomain::personal("owner").unwrap()
}
fn work() -> ContextDomain {
    ContextDomain::work("studio", "house").unwrap()
}
fn config(dir: &Path) -> WandaConfig {
    let token = dir.join("source.token");
    std::fs::write(&token, "device-secret-fixture\n").unwrap();
    WandaConfig {
        state_dir: dir.join("state"),
        graph_binary: dir.join("archie-sentry"),
        models_dir: dir.join("models"),
        operator_token_file: dir.join("operator.token"),
        domains: vec![
            DomainBinding {
                domain: domain(),
                graph_path: dir.join("personal.tsv"),
            },
            DomainBinding {
                domain: work(),
                graph_path: dir.join("work.tsv"),
            },
        ],
        sources: vec![SourceBinding {
            device_id: "phone".into(),
            domain: domain(),
            source: EventSource::AndroidNotification,
            token_file: Some(token),
            token_env: None,
            allow_presumed_owner: false,
            enabled: true,
            chat_domains: BTreeMap::from([("work-chat".into(), work())]),
        }],
        quality_tier: 50,
        deep_quality_tier: 80,
        retention_hours: 168,
    }
}
fn event(id: &str) -> WandaEvent {
    WandaEvent {
        schema_version: 1,
        event_id: id.into(),
        device_id: "phone".into(),
        actor: Actor::ConfirmedOwner,
        author: Author::Counterparty,
        source: EventSource::AndroidNotification,
        chat_id: "personal-chat".into(),
        sender: "counterparty".into(),
        source_message_id: Some(id.into()),
        sent_at_ms: None,
        observed_at_ms: 1_000,
        text: "PRIVATE-fixture tulipanes".into(),
        media_omitted: false,
        coverage: Coverage::NotificationPreview,
        monitor_id: None,
        window_id: None,
    }
}

#[test]
fn replay_is_idempotent_while_changed_content_is_a_collision() {
    let dir = tempfile::tempdir().unwrap();
    let store = WandaStore::new(config(dir.path())).unwrap();
    let original = event("one");
    assert_eq!(
        store
            .ingest("device-secret-fixture", &[original.clone()], 1_000)
            .unwrap()
            .accepted,
        1
    );
    let mut repost = original.clone();
    repost.observed_at_ms = 2_000;
    let duplicate = store
        .ingest("device-secret-fixture", &[repost], 2_000)
        .unwrap();
    assert_eq!(duplicate.duplicates, 1);
    assert_eq!(duplicate.accepted, 0);
    let mut changed = original;
    changed.text = "changed private content".into();
    let collision = store
        .ingest("device-secret-fixture", &[changed], 2_000)
        .unwrap();
    assert_eq!(collision.rejected, 1);
    assert!(collision.reasons.iter().any(|r| r == "event_id_collision"));
    let events = store.events(&domain(), 64, 2_000).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.text, "PRIVATE-fixture tulipanes");
    assert!(domain().contains(&events[0].evidence_ref));
    assert_eq!(
        store.sources(&domain(), 2_000).unwrap()[0].last_observed_at_ms,
        Some(2_000)
    );
}

#[test]
fn trusted_chat_routing_separates_identical_identifiers_and_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let vault_dir = cfg.state_dir.join("cerebra/vault");
    let store = WandaStore::new(cfg).unwrap();
    let personal = event("same");
    let mut professional = personal.clone();
    professional.chat_id = "work-chat".into();
    professional.text = "work-only-fixture".into();
    let report = store
        .ingest("device-secret-fixture", &[personal, professional], 1_000)
        .unwrap();
    assert_eq!(report.accepted, 2);
    let private = store.events(&domain(), 64, 1_000).unwrap();
    let corporate = store.events(&work(), 64, 1_000).unwrap();
    assert_eq!(private.len(), 1);
    assert_eq!(corporate.len(), 1);
    assert_ne!(private[0].evidence_ref, corporate[0].evidence_ref);
    assert!(!corporate[0].event.text.contains("PRIVATE"));
    let foreign_ref = domain().reference("other/not-a-wanda-event").unwrap();
    let mut vault = archie_cerebra::vault_io::open_vault(&vault_dir).unwrap();
    vault
        .store(
            "test",
            foreign_ref,
            archie_vault::EntryClass::PrivateFact,
            archie_types::PrivacyGrade::Restricted,
            "not JSON and must never be disclosed",
            1_000,
            None,
        )
        .unwrap();
    archie_cerebra::vault_io::persist_vault(&vault_dir, &vault).unwrap();
    drop(vault);
    assert_eq!(store.events(&domain(), 64, 1_000).unwrap().len(), 1);
}

#[test]
fn authentication_actor_binding_and_coverage_are_checked_before_deduplication() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let state = cfg.state_dir.clone();
    let store = WandaStore::new(cfg).unwrap();
    assert!(!store.owner_authenticated(""));
    assert!(!store.owner_authenticated("device-secret-fixture"));
    assert!(matches!(
        store.ingest("wrong", &[event("one")], 1_000),
        Err(WandaError::Unauthorized)
    ));
    assert!(!state.join("cerebra/vault/entries.json").exists());
    assert_eq!(
        store
            .ingest("device-secret-fixture", &[event("one")], 1_000)
            .unwrap()
            .accepted,
        1
    );
    let mut invalid = event("one");
    invalid.actor = Actor::ConfirmedOther;
    let report = store
        .ingest("device-secret-fixture", &[invalid], 1_000)
        .unwrap();
    assert_eq!(report.rejected, 1);
    assert_eq!(report.duplicates, 0);
    let mut mismatch = event("two");
    mismatch.device_id = "another-phone".into();
    let mut coverage = event("three");
    coverage.coverage = Coverage::VisibleChat;
    let mut unknown = event("four");
    unknown.actor = Actor::Unknown;
    let mut presumed = event("five");
    presumed.actor = Actor::PresumedOwner;
    let report = store
        .ingest(
            "device-secret-fixture",
            &[mismatch, coverage, unknown, presumed],
            1_000,
        )
        .unwrap();
    assert_eq!(report.rejected, 4);
    assert_eq!(store.events(&domain(), 64, 1_000).unwrap().len(), 1);
}

#[test]
fn input_and_read_budgets_are_enforced_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = WandaStore::new(config(dir.path())).unwrap();
    assert!(matches!(
        store.ingest("device-secret-fixture", &vec![event("many"); 33], 1_000),
        Err(WandaError::TooLarge)
    ));
    let mut oversized = event("oversized");
    oversized.text = "🌷".repeat(2_001);
    let mut id = event("id");
    id.event_id = "x".repeat(193);
    let report = store
        .ingest("device-secret-fixture", &[oversized, id], 1_000)
        .unwrap();
    assert_eq!(report.rejected, 2);
    assert!(store.events(&domain(), 64, 1_000).unwrap().is_empty());
    let mut unicode = event("unicode");
    unicode.text = "🌷".repeat(2_000);
    assert_eq!(
        store
            .ingest("device-secret-fixture", &[unicode], 1_000)
            .unwrap()
            .accepted,
        1
    );
    assert!(matches!(
        store.events(&domain(), 65, 1_000),
        Err(WandaError::TooLarge)
    ));
    assert!(matches!(
        store.events(&ContextDomain::personal("foreign").unwrap(), 64, 1_000),
        Err(WandaError::UnknownDomain)
    ));
}

#[test]
fn expiry_removes_only_owned_ciphertext_and_never_resurrects_old_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.retention_hours = 1;
    let vault_dir = cfg.state_dir.join("cerebra/vault");
    let store = WandaStore::new(cfg).unwrap();
    store
        .ingest("device-secret-fixture", &[event("one")], 1_000)
        .unwrap();
    assert!(store.events(&domain(), 64, 3_601_000).unwrap().is_empty());
    let replay = store
        .ingest("device-secret-fixture", &[event("one")], 3_601_000)
        .unwrap();
    assert_eq!(replay.rejected, 1);
    let entries = archie_cerebra::vault_io::open_vault(&vault_dir)
        .unwrap()
        .sealed_entries();
    assert!(entries
        .iter()
        .all(|e| !e.reference.path().contains("wanda/events/")));
}

#[test]
fn full_body_stays_encrypted_and_config_debug_reveals_no_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let state = cfg.state_dir.clone();
    let operator = cfg.operator_token_file.clone();
    assert!(!format!("{cfg:?}").contains("device-secret"));
    let store = WandaStore::new(cfg.clone()).unwrap();
    let owner_token = std::fs::read_to_string(operator).unwrap();
    assert!(store.owner_authenticated(owner_token.trim()));
    assert!(!format!("{store:?}").contains(owner_token.trim()));
    assert_eq!(
        store
            .ingest("device-secret-fixture", &[event("one")], 1_000)
            .unwrap()
            .accepted,
        1
    );
    let bytes = std::fs::read(state.join("cerebra/vault/entries.json")).unwrap();
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("PRIVATE-fixture"));
    assert_eq!(
        WandaStore::new(cfg)
            .unwrap()
            .events(&domain(), 64, 1_000)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn expired_query_snapshots_are_removed_without_touching_other_pillars() {
    let dir=tempfile::tempdir().unwrap();let store=WandaStore::new(config(dir.path())).unwrap();
    let path=store.vault_path();let mut vault=archie_cerebra::vault_io::open_vault(&path).unwrap();
    let temporary=domain().reference("wanda/query/fixture").unwrap();
    let foreign=domain().reference("forge/fixture").unwrap();
    for reference in [&temporary,&foreign] {vault.store("fixture",reference.clone(),archie_vault::EntryClass::PrivateFact,archie_types::PrivacyGrade::Restricted,"private fixture",1000,Some(2000)).unwrap();}
    archie_cerebra::vault_io::persist_vault(&path,&vault).unwrap();drop(vault);
    store.events(&domain(),64,2000).unwrap();let vault=archie_cerebra::vault_io::open_vault(&path).unwrap();
    assert!(!vault.contains(&temporary));assert!(vault.contains(&foreign));
}

#[test]
fn config_paths_defaults_and_boundaries_come_from_trusted_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wanda.toml");
    std::fs::write(
        &path,
        r#"
state_dir = "state"
graph_binary = "bin/archie-sentry"
models_dir = "models"
operator_token_file = "owner.token"
[[domains]]
domain = "personal/owner"
graph_path = "graph.tsv"
[[sources]]
device_id = "phone"
domain = "personal/owner"
source = "android_notification"
token_file = "phone.token"
"#,
    )
    .unwrap();
    let cfg = WandaConfig::load(&path).unwrap();
    assert_eq!(cfg.state_dir, dir.path().join("state"));
    assert_eq!(cfg.domains[0].graph_path, dir.path().join("graph.tsv"));
    assert_eq!(cfg.quality_tier, 50);
    assert_eq!(cfg.deep_quality_tier, 80);
    assert_eq!(cfg.retention_hours, 168);
    let mut invalid = cfg.clone();
    invalid.retention_hours = 721;
    assert!(invalid.validate().is_err());
    invalid = cfg.clone();
    invalid.deep_quality_tier = 49;
    assert!(invalid.validate().is_err());
    invalid = cfg.clone();
    invalid.sources[0]
        .chat_domains
        .insert("foreign".into(), work());
    assert!(invalid.validate().is_err());
    invalid = cfg.clone();
    invalid.domains.push(DomainBinding {
        domain: work(),
        graph_path: cfg.domains[0].graph_path.clone(),
    });
    assert!(invalid.validate().is_err());
}

#[test]
#[ignore = "spawned with explicit temporary config by the concurrent process test"]
fn process_storage_worker() {
    let cfg = WandaConfig::load(Path::new(
        &std::env::var("WANDA_STORE_TEST_CONFIG").unwrap(),
    ))
    .unwrap();
    let id = std::env::var("WANDA_STORE_TEST_EVENT").unwrap();
    let store = WandaStore::new(cfg).unwrap();
    assert_eq!(
        store
            .ingest("device-secret-fixture", &[event(&id)], 1_000)
            .unwrap()
            .accepted,
        1
    );
}

#[test]
fn concurrent_processes_share_one_owner_token_and_vault_key_without_lost_events() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    let binary = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for i in 0..6 {
        children.push(
            std::process::Command::new(&binary)
                .args(["--ignored", "--exact", "process_storage_worker"])
                .env("WANDA_STORE_TEST_CONFIG", &path)
                .env("WANDA_STORE_TEST_EVENT", format!("process-{i}"))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let owner_token = std::fs::read(&cfg.operator_token_file).unwrap();
    let key = std::fs::read(cfg.state_dir.join("cerebra/vault/vault.key")).unwrap();
    let store = WandaStore::new(cfg.clone()).unwrap();
    assert_eq!(store.events(&domain(), 64, 1_000).unwrap().len(), 6);
    assert_eq!(std::fs::read(cfg.operator_token_file).unwrap(), owner_token);
    assert_eq!(
        std::fs::read(cfg.state_dir.join("cerebra/vault/vault.key")).unwrap(),
        key
    );
}
