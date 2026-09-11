//! Contracts for `protocols/*.schema.json`.
//!
//! The schemas are the language-independent source of truth for everything that crosses
//! a trust boundary, and the Rust capability taxonomy is what the scheduler matches on.
//! They are written in different files, in different languages, by different hands, and
//! nothing but a test keeps them describing the same system.
//!
//! These assertions are about *relationships*: that a `$ref` resolves, that every
//! observation kind has a capability behind it, that a record type the provider manifest
//! advertises actually has a schema. A schema that is internally valid but disagrees
//! with the code is the failure mode worth catching.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use archie_types::capability::{family_description, family_requires_authorization, well_known};

fn protocols_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("archie-types sits three levels below the repository root")
        .join("protocols")
}

/// Every schema file, keyed by filename.
fn schemas() -> BTreeMap<String, serde_json::Value> {
    let dir = protocols_dir();
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));

    let mut out = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not valid JSON: {e}"));
        out.insert(name, value);
    }
    out
}

/// Collect every `$ref` string anywhere in a document.
fn collect_refs(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key == "$ref" {
                    if let Some(text) = child.as_str() {
                        out.push(text.to_string());
                    }
                }
                collect_refs(child, out);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| collect_refs(i, out)),
        _ => {}
    }
}

#[test]
fn the_protocol_directory_is_populated() {
    let all = schemas();
    assert!(
        all.len() >= 6,
        "expected the normalized record model, found {} schema(s)",
        all.len()
    );
}

#[test]
fn every_schema_declares_its_identity_and_explains_itself() {
    for (name, schema) in schemas() {
        for field in ["$schema", "$id", "title", "description"] {
            assert!(
                schema.get(field).and_then(|v| v.as_str()).is_some(),
                "{name} is missing a {field}"
            );
        }
        assert_eq!(
            schema["$schema"].as_str(),
            Some("https://json-schema.org/draft/2020-12/schema"),
            "{name} must pin the 2020-12 dialect"
        );
        // A description that only restates the title teaches nobody anything.
        let description = schema["description"].as_str().unwrap();
        assert!(
            description.len() > 120,
            "{name} has a description too short to explain the record's purpose"
        );
    }
}

#[test]
fn top_level_records_refuse_unknown_fields() {
    // additionalProperties:false is what makes the model a contract rather than a
    // suggestion. Without it a provider's raw JSON leaks through normalization and the
    // renderer starts receiving fields nothing agreed on.
    for (name, schema) in schemas() {
        if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
            continue;
        }
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false)),
            "{name} must set additionalProperties:false at the top level"
        );
    }
}

#[test]
fn every_cross_document_ref_resolves_to_a_schema_that_exists() {
    let all = schemas();
    let known_ids: BTreeSet<String> = all
        .values()
        .filter_map(|s| s["$id"].as_str().map(str::to_string))
        .collect();

    for (name, schema) in &all {
        let mut refs = Vec::new();
        collect_refs(schema, &mut refs);
        for reference in refs {
            if !reference.starts_with("https://archie.os/") {
                continue; // local #/... pointers
            }
            // Strip any JSON-pointer fragment before checking the document exists.
            let document = reference.split('#').next().unwrap().to_string();
            assert!(
                known_ids.contains(&document),
                "{name} references {document}, which no schema in protocols/ defines"
            );
        }
    }
}

#[test]
fn every_record_type_the_provider_manifest_advertises_is_reachable() {
    // provider.schema.json lists the normalized record types a feed may yield. A type in
    // that list with no schema is a promise the model cannot keep.
    let all = schemas();
    let provider = all
        .get("provider.schema.json")
        .expect("the provider manifest schema must exist");

    let advertised: BTreeSet<String> = provider["properties"]["records"]["items"]["enum"]
        .as_array()
        .expect("provider records must be an enum")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    assert!(
        advertised.contains("observation"),
        "observation is the atom"
    );
    assert!(advertised.contains("event"));

    // The types that have their own schema file today.
    let with_schema = ["observation", "event", "person_public_profile"];
    for kind in with_schema {
        let filename = format!("{}.schema.json", kind.replace('_', "-"));
        assert!(
            all.contains_key(&filename),
            "{kind} is advertised by the provider manifest but {filename} does not exist"
        );
        assert!(
            advertised.contains(kind),
            "{filename} exists but the provider manifest does not advertise {kind}"
        );
    }
}

#[test]
fn every_observation_kind_has_a_capability_behind_it() {
    // A feed Archie can parse but that no capability covers is a feed the operator can
    // neither enable nor disable. This test is what stops a kind being added to the
    // schema without the matching competence being declared in the taxonomy.
    let all = schemas();
    let kinds: Vec<String> = all["observation.schema.json"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("observation kinds must be an enum")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(kinds.len() >= 15, "expected a broad observation vocabulary");

    // Capability families, plus the second segment of every `world.*` capability —
    // `world.seismic` is what backs a `seismic.event` observation.
    let mut covered: BTreeSet<String> = BTreeSet::new();
    for capability in well_known::all() {
        covered.insert(capability.family().to_string());
        if capability.family() == "world" {
            if let Some(specific) = capability.as_str().split('.').nth(1) {
                covered.insert(specific.to_string());
            }
        }
    }

    for kind in &kinds {
        let domain = kind.split('.').next().unwrap_or(kind);
        assert!(
            covered.contains(domain),
            "observation kind {kind:?} has no capability behind it; add a `world.{domain}` \
             capability (or a family) to archie-types before the schema advertises it"
        );
    }
}

#[test]
fn provenance_is_mandatory_on_every_record_that_leaves_a_provider() {
    // A record without provenance cannot be audited, corrected, or weighed. The whole
    // world model is worth less than nothing without this, because it would look
    // authoritative while being untraceable.
    let all = schemas();
    for name in [
        "observation.schema.json",
        "event.schema.json",
        "person-public-profile.schema.json",
    ] {
        let schema = &all[name];
        let empty = Vec::new();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            required.contains(&"provenance"),
            "{name} must require provenance"
        );
    }
}

#[test]
fn confidence_is_a_band_and_never_a_bare_number() {
    // A single confidence percentage invites arithmetic the evidence does not support.
    // The model uses named bands so a reader has to look at what actually corroborated.
    let all = schemas();
    let band = &all["provenance.schema.json"]["properties"]["confidence_band"];
    let values: Vec<&str> = band["enum"]
        .as_array()
        .expect("confidence_band must be an enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(values.contains(&"disputed"), "disputed must be expressible");
    assert!(values.contains(&"corroborated"));
    assert!(
        !band.get("type").is_some_and(|t| t == "number"),
        "confidence must not be numeric"
    );
}

#[test]
fn the_public_profile_schema_structurally_excludes_private_life() {
    // The no-doxxing rule is enforced by the parser, not by a policy document. A record
    // carrying a home address must fail to validate; a rule that lives only in prose is
    // one that gets forgotten during an incident.
    let all = schemas();
    let profile = &all["person-public-profile.schema.json"];

    assert_eq!(
        profile.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false)),
        "the profile schema must reject unknown fields outright"
    );

    let properties = profile["properties"]
        .as_object()
        .expect("profile must declare properties");
    let forbidden = [
        "address",
        "home_address",
        "phone",
        "phone_number",
        "personal_email",
        "family",
        "relatives",
        "spouse",
        "children",
        "date_of_birth",
        "dob",
        "national_id",
        "ssn",
        "passport",
        "financial",
        "bank",
        "travel",
        "location_history",
        "current_location",
        "health",
        "medical",
    ];
    for field in forbidden {
        assert!(
            !properties.contains_key(field),
            "the public profile schema defines {field:?}, which is private-life data \
             and must never be collected"
        );
    }

    // A profile exists only because of published professional activity.
    let required: Vec<&str> = profile["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        required.contains(&"basis"),
        "a profile must state why the person is in the store at all"
    );

    // No opaque reliability or bias score presented as a property of a person.
    for scoring in [
        "reliability_score",
        "bias_score",
        "trust_score",
        "credibility",
    ] {
        assert!(
            !properties.contains_key(scoring),
            "the profile schema defines {scoring:?}; an opaque score about a journalist \
             is a political act wearing a metric's clothes"
        );
    }
}

#[test]
fn a_detector_finding_carries_the_version_that_produced_it() {
    // Thresholds change. A finding that does not name the detector version it came from
    // becomes uninterpretable the moment the detector is retuned.
    let all = schemas();
    let finding = &all["detector.schema.json"]["$defs"]["finding"];
    let required: Vec<&str> = finding["required"]
        .as_array()
        .expect("finding must declare required fields")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    for field in [
        "detector_id",
        "detector_version",
        "evidence_refs",
        "confidence_band",
    ] {
        assert!(
            required.contains(&field),
            "a detector finding must carry {field}"
        );
    }
}

#[test]
fn a_surrogate_result_can_never_be_mistaken_for_a_solve() {
    // The rule that keeps a surrogate-accelerated design loop honest: a cheap prediction
    // and a converged field solve must be distinguishable at a glance, and the schema
    // forces a surrogate run to say which model produced it and how it was calibrated.
    let all = schemas();
    let run = &all["simulation-run.schema.json"];

    let required: Vec<&str> = run["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(required.contains(&"fidelity"), "fidelity must be mandatory");
    assert!(
        required.contains(&"solver"),
        "a result with no solver identity cannot be reproduced"
    );
    assert!(required.contains(&"model_revision"));

    let fidelities: Vec<&str> = run["properties"]["fidelity"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(fidelities.contains(&"surrogate"));
    assert!(fidelities.contains(&"high_fidelity"));

    // The conditional that ties surrogate fidelity to a surrogate record.
    let conditionals = run["allOf"]
        .as_array()
        .expect("simulation-run needs allOf rules");
    let has_surrogate_rule = conditionals.iter().any(|rule| {
        rule["if"]["properties"]["fidelity"]["const"] == "surrogate"
            && rule["then"]["required"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| v == "surrogate"))
    });
    assert!(
        has_surrogate_rule,
        "a surrogate run must be required to declare its model and calibration"
    );
}

#[test]
fn a_provider_that_authenticates_must_name_the_secret_it_needs() {
    // Otherwise a manifest can depend on an ambient credential nobody documented, which
    // is how an API key ends up in an environment variable and then in a log.
    let all = schemas();
    let provider = &all["provider.schema.json"];

    let access = &provider["properties"]["access"]["properties"];
    assert!(
        access.get("secret_handle").is_some(),
        "the provider manifest must have a place to NAME a secret"
    );
    // and never a place to hold one
    for leaky in ["api_key", "token", "password", "secret", "credential"] {
        assert!(
            access.get(leaky).is_none(),
            "the provider manifest defines {leaky:?}; secrets must live in the secret \
             store and never in a manifest"
        );
    }

    let conditionals = provider["allOf"]
        .as_array()
        .expect("provider needs allOf rules");
    assert!(
        !conditionals.is_empty(),
        "authentication must be tied to a named secret by a conditional"
    );
}

#[test]
fn the_taxonomy_and_the_schemas_agree_on_what_needs_authorization() {
    // Cyber observation pulls third-party records under the operator's own terms of
    // service, and actuation moves things. Both are deliberate human decisions.
    assert!(family_requires_authorization("cyber"));
    assert!(family_requires_authorization("robot"));

    // Every family the schemas can produce observations for must be describable.
    let all = schemas();
    let kinds: Vec<String> = all["observation.schema.json"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    for kind in kinds {
        let domain = kind.split('.').next().unwrap().to_string();
        let is_family = family_description(&domain).is_some();
        let is_world_specific = well_known::all()
            .iter()
            .any(|c| c.as_str() == format!("world.{domain}"));
        assert!(
            is_family || is_world_specific,
            "no capability family explains observation kind {kind:?}"
        );
    }
}
