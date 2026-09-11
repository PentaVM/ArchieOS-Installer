//! Routing by role, through the router that already existed.
//!
//! The claim this file exists to prove is a negative one: that adding model roles
//! required no change to the router at all. Roles compile to capabilities, the router has
//! selected by capability since before any of this, and so a request for "something that
//! can render speech in Spanish" is an ordinary routing request.
//!
//! If a future change adds a `required_role` field beside `required_capabilities`, these
//! tests keep passing while the architecture quietly grows a second way to pick a model.
//! So the last test here asserts the shape of the request itself.

use archie_registry::ModelRegistry;
use archie_router::{Router, RoutingRequest, Weights};
use archie_types::{
    Availability, Capability, Locality, ModelDescriptor, ModelId, ModelRole, PrivacyGrade,
    ProviderKind, TokenCost,
};
use std::collections::BTreeMap;

/// A model that holds exactly the roles it is given.
fn model_for(id: &str, roles: &[ModelRole], quality: u8) -> ModelDescriptor {
    ModelDescriptor {
        id: ModelId::parse(id).unwrap(),
        display_name: id.into(),
        provider: ProviderKind::LlamaCpp,
        locality: Locality::Local,
        node: None,
        context_window: 32_768,
        max_output_tokens: 4096,
        capabilities: roles.iter().map(|r| r.capability()).collect(),
        max_privacy_grade: PrivacyGrade::Restricted,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: Some(400),
        quality_tier: quality,
        required_accelerator_bytes: None,
        labels: BTreeMap::new(),
    }
}

fn registry_with(models: Vec<ModelDescriptor>) -> ModelRegistry {
    let mut registry = ModelRegistry::new();
    for model in models {
        registry.register(model).expect("distinct ids");
    }
    registry
}

fn ask_for(role: ModelRole) -> RoutingRequest {
    RoutingRequest::new(PrivacyGrade::Internal, 500, 500).requiring(role.capability())
}

/// The point of the whole design: one utterance may use three different models, and each
/// is chosen for its job rather than because it is "the Archie model".
#[test]
fn a_role_selects_the_model_that_holds_it() {
    let registry = registry_with(vec![
        model_for("local.voicebox", &[ModelRole::SpeechFrontend], 40),
        model_for("local.thinker", &[ModelRole::CognitiveResponse], 70),
        model_for("local.voice", &[ModelRole::SpeechRenderer], 40),
    ]);
    let router = Router::new(Weights::default());

    for (role, expected) in [
        (ModelRole::SpeechFrontend, "local.voicebox"),
        (ModelRole::CognitiveResponse, "local.thinker"),
        (ModelRole::SpeechRenderer, "local.voice"),
    ] {
        let outcome = router.route(&registry, &ask_for(role));
        assert_eq!(
            outcome.selected_model().map(|m| m.to_string()),
            Some(expected.to_string()),
            "asking for {role} chose the wrong model: {outcome:?}"
        );
    }
}

/// A model that holds several roles answers for each of them.
///
/// This is the unified speech model case, and it must not need a special path: it says it
/// is a frontend and a renderer, and both requests find it.
#[test]
fn one_model_can_hold_several_roles() {
    let registry = registry_with(vec![model_for(
        "local.duplex",
        &[
            ModelRole::SpeechFrontend,
            ModelRole::SpeechRenderer,
            ModelRole::SpeechUnified,
        ],
        60,
    )]);
    let router = Router::new(Weights::default());

    for role in [
        ModelRole::SpeechFrontend,
        ModelRole::SpeechRenderer,
        ModelRole::SpeechUnified,
    ] {
        assert!(
            router
                .route(&registry, &ask_for(role))
                .selected_model()
                .is_some(),
            "the unified model should answer for {role}"
        );
    }
}

/// Asking for a job nobody can do must refuse, not substitute.
///
/// The failure this prevents is the quiet one: a request for a speech renderer answered
/// by a text model, which produces no audio and no error either.
#[test]
fn a_role_nobody_holds_is_refused_rather_than_substituted() {
    let registry = registry_with(vec![model_for(
        "local.thinker",
        &[ModelRole::CognitiveResponse],
        90,
    )]);
    let router = Router::new(Weights::default());

    let outcome = router.route(&registry, &ask_for(ModelRole::SpeechRenderer));
    assert_eq!(
        outcome.selected_model(),
        None,
        "a model that only thinks was handed a job it cannot do: {outcome:?}"
    );

    // And the refusal names the reason, so an operator can see what is missing rather
    // than that "nothing was available".
    let considered = match &outcome {
        archie_router::RoutingOutcome::Escalate { considered, .. }
        | archie_router::RoutingOutcome::NoModelAvailable { considered } => considered.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    };
    // The router names *which* capability was missing, not merely that one was, so the
    // reason reads `missing_capability:speech.renderer`. That is the useful form: an
    // operator sees the job nobody could do rather than that something was absent.
    assert!(
        considered.iter().any(|c| {
            c.disqualifiers.iter().any(|r| {
                r.starts_with(archie_router::reasons::MISSING_CAPABILITY)
                    && r.contains("speech.renderer")
            })
        }),
        "the refusal must name the missing role: {considered:?}"
    );
}

/// A model may be more specific than the request without being a different thing.
#[test]
fn a_more_specific_model_answers_the_general_request() {
    let mut duplex = model_for("local.duplex", &[], 60);
    duplex.capabilities = vec![Capability::parse("speech.frontend.duplex").unwrap()];
    let registry = registry_with(vec![duplex]);
    let router = Router::new(Weights::default());

    assert!(
        router
            .route(&registry, &ask_for(ModelRole::SpeechFrontend))
            .selected_model()
            .is_some(),
        "a duplex frontend is a frontend"
    );
}

/// Privacy still gates a role request, because a role is not a way around a rule.
///
/// Speech is the most sensitive stream in the system — it is a microphone in a room — so
/// a role request that reached a cloud model without passing the privacy gate would be
/// the worst possible place for that hole to be.
#[test]
fn a_role_request_is_still_subject_to_privacy() {
    let mut remote = model_for("cloud.voice", &[ModelRole::SpeechRenderer], 90);
    // A cloud model needs a cloud provider: the registry refuses `LlamaCpp` at
    // `Locality::Cloud` because a local engine cannot be somewhere else, and it is right.
    remote.provider = ProviderKind::Anthropic;
    remote.locality = Locality::Cloud;
    remote.max_privacy_grade = PrivacyGrade::Internal;
    let registry = registry_with(vec![remote]);
    let router = Router::new(Weights::default());

    let restricted = RoutingRequest::new(PrivacyGrade::Restricted, 500, 500)
        .requiring(ModelRole::SpeechRenderer.capability());

    assert_eq!(
        router.route(&registry, &restricted).selected_model(),
        None,
        "restricted audio must not reach a cloud renderer merely because it holds the role"
    );
}

/// The architectural assertion, stated so it can fail.
///
/// Roles are expressed *in* `required_capabilities`. If somebody later adds a separate
/// role field, the router grows a second selection mechanism and the two can disagree
/// about the same model. This test says: the request that asks for a role is an ordinary
/// capability request, and nothing else.
#[test]
fn a_role_request_is_an_ordinary_capability_request() {
    let request = ask_for(ModelRole::CognitiveResponse);

    assert_eq!(
        request.required_capabilities,
        vec![Capability::parse("cognition.response").unwrap()],
        "a role must travel as a capability; a parallel field would be a second router"
    );

    // And it composes with ordinary capabilities, because a model needs both a job and
    // the means to do it.
    let composed = ask_for(ModelRole::CognitiveResponse)
        .requiring(Capability::parse("compute.inference").unwrap());
    assert_eq!(composed.required_capabilities.len(), 2);
}
