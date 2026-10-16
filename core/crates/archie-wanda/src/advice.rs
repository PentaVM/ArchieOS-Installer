use crate::{types::Recommendation, WandaError};
use std::collections::BTreeSet;

pub fn validate(
    text: &str,
    references: &BTreeSet<String>,
    people: &BTreeSet<String>,
    now_ms: i64,
) -> Result<Vec<Recommendation>, WandaError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Output {
        recommendations: Vec<Recommendation>,
    }
    if text.len() > 32 * 1024 {
        return Err(WandaError::TooLarge);
    }
    let output: Output = serde_json::from_str(text).map_err(|_| WandaError::InvalidAdvice)?;
    if output.recommendations.len() > 12 {
        return Err(WandaError::TooLarge);
    }
    let mut ids = BTreeSet::new();
    for item in &output.recommendations {
        let bounded = |s: &str, max: usize| {
            !s.trim().is_empty()
                && s.chars().count() <= max
                && !s
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        };
        if !crate::config::bounded_id(&item.id)
            || !ids.insert(&item.id)
            || !matches!(
                item.kind.as_str(),
                "gift" | "scheduling" | "reply" | "relationship" | "reflection"
            )
            || !bounded(&item.title, 160)
            || !bounded(&item.explanation, 1200)
            || !bounded(&item.suggestion, 1200)
            || !bounded(&item.review_condition, 500)
            || item.standing != "inferred"
            || item.subject_ids.len() > 16
            || item.subject_ids.iter().any(|id| !people.contains(id))
            || item.evidence_refs.is_empty()
            || item.evidence_refs.len() > 64
            || item.evidence_refs.iter().any(|r| !references.contains(r))
            || item.expires_at_ms <= now_ms
            || item.expires_at_ms > now_ms.saturating_add(crate::ADVICE_HORIZON_MS)
            || item.alternatives.len() > 8
            || item.alternatives.iter().any(|s| !bounded(s, 500))
            || (matches!(item.kind.as_str(), "relationship" | "reflection")
                && item.alternatives.is_empty())
        {
            return Err(WandaError::InvalidAdvice);
        }
    }
    Ok(output.recommendations)
}
