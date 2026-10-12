//! The unified feeder: every producer → one profile-shaped pillar context →
//! one Cerebra pass. Fixtures only: synthetic hardware addresses, RFC 1918
//! addresses and messages written for this test.
use archie_cerebra::feeder::{
    collect_from_vault, consider_plan, plan, record_device_events, DeviceChange, DeviceEvent,
    DropReason, FeedItem, FeedOutcome, FeederError,
};
use archie_cerebra::focus::{FeedSource, FocusProfile, FocusRule};
use archie_cerebra::pillars::Pillar;
use archie_cerebra::sentry_bridge::{
    record_alerts, AlertInput, AlertOrigin, AlertSeverity, PassRefusal,
};
use archie_cerebra::wanda_bridge::{
    record_chat_context, Category, ChatMessage, ChatPlatform, ContextVector, SenderIdentity,
};
use archie_cerebra::{Cerebra, CerebraConfig};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::{PrivacyGrade, TraceId};
use archie_vault::{ContextDomain, ContextGateway, Vault, VaultKey};

const DEVICE: &str = "54:4e:f0:07:3d:e8";
const WORK_TEXT: &str =
    "Reunión de trabajo: revisar el proyecto con el cliente antes de la entrega";
const SOCIAL_TEXT: &str = "jaja qué buena foto, abrazo amigo";
const URGENT_TEXT: &str = "¿Puedes llamarme? Es urgente";

fn home() -> ContextDomain {
    ContextDomain::personal("owner").unwrap()
}
fn office() -> ContextDomain {
    ContextDomain::work("pentavm", "archie").unwrap()
}
fn vault() -> Vault {
    Vault::open(&VaultKey::from_bytes([9; 32])).unwrap()
}

fn alerts(vault: &mut Vault, domain: &ContextDomain) -> Vec<FeedItem> {
    let batch = record_alerts(
        vault,
        domain,
        "net",
        &[
            AlertInput::measured(
                10_000,
                AlertSeverity::Notice,
                "new-device",
                DEVICE,
                "first seen, at 192.168.1.84",
            ),
            AlertInput::measured(
                12_000,
                AlertSeverity::Warning,
                "cleartext-login",
                DEVICE,
                "opened Telnet to 192.168.1.1",
            ),
            AlertInput::measured(
                15_000,
                AlertSeverity::High,
                "port-scan",
                DEVICE,
                "opened 100 ports of 192.168.1.1",
            ),
        ],
    )
    .unwrap();
    batch.records().iter().map(FeedItem::from_alert).collect()
}

fn devices(vault: &mut Vault, domain: &ContextDomain) -> Vec<FeedItem> {
    record_device_events(
        vault,
        domain,
        &[
            DeviceEvent {
                at_ms: 9_000,
                device: DEVICE.into(),
                change: DeviceChange::Appeared,
                detail: "first seen by inventory and netwatch".into(),
                origin: AlertOrigin::Measurement,
            },
            DeviceEvent {
                at_ms: 9_500,
                device: "02:00:00:00:00:01".into(),
                change: DeviceChange::Disappeared,
                detail: "not seen for a day".into(),
                origin: AlertOrigin::Measurement,
            },
        ],
    )
    .unwrap()
    .iter()
    .map(FeedItem::from_device)
    .collect()
}

fn chats(vault: &mut Vault, domain: &ContextDomain) -> Vec<FeedItem> {
    let sender = SenderIdentity::display("Ana").unwrap();
    let messages = vec![
        ChatMessage::new(
            ChatPlatform::Whatsapp,
            "equipo",
            sender.clone(),
            Some(20_000),
            WORK_TEXT,
        )
        .unwrap(),
        ChatMessage::new(
            ChatPlatform::Whatsapp,
            "amigos",
            sender.clone(),
            Some(21_000),
            SOCIAL_TEXT,
        )
        .unwrap(),
        ChatMessage::new(
            ChatPlatform::notification("com.whatsapp").unwrap(),
            "amigos",
            sender,
            Some(22_000),
            URGENT_TEXT,
        )
        .unwrap(),
    ];
    let outcome = record_chat_context(vault, domain, "wanda", &messages).unwrap();
    let batch = outcome.batch().unwrap();
    batch
        .messages()
        .iter()
        .map(FeedItem::from_message)
        .chain(batch.chats().iter().map(FeedItem::from_chat))
        .collect()
}

fn everything(vault: &mut Vault, domain: &ContextDomain) -> Vec<FeedItem> {
    let mut items = alerts(vault, domain);
    items.extend(devices(vault, domain));
    items.extend(chats(vault, domain));
    items
}

#[test]
fn the_fixture_texts_categorize_the_way_the_profiles_assume() {
    assert_eq!(
        ContextVector::categorize(WORK_TEXT, false)
            .dominant()
            .map(|d| d.0),
        Some(Category::Work)
    );
    assert_ne!(
        ContextVector::categorize(SOCIAL_TEXT, false)
            .dominant()
            .map(|d| d.0),
        Some(Category::Work)
    );
    assert!(ContextVector::categorize(URGENT_TEXT, false).salience() >= 10);
    assert_eq!(
        ContextVector::categorize(SOCIAL_TEXT, false)
            .dominant()
            .map(|d| d.0),
        Some(Category::Social)
    );
}

#[test]
fn every_preset_is_valid_and_unknown_names_are_refused() {
    for name in FocusProfile::PRESETS {
        FocusProfile::preset(name).unwrap();
    }
    assert!(FocusProfile::preset("everything").is_err());
    let mut bad = FocusProfile::preset("red").unwrap();
    bad.rules.push(FocusRule::new(FeedSource::Device, 0, 1));
    assert!(bad.validate().is_err(), "one rule per source");
    bad = FocusProfile::preset("red").unwrap();
    bad.max_items = 65;
    assert!(
        bad.validate().is_err(),
        "the budget never exceeds the pillar context"
    );
}

#[test]
fn red_admits_every_alert_and_device_change_and_drops_everything_social() {
    let mut v = vault();
    let items = everything(&mut v, &home());
    let p = plan(
        &FocusProfile::preset("red").unwrap(),
        &home(),
        "focus-red",
        &items,
    )
    .unwrap();
    assert_eq!(p.admitted().len(), 5);
    assert!(p
        .admitted()
        .iter()
        .all(|a| matches!(a.source, FeedSource::NetwatchAlert | FeedSource::Device)));
    // Alerts outrank devices; within alerts, the port scan comes first.
    assert_eq!(p.admitted()[0].weight, 90);
    assert_eq!(p.admitted()[0].source, FeedSource::NetwatchAlert);
    assert_eq!(p.admitted()[3].source, FeedSource::Device);
    assert!(p
        .dropped()
        .iter()
        .all(|d| d.reason == DropReason::NotInProfile && d.source != FeedSource::NetwatchAlert));
    for (reference, pillars) in p.context().references() {
        assert!(home().contains(reference));
        assert_eq!(
            pillars.iter().copied().collect::<Vec<_>>(),
            vec![Pillar::Sentry]
        );
    }
}

#[test]
fn pantalla_1_never_carries_chat_or_notification_content_and_only_serious_alerts() {
    let mut v = vault();
    let items = everything(&mut v, &home());
    let p = plan(
        &FocusProfile::preset("pantalla-1").unwrap(),
        &home(),
        "screen",
        &items,
    )
    .unwrap();
    let sources: Vec<_> = p.admitted().iter().map(|a| (a.source, a.weight)).collect();
    assert_eq!(
        sources,
        vec![
            (FeedSource::NetwatchAlert, 90),
            (FeedSource::NetwatchAlert, 60),
            (FeedSource::Device, 50),
        ]
    );
    assert!(p
        .dropped()
        .iter()
        .any(|d| d.reason == DropReason::BelowWeight));
    assert!(p
        .context()
        .references()
        .values()
        .all(|p| !p.contains(&Pillar::Wanda)));
}

#[test]
fn personal_and_work_profiles_never_see_the_other_domain() {
    let mut v = vault();
    let mut items = everything(&mut v, &home());
    items.extend(chats(&mut v, &office()));
    let personal = plan(
        &FocusProfile::preset("personal").unwrap(),
        &home(),
        "me",
        &items,
    )
    .unwrap();
    assert!(personal
        .admitted()
        .iter()
        .all(|a| home().contains(&a.reference)));
    assert!(personal
        .dropped()
        .iter()
        .filter(|d| office().contains(&d.reference))
        .all(|d| d.reason == DropReason::OtherDomain));
    // Personal: chats and the urgent notification, plus only the high alert.
    assert!(personal
        .admitted()
        .iter()
        .any(|a| a.source == FeedSource::WandaNotification));
    assert_eq!(
        personal
            .admitted()
            .iter()
            .filter(|a| a.source == FeedSource::NetwatchAlert)
            .count(),
        1
    );

    let work = plan(
        &FocusProfile::preset("trabajo").unwrap(),
        &office(),
        "job",
        &items,
    )
    .unwrap();
    assert!(work
        .admitted()
        .iter()
        .all(|a| office().contains(&a.reference)));
    // Only the work-categorized message (plus chat aggregates) passes.
    let messages: Vec<_> = work
        .admitted()
        .iter()
        .filter(|a| a.source == FeedSource::WandaMessage)
        .collect();
    assert_eq!(messages.len(), 1);
    assert!(work
        .dropped()
        .iter()
        .any(|d| d.reason == DropReason::Category && office().contains(&d.reference)));

    // A profile cannot be planned in the other kind of domain.
    assert!(matches!(
        plan(
            &FocusProfile::preset("trabajo").unwrap(),
            &home(),
            "x",
            &items
        ),
        Err(FeederError::ProfileDomain)
    ));
    assert!(matches!(
        plan(
            &FocusProfile::preset("personal").unwrap(),
            &office(),
            "x",
            &items
        ),
        Err(FeederError::ProfileDomain)
    ));
}

#[test]
fn the_budget_keeps_the_highest_priorities_and_says_what_it_dropped() {
    let mut v = vault();
    let items = everything(&mut v, &home());
    let mut tight = FocusProfile::preset("red").unwrap();
    tight.max_items = 2;
    let p = plan(&tight, &home(), "tight", &items).unwrap();
    assert_eq!(p.admitted().len(), 2);
    assert!(p
        .admitted()
        .iter()
        .all(|a| a.source == FeedSource::NetwatchAlert));
    assert_eq!(
        p.dropped()
            .iter()
            .filter(|d| d.reason == DropReason::Budget)
            .count(),
        3
    );
    // Duplicates are folded, not double-counted.
    let mut doubled = items.clone();
    doubled.extend(items.iter().cloned());
    let p = plan(
        &FocusProfile::preset("red").unwrap(),
        &home(),
        "dup",
        &doubled,
    )
    .unwrap();
    assert_eq!(p.admitted().len(), 5);
    assert_eq!(
        p.dropped()
            .iter()
            .filter(|d| d.reason == DropReason::Duplicate)
            .count(),
        items.len()
    );
}

#[test]
fn a_focused_pass_is_restricted_and_refused_without_a_local_model() {
    let mut v = vault();
    let items = everything(&mut v, &home());
    let p = plan(
        &FocusProfile::preset("red").unwrap(),
        &home(),
        "focus-red",
        &items,
    )
    .unwrap();
    let observation = p.observation().unwrap();
    assert_eq!(observation.privacy, PrivacyGrade::Restricted);
    let cerebra = Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    );
    let trace_id = TraceId::new();
    match consider_plan(&cerebra, &mut v, &ModelRegistry::new(), p, trace_id, 30_000).unwrap() {
        FeedOutcome::Refused {
            pass,
            refusal,
            plan,
        } => {
            assert_eq!(refusal, PassRefusal::NoLocalModel);
            assert_eq!(pass.trace.trace_id, trace_id);
            // Stamped with the newest admitted item's own time.
            assert_eq!(pass.trace.started_at_ms, 15_000);
            assert_eq!(plan.profile(), "red");
        }
        other => panic!("restricted context without a local model is refused: {other:?}"),
    }
    // A profile that admits nothing runs no pass.
    let empty = plan(
        &FocusProfile::preset("pantalla-1").unwrap(),
        &home(),
        "e",
        &[],
    )
    .unwrap();
    assert!(matches!(
        consider_plan(
            &cerebra,
            &mut v,
            &ModelRegistry::new(),
            empty,
            TraceId::new(),
            1
        )
        .unwrap(),
        FeedOutcome::Nothing { .. }
    ));
}

#[test]
fn after_a_restart_the_vault_alone_rebuilds_the_same_items() {
    let mut v = vault();
    let live = everything(&mut v, &home());
    let mut rebuilt = collect_from_vault(&mut v, &home(), 0, 40_000);
    let mut expected = live.clone();
    let key = |i: &FeedItem| i.reference.clone();
    rebuilt.sort_by_key(key);
    expected.sort_by_key(key);
    // Chat aggregates are sealed at 0 (unknown) and keep that time.
    assert_eq!(rebuilt.len(), expected.len());
    for (r, e) in rebuilt.iter().zip(&expected) {
        assert_eq!(
            (r.source, &r.reference, r.weight, r.category),
            (e.source, &e.reference, e.weight, e.category)
        );
    }
    // `since` filters on the sealed (source) time.
    let recent = collect_from_vault(&mut v, &home(), 15_000, 40_000);
    assert!(recent.iter().all(|i| i.at_ms >= 15_000));
    assert!(recent.iter().any(|i| i.source == FeedSource::NetwatchAlert));
}

#[test]
fn device_events_are_validated_sealed_once_and_collide_loudly() {
    let mut v = vault();
    let event = DeviceEvent {
        at_ms: 1_000,
        device: DEVICE.into(),
        change: DeviceChange::AddressChanged,
        detail: "192.168.1.84 → 192.168.1.90".into(),
        origin: AlertOrigin::Hypothesis,
    };
    let first = record_device_events(&mut v, &home(), &[event.clone(), event.clone()]).unwrap();
    assert_eq!(first.len(), 1);
    let again = record_device_events(&mut v, &home(), std::slice::from_ref(&event)).unwrap();
    assert_eq!(again[0].reference(), first[0].reference());
    // A hypothesis counts half and is never upgraded.
    assert_eq!(FeedItem::from_device(&first[0]).weight, 20);
    let mut bad = event;
    bad.detail = "line\nbreak".into();
    assert!(record_device_events(&mut v, &home(), &[bad]).is_err());
}

/// The Vault pillar's documents reach work context by kind, briefs first, and never
/// the shared screen.
#[test]
fn work_documents_enter_as_the_vault_pillar_briefs_first_and_never_on_screen() {
    use archie_cerebra::vault_bridge::{record_document, DocumentInput};
    let mut v = vault();
    for (name, text) in [
        ("notas.md", "Reunión el martes."),
        ("brief.md", "Encargo: puente peatonal."),
        ("normas.txt", "No usar acero corten."),
    ] {
        record_document(
            &mut v,
            &office(),
            "docs",
            &DocumentInput {
                name: name.into(),
                text: text.into(),
                modified_ms: Some(20_000),
            },
        )
        .unwrap();
    }
    let items = collect_from_vault(&mut v, &office(), 0, 30_000);
    let weights: Vec<u8> = items
        .iter()
        .filter(|i| i.source == FeedSource::VaultDocument)
        .map(|i| i.weight)
        .collect();
    assert_eq!(weights.len(), 3);

    let work = plan(
        &FocusProfile::preset("trabajo").unwrap(),
        &office(),
        "work",
        &items,
    )
    .unwrap();
    let documents: Vec<u8> = work
        .admitted()
        .iter()
        .filter(|a| a.source == FeedSource::VaultDocument)
        .map(|a| a.weight)
        .collect();
    assert_eq!(documents, [90, 80, 40], "brief, restriction, note");
    assert!(work
        .context()
        .references()
        .values()
        .any(|p| p.contains(&Pillar::Vault)));

    let screen = plan(
        &FocusProfile::preset("pantalla-1").unwrap(),
        &office(),
        "screen",
        &items,
    )
    .unwrap();
    assert!(screen
        .admitted()
        .iter()
        .all(|a| a.source != FeedSource::VaultDocument));
    // And a personal plan never sees the work documents.
    let home_items = collect_from_vault(&mut v, &home(), 0, 30_000);
    assert!(home_items
        .iter()
        .all(|i| i.source != FeedSource::VaultDocument));
}
