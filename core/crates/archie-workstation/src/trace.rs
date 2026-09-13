use archie_types::PrivacyGrade;
use serde::{Deserialize, Serialize};

/// Content-free workstation event. Prompt, transcript, audio, documents and secrets have no field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkstationTrace {
    pub task_id: String,
    pub decision_id: String,
    pub selected_provider: Option<String>,
    pub selected_harness: Option<String>,
    pub context_ref_count: u32,
    pub context_token_count: u32,
    pub privacy_class: PrivacyGrade,
    pub clarification_reason: Option<String>,
    pub capability: Option<String>,
    pub tool: Option<String>,
    pub duration_ms: u64,
    pub result: String,
    pub error_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_no_content_fields() {
        let names = serde_json::to_value(WorkstationTrace {
            task_id: "t".into(),
            decision_id: "d".into(),
            selected_provider: None,
            selected_harness: None,
            context_ref_count: 0,
            context_token_count: 0,
            privacy_class: PrivacyGrade::Restricted,
            clarification_reason: None,
            capability: None,
            tool: None,
            duration_ms: 1,
            result: "ok".into(),
            error_code: None,
        })
        .unwrap();
        let rendered = names.to_string();
        for forbidden in ["prompt", "transcript", "audio", "document", "secret"] {
            assert!(!rendered.contains(forbidden));
        }
    }
}
