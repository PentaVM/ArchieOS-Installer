//! Provider registry contracts, and validation of the manifests this repository ships.
//!
//! Two things are being protected here. The first is that a credential can never end up
//! in a manifest — manifests are committed and shared, and a secret in one is a secret
//! in the git history forever. The second is that a provider which cannot run says *why*
//! it cannot, because "no world data on the globe" has at least four different causes
//! and they need four different responses from the operator.

use archie_registry::providers::{
    AccessClass, Authentication, Backoff, BackoffStrategy, Cadence, Quota, Transport,
};
use archie_registry::{ProviderContext, ProviderManifest, ProviderRegistry, ProviderStatus};
use archie_types::{Capability, CapabilitySet};
use std::path::{Path, PathBuf};

fn shipped_providers() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .join("providers/data")
}

fn caps(names: &[&str]) -> CapabilitySet {
    names
        .iter()
        .map(|n| Capability::parse(*n).expect("test capability must be valid"))
        .collect()
}

fn manifest_toml(extra: &str) -> String {
    format!(
        r#"
id = "test.feed"
title = "Test feed"
description = "A feed for tests"
capability = "world.seismic"
records = ["observation"]

[access]
transport = "https"
authentication = "none"
access_class = "public"

[cadence]
poll_seconds = 60
{extra}
"#
    )
}

fn parse(extra: &str) -> ProviderManifest {
    toml::from_str(&manifest_toml(extra)).expect("test manifest must parse")
}

// ────────────────────────────────────────────────────────────── manifest rules

#[test]
fn a_minimal_manifest_parses_and_validates() {
    let manifest = parse("");
    assert!(manifest.validate().is_ok());
    assert_eq!(manifest.access.transport, Transport::Https);
    assert_eq!(manifest.access.authentication, Authentication::None);
    // Backoff has a default, because a feed that fails must not become a feed that
    // hammers, and forgetting to write one is easy.
    assert_eq!(manifest.cadence.backoff, Backoff::default());
    assert!(manifest.cadence.backoff.jitter);
}

#[test]
fn a_provider_that_authenticates_must_name_its_secret() {
    // The rule that keeps credentials out of committed manifests.
    let mut manifest = parse("");
    manifest.access.authentication = Authentication::ApiKey;
    let error = manifest.validate().unwrap_err();
    assert!(
        error.contains("secret_handle"),
        "expected a complaint about the missing secret handle, got: {error}"
    );

    manifest.access.secret_handle = Some("test.api_key".into());
    assert!(manifest.validate().is_ok());
}

#[test]
fn a_provider_that_needs_no_secret_must_not_name_one() {
    // A stray secret_handle on an unauthenticated feed means someone misunderstood the
    // model, and the misunderstanding is worth surfacing before it becomes a habit.
    let mut manifest = parse("");
    manifest.access.secret_handle = Some("pointless.key".into());
    assert!(manifest.validate().is_err());
}

#[test]
fn attribution_required_means_an_attribution_string_must_exist() {
    let mut manifest = parse("");
    manifest.access.access_class = AccessClass::AttributionRequired;
    assert!(
        manifest.validate().is_err(),
        "a provider whose terms require credit must carry the credit line"
    );

    manifest.access.attribution = Some("Some Agency".into());
    assert!(manifest.validate().is_ok());
}

#[test]
fn restricted_records_may_never_reach_a_shared_display() {
    // The observatory is a wall screen. Restricted-class records are usable internally
    // and must not be rendered there.
    assert!(!AccessClass::Restricted.may_be_displayed());
    assert!(AccessClass::Public.may_be_displayed());
    assert!(AccessClass::ResearchOnly.may_be_displayed());
    assert!(AccessClass::AttributionRequired.requires_attribution());
    assert!(!AccessClass::Public.requires_attribution());
}

#[test]
fn a_malformed_id_is_rejected() {
    for bad in [
        "",
        "Has.Capitals",
        "1leading.digit",
        "has space",
        "trailing.",
    ] {
        let mut manifest = parse("");
        manifest.id = bad.to_string();
        assert!(
            manifest.validate().is_err(),
            "id {bad:?} should have been rejected"
        );
    }
}

// ───────────────────────────────────────────────────────────────────── cadence

#[test]
fn configuration_can_only_ever_make_archie_slower_than_the_terms_permit() {
    // A quota is a hard limit, not a target: exceeding one is how an operator loses
    // access to a feed they depend on. An over-eager poll_seconds is clamped, and the
    // clamping is reported rather than silently applied.
    let cadence = Cadence {
        poll_seconds: 1,
        min_poll_seconds: Some(60),
        quota: None,
        backoff: Backoff::default(),
    };
    assert_eq!(cadence.effective_poll_seconds(), 60);
    assert!(cadence.is_clamped(), "the operator should be told");

    // A quota also raises the floor.
    let quota_bound = Cadence {
        poll_seconds: 1,
        min_poll_seconds: None,
        // 100 requests per hour is one every 36 seconds.
        quota: Some(Quota {
            requests: 100,
            per_seconds: 3600,
            burst: None,
        }),
        backoff: Backoff::default(),
    };
    assert_eq!(quota_bound.effective_poll_seconds(), 36);
}

#[test]
fn a_conservative_cadence_is_left_alone() {
    let cadence = Cadence {
        poll_seconds: 600,
        min_poll_seconds: Some(60),
        quota: None,
        backoff: Backoff::default(),
    };
    assert_eq!(cadence.effective_poll_seconds(), 600);
    assert!(!cadence.is_clamped());
}

#[test]
fn backoff_grows_and_is_capped() {
    let backoff = Backoff {
        strategy: BackoffStrategy::Exponential,
        initial_seconds: 10,
        max_seconds: 100,
        jitter: false,
    };
    assert_eq!(backoff.delay_seconds(0), 10);
    assert_eq!(backoff.delay_seconds(1), 20);
    assert_eq!(backoff.delay_seconds(2), 40);
    assert_eq!(backoff.delay_seconds(3), 80);
    assert_eq!(backoff.delay_seconds(4), 100, "capped");
    // A very large attempt count must not overflow into a tiny delay.
    assert_eq!(backoff.delay_seconds(1_000_000), 100);
}

#[test]
fn a_zero_request_quota_does_not_divide_by_zero() {
    let quota = Quota {
        requests: 0,
        per_seconds: 60,
        burst: None,
    };
    assert_eq!(quota.min_interval_seconds(), u32::MAX);
}

// ──────────────────────────────────────────────────────────────── resolution

#[test]
fn a_provider_loads_when_the_node_has_its_capability() {
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();

    let plan = registry.resolve(&ProviderContext::new(caps(&["world.seismic"])));
    assert!(plan.is_available("test.feed"));
    assert_eq!(plan.active, vec!["test.feed".to_string()]);
}

#[test]
fn a_missing_capability_is_reported_as_such_and_not_as_a_missing_credential() {
    // Four different causes of an empty globe need four different fixes. This one is
    // "the operator did not enable this domain", not "add an API key".
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();

    let plan = registry.resolve(&ProviderContext::new(caps(&["compute.cpu"])));
    match &plan.statuses["test.feed"] {
        ProviderStatus::CapabilityUnavailable { missing } => {
            assert_eq!(missing.as_str(), "world.seismic");
        }
        other => panic!("expected a capability gap, got {other:?}"),
    }
    assert!(plan.active.is_empty());
    assert!(plan.excluded()[0].1.contains("world.seismic"));
}

#[test]
fn an_unconfigured_secret_is_an_ordinary_state_not_an_error() {
    let mut registry = ProviderRegistry::new();
    let mut manifest = parse("");
    manifest.access.authentication = Authentication::ApiKey;
    manifest.access.secret_handle = Some("test.api_key".into());
    registry.register(manifest).unwrap();

    let context = ProviderContext::new(caps(&["world.seismic"]));
    match &registry.resolve(&context).statuses["test.feed"] {
        ProviderStatus::NoCredentials { secret_handle } => {
            assert_eq!(secret_handle, "test.api_key")
        }
        other => panic!("expected no-credentials, got {other:?}"),
    }

    // Supplying the secret NAME (never a value) makes it available.
    let configured = ProviderContext::new(caps(&["world.seismic"])).with_secret("test.api_key");
    assert!(registry.resolve(&configured).is_available("test.feed"));
}

#[test]
fn cyber_providers_need_an_explicit_operator_grant() {
    // The data is public, but pulling it under the operator's own terms of service is a
    // decision they should make on purpose. Having the capability is not enough.
    let mut registry = ProviderRegistry::new();
    let mut manifest = parse("");
    manifest.id = "test.cyber".into();
    manifest.capability = Capability::parse("cyber.routing").unwrap();
    registry.register(manifest).unwrap();

    let unauthorized = ProviderContext::new(caps(&["cyber.routing"]));
    match &registry.resolve(&unauthorized).statuses["test.cyber"] {
        ProviderStatus::NotAuthorized { family } => assert_eq!(family, "cyber"),
        other => panic!("expected not-authorized, got {other:?}"),
    }

    let authorized = ProviderContext::new(caps(&["cyber.routing"])).authorize_family("cyber");
    assert!(registry.resolve(&authorized).is_available("test.cyber"));
}

#[test]
fn world_observation_needs_no_special_grant() {
    // Observing public geophysical data is not a privileged act; requiring a ceremony
    // for it would train operators to click through the ones that matter.
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();
    assert!(registry
        .resolve(&ProviderContext::new(caps(&["world.seismic"])))
        .is_available("test.feed"));
}

#[test]
fn an_explicitly_disabled_provider_does_not_run() {
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();
    let context = ProviderContext::new(caps(&["world.seismic"])).disable("test.feed");
    assert_eq!(
        registry.resolve(&context).statuses["test.feed"],
        ProviderStatus::Disabled
    );
}

#[test]
fn an_invalid_manifest_is_marked_invalid_and_does_not_stop_the_others() {
    let mut registry = ProviderRegistry::new();
    let mut broken = parse("");
    broken.id = "broken.feed".into();
    broken.access.authentication = Authentication::ApiKey; // no secret handle
    registry.register(broken).unwrap();
    registry.register(parse("")).unwrap();

    let plan = registry.resolve(&ProviderContext::new(caps(&["world.seismic"])));
    assert!(matches!(
        plan.statuses["broken.feed"],
        ProviderStatus::Invalid { .. }
    ));
    assert!(
        plan.is_available("test.feed"),
        "one bad manifest must not take the rest down"
    );
}

#[test]
fn a_duplicate_provider_id_is_refused() {
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();
    assert!(registry.register(parse("")).is_err());
}

#[test]
fn every_excluded_provider_carries_an_explanation() {
    let mut registry = ProviderRegistry::new();
    registry.register(parse("")).unwrap();
    let plan = registry.resolve(&ProviderContext::new(caps(&["compute.cpu"])));
    for (id, reason) in plan.excluded() {
        assert!(!reason.is_empty(), "{id} was excluded with no explanation");
    }
}

// ─────────────────────────────────────────────────── the manifests we ship

#[test]
fn every_shipped_provider_manifest_parses_and_validates() {
    let registry =
        ProviderRegistry::load_from_dir(shipped_providers()).expect("shipped providers must parse");
    assert!(
        registry.len() >= 10,
        "expected the initial provider set, found {}",
        registry.len()
    );
    for manifest in registry.manifests() {
        manifest
            .validate()
            .unwrap_or_else(|e| panic!("shipped provider {} is invalid: {e}", manifest.id));
    }
}

#[test]
fn no_shipped_manifest_contains_anything_that_looks_like_a_credential() {
    // Manifests are committed. A secret in one is a secret in the git history forever,
    // so this scans the raw text rather than the parsed structure — a credential pasted
    // into a field the parser ignores would still be in the repository.
    let dir = shipped_providers();
    let mut checked = 0;
    for path in walk(&dir) {
        let text = std::fs::read_to_string(&path).unwrap();
        let lower = text.to_lowercase();
        for needle in [
            "api_key =",
            "apikey =",
            "password =",
            "token =",
            "bearer ",
            "secret =",
            "-----begin",
        ] {
            assert!(
                !lower.contains(needle),
                "{} appears to contain a credential ({needle:?}); manifests name secrets, \
                 they never hold them",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(checked >= 10, "expected to scan the shipped manifests");
}

#[test]
fn every_shipped_provider_documents_what_its_feed_cannot_see() {
    // The field that stops an empty region being read as a quiet one. Exempt only the
    // feeds whose coverage genuinely has no interesting blind spot.
    let registry = ProviderRegistry::load_from_dir(shipped_providers()).unwrap();
    let exempt = ["noaa.swpc"];

    for manifest in registry.manifests() {
        if exempt.contains(&manifest.id.as_str()) {
            continue;
        }
        assert!(
            !manifest.coverage.known_gaps.is_empty(),
            "provider {} documents no known gaps; a layer rendered without them lies by \
             omission",
            manifest.id
        );
    }
}

#[test]
fn every_shipped_provider_respects_a_polling_floor() {
    // Either an explicit terms floor or a quota. A feed with neither invites an operator
    // to configure a one-second poll against a government endpoint.
    let registry = ProviderRegistry::load_from_dir(shipped_providers()).unwrap();
    for manifest in registry.manifests() {
        let bounded =
            manifest.cadence.min_poll_seconds.is_some() || manifest.cadence.quota.is_some();
        assert!(
            bounded,
            "provider {} declares neither min_poll_seconds nor a quota",
            manifest.id
        );
        assert!(manifest.cadence.effective_poll_seconds() >= 1);
    }
}

#[test]
fn the_free_no_credential_feeds_run_on_a_bare_node() {
    // The point of choosing USGS, GDACS, CelesTrak and SWPC first: an operator with no
    // accounts anywhere still gets a working world model.
    let registry = ProviderRegistry::load_from_dir(shipped_providers()).unwrap();
    let context = ProviderContext::new(caps(&[
        "world.seismic",
        "world.disaster",
        "world.orbital",
        "world.spaceweather",
        "world.aviation",
        "world.weather",
        "world.imagery",
    ]));
    let plan = registry.resolve(&context);

    for id in [
        "usgs.earthquakes",
        "gdacs.alerts",
        "celestrak.elements",
        "noaa.swpc",
        // adsb.lol, NWS and NASA GIBS: the same "no account anywhere" bar as the four
        // above, and — with OpenSky's registered-tier manifest requiring credentials —
        // adsb.lol is what gives a bare node any live aircraft picture at all.
        "adsblol.aircraft",
        "nws.alerts",
        "nasa.gibs.blackmarble",
    ] {
        assert!(
            plan.is_available(id),
            "{id} should run with no credentials, got: {}",
            plan.statuses[id].explain()
        );
    }

    // And the credentialed ones correctly report themselves as unconfigured.
    let firms = registry.resolve(&ProviderContext::new(caps(&["world.wildfire"])));
    assert!(matches!(
        firms.statuses["nasa.firms"],
        ProviderStatus::NoCredentials { .. }
    ));
    let opensky = registry.resolve(&ProviderContext::new(caps(&["world.aviation"])));
    assert!(matches!(
        opensky.statuses["opensky.states"],
        ProviderStatus::NoCredentials { .. }
    ));
}

#[test]
fn nws_alerts_is_the_one_world_feed_that_is_not_global() {
    // The field this whole subsystem exists to keep honest: a display that draws US
    // severe-weather alerts and stays silent everywhere else must not be read as "the
    // rest of the world is calm" — it never looked.
    let registry = ProviderRegistry::load_from_dir(shipped_providers()).unwrap();
    let nws = registry.get("nws.alerts").expect("nws.alerts is shipped");
    assert_eq!(
        nws.coverage.geographic,
        Some(archie_registry::providers::GeographicCoverage::National)
    );
    assert!(
        nws.coverage
            .known_gaps
            .iter()
            .any(|gap| gap.contains("United States")),
        "the coverage gap must name what this feed does not see, not just gesture at it"
    );

    // Every other shipped world-domain feed claims global coverage. If a second
    // national-only feed is ever added this assertion should be relaxed to a list, not
    // silently widened — but today, NWS is the one exception, and that fact is worth a
    // test noticing if it stops being true.
    let other_national: Vec<&str> = registry
        .manifests()
        .filter(|m| m.capability.family() == "world" && m.id != "nws.alerts")
        .filter(|m| {
            m.coverage.geographic != Some(archie_registry::providers::GeographicCoverage::Global)
        })
        .map(|m| m.id.as_str())
        .collect();
    assert!(
        other_national.is_empty(),
        "expected every other world feed to claim global coverage, found: {other_national:?}"
    );
}

#[test]
fn shipped_capabilities_are_all_in_the_taxonomy() {
    // A provider naming a capability nothing else knows about can never be enabled.
    let registry = ProviderRegistry::load_from_dir(shipped_providers()).unwrap();
    let known: Vec<String> = archie_types::capability::well_known::all()
        .iter()
        .map(|c| c.as_str().to_string())
        .collect();
    for manifest in registry.manifests() {
        assert!(
            known.contains(&manifest.capability.as_str().to_string()),
            "provider {} needs capability {}, which is not in the taxonomy",
            manifest.id,
            manifest.capability
        );
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
    out
}
