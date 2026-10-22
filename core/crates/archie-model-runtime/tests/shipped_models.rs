//! The reasoning models the repository ships, as the runtime would call them.

use archie_model_runtime::routed::{Question, Routed};
use archie_model_runtime::{provider_model_id, ProviderEndpoint};
use archie_types::{Locality, ModelRole, PrivacyGrade, ProviderKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn shipped_models() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .join("models")
}

/// A manifest the runtime cannot turn into an endpoint is a model the router may choose
/// and nobody can call. IRIS is the exception: its own runtime serves it.
#[test]
fn every_shipped_reasoning_model_has_an_endpoint_and_a_provider_name() {
    let routed = Routed::from_dir(shipped_models()).unwrap();
    let mut cloud = 0;
    for model in routed.registry().iter() {
        if model.provider == ProviderKind::ArchieRuntime {
            continue;
        }
        let endpoint =
            ProviderEndpoint::for_model(model).unwrap_or_else(|e| panic!("{}: {e}", model.id));
        assert_eq!(endpoint.locality(), model.locality, "{}", model.id);
        if model.locality == Locality::Cloud {
            cloud += 1;
            assert_eq!(endpoint.base_url().scheme(), "https", "{}", model.id);
            assert!(
                provider_model_id(model).unwrap().contains('/'),
                "{} names no provider model",
                model.id
            );
            assert!(
                !model.permits_privacy(PrivacyGrade::Internal),
                "{} is a cloud model of unverified retention and must see public data only",
                model.id
            );
        }
    }
    assert!(cloud >= 10, "the choice of reasoning models is shipped");
}

const PLANNER: &str = "You are the planner of Archie. A fast visual model, IRIS, does the doing. Reply with ONE JSON \
object only: {\"skill\": \"the tool in use\", \"steps\": [{\"do\": \"one short imperative\", \"seconds\": N}]}.";

/// The whole path against Nebius: the router chooses among the shipped manifests, the
/// runtime calls the choice, and what comes back is a plan IRIS can be handed.
///
/// `NEBIUS_TOKEN_FACTORY_KEY=... cargo test -p archie-model-runtime --test shipped_models -- --ignored --nocapture`
#[tokio::test]
#[ignore = "calls Nebius: needs NEBIUS_TOKEN_FACTORY_KEY and the network"]
async fn the_routed_planner_answers_with_a_plan_for_iris() {
    let routed = Routed::from_dir(shipped_models()).unwrap();
    let question = Question {
        role: ModelRole::ReasoningWorker,
        privacy: PrivacyGrade::Public,
        system: PLANNER,
        user: "TASK: draw the Mona Lisa in a paint program. TIME: 60 s once you answer.",
        image: None,
        max_tokens: 2000,
        timeout: Duration::from_secs(120),
    };
    let candidates = routed.candidates(&question).unwrap();
    println!(
        "candidates, best first: {}",
        candidates
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(
        candidates[0].as_str().starts_with("nebius.nemotron"),
        "Nemotron is the default planner"
    );

    let answered = routed.ask(&question).await.unwrap();
    println!(
        "{} answered in {:.1} s:\n{}",
        answered.model, answered.seconds, answered.text
    );
    let (start, end) = (
        answered.text.find('{').unwrap(),
        answered.text.rfind('}').unwrap(),
    );
    let plan: serde_json::Value = serde_json::from_str(&answered.text[start..=end]).unwrap();
    assert!(plan["skill"].is_string());
    assert!(!plan["steps"].as_array().unwrap().is_empty());

    // Private data stays on the machine: only the shipped local reasoning model may take it.
    let private = Question {
        privacy: PrivacyGrade::Restricted,
        ..question
    };
    for id in routed.candidates(&private).unwrap() {
        let model = routed.registry().get(&id).unwrap();
        assert_eq!(
            model.locality,
            Locality::Local,
            "{id} would see private data"
        );
    }
}

mod private {
    use super::*;
    use archie_model_runtime::routed::{Disclosure, RoutedError};
    use archie_registry::ModelRegistry;
    use archie_types::{Availability, DataRetention, ModelDescriptor, ModelId, TokenCost};
    use archie_vault::{EntityKind, Veil};
    use std::collections::BTreeMap;

    const KEY: &str = "sk-live-7Hq2ZxVb91LmTn4cRw8yKd3PfAeU6sGj";

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    /// The shipped manifests without the node's own models: what a node routes between
    /// before its local weights are installed. These tests are about what may leave.
    fn shipped_models() -> PathBuf {
        static CLOUD: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        CLOUD
            .get_or_init(|| {
                let dir = tempfile::tempdir().unwrap();
                for family in std::fs::read_dir(super::shipped_models()).unwrap() {
                    let family = family.unwrap();
                    if family.file_name() == "local" || !family.file_type().unwrap().is_dir() {
                        continue;
                    }
                    copy_tree(&family.path(), &dir.path().join(family.file_name()));
                }
                dir
            })
            .path()
            .to_path_buf()
    }

    /// The shipped models with their key present: cloud only.
    fn cloud_only() -> Routed {
        std::env::set_var(
            "NEBIUS_TOKEN_FACTORY_KEY",
            "a-key-for-tests-that-call-nothing",
        );
        Routed::from_dir(shipped_models()).unwrap()
    }

    fn question<'a>(privacy: PrivacyGrade, user: &'a str) -> Question<'a> {
        Question {
            role: ModelRole::CognitiveResponse,
            privacy,
            system: "You write configuration files.",
            user,
            image: None,
            max_tokens: 800,
            timeout: Duration::from_secs(60),
        }
    }

    #[test]
    fn sensitive_text_goes_to_a_cloud_model_only_with_its_secrets_left_behind() {
        let text = format!(
            "Write the .env for Camila Rojas's service. API key {KEY}, alerts to camila@acme.cl."
        );
        let veil = Veil::new().knowing(EntityKind::Person, "Camila Rojas");
        match cloud_only()
            .disclosure(&question(PrivacyGrade::Sensitive, &text), &veil)
            .unwrap()
        {
            Disclosure::Veiled { models, veiled } => {
                assert!(models[0].as_str().starts_with("nebius.nemotron"));
                let leaving = veiled.texts.join("\n");
                for kept_here in [KEY, "camila@acme.cl", "Camila Rojas"] {
                    assert!(
                        !leaving.contains(kept_here),
                        "{kept_here} would have left: {leaving}"
                    );
                }
                assert!(
                    leaving.contains("SECRET_1")
                        && leaving.contains("EMAIL_1")
                        && leaving.contains("PERSON_1")
                );
                assert_eq!(
                    veiled.unveil.restore("KEY=SECRET_1\nTO=EMAIL_1"),
                    format!("KEY={KEY}\nTO=camila@acme.cl")
                );
            }
            other => panic!("sensitive text must be veiled for the cloud, got {other:?}"),
        }
    }

    /// A key typed into a request nobody graded is covered all the same.
    #[test]
    fn even_public_text_is_veiled_before_a_cloud_model_sees_it() {
        let text = format!("Why does curl -H 'Authorization: Bearer {KEY}' return 401?");
        match cloud_only()
            .disclosure(&question(PrivacyGrade::Public, &text), &Veil::new())
            .unwrap()
        {
            Disclosure::Veiled { veiled, .. } => {
                assert!(!veiled.texts[1].contains(KEY), "{}", veiled.texts[1])
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn restricted_and_secret_text_and_pictures_do_not_leave() {
        let routed = cloud_only();
        let veil = Veil::new();
        for grade in [PrivacyGrade::Restricted, PrivacyGrade::Secret] {
            let refused = routed.disclosure(&question(grade, "my diary, in full"), &veil);
            assert!(
                matches!(refused, Err(RoutedError::Withheld(_))),
                "{grade}: {refused:?}"
            );
        }
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        let mut with_picture = question(PrivacyGrade::Internal, "what is on my screen?");
        with_picture.image = Some(archie_model_runtime::VisionImage {
            mime: archie_model_runtime::VisionMime::Png,
            bytes: &png,
            width: 1,
            height: 1,
        });
        assert!(matches!(
            routed.disclosure(&with_picture, &veil),
            Err(RoutedError::Withheld(_))
        ));
    }

    /// With a model on the machine, private text is given to it as it is, and to nothing else.
    #[test]
    fn a_local_model_is_asked_unveiled_and_the_cloud_is_not_asked_at_all() {
        std::env::set_var(
            "NEBIUS_TOKEN_FACTORY_KEY",
            "a-key-for-tests-that-call-nothing",
        );
        let mut registry = ModelRegistry::load_from_dir(shipped_models()).unwrap();
        registry
            .register(ModelDescriptor {
                id: ModelId::parse("local.qwen").unwrap(),
                display_name: "Qwen, here".into(),
                provider: ProviderKind::OpenAiCompatible,
                locality: Locality::Local,
                node: None,
                context_window: 32_768,
                max_output_tokens: 4096,
                capabilities: vec![ModelRole::CognitiveResponse.capability()],
                max_privacy_grade: PrivacyGrade::Restricted,
                retention: DataRetention::Unknown,
                availability: Availability::Ready,
                cost: TokenCost::FREE,
                latency_ema_ms: Some(900),
                quality_tier: 40,
                required_accelerator_bytes: None,
                labels: BTreeMap::from([(
                    "base-url".to_string(),
                    "http://127.0.0.1:11434/v1/".to_string(),
                )]),
            })
            .unwrap();
        let routed = Routed::new(registry);
        let text = format!("Rotate this key: {KEY}");
        for grade in [
            PrivacyGrade::Public,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
        ] {
            match routed
                .disclosure(&question(grade, &text), &Veil::new())
                .unwrap()
            {
                Disclosure::Kept { models } => {
                    assert_eq!(models, [ModelId::parse("local.qwen").unwrap()], "{grade}")
                }
                other => panic!("{grade}: {other:?}"),
            }
        }
    }

    #[test]
    fn chat_request_with_local_model_is_kept_and_secret_is_withheld() {
        let mut registry = ModelRegistry::load_from_dir(shipped_models()).unwrap();
        registry
            .register(ModelDescriptor {
                id: ModelId::parse("local.qwen").unwrap(),
                display_name: "Qwen 2.5 7B (local)".to_string(),
                provider: ProviderKind::OpenAiCompatible,
                locality: Locality::Local,
                node: None,
                context_window: 32_768,
                max_output_tokens: 4096,
                capabilities: vec![ModelRole::CognitiveResponse.capability()],
                max_privacy_grade: PrivacyGrade::Restricted,
                retention: DataRetention::Unknown,
                availability: Availability::Ready,
                cost: TokenCost::FREE,
                latency_ema_ms: Some(900),
                quality_tier: 40,
                required_accelerator_bytes: None,
                labels: BTreeMap::from([(
                    "base-url".to_string(),
                    "http://127.0.0.1:11434/v1/".to_string(),
                )]),
            })
            .unwrap();
        let routed = Routed::new(registry);
        let req = archie_model_runtime::ToolChatRequest {
            model: "local.qwen".into(),
            messages: vec![
                archie_model_runtime::ToolChatMessage::System("eres archie".into()),
                archie_model_runtime::ToolChatMessage::User(format!("Mi clave es {KEY}")),
            ],
            tools: vec![],
            max_tokens: 200,
            temperature: 0.3,
        };

        // Secret is withheld
        assert!(matches!(
            routed.disclose_chat(
                &req,
                PrivacyGrade::Secret,
                ModelRole::CognitiveResponse,
                &Veil::new()
            ),
            Err(RoutedError::Withheld(_))
        ));

        // Restricted is Kept because local model is available
        match routed
            .disclose_chat(
                &req,
                PrivacyGrade::Restricted,
                ModelRole::CognitiveResponse,
                &Veil::new(),
            )
            .unwrap()
        {
            archie_model_runtime::routed::ChatDisclosure::Kept { models, request } => {
                assert_eq!(models, [ModelId::parse("local.qwen").unwrap()]);
                assert_eq!(request.messages.len(), 2);
            }
            _ => panic!("expected Kept"),
        }
    }

    fn chat(
        messages: Vec<archie_model_runtime::ToolChatMessage>,
    ) -> archie_model_runtime::ToolChatRequest {
        archie_model_runtime::ToolChatRequest {
            model: "unchosen".into(),
            messages,
            tools: vec![archie_model_runtime::ToolDefinition {
                name: "deploy".into(),
                description: "Deploys the service with the token it is given.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"token": {"type": "string"}, "notify": {"type": "string"}}
                }),
            }],
            max_tokens: 200,
            temperature: 0.3,
        }
    }

    /// With only cloud models, internal and sensitive chat is veiled, not refused for want
    /// of a model at its own grade; restricted and secret chat still go nowhere.
    #[test]
    fn cloud_only_chat_is_veiled_up_to_sensitive_and_withheld_above() {
        use archie_model_runtime::routed::ChatDisclosure;
        use archie_model_runtime::ToolChatMessage as M;
        let routed = cloud_only();
        let req = chat(vec![
            M::System("eres archie".into()),
            M::User(format!("Mi clave es {KEY}")),
        ]);
        for grade in [PrivacyGrade::Internal, PrivacyGrade::Sensitive] {
            match routed.disclose_chat(&req, grade, ModelRole::CognitiveResponse, &Veil::new()) {
                Ok(ChatDisclosure::Veiled {
                    models, request, ..
                }) => {
                    assert!(!models.is_empty(), "{grade}");
                    let body = request.body().to_string();
                    assert!(!body.contains(KEY), "{grade}: {body}");
                }
                other => panic!("{grade}: expected veiled, got {other:?}"),
            }
        }
        for grade in [PrivacyGrade::Restricted, PrivacyGrade::Secret] {
            assert!(
                matches!(
                    routed.disclose_chat(&req, grade, ModelRole::CognitiveResponse, &Veil::new()),
                    Err(RoutedError::Withheld(_))
                ),
                "{grade}"
            );
        }
    }

    /// A key that appears only in the arguments of an earlier tool call is covered too,
    /// in the whole request as it would be sent, and the call keeps its id, its name and
    /// well-formed arguments.
    #[test]
    fn tool_call_arguments_leave_veiled() {
        use archie_model_runtime::routed::ChatDisclosure;
        use archie_model_runtime::{StreamEvent, StreamUnveiler, ToolCall, ToolChatMessage as M};
        let veil = Veil::new().knowing(EntityKind::Person, "Camila Rojas");
        let arguments = serde_json::json!({
            "token": KEY,
            "notify": ["camila@acme.cl", {"who": "Camila Rojas \"la jefa\""}],
            "retries": 3
        })
        .to_string();
        let req = chat(vec![
            M::System("eres archie".into()),
            M::User("despliega el servicio".into()),
            M::Assistant {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_7".into(),
                    name: "deploy".into(),
                    arguments: arguments.clone(),
                }],
            },
            M::Tool {
                call_id: "call_7".into(),
                content: "ok".into(),
            },
        ]);
        let Ok(ChatDisclosure::Veiled {
            request, unveil, ..
        }) = cloud_only().disclose_chat(
            &req,
            PrivacyGrade::Sensitive,
            ModelRole::CognitiveResponse,
            &veil,
        )
        else {
            panic!("expected veiled");
        };
        let body = request.body().to_string();
        for kept_here in [KEY, "camila@acme.cl", "Camila Rojas"] {
            assert!(
                !body.contains(kept_here),
                "{kept_here} would have left: {body}"
            );
        }
        let M::Assistant { tool_calls, .. } = &request.messages[2] else {
            panic!("{:?}", request.messages[2]);
        };
        assert_eq!(
            (tool_calls[0].id.as_str(), tool_calls[0].name.as_str()),
            ("call_7", "deploy")
        );
        let sent: serde_json::Value = serde_json::from_str(&tool_calls[0].arguments).unwrap();
        assert_eq!(sent["retries"], 3);
        assert!(sent["token"].as_str().unwrap().starts_with("SECRET_"));

        // What the model calls back with is restored inside the JSON, quotes and all.
        let mut unveiler = StreamUnveiler::new(Some(unveil));
        let back = unveiler.feed(StreamEvent::ToolCall(tool_calls[0].clone()));
        let StreamEvent::ToolCall(call) = &back[0] else {
            panic!("{back:?}");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&call.arguments).unwrap(),
            serde_json::from_str::<serde_json::Value>(&arguments).unwrap()
        );
    }

    /// A tool definition is the program's text and is not rewritten; one that holds a
    /// sensitive value is not sent at all.
    #[test]
    fn a_tool_definition_with_a_secret_is_withheld() {
        use archie_model_runtime::ToolChatMessage as M;
        let mut req = chat(vec![M::User("despliega".into())]);
        req.tools[0].description = format!("Deploys with the token {KEY}.");
        assert!(matches!(
            cloud_only().disclose_chat(
                &req,
                PrivacyGrade::Internal,
                ModelRole::CognitiveResponse,
                &Veil::new()
            ),
            Err(RoutedError::Withheld(_))
        ));
    }

    /// Against Nebius: a key, an address and a name go in veiled, Nemotron writes the file,
    /// and the real values are in what comes back.
    #[tokio::test]
    #[ignore = "calls Nebius: needs NEBIUS_TOKEN_FACTORY_KEY and the network"]
    async fn a_cloud_model_writes_a_file_with_a_key_it_never_saw() {
        let routed = Routed::from_dir(shipped_models()).unwrap();
        let text = format!(
            "Write a .env file with exactly three lines: OPENAI_API_KEY set to {KEY}, ALERT_EMAIL set to \
             camila@acme.cl, and OWNER set to Camila Rojas. Reply with the file only."
        );
        let veil = Veil::new().knowing(EntityKind::Person, "Camila Rojas");
        let asked = question(PrivacyGrade::Sensitive, &text);
        if let Disclosure::Veiled { veiled, .. } = routed.disclosure(&asked, &veil).unwrap() {
            println!("what leaves the machine:\n{}\n", veiled.texts[1]);
        }
        let answer = routed.ask_private(&asked, &veil).await.unwrap();
        println!(
            "{} answered in {:.1} s; covered {:?}\n{}",
            answer.answered.model, answer.answered.seconds, answer.covered, answer.answered.text
        );
        assert!(answer.left_the_machine);
        assert_eq!(
            answer.covered,
            BTreeMap::from([
                ("EMAIL".to_string(), 1),
                ("PERSON".to_string(), 1),
                ("SECRET".to_string(), 1)
            ])
        );
        for restored in [KEY, "camila@acme.cl", "Camila Rojas"] {
            assert!(
                answer.answered.text.contains(restored),
                "{restored} is missing from the answer"
            );
        }
    }
}
