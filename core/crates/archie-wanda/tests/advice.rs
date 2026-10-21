use archie_wanda::advice::validate;
use std::collections::BTreeSet;
fn proposal() -> serde_json::Value {
    serde_json::json!({"id":"gift-1","kind":"gift","title":"Flores para revisar",
      "explanation":"El contexto declara una preferencia por tulipanes.","suggestion":"Considera tulipanes; confirma disponibilidad y precio.",
      "subject_ids":["partner"],"evidence_refs":["vault:personal/owner/wanda/events/one"],"standing":"inferred",
      "alternatives":["Preguntar qué le gustaría"],"expires_at_ms":2000,"review_condition":"Confirma la preferencia antes de comprar."})
}
fn check(value: serde_json::Value) -> bool {
    validate(
        &serde_json::json!({"recommendations":[value]}).to_string(),
        &BTreeSet::from(["vault:personal/owner/wanda/events/one".into()]),
        &BTreeSet::from(["partner".into()]),
        1000,
    )
    .is_ok()
}
#[test]
fn admitted_hypothesis_is_usable_but_fabricated_or_expired_evidence_is_not() {
    assert!(check(proposal()));
    let mut value = proposal();
    value["evidence_refs"] = serde_json::json!(["vault:work/firm/project/secret"]);
    assert!(!check(value));
    let mut value = proposal();
    value["evidence_refs"] = serde_json::json!([]);
    assert!(!check(value));
    let mut value = proposal();
    value["subject_ids"] = serde_json::json!(["unknown"]);
    assert!(!check(value));
    let mut value = proposal();
    value["standing"] = "asserted".into();
    assert!(!check(value));
    let mut value = proposal();
    value["expires_at_ms"] = 1000.into();
    assert!(!check(value));
    let mut value = proposal();
    value["expires_at_ms"] = 100_000_000.into();
    assert!(!check(value));
    let mut value = proposal();
    value["execute"] = "buy".into();
    assert!(!check(value));
}
#[test]
fn relationship_interpretations_require_alternatives_and_review_conditions() {
    let mut value = proposal();
    value["kind"] = "relationship".into();
    assert!(check(value.clone()));
    value["alternatives"] = serde_json::json!([]);
    assert!(!check(value));
    let mut value = proposal();
    value["review_condition"] = "".into();
    assert!(!check(value));
}
