//! Actions on applications through the loop: gated by a lease and the stage policy, and
//! traced by kind and grade, never by content.

use archie_cerebra::{Cerebra, CerebraConfig};
use archie_observability::{CognitiveTrace, Stage, StageOutcome, Subject};
use archie_policy::{
    Action, CapabilityRequest, Constraints, Lease, PolicyEngine, Principal, Scope,
};
use archie_router::Router;
use archie_stage::{
    stage_resource, Key, KeyChord, NamedKey, PointerButton, StageAction, StageAdmission,
    StagePolicy, StageProposal, StageText,
};
use archie_types::{AgentId, PrivacyGrade, Timestamp, TraceId};
use archie_vault::ContextGateway;

fn cerebra() -> Cerebra {
    Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
}

fn policy() -> StagePolicy {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../archie-stage/policy/apps.toml");
    StagePolicy::load(path).unwrap()
}

fn lease(principal: &Principal, app: &str, operation: u64) -> Lease {
    PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: stage_resource(app),
            scope: Scope::OneOperation { operation },
            constraints: Constraints::with_actions([Action::Execute]),
            justification: "operate an application".into(),
        })
        .lease
        .unwrap()
}

fn proposal(app: &str, action: StageAction, grade: PrivacyGrade) -> StageProposal {
    StageProposal {
        app_id: app.into(),
        surface: 2,
        geometry_revision: 9,
        action,
        text_grade: grade,
        operation_id: 5,
        task_id: None,
        trace_id: "trace-stage".into(),
    }
}

#[test]
fn a_leased_click_proceeds_and_is_recorded_as_an_authorisation() {
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    // Issued before the clock is read, so the lease can never look like it came from the future.
    let policy = policy();
    let lease_cad = lease(&principal, "org.freecad.FreeCAD", 5);
    let now = Timestamp::now().as_millis();
    let mut trace = CognitiveTrace::new(TraceId::new(), "work the CAD model", now);
    let click = StageAction::Click {
        x: 40,
        y: 40,
        button: PointerButton::Left,
    };
    let admission = cerebra()
        .propose_stage_action(
            &mut trace,
            &policy,
            &lease_cad,
            &principal,
            false,
            &proposal("org.freecad.FreeCAD", click, PrivacyGrade::Public),
            now + 5,
        )
        .unwrap();
    assert!(admission.allowed().is_some());
    let record = trace.last(Stage::Authorize).unwrap();
    assert_eq!(record.outcome, StageOutcome::Proceeded);
    assert_eq!(
        record.subject,
        Some(Subject::Tool {
            server: "stage".into(),
            tool: "click".into()
        })
    );
    assert_eq!(record.privacy, Some(PrivacyGrade::Public));
}

#[test]
fn a_command_in_a_terminal_waits_for_the_operator_and_the_hud_says_so() {
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    // Issued before the clock is read, so the lease can never look like it came from the future.
    let policy = policy();
    let lease_foot = lease(&principal, "foot", 5);
    let now = Timestamp::now().as_millis();
    let mut trace = CognitiveTrace::new(TraceId::new(), "run the build", now);
    let enter = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Enter)),
    };
    let admission = cerebra()
        .propose_stage_action(
            &mut trace,
            &policy,
            &lease_foot,
            &principal,
            false,
            &proposal("foot", enter, PrivacyGrade::Public),
            now + 5,
        )
        .unwrap();
    assert!(matches!(admission, StageAdmission::NeedsApproval { .. }));
    assert!(matches!(
        trace.hud().authorization,
        Some(StageOutcome::AwaitingApproval { .. })
    ));
}

#[test]
fn a_secret_typed_by_the_agent_is_refused_and_never_written_down() {
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    // Issued before the clock is read, so the lease can never look like it came from the future.
    let policy = policy();
    let lease_foot = lease(&principal, "foot", 5);
    let now = Timestamp::now().as_millis();
    let mut trace = CognitiveTrace::new(TraceId::new(), "log in", now);
    let password = StageAction::Text {
        text: StageText::new("correct horse battery staple"),
    };
    let admission = cerebra()
        .propose_stage_action(
            &mut trace,
            &policy,
            &lease_foot,
            &principal,
            false,
            &proposal("foot", password, PrivacyGrade::Secret),
            now + 5,
        )
        .unwrap();
    assert!(admission.refusal().is_some());
    assert_eq!(
        trace.last(Stage::Authorize).unwrap().outcome,
        StageOutcome::Refused {
            reason: "secret_in_text".into()
        }
    );
    assert_eq!(trace.hud().privacy, Some(PrivacyGrade::Secret));
    let written = serde_json::to_string(&trace).unwrap();
    assert!(!written.contains("horse"), "{written}");
}
