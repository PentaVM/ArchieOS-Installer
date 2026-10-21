//! The Wanda service with a local expert: events are read into a persistent, domain-scoped
//! social state in the Vault, questions get states 1 and 2, the owner can see and edit what to
//! expect of each person, and a crisis is never interpreted.
//!
//! The expert here is the tiny preset trained for a handful of steps: it proves the wiring and
//! the integrity checks, not the quality of any answer.

use archie_vault::ContextDomain;
use archie_wanda::config::{DomainBinding, LncmBinding, SourceBinding, WandaConfig};
use archie_wanda::social::{self, EditAction};
use archie_wanda::store::WandaStore;
use archie_wanda::types::{Actor, Author, Coverage, EventSource, WandaEvent};
use archie_wanda_lncm::synth::{self, SynthConfig};
use archie_wanda_lncm::train::{self, TaskMix, TrainOptions};
use std::collections::BTreeMap;
use std::path::Path;

fn tiny_checkpoint(dir: &Path) {
    let examples = synth::generate(&SynthConfig {
        worlds: 2,
        min_turns: 30,
        max_turns: 40,
        ..Default::default()
    })
    .unwrap()
    .examples;
    let device = train::device("cpu").unwrap();
    train::init(dir, "tiny", 400, &examples).unwrap();
    let opts = TrainOptions {
        steps: 4,
        batch: 2,
        warmup: 1,
        save_every: 4,
        eval_every: 0,
        eval_limit: 1,
        ..Default::default()
    };
    train::train_tokenizer(dir, &examples, &[], &opts, &device).unwrap();
    train::train_generator(dir, &examples, &[], &opts, &TaskMix::default(), &device).unwrap();
}

fn config(dir: &Path, checkpoint: &Path) -> WandaConfig {
    let token = dir.join("source.token");
    std::fs::write(&token, "whatsapp-source-fixture\n").unwrap();
    let domain = ContextDomain::personal("owner").unwrap();
    WandaConfig {
        state_dir: dir.join("state"),
        graph_binary: dir.join("missing"),
        models_dir: dir.join("models"),
        operator_token_file: dir.join("owner.token"),
        domains: vec![DomainBinding {
            domain: domain.clone(),
            graph_path: dir.join("graph.tsv"),
        }],
        sources: vec![SourceBinding {
            device_id: "laptop".into(),
            domain,
            source: EventSource::WhatsappVisibleDom,
            token_file: Some(token),
            token_env: None,
            allow_presumed_owner: false,
            enabled: true,
            chat_domains: BTreeMap::new(),
        }],
        quality_tier: 50,
        deep_quality_tier: 80,
        retention_hours: 168,
        parallel_queries: 1,
        lncm: Some(LncmBinding {
            dir: checkpoint.to_path_buf(),
            device: "cpu".into(),
            lang: Some(archie_wanda_lncm::Lang::Es),
            adapters: BTreeMap::new(),
            gate_threshold: 0.0,
            update_threshold: 0.0,
            owner_label: "Yo".into(),
        }),
    }
}

fn message(id: &str, author: Author, sender: &str, text: &str, at: i64) -> WandaEvent {
    WandaEvent {
        schema_version: 1,
        event_id: id.into(),
        device_id: "laptop".into(),
        actor: Actor::ConfirmedOwner,
        author,
        source: EventSource::WhatsappVisibleDom,
        chat_id: "120363042000000000@g.us".into(),
        sender: sender.into(),
        source_message_id: None,
        sent_at_ms: Some(at),
        observed_at_ms: at,
        text: text.into(),
        media_omitted: false,
        coverage: Coverage::VisibleChat,
        monitor_id: None,
        window_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_expert_keeps_a_persistent_state_and_answers_from_it() {
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("lncm");
    tiny_checkpoint(&checkpoint);
    let store = WandaStore::new(config(dir.path(), &checkpoint)).unwrap();
    assert!(store.lncm().is_some());
    let domain = ContextDomain::personal("owner").unwrap();
    let now = archie_wanda::server::now_ms();
    let events = vec![
        message(
            "1",
            Author::Owner,
            "Yo",
            "¿Nos vemos el viernes?",
            now - 60_000,
        ),
        message(
            "2",
            Author::Counterparty,
            "Pedro",
            "Perdón, voy diez minutos tarde.",
            now - 50_000,
        ),
        message(
            "3",
            Author::Counterparty,
            "Ana",
            "El viaje es en julio.",
            now - 40_000,
        ),
        message(
            "4",
            Author::Counterparty,
            "Pedro",
            "Hecho, ya lo envié como dije.",
            now - 30_000,
        ),
    ];
    let ingested = store
        .ingest("whatsapp-source-fixture", &events, now)
        .unwrap();
    assert_eq!(ingested.accepted, 4);

    let report = social::read_new(&store, &domain, now, usize::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(report.turns, 4);
    assert!(report.chunks >= 1);
    // The state persisted: reading again finds nothing new.
    let again = social::read_new(&store, &domain, now, usize::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(again.turns, 0);
    let people = social::people(&store, &domain, now).unwrap();
    let names: Vec<&str> = people.iter().map(|p| p.label.as_str()).collect();
    assert!(
        names.contains(&"Pedro") && names.contains(&"Ana") && names.contains(&"Yo"),
        "{names:?}"
    );
    assert_eq!(
        people.iter().find(|p| p.label == "Pedro").unwrap().messages,
        2
    );

    // With the gate at zero every question is social: states 1 and 2 are computed.
    assert!(
        social::answer(&store, &domain, "¿Llegará tarde Pedro?", None, now)
            .unwrap()
            .is_some()
    );
    let response =
        archie_wanda::runtime::query(&store, &domain, "¿Llegará tarde Pedro?", false, now)
            .await
            .unwrap();
    assert_eq!(
        response.state, "no_model",
        "no reasoning model is registered here"
    );

    // A crisis is answered by the fixed rule, never by a model.
    let crisis = archie_wanda::runtime::query(&store, &domain, "No quiero vivir más", false, now)
        .await
        .unwrap();
    assert_eq!(crisis.state, "crisis");
    assert!(crisis.speech_text().contains("emergencias"));
    assert!(crisis.quick.is_none() && crisis.refined.is_none());

    // The owner can forget someone; the owner cannot be forgotten.
    assert!(social::edit(&store, &domain, "Pedro", None, EditAction::Forget, now).unwrap());
    assert!(!social::edit(&store, &domain, "Yo", None, EditAction::Forget, now).unwrap());
    assert!(social::edit(
        &store,
        &domain,
        "Pedro",
        Some("late"),
        EditAction::Forget,
        now
    )
    .is_err());
    let people = social::people(&store, &domain, now).unwrap();
    assert!(!people.iter().any(|p| p.label == "Pedro"));

    // What IRIS read on a screen enters the same state, in place, through the S-LINK sink;
    // the conversation cursor does not move, so chat reading is unaffected.
    use archie_slink::links::{SceneTextLine, TextOrigin};
    use archie_slink::text::StateUpdate;
    let seen = |track: u64, text: &str| SceneTextLine {
        track,
        text: text.into(),
        origin: TextOrigin::IrisRead,
        confidence: 0.9,
        observed_at_ms: now as u64,
    };
    let lines = [
        seen(1, "Ana: el viaje se mueve a agosto"),
        seen(2, "Reunión 18:00"),
    ];
    let report = social::read_scene(&store, &domain, "screen-1", &lines, now)
        .unwrap()
        .unwrap();
    assert_eq!(report.turns, 2);
    let mut sink = social::SceneSink {
        store: store.clone(),
        domain: domain.clone(),
    };
    sink.update("screen-1", &lines[..1]).unwrap();
    assert!(social::people(&store, &domain, now)
        .unwrap()
        .iter()
        .any(|p| p.label == "Pantalla"));
    assert_eq!(
        social::read_new(&store, &domain, now, usize::MAX)
            .unwrap()
            .unwrap()
            .turns,
        0
    );
    let too_many = vec![seen(9, "x"); social::MAX_SCENE_LINES + 1];
    assert!(social::read_scene(&store, &domain, "screen-1", &too_many, now).is_err());

    // An expert whose weights changed after measurement is refused at start.
    let vq = checkpoint.join("vq.safetensors");
    let mut bytes = std::fs::read(&vq).unwrap();
    bytes[100] ^= 0xff;
    std::fs::write(&vq, bytes).unwrap();
    assert!(WandaStore::new(config(dir.path(), &checkpoint)).is_err());
}
