use archie_vault::{ContextDomain, VaultRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    ConfirmedOwner,
    PresumedOwner,
    Unknown,
    ConfirmedOther,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    Owner,
    Counterparty,
    System,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    AndroidNotification,
    WhatsappVisibleDom,
    IrisSemantic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    NotificationPreview,
    VisibleChat,
    SemanticObservation,
}

/// The content channel is independent from Sentry's metadata capture event.
/// A producer never supplies the storage domain or a filesystem path.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WandaEvent {
    pub schema_version: u16,
    pub event_id: String,
    pub device_id: String,
    pub actor: Actor,
    pub author: Author,
    pub source: EventSource,
    pub chat_id: String,
    pub sender: String,
    pub source_message_id: Option<String>,
    pub sent_at_ms: Option<i64>,
    pub observed_at_ms: i64,
    pub text: String,
    pub media_omitted: bool,
    pub coverage: Coverage,
    #[serde(default)]
    pub monitor_id: Option<String>,
    #[serde(default)]
    pub window_id: Option<String>,
}
impl std::fmt::Debug for WandaEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WandaEvent(redacted)")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    pub domain: ContextDomain,
    pub evidence_ref: VaultRef,
    pub event: WandaEvent,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IngestReport {
    pub accepted: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraitView {
    pub key: String,
    pub value: Option<String>,
    pub basis: String,
    pub sensitivity: String,
    pub source_note: String,
    pub redacted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonView {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub degree: Option<usize>,
    pub path_to_owner: Option<Vec<String>>,
    pub tags: Vec<TraitView>,
    pub traits: Vec<TraitView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationView {
    pub from_id: String,
    pub to_id: String,
    pub kind: String,
    pub basis: String,
    pub declarer_id: String,
    pub source_note: String,
    pub standing: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonProjection {
    pub schema_version: u16,
    pub owner_id: String,
    pub nodes: Vec<PersonView>,
    pub edges: Vec<RelationView>,
}

/// An interpretation is advisory and never establishes a relationship or consent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recommendation {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub explanation: String,
    pub suggestion: String,
    pub subject_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub standing: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
    pub expires_at_ms: i64,
    #[serde(default)]
    pub review_condition: String,
}

/// State 2 of the local expert: the long answer, refined from the draft and the social state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refined {
    pub plan: Option<String>,
    pub answer: String,
    /// Capitalised words the expert wrote that are no one known and appear nowhere in the
    /// context. When any is present the answer is not spoken.
    pub unverified_names: Vec<String>,
    pub seconds: f32,
    /// The answer passed the checks against the state and the n-gram index. When it did not,
    /// it is not spoken.
    #[serde(default)]
    pub verified: bool,
    /// Values the checks replaced: (what the expert wrote, what the state says).
    #[serde(default)]
    pub corrected_values: Vec<(String, String)>,
    /// Why it is not spoken, when it is not.
    #[serde(default)]
    pub problems: Vec<String>,
}

/// A refined answer this close to state 1's (character trigram F1) says nothing new.
pub const REPEATS_QUICK: f32 = 0.5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub recommendations: Vec<Recommendation>,
    pub model: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    /// State 1 of the local expert: an immediate yes/no (or "cannot know") with its confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<archie_wanda_lncm::QuickAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refined: Option<Refined>,
}
/// Below this, state 1 is not said aloud: an unsure "yes" is worse than none.
pub const QUICK_CONFIDENCE: f32 = 0.6;
impl QueryResponse {
    pub fn speech_text(&self) -> String {
        let mut spoken = Vec::new();
        let quick = self
            .quick
            .as_ref()
            .filter(|q| q.confidence >= QUICK_CONFIDENCE);
        if let Some(quick) = quick {
            spoken.push(quick.text.clone());
        }
        // What the state answers by itself is the whole answer: a long answer could only repeat
        // it or contradict it.
        let from_state = quick.is_some_and(|q| q.source == archie_wanda_lncm::AnswerSource::State);
        if let Some(refined) = self.refined.as_ref().filter(|r| {
            !from_state
                && r.verified
                && r.unverified_names.is_empty()
                && !r.answer.trim().is_empty()
                && quick.is_none_or(|q| {
                    archie_wanda_lncm::model::char_ngram_f1(&q.text, &r.answer) < REPEATS_QUICK
                })
        }) {
            spoken.push(refined.answer.clone());
        }
        if !spoken.is_empty() {
            return spoken.join(" ").chars().take(1200).collect();
        }
        let text = self
            .recommendations
            .iter()
            .take(3)
            .map(|r| format!("{} {}", r.explanation, r.suggestion))
            .collect::<Vec<_>>()
            .join(" ");
        let answer = if text.is_empty() {
            self.reason.clone().unwrap_or_else(|| {
                "Todavía no tengo contexto suficiente para recomendarte algo.".into()
            })
        } else {
            text
        };
        answer.chars().take(1200).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceView {
    pub device_id: String,
    pub source: EventSource,
    pub coverage: Option<Coverage>,
    pub last_observed_at_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WandaView {
    pub schema_version: u16,
    pub domain: ContextDomain,
    pub generated_at_ms: i64,
    pub availability: String,
    pub graph: Option<PersonProjection>,
    pub recommendations: Vec<Recommendation>,
    pub sources: Vec<SourceView>,
    pub pending_events: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_wanda_lncm::{AnswerSource, QuickAnswer};

    fn response(source: AnswerSource, confidence: f32, verified: bool) -> QueryResponse {
        QueryResponse {
            recommendations: Vec::new(),
            model: None,
            state: "answered".into(),
            reason: None,
            quick: Some(QuickAnswer {
                text: "No hace falta: Héctor ya sabe que la cena es martes.".into(),
                answer: Some(false),
                confidence,
                source,
            }),
            refined: Some(Refined {
                plan: None,
                answer: "Pregúntale si le viene bien el horario.".into(),
                unverified_names: Vec::new(),
                seconds: 0.1,
                verified,
                corrected_values: Vec::new(),
                problems: Vec::new(),
            }),
        }
    }

    #[test]
    fn the_state_speaks_alone_and_only_a_checked_long_answer_is_spoken() {
        let state = response(AnswerSource::State, 0.95, true).speech_text();
        assert_eq!(
            state,
            "No hace falta: Héctor ya sabe que la cena es martes."
        );
        let model = response(AnswerSource::Model, 0.9, true).speech_text();
        assert!(
            model.ends_with("Pregúntale si le viene bien el horario."),
            "{model}"
        );
        let unchecked = response(AnswerSource::Model, 0.9, false).speech_text();
        assert!(!unchecked.contains("Pregúntale"), "{unchecked}");
    }
}
