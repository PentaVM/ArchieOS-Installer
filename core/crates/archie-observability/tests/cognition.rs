//! The cognitive trace, and the one property it exists to keep.
//!
//! A trace is the artefact most likely to leave the machine — into a dashboard, an
//! issue, a screenshot. So the tests that matter most here are the ones that try to put
//! private material into it and fail.

use archie_observability::{
    CognitiveTrace, Stage, StageCost, StageOutcome, StageRecord, Subject, TraceError,
};

#[test]
fn advisory_success_has_a_stable_code_and_prose_is_rejected() {
    let mut record = StageRecord::new(Stage::Interpret, 1000, StageOutcome::Proceeded)
        .decision("evidence_aligned");
    assert!(record.validate().is_ok());
    assert!(serde_json::to_string(&record)
        .unwrap()
        .contains("evidence_aligned"));
    record.decision_code = Some("model said something".into());
    assert!(record.validate().is_err());
}
use archie_types::{PrivacyGrade, TaskId, TraceId};

const T0: i64 = 1_760_000_000_000;

fn trace() -> CognitiveTrace {
    CognitiveTrace::new(TraceId::new(), "summarise-inbox", T0).for_task(TaskId::new())
}

fn refused(reason: &str) -> StageOutcome {
    StageOutcome::Refused {
        reason: reason.into(),
    }
}

#[test]
fn a_pass_that_does_nothing_is_recorded_as_having_done_nothing() {
    // The most common correct ending for an always-on loop, and the one a trace format
    // built only around actions cannot express.
    let mut trace = trace();
    trace
        .record(StageRecord::new(
            Stage::Observe,
            T0,
            StageOutcome::Proceeded,
        ))
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Interpret,
            T0 + 5,
            StageOutcome::Silent {
                reason: "thought_not_complete".into(),
            },
        ))
        .unwrap();

    assert!(trace.was_silent());
    let hud = trace.hud();
    assert!(hud.silent);
    assert!(hud.execution.is_none(), "nothing was executed");
}

#[test]
fn a_refusal_is_distinguishable_from_a_silence_and_from_a_failure() {
    // Three different things a person would respond to differently, and which a single
    // "did not happen" flag would flatten into one.
    let mut trace = trace();
    for (stage, outcome) in [
        (Stage::Authorize, refused("privacy_grade_not_permitted")),
        (
            Stage::Act,
            StageOutcome::Failed {
                reason: "provider_timeout".into(),
            },
        ),
        (
            Stage::Remember,
            StageOutcome::Silent {
                reason: "below_salience_threshold".into(),
            },
        ),
    ] {
        trace
            .record(StageRecord::new(stage, T0 + 1, outcome))
            .unwrap();
    }

    let hud = trace.hud();
    assert!(matches!(
        hud.authorization,
        Some(StageOutcome::Refused { .. })
    ));
    assert!(matches!(hud.execution, Some(StageOutcome::Failed { .. })));
    assert!(matches!(hud.memory, Some(StageOutcome::Silent { .. })));
}

#[test]
fn a_reason_must_be_a_code_rather_than_a_sentence() {
    // The failure this prevents: "refused because the note about John Smith is
    // restricted" — a perfectly natural thing to write, and a disclosure.
    let mut trace = trace();
    let prose = StageRecord::new(
        Stage::Authorize,
        T0,
        refused("refused because the note about John Smith is restricted"),
    );
    assert_eq!(trace.record(prose).unwrap_err(), TraceError::ReasonIsProse);

    // The code form is accepted.
    assert!(trace
        .record(StageRecord::new(
            Stage::Authorize,
            T0,
            refused("privacy_grade_not_permitted")
        ))
        .is_ok());
    assert_eq!(trace.stages.len(), 1, "the prose record was not stored");
}

#[test]
fn a_goal_label_cannot_hold_the_request_it_labels() {
    let utterance = "can you look through my inbox and tell me what the lawyer said about \
                     the settlement, and whether I should sign it before Friday, and also \
                     remind me what I owe on the second invoice";
    let trace = CognitiveTrace::new(TraceId::new(), utterance, T0);
    assert_eq!(trace.validate().unwrap_err(), TraceError::GoalLabel);

    assert!(CognitiveTrace::new(TraceId::new(), "summarise-inbox", T0)
        .validate()
        .is_ok());
}

#[test]
fn an_over_long_identifier_is_refused() {
    let mut trace = trace();
    let record =
        StageRecord::new(Stage::Reason, T0, StageOutcome::Proceeded).about(Subject::Model {
            id: "x".repeat(200),
        });
    assert_eq!(trace.record(record).unwrap_err(), TraceError::Identifier);
}

#[test]
fn the_hud_shows_what_happened_and_none_of_what_was_said() {
    // The demo case, end to end: private sources consulted, a model chosen, a tool run,
    // memory declining to keep it.
    let mut trace = CognitiveTrace::new(TraceId::new(), "draft-reply", T0)
        .for_task(TaskId::new())
        .sourced_from(Subject::VaultEntry {
            reference: "vault:notes/deal".into(),
        })
        .sourced_from(Subject::VaultEntry {
            reference: "vault:notes/contacts".into(),
        });

    trace
        .record(
            StageRecord::new(Stage::Contextualize, T0 + 2, StageOutcome::Proceeded)
                .handling(PrivacyGrade::Internal)
                .about(Subject::Capsule {
                    id: "capsule-trace-1".into(),
                }),
        )
        .unwrap();
    trace
        .record(
            StageRecord::new(Stage::Reason, T0 + 40, StageOutcome::Proceeded)
                .handling(PrivacyGrade::Internal)
                .about(Subject::Model {
                    id: "nebius.nemotron".into(),
                })
                .costing(StageCost {
                    latency_ms: Some(380),
                    input_tokens: Some(1200),
                    output_tokens: Some(210),
                    cost_micros: Some(4200),
                }),
        )
        .unwrap();
    trace
        .record(
            StageRecord::new(Stage::SelectCapability, T0 + 50, StageOutcome::Proceeded).about(
                Subject::Tool {
                    server: "github".into(),
                    tool: "create_issue".into(),
                },
            ),
        )
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Authorize,
            T0 + 52,
            StageOutcome::Proceeded,
        ))
        .unwrap();
    trace
        .record(
            StageRecord::new(Stage::Act, T0 + 300, StageOutcome::Proceeded).costing(StageCost {
                latency_ms: Some(240),
                ..Default::default()
            }),
        )
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Verify,
            T0 + 310,
            StageOutcome::Proceeded,
        ))
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Remember,
            T0 + 320,
            StageOutcome::Silent {
                reason: "no_novel_claim".into(),
            },
        ))
        .unwrap();

    trace.validate().unwrap();
    let hud = trace.hud();

    assert_eq!(hud.context_source_count, 2);
    assert_eq!(hud.privacy, Some(PrivacyGrade::Internal));
    assert!(!hud.silent, "a tool was actually run");
    assert_eq!(hud.total_latency_ms, 620);
    assert_eq!(hud.total_cost_micros, 4200);
    assert!(matches!(hud.model, Some(Subject::Model { .. })));
    assert!(matches!(hud.capability, Some(Subject::Tool { .. })));

    // And the whole thing, serialised, carries no content.
    let rendered = serde_json::to_string(&hud).unwrap();
    let full = serde_json::to_string(&trace).unwrap();
    for forbidden in [
        "John Smith",
        "the lawyer said",
        "Dear ",
        "settlement",
        "tok_",
        "ghp_",
    ] {
        assert!(!rendered.contains(forbidden), "hud leaked {forbidden}");
        assert!(!full.contains(forbidden), "trace leaked {forbidden}");
    }
    // It still says enough to be useful.
    assert!(rendered.contains("nebius.nemotron"));
    assert!(rendered.contains("create_issue"));
    assert!(rendered.contains("internal"));
}

#[test]
fn the_peak_grade_is_the_most_sensitive_the_pass_touched() {
    // A HUD showing the last grade rather than the highest would under-report a pass
    // that read something restricted and then finished on something public.
    let mut trace = trace();
    trace
        .record(
            StageRecord::new(Stage::Contextualize, T0, StageOutcome::Proceeded)
                .handling(PrivacyGrade::Restricted),
        )
        .unwrap();
    trace
        .record(
            StageRecord::new(Stage::Reason, T0 + 1, StageOutcome::Proceeded)
                .handling(PrivacyGrade::Public),
        )
        .unwrap();

    assert_eq!(trace.peak_privacy(), Some(PrivacyGrade::Restricted));
    assert_eq!(trace.hud().privacy, Some(PrivacyGrade::Restricted));
}

#[test]
fn a_trace_cannot_grow_without_bound() {
    let mut trace = trace();
    for i in 0..archie_observability::cognition::limits::MAX_STAGES {
        trace
            .record(StageRecord::new(
                Stage::Observe,
                T0 + i as i64,
                StageOutcome::Proceeded,
            ))
            .unwrap();
    }
    assert_eq!(
        trace
            .record(StageRecord::new(
                Stage::Observe,
                T0,
                StageOutcome::Proceeded
            ))
            .unwrap_err(),
        TraceError::TooManyStages
    );
}

#[test]
fn a_trace_round_trips_through_json() {
    let mut trace = trace();
    trace
        .record(
            StageRecord::new(Stage::Reason, T0, StageOutcome::Proceeded)
                .handling(PrivacyGrade::Sensitive)
                .about(Subject::Model {
                    id: "local.qwen3".into(),
                }),
        )
        .unwrap();

    let json = serde_json::to_string(&trace).unwrap();
    let parsed: CognitiveTrace = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, trace);
    parsed.validate().unwrap();
}

#[test]
fn an_awaiting_approval_pass_is_neither_done_nor_refused() {
    // The MCP gate's third answer needs somewhere to land, or a call held for a person
    // looks identical to one that was blocked.
    let mut trace = trace();
    trace
        .record(StageRecord::new(
            Stage::Authorize,
            T0,
            StageOutcome::AwaitingApproval {
                reason: "operator_approval_required".into(),
            },
        ))
        .unwrap();

    let hud = trace.hud();
    assert!(matches!(
        hud.authorization,
        Some(StageOutcome::AwaitingApproval { .. })
    ));
    assert!(hud.silent, "nothing has been done yet");
}

#[test]
fn a_pass_still_running_is_not_reported_as_silence() {
    // The sequencer hands a capsule back for inference and the result returns later, so
    // a trace whose last stage proceeded has not finished. Rendering that as silence
    // would make a system that is thinking look like one with nothing to say, and the
    // two call for opposite responses from whoever is watching.
    let mut trace = trace();
    trace
        .record(StageRecord::new(
            Stage::Observe,
            T0,
            StageOutcome::Proceeded,
        ))
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Reason,
            T0 + 1,
            StageOutcome::Proceeded,
        ))
        .unwrap();

    assert!(trace.in_progress());
    assert!(!trace.was_silent());
    assert!(!trace.has_acted());

    let hud = trace.hud();
    assert!(hud.in_progress);
    assert!(!hud.silent);
}

#[test]
fn silence_and_progress_are_never_both_true() {
    // Whatever sequence of stages arrives, a display can rely on these being exclusive.
    for outcome in [
        StageOutcome::Proceeded,
        StageOutcome::Silent {
            reason: "thought_not_complete".into(),
        },
        StageOutcome::Refused {
            reason: "privacy_grade_not_permitted".into(),
        },
        StageOutcome::Failed {
            reason: "provider_timeout".into(),
        },
        StageOutcome::AwaitingApproval {
            reason: "operator_approval_required".into(),
        },
    ] {
        for stage in [Stage::Interpret, Stage::Reason, Stage::Act, Stage::Remember] {
            let mut trace = trace();
            trace
                .record(StageRecord::new(stage, T0, outcome.clone()))
                .unwrap();
            let hud = trace.hud();
            assert!(
                !(hud.silent && hud.in_progress),
                "{stage} / {outcome:?} reported both"
            );
        }
    }
}

#[test]
fn a_pass_that_acted_is_neither_silent_nor_in_progress() {
    let mut trace = trace();
    trace
        .record(StageRecord::new(Stage::Act, T0, StageOutcome::Proceeded))
        .unwrap();
    trace
        .record(StageRecord::new(
            Stage::Remember,
            T0 + 1,
            StageOutcome::Silent {
                reason: "no_novel_claim".into(),
            },
        ))
        .unwrap();

    assert!(trace.has_acted());
    assert!(
        !trace.was_silent(),
        "memory declining is not Archie staying quiet"
    );
    assert!(!trace.in_progress());
}
