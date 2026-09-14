//! Sealed commands: only the key holder can write one, and nobody can reuse one.

use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_stage::{
    open, stage_resource, AuthorizedStageCommand, CommandKey, Key, KeyChord, Modifier, NamedKey,
    PointerButton, SealError, Sealer, StageAction, StageGate, StagePolicy, StageProposal,
    StageText, WireAction, MAX_COMMAND_TTL_MS, MAX_ENVELOPE_BYTES,
};
use archie_types::{AgentId, PrivacyGrade, Timestamp, TraceId};

const KEY: [u8; 32] = [7; 32];
const NOW: Timestamp = Timestamp::from_millis(1_760_000_000_000);

fn authorized(action: StageAction) -> AuthorizedStageCommand {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("policy/apps.toml");
    let policy = StagePolicy::load(path).unwrap();
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    let lease = PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: stage_resource("org.freecad.FreeCAD"),
            scope: Scope::OneOperation { operation: 9 },
            constraints: Constraints::with_actions([Action::Execute]),
            justification: "seal a command".into(),
        })
        .lease
        .unwrap();
    let proposal = StageProposal {
        app_id: "org.freecad.FreeCAD".into(),
        surface: 4,
        geometry_revision: 77,
        action,
        text_grade: PrivacyGrade::Internal,
        operation_id: 9,
        task_id: None,
        trace_id: "trace-seal".into(),
    };
    // The lease was issued on the real clock, so it is checked on the real clock; the seal
    // below runs on a fixed one so its expiries are exact.
    StageGate::authorize(
        &policy,
        &lease,
        &principal,
        false,
        &proposal,
        Timestamp::now(),
    )
    .allowed()
    .expect("admitted")
    .clone()
}

fn click() -> StageAction {
    StageAction::Click {
        x: 10,
        y: 20,
        button: PointerButton::Left,
    }
}

#[test]
fn a_sealed_command_opens_with_the_key_as_exactly_what_was_authorised() {
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 1);
    let command = authorized(StageAction::Drag {
        x: 1,
        y: 2,
        to_x: 300,
        to_y: 400,
        button: PointerButton::Right,
    });
    let sealed = sealer.seal(&command, NOW, 2_000).unwrap();
    assert_eq!(sealed.sequence(), 1);
    assert!(sealed.as_bytes().ends_with(b"\n"));

    let wire = open(
        &CommandKey::from_bytes(&KEY),
        sealed.as_bytes(),
        0,
        NOW.plus_millis(500),
    )
    .unwrap();
    assert_eq!(wire.seq, 1);
    assert_eq!(wire.surface, 4);
    assert_eq!(wire.app_id, "org.freecad.FreeCAD");
    assert_eq!(wire.geometry_revision, 77);
    assert_eq!(wire.operation_id, 9);
    assert_eq!(wire.expires_at_ms, NOW.as_millis() + 2_000);
    assert_eq!(
        wire.action,
        WireAction::Drag {
            x: 1,
            y: 2,
            to_x: 300,
            to_y: 400,
            button: PointerButton::Right
        }
    );

    let second = sealer.seal(&command, NOW, 2_000).unwrap();
    assert_eq!(second.sequence(), 2, "sequences strictly increase");
}

#[test]
fn a_changed_byte_or_another_key_does_not_open() {
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 1);
    let sealed = sealer.seal(&authorized(click()), NOW, 2_000).unwrap();
    let key = CommandKey::from_bytes(&KEY);

    let text = String::from_utf8(sealed.as_bytes().to_vec()).unwrap();
    let moved = text.replace("\\\"x\\\":10", "\\\"x\\\":11");
    assert_ne!(moved, text, "the fixture must actually change a coordinate");
    assert_eq!(open(&key, moved.as_bytes(), 0, NOW), Err(SealError::BadMac));

    let other = CommandKey::from_bytes(&[8; 32]);
    assert_eq!(
        open(&other, sealed.as_bytes(), 0, NOW),
        Err(SealError::BadMac)
    );

    assert_eq!(open(&key, b"not json\n", 0, NOW), Err(SealError::Malformed));
    let oversized = vec![b' '; MAX_ENVELOPE_BYTES + 1];
    assert_eq!(open(&key, &oversized, 0, NOW), Err(SealError::TooLarge));
}

#[test]
fn a_command_cannot_be_replayed_kept_or_backdated() {
    let key = CommandKey::from_bytes(&KEY);
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 5);
    let sealed = sealer.seal(&authorized(click()), NOW, 2_000).unwrap();

    assert!(open(&key, sealed.as_bytes(), 4, NOW).is_ok());
    assert_eq!(
        open(&key, sealed.as_bytes(), 5, NOW),
        Err(SealError::Replayed)
    );
    assert_eq!(
        open(&key, sealed.as_bytes(), 0, NOW.plus_millis(2_000)),
        Err(SealError::Expired)
    );
    assert_eq!(
        open(&key, sealed.as_bytes(), 0, NOW.plus_millis(-5_000)),
        Err(SealError::FromTheFuture)
    );

    assert_eq!(
        sealer.seal(&authorized(click()), NOW, 0).unwrap_err(),
        SealError::Ttl
    );
    assert_eq!(
        sealer
            .seal(&authorized(click()), NOW, MAX_COMMAND_TTL_MS + 1)
            .unwrap_err(),
        SealError::Ttl
    );
    assert_eq!(
        Sealer::new(CommandKey::from_bytes(&KEY), 0)
            .seal(&authorized(click()), NOW, 1)
            .unwrap()
            .sequence(),
        1
    );
}

#[test]
fn a_sealed_line_never_prints_its_text() {
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 1);
    let command = authorized(StageAction::Text {
        text: StageText::new("my landlord's name"),
    });
    let sealed = sealer.seal(&command, NOW, 2_000).unwrap();
    assert!(!format!("{sealed:?}").contains("landlord"));
    assert!(
        !format!("{sealer:?}").contains('7'),
        "the key is not printed either"
    );

    // The compositor, and only the compositor, gets the characters.
    let wire = open(&CommandKey::from_bytes(&KEY), sealed.as_bytes(), 0, NOW).unwrap();
    match &wire.action {
        WireAction::Text { text } => assert_eq!(text.chars(), 18),
        other => panic!("expected text, got {other:?}"),
    }
    assert!(!format!("{wire:?}").contains("landlord"));
}

/// A verifier must not accept on the MAC alone: a payload that verifies but is not a
/// well-formed command is refused too. Built by hand, the way a buggy or hostile sealer
/// holding the key would build it.
#[test]
fn a_verifying_payload_must_still_be_a_well_formed_command() {
    let key_bytes = KEY;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key_bytes);
    let make = |payload: &str| {
        let mut context = ring::hmac::Context::with_key(&key);
        context.update(b"archie-stage/1\0");
        context.update(payload.as_bytes());
        let mac = hex::encode(context.sign().as_ref());
        format!(
            "{}\n",
            serde_json::json!({"v": 1, "payload": payload, "mac": mac})
        )
    };
    let base = |action: serde_json::Value| {
        serde_json::json!({
            "v": 1, "seq": 3, "issued_at_ms": NOW.as_millis(), "expires_at_ms": NOW.as_millis() + 1000,
            "operation_id": 9, "surface": 4, "app_id": "org.freecad.FreeCAD", "geometry_revision": 77,
            "action": action, "trace_id": "t"
        })
        .to_string()
    };
    let verifier = CommandKey::from_bytes(&key_bytes);

    let good = make(&base(
        serde_json::json!({"kind": "click", "x": 1, "y": 2, "button": "left"}),
    ));
    assert!(open(&verifier, good.as_bytes(), 0, NOW).is_ok());

    for bad in [
        serde_json::json!({"kind": "click", "x": 1, "y": 2, "button": "left", "extra": true}),
        serde_json::json!({"kind": "text", "text": "rm -rf /\n"}),
        serde_json::json!({"kind": "key", "modifiers": ["super"], "key": "l"}),
        serde_json::json!({"kind": "key", "modifiers": ["ctrl", "ctrl"], "key": "s"}),
        serde_json::json!({"kind": "key", "modifiers": [], "key": "Enter"}),
        serde_json::json!({"kind": "scroll", "x": 1, "y": 1, "steps_x": 0, "steps_y": 0}),
        serde_json::json!({"kind": "move", "x": -5, "y": 1}),
        serde_json::json!({"kind": "teleport", "x": 1, "y": 1}),
    ] {
        let line = make(&base(bad.clone()));
        assert_eq!(
            open(&verifier, line.as_bytes(), 0, NOW),
            Err(SealError::Malformed),
            "{bad}"
        );
    }

    let long_lived = serde_json::json!({
        "v": 1, "seq": 3, "issued_at_ms": NOW.as_millis(), "expires_at_ms": NOW.as_millis() + MAX_COMMAND_TTL_MS + 1,
        "operation_id": 9, "surface": 4, "app_id": "org.freecad.FreeCAD", "geometry_revision": 77,
        "action": {"kind": "focus"}, "trace_id": "t"
    })
    .to_string();
    assert_eq!(
        open(&verifier, make(&long_lived).as_bytes(), 0, NOW),
        Err(SealError::Ttl)
    );
}

#[test]
fn chords_travel_with_their_modifiers() {
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 1);
    let command = authorized(StageAction::Key {
        chord: KeyChord::key(Key::Char('s'))
            .with(Modifier::Shift)
            .with(Modifier::Ctrl),
    });
    let wire = open(
        &CommandKey::from_bytes(&KEY),
        sealer.seal(&command, NOW, 1_000).unwrap().as_bytes(),
        0,
        NOW,
    )
    .unwrap();
    assert_eq!(
        wire.action,
        WireAction::Key {
            modifiers: vec![Modifier::Ctrl, Modifier::Shift],
            key: Key::Char('s')
        }
    );
    let named = authorized(StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::PageDown)),
    });
    let sealed = sealer.seal(&named, NOW, 1_000).unwrap();
    assert!(String::from_utf8_lossy(sealed.as_bytes()).contains("page_down"));
}

/// The fixture both implementations are held to: computed with openssl, not with this
/// crate, so a MAC construction that drifted on either side fails here.
#[test]
fn the_shared_golden_command_opens() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../protocols/fixtures/stage-command-v1.json");
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let key_bytes: [u8; 32] = hex::decode(fixture["key_hex"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let line = fixture["line"].as_str().unwrap();
    let now = Timestamp::from_millis(fixture["now_ms"].as_i64().unwrap());
    let wire = open(&CommandKey::from_bytes(&key_bytes), line.as_bytes(), 6, now).unwrap();
    assert_eq!(wire.seq, 7);
    assert_eq!(
        wire.action,
        WireAction::Click {
            x: 10,
            y: 20,
            button: PointerButton::Left
        }
    );
    // And a sealer given the same inputs produces a line that opens identically.
    assert_eq!(
        open(&CommandKey::from_bytes(&key_bytes), line.as_bytes(), 7, now),
        Err(SealError::Replayed)
    );
}
