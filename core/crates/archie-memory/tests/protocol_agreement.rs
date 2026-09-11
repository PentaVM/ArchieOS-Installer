//! The Rust types and the JSON schemas must describe the same system.
//!
//! `claim.schema.json` is the language-independent contract that Python workers and
//! external tooling read; `archie_memory::claim` is what the control plane enforces.
//! They are written in different files, in different languages, and nothing but a test
//! keeps them agreeing.
//!
//! Drift here is quiet and expensive: a stage added to the schema but not the code is a
//! stage nothing enforces, and a flag added to the code but not the schema is a
//! qualification that vanishes the moment a record crosses a process boundary.

use archie_memory::claim::{ClaimFlag, VerificationStage};
use archie_memory::CorrectionKind;
use archie_memory::{ContentClass, EvidenceStance, SensorSource};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn protocols_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("archie-memory sits three levels below the repository root")
        .join("protocols")
}

fn schema(name: &str) -> serde_json::Value {
    let path = protocols_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not valid JSON: {e}"))
}

fn string_enum(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("expected an enum array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_verification_stages_match_exactly() {
    let in_schema = string_enum(&schema("claim.schema.json")["properties"]["stage"]["enum"]);
    let in_code: BTreeSet<String> = VerificationStage::all()
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();

    let missing_from_code: Vec<&String> = in_schema.difference(&in_code).collect();
    let missing_from_schema: Vec<&String> = in_code.difference(&in_schema).collect();

    assert!(
        missing_from_code.is_empty(),
        "the schema defines stages the code does not enforce: {missing_from_code:?}"
    );
    assert!(
        missing_from_schema.is_empty(),
        "the code has stages the schema does not describe: {missing_from_schema:?}"
    );
}

#[test]
fn the_stage_order_in_the_schema_matches_the_code() {
    // A set comparison would pass even if the schema listed the stages in an order that
    // contradicted the code's progression, and the order IS the meaning here.
    let schema_order: Vec<String> = schema("claim.schema.json")["properties"]["stage"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let code_order: Vec<String> = VerificationStage::all()
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();
    assert_eq!(
        schema_order, code_order,
        "the schema lists the verification path in a different order from the code"
    );
}

#[test]
fn the_claim_flags_match_exactly() {
    let in_schema =
        string_enum(&schema("claim.schema.json")["properties"]["flags"]["items"]["enum"]);
    let in_code: BTreeSet<String> = [
        ClaimFlag::Disputed,
        ClaimFlag::Superseded,
        ClaimFlag::Retracted,
        ClaimFlag::Conjectural,
        ClaimFlag::Approximation,
        ClaimFlag::DomainLimited,
        ClaimFlag::UserOverride,
        ClaimFlag::ArchieHypothesis,
    ]
    .iter()
    .map(|f| f.as_str().to_string())
    .collect();

    assert_eq!(
        in_schema, in_code,
        "a qualification that exists on only one side vanishes when a record crosses a \
         process boundary"
    );
}

#[test]
fn the_schema_enforces_the_same_cross_source_rule_as_the_code() {
    // The code refuses to advance to cross_source_supported below two independent
    // sources. The schema must refuse to accept such a record, or a Python worker could
    // write one the control plane would never have produced.
    let claim = schema("claim.schema.json");
    let rules = claim["allOf"].as_array().expect("claim needs allOf rules");

    let enforced = rules.iter().any(|rule| {
        let stages = rule["if"]["properties"]["stage"]["enum"].as_array();
        let minimum = rule["then"]["properties"]["independent_sources"]["minimum"].as_u64();
        stages.is_some_and(|s| s.iter().any(|v| v == "cross_source_supported"))
            && minimum == Some(2)
    });
    assert!(
        enforced,
        "claim.schema.json must require at least two independent sources for the stages \
         that claim cross-source support"
    );
}

#[test]
fn the_override_overlay_is_described_as_an_overlay() {
    // The property the whole design rests on: an override never edits the claim. The
    // schema expresses that by defining the override as a separate object that names a
    // claim_id, rather than as a mutable field on the claim itself.
    let claim = schema("claim.schema.json");

    let properties = claim["properties"].as_object().unwrap();
    for editable in ["effective_statement", "overridden_statement", "override"] {
        assert!(
            !properties.contains_key(editable),
            "claim.schema.json defines {editable:?}; an override must be a separate \
             overlay, not a field that replaces the record"
        );
    }

    let overlay = &claim["$defs"]["userOverride"];
    assert!(
        overlay.is_object(),
        "the override overlay must be defined alongside the claim"
    );
    let required: Vec<&str> = overlay["required"]
        .as_array()
        .expect("the overlay must declare required fields")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for field in ["claim_id", "scope", "asserts", "authority"] {
        assert!(
            required.contains(&field),
            "an override must carry {field}: one with no author or scope cannot be reviewed"
        );
    }
}

#[test]
fn a_global_override_must_state_a_reason_in_both_the_schema_and_the_code() {
    use archie_memory::claim::{OverrideScope, UserOverride};

    // Code side.
    let mut global = UserOverride {
        claim_id: "c".into(),
        scope: OverrideScope::Global,
        asserts: "assume otherwise".into(),
        authority: "operator".into(),
        rationale: String::new(),
        created_at_ms: 0,
    };
    assert!(
        global.validate().is_err(),
        "the code must refuse a global override with no rationale"
    );
    global.rationale = "because the supplier data disagrees".into();
    assert!(global.validate().is_ok());

    // Schema side.
    let overlay = &schema("claim.schema.json")["$defs"]["userOverride"];
    let rules = overlay["allOf"]
        .as_array()
        .expect("the overlay needs allOf rules");
    let enforced = rules.iter().any(|rule| {
        rule["if"]["properties"]["scope"]["properties"]["kind"]["const"] == "global"
            && rule["then"]["required"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| v == "rationale"))
    });
    assert!(
        enforced,
        "claim.schema.json must require a rationale for a global override"
    );
}

#[test]
fn a_claim_carries_provenance_like_every_other_record() {
    let required: Vec<String> = schema("claim.schema.json")["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        required.contains(&"provenance".to_string()),
        "a claim without provenance cannot be audited or corrected"
    );
}

#[test]
fn the_claim_schema_has_no_confidence_percentage() {
    // Uncertainty is expressed as a stated error bound, which is checkable. A single
    // percentage invites arithmetic the evidence does not support.
    let properties = schema("claim.schema.json")["properties"]
        .as_object()
        .unwrap()
        .clone();
    for numeric in [
        "confidence",
        "confidence_score",
        "probability",
        "score",
        "certainty",
    ] {
        assert!(
            !properties.contains_key(numeric),
            "claim.schema.json defines {numeric:?}; the model uses stated bounds and \
             named bands, never a bare confidence number"
        );
    }
    assert!(
        properties.contains_key("uncertainty"),
        "a claim should be able to state its error bound"
    );
}

#[test]
fn correction_trajectory_schema_matches_the_typed_correction_vocabulary() {
    let trajectory = schema("cortex-correction-trajectory.schema.json");
    let in_schema = string_enum(&trajectory["$defs"]["correctionKind"]["enum"]);
    let in_code: BTreeSet<String> = [
        CorrectionKind::EntityCorrection,
        CorrectionKind::RelationCorrection,
        CorrectionKind::CoreferenceCorrection,
        CorrectionKind::RetrievalPositive,
        CorrectionKind::RetrievalNegative,
        CorrectionKind::MissingContextLabel,
        CorrectionKind::StaleKnowledgeCorrection,
        CorrectionKind::ContextPreference,
        CorrectionKind::AnswerFailure,
    ]
    .into_iter()
    .map(|kind| kind.as_str().to_string())
    .collect();
    assert_eq!(
        in_schema, in_code,
        "schema and Rust correction kinds diverged"
    );

    let properties = trajectory["properties"].as_object().unwrap();
    for raw_field in ["response", "voice", "message", "biometric", "rationale"] {
        assert!(
            !properties.contains_key(raw_field),
            "a Cortex trajectory must contain a reference/digest, not raw {raw_field}"
        );
    }
    let required: Vec<&str> = trajectory["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(required.contains(&"graph_update"));
    assert!(required.contains(&"response_digest"));
}

#[test]
fn learning_candidate_wire_vocabulary_matches_the_admission_firewall() {
    let candidate = schema("learning-candidate.schema.json");
    let sources = string_enum(&candidate["properties"]["source"]["enum"]);
    let code_sources: BTreeSet<String> = [
        SensorSource::Sentry,
        SensorSource::Wanda,
        SensorSource::OperatorCorrection,
        SensorSource::TaskRuntime,
        SensorSource::DerivedKnowledge,
    ]
    .into_iter()
    .map(|value| {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    assert_eq!(sources, code_sources);

    let classes = string_enum(&candidate["properties"]["content_class"]["enum"]);
    let code_classes: BTreeSet<String> = [
        ContentClass::RawAudio,
        ContentClass::RawVideo,
        ContentClass::RawImage,
        ContentClass::Transcript,
        ContentClass::StructuredObservation,
        ContentClass::SanitizedAggregate,
        ContentClass::SocialHypothesis,
        ContentClass::CorrectionTrajectory,
        ContentClass::TaskOutcome,
    ]
    .into_iter()
    .map(|value| {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    assert_eq!(classes, code_classes);
}

#[test]
fn analytical_signal_stances_match_the_synthesis_kernel() {
    let signal = schema("analytical-signal.schema.json");
    let stances = string_enum(&signal["properties"]["stance"]["enum"]);
    let code_stances: BTreeSet<String> = [
        EvidenceStance::Supports,
        EvidenceStance::Contradicts,
        EvidenceStance::Unknown,
    ]
    .into_iter()
    .map(|value| {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    assert_eq!(stances, code_stances);
}
