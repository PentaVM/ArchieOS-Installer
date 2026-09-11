//! The catalogue is a set of claims about the outside world. These check the ones that
//! can be checked without leaving the machine.
//!
//! The rule these exist to hold is Nelson's: everything offline where possible, and free
//! without exception. That rule does not break because somebody decides to spend money. It
//! breaks because a tile URL gets pasted in during a rendering problem, and the bill
//! arrives a month later.

use archie_worldd::layers::{
    available_without_setup, excluded_source, layer, Coverage, LayerAccess, LayerId, LayerKind,
    EXCLUDED, LAYERS,
};

const EVERY_ID: [LayerId; 8] = [
    LayerId::UsgsEarthquakes,
    LayerId::OpenSkyAircraft,
    LayerId::AdsbLolAircraft,
    LayerId::CelestrakSatellites,
    LayerId::NwsAlerts,
    LayerId::GdacsAlerts,
    LayerId::NasaBlackMarble,
    LayerId::NasaFirmsHotspots,
];

// ----------------------------------------------------------------- nothing costs money

/// No layer reaches a source that bills.
///
/// The excluded hosts are the ones somebody would actually reach for: Google's
/// photorealistic tiles because they look extraordinary, Mapbox because it is the default
/// in every tutorial, Cesium ion because it is the path of least resistance once Cesium
/// Native is already linked.
#[test]
fn no_layer_reaches_a_source_that_charges() {
    for entry in LAYERS {
        assert_eq!(
            excluded_source(entry.endpoint),
            None,
            "{:?} points at an excluded source: {}",
            entry.id,
            entry.endpoint
        );
    }
}

/// And the exclusion actually catches the thing it names.
///
/// A test that only checks the current catalogue would pass just as well against a
/// function that always returns `None`.
#[test]
fn a_billed_source_is_recognised_as_one() {
    assert!(
        excluded_source("https://tile.googleapis.com/v1/3dtiles/root.json?key=anything")
            .is_some_and(|reason| reason.contains("billed"))
    );

    assert!(excluded_source("https://api.cesium.com/v1/assets/1/endpoint").is_some());
    assert!(excluded_source("https://api.mapbox.com/styles/v1/mapbox/satellite").is_some());

    for (host, _) in EXCLUDED {
        assert!(
            excluded_source(&format!("https://{host}/anything")).is_some(),
            "{host} is listed as excluded and must be caught"
        );
    }
}

/// The check is on the host, not on the text.
///
/// A NASA URL is not billed because it contains the word "google" in a tile matrix set
/// name, and refusing it would be an unexplainable missing layer.
#[test]
fn a_vendor_name_inside_a_path_is_not_a_billed_source() {
    assert_eq!(
        excluded_source(layer(LayerId::NasaBlackMarble).endpoint),
        None,
        "GoogleMapsCompatible is a tile matrix set, not a Google service"
    );
    assert_eq!(
        excluded_source("https://example.org/proxy?upstream=api.mapbox.com"),
        None
    );
}

/// A port or credentials in the URL do not smuggle a host past the check.
#[test]
fn a_billed_host_is_caught_however_the_url_is_dressed() {
    assert!(excluded_source("https://api.mapbox.com:443/styles").is_some());
    assert!(excluded_source("https://user:pass@api.mapbox.com/styles").is_some());
    assert!(excluded_source("https://api.mapbox.com?access_token=x").is_some());
    assert!(excluded_source("https://api.mapbox.com").is_some());
    assert_eq!(excluded_source("not a url at all"), None);
}

// ----------------------------------------------------------------- keys

/// Free-with-a-key and free-without-one are different, and the difference is visible.
#[test]
fn a_layer_needing_a_key_says_so_and_stays_off_until_it_has_one() {
    let fires = layer(LayerId::NasaFirmsHotspots);
    assert!(!fires.available_without_setup());
    assert!(fires.access.needs_a_key());

    let told = fires.setup_required().expect("it explains itself");
    assert!(told.contains("ARCHIE_FIRMS_MAP_KEY"), "{told}");
    assert!(told.contains("free"), "{told}");
}

/// A keyless layer has nothing to explain and nothing to set up.
#[test]
fn a_keyless_layer_needs_no_explanation() {
    for entry in available_without_setup() {
        assert_eq!(entry.access, LayerAccess::Keyless);
        assert_eq!(entry.setup_required(), None, "{:?}", entry.id);
    }
}

/// A credential placeholder in a URL means the layer needs a credential.
///
/// The failure this catches is a layer marked keyless whose endpoint still contains
/// `{map_key}` — it would be enabled by default and every request would fail.
#[test]
fn an_endpoint_with_a_key_placeholder_declares_that_it_needs_one() {
    for entry in LAYERS {
        let looks_like_it_needs_one = entry.endpoint.contains("{map_key}")
            || entry.endpoint.contains("{api_key}")
            || entry.endpoint.contains("{token}");
        assert_eq!(
            looks_like_it_needs_one,
            entry.access.needs_a_key(),
            "{:?}: the endpoint and the access declaration disagree",
            entry.id
        );
    }
}

/// A key never sits in the catalogue.
///
/// It is read from the environment at use, so that a checkout of this repository carries
/// nobody's credential.
#[test]
fn no_credential_is_written_down_here() {
    for entry in LAYERS {
        if let LayerAccess::FreeCredential { env_var } = entry.access {
            assert!(
                env_var.starts_with("ARCHIE_"),
                "{env_var} should be namespaced to Archie"
            );
            assert!(
                !entry.endpoint.contains(env_var),
                "the variable's name is not its value, and neither belongs in the URL"
            );
        }
    }
}

// ----------------------------------------------------------------- politeness

/// Every layer respects the rate its provider asked for.
///
/// Polling a keyless service faster than it invited is how it stops being keyless: OpenSky
/// runs out of anonymous credits, and CelesTrak puts the address behind its firewall.
#[test]
fn every_layer_carries_the_refresh_floor_its_provider_asked_for() {
    for entry in LAYERS {
        assert!(
            entry.min_refresh_seconds > 0,
            "{:?} has no floor at all",
            entry.id
        );
    }

    assert!(
        layer(LayerId::CelestrakSatellites).min_refresh_seconds >= 7_200,
        "CelesTrak recomputes every two hours and firewalls addresses that ask faster"
    );
    assert!(
        layer(LayerId::OpenSkyAircraft).min_refresh_seconds >= 864,
        "400 anonymous credits a day at 4 credits per global query is one every 15 minutes"
    );
    assert_eq!(
        layer(LayerId::CelestrakSatellites).kind,
        LayerKind::Ephemeris,
        "satellite positions come from propagating elements locally, not from re-asking"
    );
}

/// Every provider here asks clients to identify themselves, and every one is told.
#[test]
fn archie_identifies_itself_to_every_provider() {
    for entry in LAYERS {
        assert!(
            entry.identifies_the_client,
            "{:?} would be an anonymous request against a service that asks who is calling",
            entry.id
        );
    }
}

// ----------------------------------------------------------------- honesty of the entries

/// A national feed is marked as one.
///
/// Drawing US weather alerts on a globe with no note implies the rest of the world has
/// nothing to warn about, and a quiet Chile is a different claim from an unwatched one.
#[test]
fn a_feed_that_only_covers_one_country_says_so() {
    assert_eq!(
        layer(LayerId::NwsAlerts).coverage,
        Coverage::Regional("the United States and its territories")
    );
    assert_eq!(layer(LayerId::GdacsAlerts).coverage, Coverage::Global);
}

/// Every entry names who to credit, because most of these licences require it.
#[test]
fn every_layer_names_its_attribution_and_authority() {
    for entry in LAYERS {
        assert!(!entry.attribution.trim().is_empty(), "{:?}", entry.id);
        assert!(!entry.authority.trim().is_empty(), "{:?}", entry.id);
        assert!(!entry.dataset.trim().is_empty(), "{:?}", entry.id);
    }
}

/// Every entry records the day somebody actually fetched it.
///
/// Endpoints move. A date is the difference between a claim that was true once and a claim
/// nobody has ever checked.
#[test]
fn every_layer_records_when_its_endpoint_was_last_fetched() {
    for entry in LAYERS {
        let date = entry.verified_on;
        assert_eq!(date.len(), 10, "{:?}: {date}", entry.id);
        assert!(
            date.chars()
                .enumerate()
                .all(|(index, character)| match index {
                    4 | 7 => character == '-',
                    _ => character.is_ascii_digit(),
                }),
            "{:?}: {date} is not an ISO date",
            entry.id
        );
    }
}

/// Transport is encrypted, always.
#[test]
fn every_endpoint_is_https() {
    for entry in LAYERS {
        assert!(
            entry.endpoint.starts_with("https://"),
            "{:?}: {}",
            entry.id,
            entry.endpoint
        );
    }
}

// ----------------------------------------------------------------- the table itself

/// Every id resolves, so the lookup cannot panic.
#[test]
fn every_layer_id_has_an_entry() {
    for id in EVERY_ID {
        assert_eq!(layer(id).id, id);
    }
    assert_eq!(
        LAYERS.len(),
        EVERY_ID.len(),
        "a layer was added to the table without being listed here, or the reverse"
    );
}

/// No id appears twice, or the lookup silently prefers one of them.
#[test]
fn no_layer_id_is_used_twice() {
    let mut seen: Vec<LayerId> = LAYERS.iter().map(|entry| entry.id).collect();
    seen.sort();
    let count = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), count);
}

/// The catalogue crosses to the renderer, which does not link this crate.
///
/// It travels one way. The entries borrow from the binary, so they serialise out and are
/// never read back — an id comes back instead, which is why `LayerId` is the type that
/// deserialises.
#[test]
fn the_catalogue_serialises_for_the_renderer_and_ids_come_back() {
    let text = serde_json::to_string(&LAYERS).expect("serialisable");
    let sent: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    let entries = sent.as_array().expect("an array of layers");
    assert_eq!(entries.len(), LAYERS.len());

    for (sent, entry) in entries.iter().zip(LAYERS) {
        for field in [
            "id",
            "authority",
            "dataset",
            "endpoint",
            "access",
            "kind",
            "coverage",
            "min_refresh_seconds",
            "attribution",
            "verified_on",
        ] {
            assert!(sent.get(field).is_some(), "{:?} lost {field}", entry.id);
        }
        let id: LayerId =
            serde_json::from_value(sent["id"].clone()).expect("the renderer can name it back");
        assert_eq!(id, entry.id);
    }

    assert_eq!(
        serde_json::to_string(&LayerId::NasaBlackMarble).unwrap(),
        "\"nasa_black_marble\"",
        "the wire name is stable; a renderer stores it"
    );
}
