use archie_cerebra::{ClarificationAssessment, ClarificationDecision, ClarificationSession};
use archie_policy::{
    Action, CapabilityRequest, Constraints, PolicyConfig, PolicyEngine, Principal, Resource, Scope,
};
use archie_types::{TaskId, TraceId};
use archie_workstation::freecad::{motor_bracket_script, AuthorizedWorkspace, FreeCadExecutor};
use archie_workstation::speech::parse_live_event;

/// Runs only when the official FreeCAD binary is available in the validation distro.
#[test]
#[ignore = "requires the packaged Arch Linux FreeCAD runtime"]
fn voice_contract_to_clarification_to_policy_to_real_freecad() {
    let command = std::env::var_os("ARCHIE_REAL_FREECAD").expect("set ARCHIE_REAL_FREECAD");
    let transcript = parse_live_event(
        "demo",
        1,
        "[live final @ 0.80s] create a parametric motor bracket",
    )
    .unwrap();
    assert!(transcript.is_final);

    let assessment = ClarificationAssessment {
        ambiguity: 800,
        missing_information: 900,
        decision_impact: 900,
        reversibility: 100,
        risk: 500,
        confidence: 300,
        cost_of_wrong_assumption: 900,
        cost_of_interrupting_user: 100,
        can_progress_without_answer: false,
        expected_information_gain: 900,
        latency_cost: 50,
        external_effect_pending: false,
        reason_code: "missing_bracket_dimensions".into(),
        refs_consulted: vec!["task:motor-bracket".into()],
    };
    let mut clarification = ClarificationSession::default();
    assert_eq!(
        clarification.begin(&assessment, true).unwrap(),
        ClarificationDecision::AskBeforeCommit
    );
    clarification
        .answer(
            "missing_bracket_dimensions",
            "answer:dimensions-80-40-5-8".into(),
            600,
        )
        .unwrap();

    let temp = tempfile::tempdir().unwrap();
    let workspace_root = temp.path().join("motor-bracket");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::write(
        workspace_root.join("motor-bracket.py"),
        motor_bracket_script(80, 40, 5, 8).unwrap(),
    )
    .unwrap();

    let task = TaskId::new();
    let principal = Principal::Human {
        id: "e2e-operator".into(),
        authenticated: true,
    };
    let decision = PolicyEngine::new(PolicyConfig::default()).decide(&CapabilityRequest {
        trace_id: TraceId::new(),
        task_id: Some(task),
        principal: principal.clone(),
        resource: Resource::Filesystem {
            path: workspace_root.display().to_string(),
        },
        scope: Scope::OneTask { task },
        constraints: Constraints::with_actions([Action::Write]).limited_to(1),
        justification: "create the clarified parametric bracket".into(),
    });
    let mut lease = decision
        .lease
        .expect("policy must issue a bounded workspace lease");
    let workspace =
        AuthorizedWorkspace::from_lease(&workspace_root, principal, Some(task), &mut lease)
            .unwrap();
    FreeCadExecutor::new(command)
        .run_script(&workspace, std::path::Path::new("motor-bracket.py"))
        .unwrap();

    for artifact in [
        "motor-bracket.FCStd",
        "motor-bracket.step",
        "motor-bracket.stl",
    ] {
        assert!(workspace_root.join(artifact).metadata().unwrap().len() > 0);
    }
}
