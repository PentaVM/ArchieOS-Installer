//! Distillation end to end against loopback OpenAI-compatible servers standing in for teachers,
//! a judge and a draft model. It exercises the registry, the egress gate, resumption and
//! selection; the stand-ins' answers are canned, so nothing here measures any real model.

use archie_wanda_lncm::data::{Case, Provenance, Split, Target};
use archie_wanda_lncm::distill::{self, DistillConfig, ModelUse, Runner, Sample};
use archie_wanda_lncm::{data, Lang, Task};
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};

async fn chat(Json(request): Json<Value>) -> Json<Value> {
    let system = request["messages"][0]["content"].as_str().unwrap_or("");
    let model = request["model"].as_str().unwrap_or("");
    let content = if system.contains("Evalúas") || system.contains("You evaluate") {
        r#"{"tom": 8, "evidence": 8, "uncertainty": 7, "actionable": 8, "safety": 10}"#.to_string()
    } else if system.starts_with("Responde a la pregunta")
        || system.starts_with("Answer the question")
    {
        "Creo que Pedro no lo sabe.".to_string()
    } else if system.contains("\"delta\"") {
        r#"<think>Ana writes the trip moved.</think>{"delta": ["fact el viaje = julio @3"]}"#
            .to_string()
    } else if system.contains("\"choice\"") {
        r#"{"choice": 1, "confidence": 0.8}"#.to_string()
    } else {
        format!(
            r#"{{"plan": {{"stance": "directo", "key": "Pedro no estaba", "action": "avisar"}}, "answer": "Avísale a Pedro hoy ({model})."}}"#
        )
    };
    Json(json!({
        "model": model,
        "choices": [{"message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 7, "total_tokens": 17}
    }))
}

fn manifest(id: &str, provider_model: &str, base: &str) -> String {
    format!(
        r#"id = "{id}"
display_name = "{id}"
provider = "open-ai-compatible"
locality = "local"
context_window = 32768
max_output_tokens = 4096
capabilities = ["cognition.response", "cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 50

[cost]
input_micros_per_mtok = 1000000
output_micros_per_mtok = 3000000

[labels]
base-url = "{base}"
provider-model = "{provider_model}"
"#
    )
}

fn case(id: &str, task: Task) -> Case {
    Case {
        id: id.into(),
        task,
        lang: Lang::Es,
        split: Split::Train,
        adapter: None,
        question: (task != Task::Update).then(|| "¿Tengo que avisar a Pedro?".to_string()),
        options: if task == Task::Decide {
            vec!["sí".into(), "no".into()]
        } else {
            vec![]
        },
        state: Some("Personas: Pedro, Ana\n- Pedro: nunca vio nada sobre el viaje.".into()),
        chunk: (task == Task::Update)
            .then(|| "[3] Ana: Cambio de planes: ahora el viaje es en julio.".to_string()),
        gold: None,
        provenance: Provenance::OwnedProcedural {
            generator: "test".into(),
            seed: 1,
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn teaches_drafts_judges_and_selects() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/chat/completions", post(chat)),
        )
        .await
        .unwrap();
    });

    let tmp = tempfile::tempdir().unwrap();
    let models = tmp.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(
        models.join("a.toml"),
        manifest("local.teacher-a", "vendor-a/teacher", &base),
    )
    .unwrap();
    std::fs::write(
        models.join("b.toml"),
        manifest("local.judge-b", "vendor-b/judge", &base),
    )
    .unwrap();
    std::fs::write(
        models.join("c.toml"),
        manifest("local.draft-c", "vendor-c/draft", &base),
    )
    .unwrap();
    let use_ = |model: &str, samples: usize| ModelUse {
        model: model.into(),
        samples,
        temperature: 0.7,
        max_tokens: 512,
    };
    let config = DistillConfig {
        models_dir: models,
        concurrency: 2,
        timeout_seconds: 30,
        teachers: vec![use_("local.teacher-a", 2)],
        judges: vec![use_("local.judge-b", 1)],
        drafts: vec![use_("local.draft-c", 1)],
        max_cases: None,
        max_completion_tokens: None,
        max_cost_usd: Some(1.0),
        ledger: Some(tmp.path().join("ledger.jsonl")),
        min_agreement: 0.5,
        min_safety: 6,
        eval_only: vec![],
        max_eval_overlap: 2,
        trust_gold: true,
    };
    config.validate().unwrap();
    let mut leaky = case("leaky", Task::Compose);
    leaky.state = Some("Escríbele a pedro@example.com".into());
    let mut gold_decide = case("gold", Task::Decide);
    gold_decide.gold = Some(Target {
        choice: Some(data::Choice::Option(0)),
        ..Default::default()
    });
    let cases = vec![
        case("compose", Task::Compose),
        case("decide", Task::Decide),
        case("update", Task::Update),
        gold_decide,
        leaky,
    ];
    let runner = Runner::new(config.clone()).unwrap();
    let teachers = tmp.path().join("teachers.jsonl");
    let report = runner.teach(&cases, &teachers).await.unwrap();
    assert_eq!(report.refused.get("sensitive_shape"), Some(&1));
    // compose, decide and update twice each; the gold decision is not asked.
    assert_eq!(report.requested, 6);
    assert_eq!(report.completed, 6);
    assert_eq!(report.unparsed, 0);
    assert_eq!(report.completion_tokens, 42);
    // 10 prompt and 7 completion tokens at $1 and $3 per million: 31 millionths a call.
    assert_eq!(report.cost_micros, 6 * 31);
    assert_eq!(
        distill::ledger_total(&tmp.path().join("ledger.jsonl")).unwrap(),
        6 * 31
    );
    let again = runner.teach(&cases, &teachers).await.unwrap();
    assert_eq!(again.already_done, 6);
    assert_eq!(again.completed, 0);

    let drafts = tmp.path().join("drafts.jsonl");
    let drafted = runner.draft(&cases, &drafts).await.unwrap();
    assert_eq!(drafted.completed, 3, "{drafted:?}");
    let judges = tmp.path().join("judges.jsonl");
    let judged = runner.judge(&cases, &teachers, &judges).await.unwrap();
    assert_eq!(
        judged.completed, 2,
        "both compose samples are judged by another family"
    );

    // A budget that cannot cover one call's worst case starts nothing, across runs.
    let spent = distill::ledger_total(&tmp.path().join("ledger.jsonl")).unwrap();
    let tight = Runner::new(DistillConfig {
        max_cost_usd: Some((spent + 1000) as f64 / 1_000_000.0),
        ..config.clone()
    })
    .unwrap();
    let starved = tight
        .teach(&cases, &tmp.path().join("more.jsonl"))
        .await
        .unwrap();
    assert_eq!(starved.completed, 0);
    assert_eq!(starved.not_started_budget, 6);
    assert_eq!(starved.ledger_before_micros, spent);

    let mut all: Vec<Sample> = Vec::new();
    for p in [&teachers, &drafts, &judges] {
        all.extend(data::read_jsonl::<Sample>(p, 1000).unwrap());
    }
    let (examples, report) = distill::select(&config, &cases, &all);
    assert_eq!(report.from_gold, 1);
    let compose = examples
        .iter()
        .find(|e| e.id == "compose:selected")
        .unwrap();
    assert_eq!(
        compose.target.text.as_deref(),
        Some("Avísale a Pedro hoy (vendor-a/teacher).")
    );
    assert_eq!(
        compose.target.plan.as_deref(),
        Some("directo | Pedro no estaba | avisar")
    );
    let update = examples.iter().find(|e| e.id == "update:selected").unwrap();
    assert_eq!(
        update.target.text.as_deref(),
        Some("fact el viaje = julio @3")
    );
    assert!(examples
        .iter()
        .any(|e| e.draft.as_deref() == Some("Creo que Pedro no lo sabe.")));
    for e in &examples {
        e.validate().unwrap();
    }
}
