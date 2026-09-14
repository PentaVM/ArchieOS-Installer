//! The stage gate: which actions reach the compositor, and which never do.

use archie_policy::{
    Action, CapabilityRequest, Constraints, Lease, LeaseRejection, PolicyEngine, Principal,
    Resource, Scope,
};
use archie_stage::{
    stage_resource, ActionKind, Key, KeyChord, Modifier, NamedKey, PointerButton, StageAction,
    StageAdmission, StageGate, StagePolicy, StageProposal, StageRefusal, StageText,
};
use archie_types::{AgentId, PrivacyGrade, Timestamp, TraceId};

const OPERATION: u64 = 41;
const FREECAD: &str = "org.freecad.FreeCAD";
const TERMINAL: &str = "foot";

fn policy() -> StagePolicy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("policy/apps.toml");
    StagePolicy::load(path).expect("the shipped stage policy must load")
}

fn agent() -> Principal {
    Principal::Agent {
        agent: AgentId::new(),
    }
}

fn lease_for(principal: &Principal, resource: Resource, action: Action, operation: u64) -> Lease {
    PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource,
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([action]),
            justification: "operate one application on the stage".into(),
        })
        .lease
        .expect("policy grants a one-operation lease")
}

fn proposal(app: &str, action: StageAction) -> StageProposal {
    StageProposal {
        app_id: app.into(),
        surface: 3,
        geometry_revision: 12,
        action,
        text_grade: PrivacyGrade::Public,
        operation_id: OPERATION,
        task_id: None,
        trace_id: "trace-1".into(),
    }
}

fn click() -> StageAction {
    StageAction::Click {
        x: 120,
        y: 80,
        button: PointerButton::Left,
    }
}

fn gate(app: &str, action: StageAction) -> StageAdmission {
    let principal = agent();
    let lease = lease_for(&principal, stage_resource(app), Action::Execute, OPERATION);
    StageGate::authorize(
        &policy(),
        &lease,
        &principal,
        false,
        &proposal(app, action),
        Timestamp::now(),
    )
}

fn refusal(admission: StageAdmission) -> StageRefusal {
    admission
        .refusal()
        .cloned()
        .unwrap_or_else(|| panic!("expected a refusal, got {admission:?}"))
}

#[test]
fn a_leased_click_on_a_listed_application_is_admitted_unchanged() {
    let admission = gate(FREECAD, click());
    let command = admission
        .allowed()
        .expect("a click on FreeCAD under its lease");
    assert_eq!(command.app_id(), FREECAD);
    assert_eq!(command.surface(), 3);
    assert_eq!(command.geometry_revision(), 12);
    assert_eq!(command.operation_id(), OPERATION);
    assert_eq!(command.action(), &click());
}

#[test]
fn an_application_missing_from_the_policy_gets_nothing_whatever_the_lease_says() {
    assert_eq!(
        refusal(gate("org.mozilla.firefox", click())),
        StageRefusal::AppNotInPolicy("org.mozilla.firefox".into())
    );
}

#[test]
fn only_the_action_kinds_an_entry_lists_are_allowed() {
    let drag = StageAction::Drag {
        x: 1,
        y: 1,
        to_x: 50,
        to_y: 50,
        button: PointerButton::Left,
    };
    assert_eq!(
        refusal(gate(TERMINAL, drag)),
        StageRefusal::ActionNotAllowed {
            app_id: TERMINAL.into(),
            kind: ActionKind::Drag
        }
    );
}

#[test]
fn no_lease_for_this_application_means_no_command() {
    let principal = agent();
    let policy = policy();
    let other = lease_for(
        &principal,
        stage_resource(TERMINAL),
        Action::Execute,
        OPERATION,
    );
    let read = lease_for(&principal, stage_resource(FREECAD), Action::Read, OPERATION);
    let elsewhere = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION + 1,
    );
    let mine = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION,
    );
    let mut revoked = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION,
    );
    revoked.revoke();
    // Read after every lease is issued: an instant before one of them is a clock that went
    // backwards, which the lease refuses before it looks at anything else.
    let now = Timestamp::now();

    // A lease for another application.
    let admission = StageGate::authorize(
        &policy,
        &other,
        &principal,
        false,
        &proposal(FREECAD, click()),
        now,
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::Lease(LeaseRejection::ResourceMismatch)
    );

    // A read-only lease.
    let admission = StageGate::authorize(
        &policy,
        &read,
        &principal,
        false,
        &proposal(FREECAD, click()),
        now,
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::Lease(LeaseRejection::ActionNotPermitted)
    );

    // A lease granted for a different prepared operation.
    let admission = StageGate::authorize(
        &policy,
        &elsewhere,
        &principal,
        false,
        &proposal(FREECAD, click()),
        now,
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::Lease(LeaseRejection::OperationMismatch)
    );

    // Somebody else's lease.
    let admission = StageGate::authorize(
        &policy,
        &mine,
        &agent(),
        false,
        &proposal(FREECAD, click()),
        now,
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::Lease(LeaseRejection::PrincipalMismatch)
    );

    // A revoked one.
    let admission = StageGate::authorize(
        &policy,
        &revoked,
        &principal,
        false,
        &proposal(FREECAD, click()),
        now,
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::Lease(LeaseRejection::Revoked)
    );

    let mut unnamed = proposal(FREECAD, click());
    unnamed.operation_id = 0;
    let admission = StageGate::authorize(&policy, &mine, &principal, false, &unnamed, now);
    assert_eq!(refusal(admission), StageRefusal::NoOperation);
}

#[test]
fn the_gate_checks_the_lease_but_does_not_spend_it() {
    let principal = agent();
    let lease = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION,
    );
    let before = lease.uses;
    let admission = StageGate::authorize(
        &policy(),
        &lease,
        &principal,
        false,
        &proposal(FREECAD, click()),
        Timestamp::now(),
    );
    assert!(admission.allowed().is_some());
    assert_eq!(
        lease.uses, before,
        "a lease is spent when the compositor's receipt says the action ran"
    );
}

#[test]
fn typed_text_is_graded_bounded_and_plain() {
    let principal = agent();
    let lease = lease_for(
        &principal,
        stage_resource(TERMINAL),
        Action::Execute,
        OPERATION,
    );
    let policy = policy();
    let now = Timestamp::now();
    let typing = |text: &str, grade: PrivacyGrade| {
        let mut p = proposal(
            TERMINAL,
            StageAction::Text {
                text: StageText::new(text),
            },
        );
        p.text_grade = grade;
        StageGate::authorize(&policy, &lease, &principal, false, &p, now)
    };

    assert!(typing("cmake --build build", PrivacyGrade::Internal)
        .allowed()
        .is_some());
    assert_eq!(
        refusal(typing("hunter2", PrivacyGrade::Secret)),
        StageRefusal::SecretText
    );
    assert_eq!(
        refusal(typing("the patient's address", PrivacyGrade::Sensitive)),
        StageRefusal::TextExceedsAppGrade {
            app_id: TERMINAL.into(),
            grade: PrivacyGrade::Sensitive
        }
    );
    assert_eq!(
        refusal(typing("", PrivacyGrade::Public)),
        StageRefusal::TextOutOfBounds {
            app_id: TERMINAL.into()
        }
    );
    assert_eq!(
        refusal(typing(&"x".repeat(513), PrivacyGrade::Public)),
        StageRefusal::TextOutOfBounds {
            app_id: TERMINAL.into()
        }
    );
    assert!(
        typing(&"ñ".repeat(512), PrivacyGrade::Public)
            .allowed()
            .is_some(),
        "counted in characters"
    );
    // A newline inside text would run the command without the Enter key's approval.
    assert_eq!(
        refusal(typing("rm -rf ./build\n", PrivacyGrade::Public)),
        StageRefusal::TextNotPlain
    );
    assert_eq!(
        refusal(typing("clear\u{1b}[2J", PrivacyGrade::Public)),
        StageRefusal::TextNotPlain
    );
}

#[test]
fn a_refusal_or_receipt_never_contains_the_text() {
    let private = "my landlord's name";
    let principal = agent();
    let lease = lease_for(
        &principal,
        stage_resource(TERMINAL),
        Action::Execute,
        OPERATION,
    );
    let mut p = proposal(
        TERMINAL,
        StageAction::Text {
            text: StageText::new(private),
        },
    );
    p.text_grade = PrivacyGrade::Internal;
    let admission =
        StageGate::authorize(&policy(), &lease, &principal, false, &p, Timestamp::now());
    let command = admission.allowed().unwrap();

    for rendered in [
        format!("{p:?}"),
        format!("{admission:?}"),
        format!("{command:?}"),
    ] {
        assert!(!rendered.contains("landlord"), "{rendered}");
    }
    let receipt = command.receipt();
    assert_eq!(receipt.text_chars, private.chars().count());
    assert_eq!(receipt.grade, PrivacyGrade::Internal);
    let written = serde_json::to_string(&receipt).unwrap();
    assert!(!written.contains("landlord"), "{written}");

    p.text_grade = PrivacyGrade::Secret;
    let refused = StageGate::authorize(&policy(), &lease, &principal, false, &p, Timestamp::now());
    let refusal = refused.refusal().unwrap();
    assert!(!format!("{refusal} {refusal:?}").contains("landlord"));
    assert!(!refusal.code().contains(' '));
}

#[test]
fn system_chords_are_never_sent_and_modifiers_need_a_grant() {
    let reserved = [
        KeyChord::key(Key::Char('l')).with(Modifier::Super),
        KeyChord::key(Key::Named(NamedKey::F2))
            .with(Modifier::Ctrl)
            .with(Modifier::Alt),
        KeyChord::key(Key::Named(NamedKey::Delete))
            .with(Modifier::Ctrl)
            .with(Modifier::Alt),
    ];
    for chord in reserved {
        assert_eq!(
            refusal(gate(FREECAD, StageAction::Key { chord })),
            StageRefusal::ChordReserved
        );
    }

    let mut strict = StagePolicy::new();
    let mut entry = policy().app(FREECAD).unwrap().clone();
    entry.allow_modified_keys = false;
    strict.allow(entry).unwrap();
    let principal = agent();
    let lease = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION,
    );
    let save = StageAction::Key {
        chord: KeyChord::key(Key::Char('s')).with(Modifier::Ctrl),
    };
    let admission = StageGate::authorize(
        &strict,
        &lease,
        &principal,
        false,
        &proposal(FREECAD, save),
        Timestamp::now(),
    );
    assert_eq!(
        refusal(admission),
        StageRefusal::ModifiedKeysNotAllowed(FREECAD.into())
    );

    let plain = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Tab)),
    };
    let admission = StageGate::authorize(
        &strict,
        &lease,
        &principal,
        false,
        &proposal(FREECAD, plain),
        Timestamp::now(),
    );
    assert!(admission.allowed().is_some());
}

#[test]
fn enter_in_a_terminal_waits_for_a_person() {
    let enter = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Enter)),
    };
    match gate(TERMINAL, enter) {
        StageAdmission::NeedsApproval { command, .. } => assert_eq!(command.app_id(), TERMINAL),
        other => panic!("expected approval to be required, got {other:?}"),
    }
    // Shift+Enter ends in Enter too.
    let shifted = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Enter)).with(Modifier::Shift),
    };
    assert!(matches!(
        gate(TERMINAL, shifted),
        StageAdmission::NeedsApproval { .. }
    ));
    // Tab does not.
    let tab = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Tab)),
    };
    assert!(gate(TERMINAL, tab).allowed().is_some());
}

#[test]
fn coordinates_and_scrolls_are_bounded() {
    let negative = StageAction::Click {
        x: -1,
        y: 10,
        button: PointerButton::Left,
    };
    assert_eq!(
        refusal(gate(FREECAD, negative)),
        StageRefusal::CoordinateOutOfRange
    );
    let huge = StageAction::Drag {
        x: 1,
        y: 1,
        to_x: 40_000,
        to_y: 1,
        button: PointerButton::Left,
    };
    assert_eq!(
        refusal(gate(FREECAD, huge)),
        StageRefusal::CoordinateOutOfRange
    );

    let nothing = StageAction::Scroll {
        x: 5,
        y: 5,
        steps_x: 0,
        steps_y: 0,
    };
    assert_eq!(
        refusal(gate(FREECAD, nothing)),
        StageRefusal::ScrollOutOfRange
    );
    let flood = StageAction::Scroll {
        x: 5,
        y: 5,
        steps_x: 0,
        steps_y: -61,
    };
    assert_eq!(
        refusal(gate(FREECAD, flood)),
        StageRefusal::ScrollOutOfRange
    );
}

#[test]
fn every_refusal_has_a_stable_code() {
    let codes = [
        StageRefusal::NoOperation.code(),
        StageRefusal::AppNotInPolicy("x".into()).code(),
        StageRefusal::Lease(LeaseRejection::Expired).code(),
        StageRefusal::SecretText.code(),
        StageRefusal::ChordReserved.code(),
    ];
    for code in codes {
        assert!(
            code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{code}"
        );
        assert!(code.len() <= 64);
    }
}

#[test]
fn looking_needs_a_read_lease_and_a_read_lease_never_touches() {
    let policy = StagePolicy::parse(
        r#"
[[app]]
app_id = "org.freecad.FreeCAD"
display_name = "FreeCAD"
allow = ["click", "snapshot"]
"#,
    )
    .unwrap();
    let principal = agent();
    let authorize = |lease: &Lease, action: StageAction| {
        StageGate::authorize(
            &policy,
            lease,
            &principal,
            false,
            &proposal(FREECAD, action),
            Timestamp::now(),
        )
    };
    let read = lease_for(&principal, stage_resource(FREECAD), Action::Read, OPERATION);
    let execute = lease_for(
        &principal,
        stage_resource(FREECAD),
        Action::Execute,
        OPERATION,
    );

    assert!(matches!(
        authorize(&read, StageAction::Snapshot),
        StageAdmission::Allowed(_)
    ));
    assert!(
        matches!(
            authorize(&read, click()),
            StageAdmission::Refused(StageRefusal::Lease(LeaseRejection::ActionNotPermitted))
        ),
        "a lease to look is not a lease to click"
    );
    assert!(
        matches!(
            authorize(&execute, StageAction::Snapshot),
            StageAdmission::Refused(StageRefusal::Lease(LeaseRejection::ActionNotPermitted))
        ),
        "nor is a lease to click one to look"
    );
    assert!(
        matches!(
            gate(FREECAD, StageAction::Snapshot),
            StageAdmission::Refused(StageRefusal::ActionNotAllowed {
                kind: ActionKind::Snapshot,
                ..
            })
        ),
        "the shipped policy grants no capture of anything"
    );
}
