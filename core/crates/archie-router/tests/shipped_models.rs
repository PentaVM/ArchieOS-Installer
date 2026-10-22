//! The model manifests this repository ships, loaded and routed as a node would.
//!
//! A manifest that parses but cannot be chosen is worse than a missing one: it looks
//! installed. These tests take `models/` exactly as it is staged onto a node and ask the
//! router the questions the rest of the system asks.

use archie_registry::ModelRegistry;
use archie_router::{Router, RoutingRequest, Weights};
use archie_types::{Availability, Locality, ModelRole, PrivacyGrade};
use std::path::{Path, PathBuf};

fn shipped_models() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .join("models")
}

fn shipped() -> ModelRegistry {
    ModelRegistry::load_from_dir(shipped_models()).expect("every shipped model manifest loads")
}

/// A screen can show anything, so whatever acts on it must be allowed restricted data.
/// IRIS is local; if its manifest ever said otherwise this is where it would show.
#[test]
fn the_screen_is_handed_to_iris_even_when_it_is_restricted() {
    let registry = shipped();
    let request = RoutingRequest::new(PrivacyGrade::Restricted, 24, 8)
        .requiring(ModelRole::VisualActor.capability());
    let outcome = Router::new(Weights::default()).route(&registry, &request);
    let chosen = outcome
        .selected_model()
        .unwrap_or_else(|| panic!("nothing can act on a screen: {}", outcome.explain()));
    let model = registry
        .get(chosen)
        .expect("the chosen model is registered");
    assert_eq!(model.locality, Locality::Local);
    assert!(model.id.as_str().starts_with("iris."), "chose {}", model.id);
}

#[test]
fn iris_holds_the_roles_its_manifest_claims_and_no_speech_or_reasoning_role() {
    let registry = shipped();
    let iris = registry
        .iter()
        .find(|m| m.id.as_str().starts_with("iris."))
        .expect("IRIS is shipped");
    for role in [
        ModelRole::VisionModel,
        ModelRole::VisualActor,
        ModelRole::EmbeddingModel,
    ] {
        assert!(iris.provides(&role.capability()), "IRIS should hold {role}");
    }
    for role in [
        ModelRole::CognitiveResponse,
        ModelRole::ReasoningWorker,
        ModelRole::Tts,
    ] {
        assert!(
            !iris.provides(&role.capability()),
            "IRIS must not claim {role}"
        );
    }
}

/// The weights a manifest names are fetched and checked by hash before they are loaded,
/// so a manifest without one cannot be installed.
#[test]
fn a_local_model_names_the_weights_it_runs_by_hash() {
    for model in shipped().iter().filter(|m| m.locality == Locality::Local) {
        let hash = model
            .labels
            .get("weights_sha256")
            .map(String::as_str)
            .unwrap_or("");
        assert!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "{} names no SHA-256 for its weights",
            model.id
        );
    }
}

/// IRIS sees and acts; it does not reason. What to do is asked of a reasoning model, and
/// among the ones shipped the default is a Nemotron.
#[test]
fn thinking_is_asked_of_a_reasoning_model_and_never_of_iris() {
    let registry = shipped();
    for role in [ModelRole::ReasoningWorker, ModelRole::CognitiveResponse] {
        let request =
            RoutingRequest::new(PrivacyGrade::Public, 400, 2000).requiring(role.capability());
        let outcome = Router::new(Weights::default()).route(&registry, &request);
        let chosen = outcome
            .selected_model()
            .unwrap_or_else(|| panic!("{role}: {}", outcome.explain()));
        assert!(
            chosen.as_str().starts_with("nebius.nemotron"),
            "{role} went to {chosen}"
        );
    }
}

/// Bonsai ships offline, so as staged onto a node without llama-server only cloud
/// reasoning models are usable and a private request is refused. It is not sent to a
/// cloud model, and it is not handed to IRIS because IRIS happens to be local.
#[test]
fn a_private_question_is_refused_when_no_local_model_reasons() {
    let registry = shipped();
    assert!(
        registry.iter().any(|m| m.locality == Locality::Local
            && m.provides(&ModelRole::ReasoningWorker.capability())
            && m.availability == Availability::Offline),
        "Bonsai is shipped, offline"
    );
    let request = RoutingRequest::new(PrivacyGrade::Restricted, 400, 2000)
        .requiring(ModelRole::ReasoningWorker.capability());
    let outcome = Router::new(Weights::default()).route(&registry, &request);
    assert_eq!(outcome.selected_model(), None, "{}", outcome.explain());
}

/// On a node whose llama-server serves Bonsai, its installed manifests say ready; the
/// local model then reasons over private context and nothing goes to a cloud model.
#[test]
fn a_private_question_is_reasoned_by_the_local_model() {
    let mut registry = shipped();
    let offline: Vec<_> = registry
        .iter()
        .filter(|m| m.availability == Availability::Offline)
        .cloned()
        .collect();
    for mut model in offline {
        registry.remove(&model.id);
        model.availability = Availability::Ready;
        registry.register(model).unwrap();
    }
    let request = RoutingRequest::new(PrivacyGrade::Restricted, 400, 2000)
        .requiring(ModelRole::ReasoningWorker.capability());
    let outcome = Router::new(Weights::default()).route(&registry, &request);
    let chosen = outcome
        .selected_model()
        .unwrap_or_else(|| panic!("{}", outcome.explain()));
    let model = registry.iter().find(|m| &m.id == chosen).unwrap();
    assert_eq!(model.locality, Locality::Local);
    assert!(
        chosen.as_str().starts_with("local.bonsai"),
        "chose {chosen}"
    );
}
