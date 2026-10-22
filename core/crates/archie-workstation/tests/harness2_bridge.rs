//! Real JSONL child lifecycle tests. The fixture substitutes the external worker,
//! while the opt-in test below exercises the actual pinned Python engine.
use archie_vault::ContextDomain;
use archie_workstation::harness2::{
    run_worker, Evidence, Harness2Handler, Harness2Limits, Harness2Task, StageFuture, StageReply,
    StageRequest, StageUsage, WorkerProgram,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn task() -> Harness2Task {
    Harness2Task {
        run_id: "run-a".into(),
        task_id: "task-a".into(),
        task_family: "summaries".into(),
        domain: ContextDomain::work("acme", "project").unwrap(),
        focus_id: "focus-a".into(),
        objective_ref: "objective-a".into(),
        policy_revision_ref: "policy-v1".into(),
        model_identity_sha256: "a".repeat(64),
        input_sha256: sha("Summarize Reference A"),
        task: "Summarize Reference A".into(),
        evidence: vec![Evidence {
            reference: "evidence-a".into(),
            sha256: sha("Reference A"),
            text: "Reference A".into(),
        }],
        base_harness: BTreeMap::from([
            (
                "systemprompt.md".into(),
                "Use the admitted evidence.".into(),
            ),
            ("guardrails.md".into(), "Policy is immutable.".into()),
            ("LongTermMEMORY.md".into(), "No previous lessons.".into()),
            (
                "tool_descriptions/tool_guidance.md".into(),
                "Tools are unavailable.".into(),
            ),
        ]),
        limits: Harness2Limits {
            max_calls: 12,
            max_input_tokens: 500_000,
            max_output_tokens: 50_000,
            max_cost_micros: Some(12_000),
            per_call_input_tokens: 100_000,
            per_call_output_tokens: 4096,
            per_call_cost_micros: Some(1000),
            max_frame_bytes: 1_048_576,
            deadline_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60_000,
        },
    }
}

struct Parent {
    tree: BTreeMap<String, String>,
    calls: usize,
}

impl Harness2Handler for Parent {
    fn infer<'a>(&'a mut self, stage: &'a StageRequest) -> StageFuture<'a> {
        self.calls += 1;
        let text = match stage.response_format.as_str() {
            "artifact" => serde_json::json!({
                "artifact": "Reference A summary.",
                "trajectory": "[ACTION] Read evidence-a\n[OBSERVATION] Reference A"
            })
            .to_string(),
            "harness" => {
                let mut tree = self.tree.clone();
                tree.insert("LongTermMEMORY.md".into(), "Preserve evidence-a.".into());
                serde_json::json!({"harness":tree,"notes":"Keep attribution."}).to_string()
            }
            "experience" => "<experience>\ncue: source summary\nlesson: preserve references\nevidence: evidence-a execution contrast\nconfidence: limited\n</experience>".into(),
            _ => return Box::pin(async { Err("unexpected stage".into()) }),
        };
        Box::pin(async move {
            Ok(StageReply {
                text,
                usage: StageUsage {
                    input_tokens: Some(20),
                    output_tokens: Some(20),
                    cost_micros: None,
                },
            })
        })
    }
}

/// Explicit opt-in rather than a silent skip: run with --ignored after setting
/// ARCHIE_HARNESS2_PYTHON to a POSIX Python 3.12/3.13 executable. No model calls.
#[tokio::test]
#[ignore = "requires configured POSIX Python 3.12/3.13; set ARCHIE_HARNESS2_PYTHON"]
async fn pinned_engine_runs_through_rust_bridge_and_default_refuses_promotion() {
    let executable = PathBuf::from(
        std::env::var_os("ARCHIE_HARNESS2_PYTHON")
            .expect("ARCHIE_HARNESS2_PYTHON must explicitly name a supported interpreter"),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let program = WorkerProgram {
        executable,
        arguments: vec!["-I".into(), "-S".into(), "-B".into()],
        worker: root.join("tools/harness2/worker.py"),
        state_root: directory.path().to_path_buf(),
    };
    let task = task();
    let mut parent = Parent {
        tree: task.base_harness.clone(),
        calls: 0,
    };
    let result = run_worker(&program, task, &mut parent)
        .await
        .unwrap_or_else(|error| panic!("{error}; completed handler calls={}", parent.calls));
    assert_eq!(result.status, "completed");
    assert_eq!(result.ledger["R"], 4);
    assert_eq!(result.ledger["P"], 3);
    assert_eq!(parent.calls, 8);
    assert_eq!(result.usage.charged_cost_micros, Some(8000));
    assert_eq!(result.usage.observed_cost_micros, None);
    assert_eq!(result.promotion.status, "baseline_retained");
    assert!(!result.promotion.improvement_verified);
    assert!(result.library.committed.is_empty());
}

#[tokio::test]
#[ignore = "requires configured POSIX Python 3.12/3.13; set ARCHIE_HARNESS2_PYTHON"]
async fn pinned_engine_preserves_unknown_monetary_bounds_through_rust() {
    let executable = PathBuf::from(
        std::env::var_os("ARCHIE_HARNESS2_PYTHON")
            .expect("ARCHIE_HARNESS2_PYTHON must explicitly name a supported interpreter"),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let program = WorkerProgram {
        executable,
        arguments: vec!["-I".into(), "-S".into(), "-B".into()],
        worker: root.join("tools/harness2/worker.py"),
        state_root: directory.path().to_path_buf(),
    };
    let mut request = task();
    request.limits.max_cost_micros = None;
    request.limits.per_call_cost_micros = None;
    let mut parent = Parent {
        tree: request.base_harness.clone(),
        calls: 0,
    };
    let result = run_worker(&program, request, &mut parent).await.unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(parent.calls, 8);
    assert_eq!(result.usage.charged_cost_micros, None);
    assert_eq!(result.usage.observed_cost_micros, None);
    assert_eq!(result.usage.charged_output_tokens, 160);
    assert!(!result.promotion.improvement_verified);
    assert_eq!(result.promotion.status, "baseline_retained");
}

#[tokio::test]
async fn invalid_task_fails_before_a_process_or_handler_can_run() {
    let directory = tempfile::tempdir().unwrap();
    let program = WorkerProgram {
        executable: directory.path().join("must-not-launch"),
        arguments: vec![],
        worker: directory.path().join("worker.py"),
        state_root: directory.path().to_path_buf(),
    };
    let mut request = task();
    request.task_id = "../escape".into();
    let mut parent = Parent {
        tree: request.base_harness.clone(),
        calls: 0,
    };
    let error = run_worker(&program, request, &mut parent)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request");
    assert_eq!(parent.calls, 0);
}

fn fixture(case: &str, state_root: &std::path::Path) -> WorkerProgram {
    let python = std::env::var_os("ARCHIE_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from("C:/Python312/python.exe")
            } else {
                PathBuf::from("/usr/bin/python3")
            }
        });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    WorkerProgram {
        executable: python,
        arguments: vec![root
            .join("tools/harness2/tests/bridge_fixture.py")
            .to_string_lossy()
            .into_owned()],
        // The configured worker position supplies the fixture's behavior argument.
        worker: state_root.join(case),
        state_root: state_root.to_path_buf(),
    }
}

#[tokio::test]
async fn forged_context_digest_policy_and_budget_never_reach_the_handler() {
    for case in [
        "domain",
        "policy",
        "digest",
        "reservation",
        "stage",
        "extra",
        "duplicate",
        "oversized",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let program = fixture(case, directory.path());
        let mut task = task();
        task.limits.max_frame_bytes = 4096;
        let mut parent = Parent {
            tree: task.base_harness.clone(),
            calls: 0,
        };
        let error = run_worker(&program, task, &mut parent).await.unwrap_err();
        assert_eq!(error.code(), "protocol_violation", "case {case}: {error}");
        assert_eq!(parent.calls, 0, "case {case}");
    }
}

#[tokio::test]
async fn admitted_stage_is_forwarded_but_failure_never_becomes_success() {
    let directory = tempfile::tempdir().unwrap();
    let program = fixture("valid", directory.path());
    let task = task();
    let mut parent = Parent {
        tree: task.base_harness.clone(),
        calls: 0,
    };
    let error = run_worker(&program, task, &mut parent).await.unwrap_err();
    assert_eq!(error.code(), "fixture_terminal");
    assert_eq!(parent.calls, 1);
}

#[tokio::test]
async fn unknown_monetary_ceiling_still_admits_a_bounded_stage() {
    let directory = tempfile::tempdir().unwrap();
    let program = fixture("valid", directory.path());
    let mut encoded = serde_json::to_value(task()).unwrap();
    encoded["limits"]["max_cost_micros"] = serde_json::Value::Null;
    encoded["limits"]["per_call_cost_micros"] = serde_json::Value::Null;
    let request: Harness2Task = serde_json::from_value(encoded)
        .expect("unknown monetary bounds are explicit null rather than fictional reserves");
    let mut parent = Parent {
        tree: request.base_harness.clone(),
        calls: 0,
    };
    let error = run_worker(&program, request, &mut parent)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "fixture_terminal");
    assert_eq!(parent.calls, 1);
}

#[tokio::test]
async fn inconsistent_monetary_bounds_are_rejected_before_launch() {
    let directory = tempfile::tempdir().unwrap();
    let program = WorkerProgram {
        executable: directory.path().join("must-not-launch"),
        arguments: vec![],
        worker: directory.path().join("worker.py"),
        state_root: directory.path().to_path_buf(),
    };
    for (maximum, per_call) in [
        (None, Some(1000)),
        (Some(12000), None),
        (Some(0), Some(1000)),
        (Some(12000), Some(0)),
    ] {
        let mut request = task();
        request.limits.max_cost_micros = maximum;
        request.limits.per_call_cost_micros = per_call;
        let mut parent = Parent {
            tree: request.base_harness.clone(),
            calls: 0,
        };
        let error = run_worker(&program, request, &mut parent)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_request");
        assert_eq!(parent.calls, 0);
    }
}

#[tokio::test]
async fn deadline_covers_child_and_callback_and_kills_the_owned_child() {
    struct Slow;
    impl Harness2Handler for Slow {
        fn infer<'a>(&'a mut self, _stage: &'a StageRequest) -> StageFuture<'a> {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Err("late".into())
            })
        }
    }
    let directory = tempfile::tempdir().unwrap();
    for case in ["hang", "valid"] {
        let mut program = fixture(case, directory.path());
        let marker = directory.path().join("survived");
        if case == "hang" {
            program
                .arguments
                .push(program.worker.to_string_lossy().into_owned());
            program.worker = marker.clone();
        }
        let mut request = task();
        request.limits.deadline_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 250;
        let error = run_worker(&program, request, &mut Slow).await.unwrap_err();
        assert_eq!(error.code(), "deadline_exceeded");
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
        assert!(
            !marker.exists(),
            "deadline must kill child before its delayed write"
        );
    }
}
