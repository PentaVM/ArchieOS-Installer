//! Wanda -> Cortex `wanda.decision_v1` strategic decision output contract.
//!
//! Implements the exact channel record schema and five compatibility rules from
//! `_dev/verification/rtx3070/2026-10-16-wanda-cortex-decision-design.md`.

use crate::{config::bounded_id, WandaError};
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const DECISION_SCHEMA_VERSION: u16 = 1;
pub const DECISION_KIND_V1: &str = "wanda.decision_v1";
pub const MAX_DECISION_UTF8_BYTES: usize = 16 * 1024;
pub const MAX_TASK_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserObjective {
    pub objective_ref: String,
    pub priority: u8,
    pub horizon: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeNote {
    pub description: String,
    pub horizon: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionAlternative {
    pub option_id: String,
    pub action_summary: String,
    pub supporting_refs: Vec<String>,
    pub uncertainties: Vec<String>,
    pub consequences: Vec<OutcomeNote>,
    pub relational_costs: Vec<OutcomeNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionForecast {
    pub event: String,
    pub probability_permille: u16,
    pub resolution_condition: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionUncertainty {
    pub missing_information: Vec<String>,
    pub competing_explanations: Vec<String>,
    pub forecasts: Vec<DecisionForecast>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategicAnalysis {
    pub user_objectives: Vec<UserObjective>,
    pub alternatives: Vec<DecisionAlternative>,
    pub selected_option_id: Option<String>,
    pub justification: String,
    pub uncertainty: DecisionUncertainty,
    pub revision_conditions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionEnvelope {
    pub schema_version: u16,
    pub kind: String,
    pub decision_id: String,
    pub task_id: String,
    pub domain: ContextDomain,
    pub produced_at_ms: i64,
    pub expires_at_ms: i64,
    pub source_run_ref: String,
    pub objective_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub analysis: StrategicAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCorrelation {
    pub schema_version: u16,
    pub decision_id: String,
    pub decision_ref: String,
    pub decision_sha256: String,
    pub task_id: String,
    pub domain: ContextDomain,
    pub proposal_id: Option<String>,
    pub hypothesis_id: Option<String>,
    pub recommendation_ids: Vec<String>,
    pub source_run_ref: String,
    pub produced_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CortexDocumentNode {
    pub node_id: String,
    pub kind: String,
    pub domain: ContextDomain,
    pub provenance_ref: String,
    pub standing: String,
    pub independent_evidence_vote: bool,
    pub cited_evidence_refs: Vec<String>,
    pub summary: String,
    pub produced_at_ms: i64,
    pub expires_at_ms: i64,
}

fn bounded_text(s: &str, max_chars: usize) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= max_chars
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}

fn check_refs(
    refs: &[String],
    admitted: &BTreeSet<String>,
    min_len: usize,
    max_len: usize,
) -> Result<(), WandaError> {
    if refs.len() < min_len || refs.len() > max_len {
        return Err(WandaError::InvalidAdvice);
    }
    let mut seen = BTreeSet::new();
    for r in refs {
        if !bounded_id(r) || !seen.insert(r.as_str()) || !admitted.contains(r) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    Ok(())
}

fn check_outcomes(items: &[OutcomeNote], admitted: &BTreeSet<String>) -> Result<(), WandaError> {
    if items.len() > 8 {
        return Err(WandaError::InvalidAdvice);
    }
    for item in items {
        if !bounded_text(&item.description, 500) || !bounded_text(&item.horizon, 500) {
            return Err(WandaError::InvalidAdvice);
        }
        check_refs(&item.evidence_refs, admitted, 0, 16)?;
    }
    Ok(())
}

pub fn validate_analysis(
    analysis: &StrategicAnalysis,
    admitted_refs: &BTreeSet<String>,
    admitted_objectives: Option<&BTreeSet<String>>,
) -> Result<(), WandaError> {
    if analysis.user_objectives.is_empty() || analysis.user_objectives.len() > 8 {
        return Err(WandaError::InvalidAdvice);
    }
    let mut seen_obj = BTreeSet::new();
    for obj in &analysis.user_objectives {
        if !bounded_id(&obj.objective_ref)
            || !seen_obj.insert(obj.objective_ref.as_str())
            || !(1..=8).contains(&obj.priority)
            || !bounded_text(&obj.horizon, 500)
            || !bounded_text(&obj.summary, 500)
        {
            return Err(WandaError::InvalidAdvice);
        }
        if let Some(allowed) = admitted_objectives {
            if !allowed.contains(&obj.objective_ref) {
                return Err(WandaError::InvalidAdvice);
            }
        }
    }

    if analysis.alternatives.is_empty() || analysis.alternatives.len() > 8 {
        return Err(WandaError::InvalidAdvice);
    }
    let mut option_ids = BTreeSet::new();
    for alt in &analysis.alternatives {
        if !bounded_id(&alt.option_id)
            || !option_ids.insert(alt.option_id.as_str())
            || !bounded_text(&alt.action_summary, 1200)
            || alt.uncertainties.len() > 8
            || alt.uncertainties.iter().any(|u| !bounded_text(u, 500))
        {
            return Err(WandaError::InvalidAdvice);
        }
        check_refs(&alt.supporting_refs, admitted_refs, 0, 16)?;
        check_outcomes(&alt.consequences, admitted_refs)?;
        check_outcomes(&alt.relational_costs, admitted_refs)?;
    }

    if let Some(selected) = &analysis.selected_option_id {
        if !option_ids.contains(selected.as_str()) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    if !bounded_text(&analysis.justification, 1200) {
        return Err(WandaError::InvalidAdvice);
    }

    let unc = &analysis.uncertainty;
    if unc.missing_information.len() > 8
        || unc
            .missing_information
            .iter()
            .any(|s| !bounded_text(s, 500))
        || unc.competing_explanations.len() > 8
        || unc
            .competing_explanations
            .iter()
            .any(|s| !bounded_text(s, 500))
        || unc.forecasts.len() > 8
    {
        return Err(WandaError::InvalidAdvice);
    }
    for fc in &unc.forecasts {
        if fc.probability_permille > 1000
            || !bounded_text(&fc.event, 500)
            || !bounded_text(&fc.resolution_condition, 500)
        {
            return Err(WandaError::InvalidAdvice);
        }
        check_refs(&fc.evidence_refs, admitted_refs, 0, 16)?;
    }

    if analysis.revision_conditions.is_empty()
        || analysis.revision_conditions.len() > 8
        || analysis
            .revision_conditions
            .iter()
            .any(|r| !bounded_text(r, 500))
    {
        return Err(WandaError::InvalidAdvice);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn build_envelope(
    analysis: StrategicAnalysis,
    decision_id: String,
    task_id: String,
    domain: ContextDomain,
    produced_at_ms: i64,
    expires_at_ms: i64,
    source_run_ref: String,
    objective_refs: Vec<String>,
    evidence_refs: Vec<String>,
    admitted_refs: &BTreeSet<String>,
    max_evidence_expiry_ms: Option<i64>,
) -> Result<DecisionEnvelope, WandaError> {
    let effective_expiry = max_evidence_expiry_ms
        .map(|m| expires_at_ms.min(m))
        .unwrap_or(expires_at_ms);
    let envelope = DecisionEnvelope {
        schema_version: DECISION_SCHEMA_VERSION,
        kind: DECISION_KIND_V1.to_string(),
        decision_id,
        task_id,
        domain: domain.clone(),
        produced_at_ms,
        expires_at_ms: effective_expiry,
        source_run_ref,
        objective_refs,
        evidence_refs,
        analysis,
    };
    validate_envelope(&envelope, &domain, admitted_refs, None)?;
    Ok(envelope)
}

pub fn validate_envelope(
    envelope: &DecisionEnvelope,
    expected_domain: &ContextDomain,
    admitted_refs: &BTreeSet<String>,
    now_ms: Option<i64>,
) -> Result<(), WandaError> {
    if envelope.schema_version != DECISION_SCHEMA_VERSION || envelope.kind != DECISION_KIND_V1 {
        return Err(WandaError::InvalidAdvice);
    }
    if !bounded_id(&envelope.decision_id)
        || !bounded_id(&envelope.task_id)
        || envelope.task_id.len() > MAX_TASK_ID_BYTES
        || !bounded_id(&envelope.source_run_ref)
        || &envelope.domain != expected_domain
    {
        return Err(WandaError::InvalidAdvice);
    }
    if envelope.produced_at_ms <= 0
        || envelope.expires_at_ms <= envelope.produced_at_ms
        || envelope
            .expires_at_ms
            .saturating_sub(envelope.produced_at_ms)
            > crate::ADVICE_HORIZON_MS
    {
        return Err(WandaError::InvalidAdvice);
    }
    if let Some(now) = now_ms {
        if envelope.expires_at_ms <= now {
            return Err(WandaError::InvalidAdvice);
        }
    }
    if envelope.objective_refs.len() > 8 {
        return Err(WandaError::InvalidAdvice);
    }
    let mut obj_set = BTreeSet::new();
    for r in &envelope.objective_refs {
        if !bounded_id(r) || !obj_set.insert(r.clone()) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    check_refs(&envelope.evidence_refs, admitted_refs, 1, 64)?;
    let cited_subset: BTreeSet<String> = envelope.evidence_refs.iter().cloned().collect();
    let obj_filter = if obj_set.is_empty() {
        None
    } else {
        Some(&obj_set)
    };
    validate_analysis(&envelope.analysis, &cited_subset, obj_filter)?;
    let encoded = serde_json::to_vec(envelope).map_err(|_| WandaError::InvalidAdvice)?;
    if encoded.len() > MAX_DECISION_UTF8_BYTES {
        return Err(WandaError::TooLarge);
    }
    Ok(())
}

pub fn envelope_sha256(envelope: &DecisionEnvelope) -> Result<String, WandaError> {
    let bytes = serde_json::to_vec(envelope).map_err(|_| WandaError::InvalidAdvice)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn build_correlation(
    envelope: &DecisionEnvelope,
    proposal_id: Option<String>,
    hypothesis_id: Option<String>,
    recommendation_ids: Vec<String>,
) -> Result<DecisionCorrelation, WandaError> {
    if let Some(ref pid) = proposal_id {
        if !bounded_id(pid) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    if let Some(ref hid) = hypothesis_id {
        if !bounded_id(hid) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    if recommendation_ids.len() > 12 {
        return Err(WandaError::InvalidAdvice);
    }
    let mut seen = BTreeSet::new();
    for r in &recommendation_ids {
        if !bounded_id(r) || !seen.insert(r.as_str()) {
            return Err(WandaError::InvalidAdvice);
        }
    }
    let decision_ref = envelope
        .domain
        .reference(&format!("wanda/decisions/{}", envelope.decision_id))
        .map_err(|_| WandaError::InvalidAdvice)?
        .path()
        .to_string();
    Ok(DecisionCorrelation {
        schema_version: DECISION_SCHEMA_VERSION,
        decision_id: envelope.decision_id.clone(),
        decision_ref,
        decision_sha256: envelope_sha256(envelope)?,
        task_id: envelope.task_id.clone(),
        domain: envelope.domain.clone(),
        proposal_id,
        hypothesis_id,
        recommendation_ids,
        source_run_ref: envelope.source_run_ref.clone(),
        produced_at_ms: envelope.produced_at_ms,
        expires_at_ms: envelope.expires_at_ms,
    })
}

pub fn verify_correlation(
    envelope: &DecisionEnvelope,
    correlation: &DecisionCorrelation,
) -> Result<(), WandaError> {
    let expected = build_correlation(
        envelope,
        correlation.proposal_id.clone(),
        correlation.hypothesis_id.clone(),
        correlation.recommendation_ids.clone(),
    )?;
    if correlation != &expected {
        return Err(WandaError::InvalidAdvice);
    }
    Ok(())
}

pub fn project_cortex_document_node(
    envelope: &DecisionEnvelope,
) -> Result<CortexDocumentNode, WandaError> {
    let decision_ref = envelope
        .domain
        .reference(&format!("wanda/decisions/{}", envelope.decision_id))
        .map_err(|_| WandaError::InvalidAdvice)?
        .path()
        .to_string();
    let choice = envelope
        .analysis
        .selected_option_id
        .as_deref()
        .map(|s| format!("selected={s}"))
        .unwrap_or_else(|| "selected=none".to_string());
    let summary: String = format!(
        "[inferred model analysis; {choice}] {}",
        envelope.analysis.justification
    )
    .chars()
    .take(500)
    .collect();
    Ok(CortexDocumentNode {
        node_id: format!("doc:{}", envelope.decision_id),
        kind: "document".to_string(),
        domain: envelope.domain.clone(),
        provenance_ref: decision_ref,
        standing: "inferred".to_string(),
        independent_evidence_vote: false,
        cited_evidence_refs: envelope.evidence_refs.clone(),
        summary,
        produced_at_ms: envelope.produced_at_ms,
        expires_at_ms: envelope.expires_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_analysis() -> StrategicAnalysis {
        StrategicAnalysis {
            user_objectives: vec![UserObjective {
                objective_ref: "obj:trip-coordination".into(),
                priority: 1,
                horizon: "48h".into(),
                summary: "Ensure everyone knows the trip moved to July before booking flights."
                    .into(),
            }],
            alternatives: vec![
                DecisionAlternative {
                    option_id: "opt-direct-ping".into(),
                    action_summary:
                        "Send Alice a direct update noting the trip moved from May to July.".into(),
                    supporting_refs: vec!["personal/ev-001".into(), "personal/ev-002".into()],
                    uncertainties: vec!["Whether Alice already checked May flight prices.".into()],
                    consequences: vec![OutcomeNote {
                        description: "Alice sees the July dates before purchasing tickets.".into(),
                        horizon: "immediate".into(),
                        evidence_refs: vec!["personal/ev-001".into()],
                    }],
                    relational_costs: vec![OutcomeNote {
                        description: "Minimal interruption via a brief one-line heads-up.".into(),
                        horizon: "immediate".into(),
                        evidence_refs: vec!["personal/ev-002".into()],
                    }],
                },
                DecisionAlternative {
                    option_id: "opt-wait".into(),
                    action_summary:
                        "Wait until tomorrow morning to post a reminder in the group chat.".into(),
                    supporting_refs: vec!["personal/ev-002".into()],
                    uncertainties: vec!["Alice may book May tickets tonight.".into()],
                    consequences: vec![OutcomeNote {
                        description: "Risk of booking mismatch if Alice acts tonight.".into(),
                        horizon: "24h".into(),
                        evidence_refs: vec!["personal/ev-001".into()],
                    }],
                    relational_costs: vec![],
                },
            ],
            selected_option_id: Some("opt-direct-ping".into()),
            justification: "Alice was absent when July was announced in personal/ev-002.".into(),
            uncertainty: DecisionUncertainty {
                missing_information: vec!["Whether Bruno already messaged Alice privately.".into()],
                competing_explanations: vec![
                    "Alice might have muted the group chat temporarily.".into()
                ],
                forecasts: vec![DecisionForecast {
                    event: "Alice confirms the July schedule once messaged directly.".into(),
                    probability_permille: 850,
                    resolution_condition: "Alice replies acknowledging July.".into(),
                    evidence_refs: vec!["personal/ev-001".into(), "personal/ev-002".into()],
                }],
            },
            revision_conditions: vec![
                "Alice posts in the group chat acknowledging the July update.".into(),
            ],
        }
    }

    #[test]
    fn round_trip_envelope_correlation_and_cortex_document_node() {
        let domain = ContextDomain::personal("owner").unwrap();
        let admitted: BTreeSet<String> = ["personal/ev-001", "personal/ev-002", "personal/ev-003"]
            .into_iter()
            .map(String::from)
            .collect();
        let env = build_envelope(
            sample_analysis(),
            "dec-2026-0001".into(),
            "task-042".into(),
            domain,
            1_700_000_000_000,
            1_700_086_400_000,
            "run:wanda-bonsai2-9b".into(),
            vec!["obj:trip-coordination".into()],
            vec!["personal/ev-001".into(), "personal/ev-002".into()],
            &admitted,
            Some(1_700_030_000_000),
        )
        .unwrap();
        assert_eq!(env.expires_at_ms, 1_700_030_000_000);

        let corr = build_correlation(
            &env,
            Some("prop-1".into()),
            Some("hyp-1".into()),
            vec!["rec-1".into()],
        )
        .unwrap();
        verify_correlation(&env, &corr).unwrap();

        let mut tampered = corr.clone();
        tampered.decision_sha256 = "0".repeat(64);
        assert!(verify_correlation(&env, &tampered).is_err());

        let node = project_cortex_document_node(&env).unwrap();
        assert_eq!(node.kind, "document");
        assert_eq!(node.standing, "inferred");
        assert!(!node.independent_evidence_vote);
        assert_eq!(
            node.provenance_ref,
            "personal/owner/wanda/decisions/dec-2026-0001"
        );
    }

    #[test]
    fn rejects_unadmitted_refs_domain_mismatch_and_invalid_options() {
        let personal = ContextDomain::personal("owner").unwrap();
        let work = ContextDomain::work("acme", "ops").unwrap();
        let admitted: BTreeSet<String> = ["personal/ev-001", "personal/ev-002"]
            .into_iter()
            .map(String::from)
            .collect();
        let mut bad_analysis = sample_analysis();
        bad_analysis.selected_option_id = Some("opt-missing".into());
        assert!(validate_analysis(&bad_analysis, &admitted, None).is_err());

        let env = build_envelope(
            sample_analysis(),
            "dec-2026-0002".into(),
            "task-042".into(),
            personal.clone(),
            1_700_000_000_000,
            1_700_036_000_000,
            "run:wanda-bonsai2-9b".into(),
            vec!["obj:trip-coordination".into()],
            vec!["personal/ev-001".into(), "personal/ev-002".into()],
            &admitted,
            None,
        )
        .unwrap();
        assert!(validate_envelope(&env, &work, &admitted, None).is_err());
        assert!(validate_envelope(&env, &personal, &admitted, Some(1_700_040_000_000)).is_err());
    }
}
