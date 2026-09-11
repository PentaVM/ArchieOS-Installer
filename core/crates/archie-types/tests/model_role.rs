//! Roles are capabilities, and that has to keep being true.
//!
//! The whole reason this type is thin is that the router already selects by capability.
//! If a role ever stops compiling to a valid one, or two roles collide, or a role starts
//! matching an ordinary compute capability, the thinness stops being free and becomes a
//! second broken selection mechanism.

use archie_types::{Capability, CompatibilityStatus, ModelRole};
use std::collections::BTreeSet;

/// Every role has to produce a capability the router will accept.
///
/// `ModelRole::capability` unwraps, which is safe only while this holds. It is checked
/// here rather than trusted, because the failure would be a panic at the moment a node
/// advertises itself.
#[test]
fn every_role_names_a_capability_the_router_accepts() {
    for role in ModelRole::ALL {
        let parsed = Capability::parse(role.as_str());
        assert!(
            parsed.is_ok(),
            "{role} is not a valid capability: {parsed:?}"
        );
        assert_eq!(role.capability().to_string(), role.as_str());
    }
}

/// Two roles that name the same capability are one role with two names, and a request
/// for either would silently match a model that meant the other.
#[test]
fn no_two_roles_share_a_capability() {
    let mut seen = BTreeSet::new();
    for role in ModelRole::ALL {
        assert!(
            seen.insert(role.as_str()),
            "{role} duplicates a capability already used by another role"
        );
    }
    assert_eq!(seen.len(), ModelRole::ALL.len());
}

/// The hierarchy has to be real, because it is what lets a model be more specific than
/// the request without being a different kind of thing.
#[test]
fn a_more_specific_capability_still_satisfies_the_role() {
    let frontend = ModelRole::SpeechFrontend.capability();
    let duplex = Capability::parse("speech.frontend.duplex").expect("valid");

    assert!(
        duplex.satisfies(&frontend),
        "a duplex frontend is a frontend, and a request for one must find it"
    );
    assert!(
        !frontend.satisfies(&duplex),
        "a plain frontend must not answer a request that needed duplex"
    );
}

/// A role must not quietly capture capabilities that are not roles.
///
/// `compute.inference` is what every model that runs at all advertises. If it resolved to
/// a role, every model would claim to be a speech renderer.
#[test]
fn ordinary_capabilities_are_not_roles() {
    for ordinary in [
        "compute.inference",
        "compute.embedding",
        "compute.cpu",
        "network.lan",
        "storage.object",
    ] {
        let capability = Capability::parse(ordinary).expect("valid");
        assert_eq!(
            ModelRole::from_capability(&capability),
            None,
            "{ordinary} was mistaken for a role"
        );
    }
}

/// And the mapping goes both ways for the things that are roles.
#[test]
fn a_role_capability_resolves_back_to_its_role() {
    for role in ModelRole::ALL {
        assert_eq!(ModelRole::from_capability(&role.capability()), Some(role));
    }

    // Including a more specific form, which is how a model advertises a stronger version
    // of the same job.
    let duplex = Capability::parse("speech.frontend.duplex").expect("valid");
    assert_eq!(
        ModelRole::from_capability(&duplex),
        Some(ModelRole::SpeechFrontend)
    );
}

/// Speaking and hearing are different jobs, and a unified model is a third thing.
///
/// Collapsing them would mean a request for a renderer could be answered by something
/// that only listens — which fails silently, as no audio.
#[test]
fn the_speech_roles_are_three_distinct_jobs() {
    let frontend = ModelRole::SpeechFrontend.capability();
    let renderer = ModelRole::SpeechRenderer.capability();
    let unified = ModelRole::SpeechUnified.capability();

    assert!(!frontend.satisfies(&renderer));
    assert!(!renderer.satisfies(&frontend));
    assert!(!unified.satisfies(&frontend));
    assert!(!frontend.satisfies(&unified));
}

/// Estimated is not measured, and nothing may promote it.
///
/// This is the distinction the whole type exists for. Arithmetic saying a model fits is
/// not the same as having watched it run, and a wizard that conflates them tells somebody
/// a guess in the voice of a fact.
#[test]
fn only_a_measured_status_counts_as_evidence() {
    assert!(CompatibilityStatus::SupportedMeasured.is_measured());

    for guessed in [
        CompatibilityStatus::SupportedDocumented,
        CompatibilityStatus::CompatibleEstimated,
        CompatibilityStatus::UnsupportedDocumented,
        CompatibilityStatus::InsufficientResources,
        CompatibilityStatus::MissingRuntime,
        CompatibilityStatus::MissingDependency,
        CompatibilityStatus::Unknown,
    ] {
        assert!(
            !guessed.is_measured(),
            "{guessed:?} is a prediction and must not read as evidence"
        );
    }
}

/// But unmeasured must still be installable, or nothing could ever become measured.
#[test]
fn something_unmeasured_can_still_be_offered() {
    assert!(CompatibilityStatus::CompatibleEstimated.is_installable());
    assert!(CompatibilityStatus::SupportedDocumented.is_installable());

    for refused in [
        CompatibilityStatus::UnsupportedDocumented,
        CompatibilityStatus::InsufficientResources,
        CompatibilityStatus::MissingRuntime,
        CompatibilityStatus::MissingDependency,
        CompatibilityStatus::Unknown,
    ] {
        assert!(
            !refused.is_installable(),
            "{refused:?} would fail, and offering it wastes a download"
        );
    }
}

/// Every status says something a person can act on.
#[test]
fn every_status_explains_itself() {
    for status in [
        CompatibilityStatus::SupportedMeasured,
        CompatibilityStatus::SupportedDocumented,
        CompatibilityStatus::CompatibleEstimated,
        CompatibilityStatus::UnsupportedDocumented,
        CompatibilityStatus::InsufficientResources,
        CompatibilityStatus::MissingRuntime,
        CompatibilityStatus::MissingDependency,
        CompatibilityStatus::Unknown,
    ] {
        assert!(status.explain().len() > 12, "{status:?} explains nothing");
    }
}

/// Roles survive the wire, because a manifest and a fabric advertisement both carry them.
#[test]
fn roles_and_statuses_round_trip_through_json() {
    for role in ModelRole::ALL {
        let text = serde_json::to_string(&role).expect("serialisable");
        assert_eq!(serde_json::from_str::<ModelRole>(&text).unwrap(), role);
    }
    let status = CompatibilityStatus::CompatibleEstimated;
    let text = serde_json::to_string(&status).expect("serialisable");
    assert_eq!(text, "\"compatible_estimated\"");
    assert_eq!(
        serde_json::from_str::<CompatibilityStatus>(&text).unwrap(),
        status
    );
}
