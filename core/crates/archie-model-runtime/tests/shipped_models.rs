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
        let endpoint = ProviderEndpoint::for_model(model).unwrap_or_else(|e| panic!("{}: {e}", model.id));
        assert_eq!(endpoint.locality(), model.locality, "{}", model.id);
        if model.locality == Locality::Cloud {
            cloud += 1;
            assert_eq!(endpoint.base_url().scheme(), "https", "{}", model.id);
            assert!(provider_model_id(model).unwrap().contains('/'), "{} names no provider model", model.id);
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
    println!("candidates, best first: {}", candidates.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", "));
    assert!(candidates[0].as_str().starts_with("nebius.nemotron"), "Nemotron is the default planner");

    let answered = routed.ask(&question).await.unwrap();
    println!("{} answered in {:.1} s:\n{}", answered.model, answered.seconds, answered.text);
    let (start, end) = (answered.text.find('{').unwrap(), answered.text.rfind('}').unwrap());
    let plan: serde_json::Value = serde_json::from_str(&answered.text[start..=end]).unwrap();
    assert!(plan["skill"].is_string());
    assert!(!plan["steps"].as_array().unwrap().is_empty());

    // Private data has no local reasoning model to go to here, so it is refused, not sent.
    let private = Question { privacy: PrivacyGrade::Restricted, ..question };
    assert!(routed.candidates(&private).is_err());
}
