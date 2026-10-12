//! The Wanda chat-context bridge: exports and notification dumps become categorized,
//! graded, deduplicated context records for the Wanda pillar.
//!
//! Every fixture here is synthetic: invented names, invented numbers, invented
//! timestamps. No export, device or notification of anyone's took part.

use archie_cerebra::pillars::Pillar;
use archie_cerebra::wanda_bridge::{
    parse_instagram_export, parse_notification_dump, parse_whatsapp_export, record_chat_context,
    Category, ChatBatchOutcome, ChatMessage, ChatPlatform, ContextVector, ParseError,
    SenderIdentity, WandaError, EXCERPT_SALIENCE_THRESHOLD, MAX_MESSAGES_PER_BATCH,
};
use archie_types::PrivacyGrade;
use archie_vault::{AuditAction, ContextDomain, Vault, VaultKey};

/// A fixture salt. A real deployment reads it from local config; tests use a constant.
const SALT: &str = "wanda-fixture-salt";

fn domain() -> ContextDomain {
    ContextDomain::personal("owner").unwrap()
}

fn open_vault() -> Vault {
    Vault::open(&VaultKey::from_bytes([7; 32])).unwrap()
}

/// A synthetic WhatsApp export exercising both timestamp layouts, a system line, a
/// media marker, a multiline message and a phone-number sender. Fixture: every name,
/// number and line was written for this test.
const WHATSAPP_FIXTURE: &str = "\u{feff}[25/12/2024, 10:30:45] Ana: Hola, ¿qué tal?
25/12/2024, 10:31 - Carlos: todo bien, ¿y tú?
[25/12/2024, 10:32:10] Ana: <Media omitted>
25/12/2024, 10:33 - Messages and calls are end-to-end encrypted. Learn more.
[25/12/2024, 10:34:00] Carlos: nos vemos mañana
en la oficina para la presentación
25/12/2024, 10:35 - +34 600 123 456: ¿Puedes revisar el informe?
";

/// A synthetic Instagram data-export file: a plain message, a share, a media-only
/// message, and one malformed entry. The first message's content is mojibake on
/// purpose — the parser keeps it as decoded, per the module's documented assumption.
const INSTAGRAM_FIXTURE: &str = r#"{
  "participants": [{"name": "Ana"}, {"name": "Nelson"}],
  "messages": [
    {"sender_name": "Ana", "timestamp_ms": 1735122645000, "content": "Ãsta es una prueba"},
    {"sender_name": "Nelson", "timestamp_ms": 1735122700000,
     "share": {"link": "https://example.invalid/fixture", "share_text": "mira esto"}},
    {"sender_name": "Ana", "timestamp_ms": 1735122800000,
     "photos": [{"uri": "media/1.jpg"}]},
    {"timestamp_ms": 1735122900000, "content": "no sender, no message"}
  ]
}"#;

/// A synthetic notification dump: one duplicate pair, one malformed line, one entry
/// with an empty package.
const NOTIFICATION_FIXTURE: &str = r#"{"package":"com.whatsapp","title":"Ana","text":"¿Vienes mañana?","posted_at_ms":1735122645000}
{"package":"com.whatsapp","title":"Ana","text":"¿Vienes mañana?","posted_at_ms":1735122645000}
{"package":"com.instagram","title":"carlos_ig","text":"te paso el plan del finde","posted_at_ms":1735122700000}
this is not json
{"package":"","title":"x","text":"y","posted_at_ms":1}
"#;

#[test]
fn whatsapp_export_parses_both_layouts_media_system_lines_and_multiline() {
    let outcome = parse_whatsapp_export("family", WHATSAPP_FIXTURE, SALT).unwrap();
    // Five messages: two plain, one media, one multiline, one from a number.
    assert_eq!(outcome.messages.len(), 5);
    assert_eq!(
        outcome.system_lines, 1,
        "the encryption notice is skipped and counted"
    );
    assert_eq!(outcome.media_omitted, 1);
    assert_eq!(outcome.malformed, 0);

    let first = &outcome.messages[0];
    assert_eq!(first.platform(), &ChatPlatform::Whatsapp);
    assert_eq!(first.chat_id(), "family");
    // 2024-12-25 10:30:45 UTC — the source's own timestamp, carried verbatim.
    assert_eq!(first.sent_at(), Some(1_735_122_645_000));
    assert_eq!(first.text(), "Hola, ¿qué tal?");

    // The media marker is a flag, not text.
    let media = &outcome.messages[2];
    assert!(media.media_omitted());
    assert!(media.text().is_empty());

    // The multiline message is joined, with the newline folded to a space.
    let multiline = &outcome.messages[3];
    assert_eq!(
        multiline.text(),
        "nos vemos mañana en la oficina para la presentación"
    );

    // The phone-number sender is hashed; the raw number is nowhere.
    let numbered = &outcome.messages[4];
    assert!(matches!(numbered.sender(), SenderIdentity::NumberHash(_)));
    assert!(!numbered.sender().as_str().contains("600"));
}

#[test]
fn instagram_export_parses_shares_media_and_keeps_mojibake_as_decoded() {
    let outcome = parse_instagram_export(INSTAGRAM_FIXTURE, SALT).unwrap();
    assert_eq!(
        outcome.messages.len(),
        3,
        "the sender-less entry is malformed"
    );
    assert_eq!(outcome.malformed, 1);

    // The chat id is derived from the participants, salted: stable, and never a name.
    let chat_id = outcome.messages[0].chat_id().to_string();
    assert!(chat_id.starts_with("ig-"), "{chat_id}");
    assert!(!chat_id.contains("Ana"));

    let plain = &outcome.messages[0];
    assert_eq!(plain.platform(), &ChatPlatform::Instagram);
    assert_eq!(plain.sent_at(), Some(1_735_122_645_000));
    // Mojibake is kept exactly as decoded — documented, not repaired.
    assert_eq!(plain.text(), "Ãsta es una prueba");

    let share = &outcome.messages[1];
    assert_eq!(
        share.text(),
        "mira esto",
        "share text stands in for content"
    );

    let photo = &outcome.messages[2];
    assert!(photo.media_omitted());
    assert_eq!(outcome.media_omitted, 1);
}

#[test]
fn notification_dump_dedupes_and_counts_malformed_lines() {
    let outcome = parse_notification_dump(NOTIFICATION_FIXTURE, SALT).unwrap();
    assert_eq!(outcome.messages.len(), 2);
    assert_eq!(outcome.duplicates, 1, "the repeated notification folds");
    assert_eq!(outcome.malformed, 2, "not-JSON and empty package");

    let whatsapp = &outcome.messages[0];
    assert_eq!(
        whatsapp.platform(),
        &ChatPlatform::notification("com.whatsapp").unwrap()
    );
    assert_eq!(whatsapp.chat_id(), "com.whatsapp");
    assert_eq!(whatsapp.sent_at(), Some(1_735_122_645_000));

    let instagram = &outcome.messages[1];
    assert_eq!(instagram.chat_id(), "com.instagram");
    assert_eq!(instagram.sender().as_str(), "carlos_ig");
}

#[test]
fn a_number_sender_is_hashed_salted_and_never_the_raw_number() {
    let one = SenderIdentity::from_number("+34 600 123 456", SALT).unwrap();
    let two = SenderIdentity::from_number("34600123456", SALT).unwrap();
    let other_salt = SenderIdentity::from_number("+34 600 123 456", "another-salt").unwrap();
    let SenderIdentity::NumberHash(hash) = &one else {
        panic!("a number sender is a hash");
    };
    // Stable, formatting-independent, salt-keyed, and shaped like a hash.
    assert_eq!(
        one, two,
        "the same number hashes the same however formatted"
    );
    assert_ne!(one, other_salt, "the salt keys the hash");
    assert_eq!(hash.len(), 32);
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));

    // And through a whole batch, the raw number appears in no stored record.
    let outcome = parse_whatsapp_export("family", WHATSAPP_FIXTURE, SALT).unwrap();
    let mut vault = open_vault();
    let batch = match record_chat_context(&mut vault, &domain(), "wanda-ingest", &outcome.messages)
        .unwrap()
    {
        ChatBatchOutcome::Recorded(batch) => batch,
        ChatBatchOutcome::NoMessages => panic!("the fixture has messages"),
    };
    for record in batch.messages() {
        assert!(!record.content().contains("600123456"));
        assert!(!record.content().contains("+34 600 123 456"));
    }
    // A display name, on the other hand, is kept as-is.
    let ana = SenderIdentity::display("Ana").unwrap();
    assert_eq!(ana.as_str(), "Ana");
}

#[test]
fn the_categorizer_scores_bilingual_keywords_and_questions() {
    // A work message is work-dominant, in Spanish...
    let work_es = ContextVector::categorize("El informe para la reunión del cliente", false);
    assert_eq!(work_es.dominant(), Some((Category::Work, 100)));
    // ...and in English.
    let work_en = ContextVector::categorize("Can you review the report before the meeting?", false);
    assert_eq!(work_en.dominant(), Some((Category::Work, 80)));

    // An urgent question scores urgency and question; the question dominates.
    let urgent = ContextVector::categorize("¿Dónde estás? ¡Urgente!", false);
    assert!(urgent.score(Category::Urgency) >= 40);
    assert!(
        urgent.score(Category::Question) >= 90,
        "mark plus keyword: {urgent:?}"
    );
    assert_eq!(urgent.dominant().unwrap().0, Category::Question);

    // Scores are clamped at 100 no matter how many keywords hit.
    let saturated = ContextVector::categorize(
        "URGENTE emergencia: la factura del banco vence mañana, ¿puedes pagarla ahora mismo?",
        false,
    );
    assert_eq!(saturated.score(Category::Urgency), 100);
    assert!(saturated.salience() >= EXCERPT_SALIENCE_THRESHOLD);

    // Bare words match whole tokens only: "momento" is not "mom".
    let token = ContextVector::categorize("un momento por favor", false);
    assert_eq!(token.score(Category::Family), 0);

    // No signal, no dominant category — and media halves salience.
    let none = ContextVector::categorize("xxxx", false);
    assert_eq!(none.dominant(), None);
    let media = ContextVector::categorize("¿Dónde estás? ¡Urgente!", true);
    assert_eq!(media.salience(), urgent.salience() / 2);
}

#[test]
fn records_carry_source_times_restricted_grade_and_wanda_references() {
    let outcome = parse_whatsapp_export("family", WHATSAPP_FIXTURE, SALT).unwrap();
    let domain = domain();
    let mut vault = open_vault();
    let batch = match record_chat_context(&mut vault, &domain, "wanda-ingest", &outcome.messages)
        .unwrap()
    {
        ChatBatchOutcome::Recorded(batch) => batch,
        ChatBatchOutcome::NoMessages => panic!("the fixture has messages"),
    };

    // One chat aggregate for the one chat, and every reference is Wanda's.
    assert_eq!(batch.chats().len(), 1);
    assert_eq!(batch.chats()[0].message_count(), 5);
    for record in batch
        .messages()
        .iter()
        .map(|r| r.reference())
        .chain(batch.chats().iter().map(|c| c.reference()))
    {
        assert!(domain.contains(record), "{record}");
        assert!(batch.context().references()[record].contains(&Pillar::Wanda));
    }

    // The vault holds what the records say: source timestamps, Restricted grade,
    // PrivateFact class — verified through disclosure and the audit log.
    for record in batch.messages() {
        let disclosure = vault
            .disclose("test", record.reference(), PrivacyGrade::Restricted, 20_000)
            .unwrap();
        assert_eq!(disclosure.text(), Some(record.content()));
        if let Some(at) = record.sent_at() {
            assert!(record.content().contains(&format!("\"sent_at\":{at}")));
        }
    }
    let stores = vault
        .audit_log()
        .iter()
        .filter(|entry| entry.action == AuditAction::Store)
        .count();
    assert_eq!(stores, batch.messages().len() + batch.chats().len());
    assert!(vault
        .audit_log()
        .iter()
        .all(|entry| entry.grade == PrivacyGrade::Restricted));
}

#[test]
fn raw_text_stays_out_of_the_vault_below_the_salience_threshold() {
    let low = ChatMessage::new(
        ChatPlatform::Whatsapp,
        "family",
        SenderIdentity::display("Ana").unwrap(),
        Some(1_735_122_645_000),
        "jaja muy bueno",
    )
    .unwrap();
    let high = ChatMessage::new(
        ChatPlatform::Whatsapp,
        "family",
        SenderIdentity::display("Ana").unwrap(),
        Some(1_735_123_000_000),
        "URGENTE emergencia: la factura del banco vence mañana, ¿puedes pagarla ahora mismo? Es entre tú y yo",
    )
    .unwrap();
    assert!(ContextVector::categorize(low.text(), false).salience() < EXCERPT_SALIENCE_THRESHOLD);

    let mut vault = open_vault();
    let batch =
        match record_chat_context(&mut vault, &domain(), "wanda-ingest", &[low, high]).unwrap() {
            ChatBatchOutcome::Recorded(batch) => batch,
            ChatBatchOutcome::NoMessages => panic!("two messages went in"),
        };
    assert_eq!(batch.messages().len(), 2);

    let low_record = &batch.messages()[0];
    assert!(low_record.excerpt().is_none());
    assert!(
        !low_record.content().contains("jaja"),
        "no excerpt below threshold"
    );
    assert!(!low_record.content().contains("\"excerpt\""));

    let high_record = &batch.messages()[1];
    let excerpt = high_record
        .excerpt()
        .expect("a high-salience message leaves an excerpt");
    assert!(excerpt.chars().count() <= 280);
    assert!(excerpt.starts_with("URGENTE"));
    assert!(high_record.content().contains("\"excerpt\""));
    // The chat aggregate holds digests, never excerpts.
    assert!(!batch.chats()[0].content().contains("URGENTE"));
}

#[test]
fn a_missing_source_time_is_recorded_as_null_not_invented() {
    let message = ChatMessage::new(
        ChatPlatform::Instagram,
        "ig-test",
        SenderIdentity::display("Ana").unwrap(),
        None,
        "hola sin hora",
    )
    .unwrap();
    let mut vault = open_vault();
    let batch =
        match record_chat_context(&mut vault, &domain(), "wanda-ingest", &[message]).unwrap() {
            ChatBatchOutcome::Recorded(batch) => batch,
            ChatBatchOutcome::NoMessages => panic!("one message went in"),
        };
    assert!(batch.messages()[0].content().contains("\"sent_at\":null"));
}

#[test]
fn replays_dedupe_and_a_foreign_record_at_the_reference_is_a_collision() {
    let outcome = parse_whatsapp_export("family", WHATSAPP_FIXTURE, SALT).unwrap();
    let domain = domain();
    let mut vault = open_vault();

    let first = match record_chat_context(&mut vault, &domain, "t", &outcome.messages).unwrap() {
        ChatBatchOutcome::Recorded(batch) => batch,
        ChatBatchOutcome::NoMessages => panic!("the fixture has messages"),
    };
    let stores = vault
        .audit_log()
        .iter()
        .filter(|entry| entry.action == AuditAction::Store)
        .count();

    // Re-ingesting the same export after a restart stores nothing twice.
    let second = match record_chat_context(&mut vault, &domain, "t", &outcome.messages).unwrap() {
        ChatBatchOutcome::Recorded(batch) => batch,
        ChatBatchOutcome::NoMessages => panic!("the fixture has messages"),
    };
    let first_refs: Vec<_> = first
        .messages()
        .iter()
        .map(|r| r.reference().clone())
        .collect();
    let second_refs: Vec<_> = second
        .messages()
        .iter()
        .map(|r| r.reference().clone())
        .collect();
    assert_eq!(first_refs, second_refs);
    assert_eq!(
        vault
            .audit_log()
            .iter()
            .filter(|entry| entry.action == AuditAction::Store)
            .count(),
        stores,
        "a replay stores nothing new"
    );

    // Something else sitting at a message's reference is named, never overwritten.
    let reference = first_refs[0].clone();
    let mut foreign = open_vault();
    foreign
        .store(
            "test",
            reference.clone(),
            archie_vault::EntryClass::PrivateFact,
            PrivacyGrade::Restricted,
            "something else entirely",
            1,
            None,
        )
        .unwrap();
    assert!(matches!(
        record_chat_context(&mut foreign, &domain, "t", &outcome.messages),
        Err(WandaError::ReferenceCollision { reference: found }) if found == reference
    ));
}

#[test]
fn empty_oversized_and_malformed_inputs_are_typed_never_panics() {
    let domain = domain();

    // An empty batch is the typed NoMessages outcome, and nothing is stored.
    let mut vault = open_vault();
    let outcome = record_chat_context(&mut vault, &domain, "t", &[]).unwrap();
    assert!(matches!(outcome, ChatBatchOutcome::NoMessages));
    assert!(outcome.batch().is_none());
    assert!(vault.audit_log().is_empty());

    // An oversized batch is refused before the vault is touched.
    let message = || {
        ChatMessage::new(
            ChatPlatform::Whatsapp,
            "family",
            SenderIdentity::display("Ana").unwrap(),
            Some(1_735_122_645_000),
            "hola",
        )
        .unwrap()
    };
    let oversized: Vec<_> = (0..=MAX_MESSAGES_PER_BATCH).map(|_| message()).collect();
    assert!(matches!(
        record_chat_context(&mut vault, &domain, "t", &oversized),
        Err(WandaError::BatchTooLarge { found, cap })
            if found == MAX_MESSAGES_PER_BATCH + 1 && cap == MAX_MESSAGES_PER_BATCH
    ));
    assert!(vault.audit_log().is_empty(), "nothing was stored");

    // Invalid messages fail at construction with the field named.
    for (message, field) in [
        (
            ChatMessage::new(
                ChatPlatform::Whatsapp,
                "",
                SenderIdentity::display("Ana").unwrap(),
                Some(1),
                "hola",
            ),
            "chat_id",
        ),
        (
            ChatMessage::new(
                ChatPlatform::Whatsapp,
                "family",
                SenderIdentity::display("Ana").unwrap(),
                Some(0),
                "hola",
            ),
            "sent_at",
        ),
        (
            ChatMessage::new(
                ChatPlatform::Whatsapp,
                "family",
                SenderIdentity::display("Ana").unwrap(),
                Some(1),
                "x".repeat(2001),
            ),
            "text",
        ),
        (
            ChatMessage::new(
                ChatPlatform::Whatsapp,
                "family",
                SenderIdentity::display("Ana").unwrap(),
                Some(1),
                "",
            ),
            "text",
        ),
    ] {
        match message {
            Err(WandaError::InvalidMessage { field: found, .. }) => assert_eq!(found, field),
            other => panic!("expected a typed invalid-message error for {field}: {other:?}"),
        }
    }

    // Unsalable salts and unreadable files are typed errors with counts.
    assert!(matches!(
        SenderIdentity::from_number("+34 600 123 456", ""),
        Err(WandaError::InvalidSalt)
    ));
    assert!(matches!(
        parse_whatsapp_export("family", "", SALT),
        Err(ParseError::NoMessages {
            malformed: 0,
            system: 0
        })
    ));
    assert!(matches!(
        parse_instagram_export("not json", SALT),
        Err(ParseError::MalformedDocument { .. })
    ));
    assert!(matches!(
        parse_instagram_export(r#"{"messages": [{"content": "no sender"}]}"#, SALT),
        Err(ParseError::NoMessages { malformed: 1, .. })
    ));
    assert!(matches!(
        parse_notification_dump("", SALT),
        Err(ParseError::NoMessages { .. })
    ));
}
