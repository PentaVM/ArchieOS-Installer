//! Wanda's reasoning through Core's single inference path. The question is one
//! `AiInfer` on Core's operator socket, pinned to the model Cerebra admitted: Core's
//! profile admits it, Policy leases it, the router checks grade and locality again,
//! and the answer is sealed in Core's Vault with a receipt before Wanda sees it.
use crate::config::CoreBinding;
use archie_core::{
    control::{request_over, ControlRequest, ControlResponse},
    endpoint,
};
use archie_memory::inference::InferenceStatus;
use archie_state::StatePaths;
use archie_types::{ModelId, PrivacyGrade};
use archie_vault::ContextDomain;
use archie_workstation::ai::{InferenceBudget, InferenceRequest, RunnerChoice};
use std::time::Duration;

/// What Core answered, and the receipt that records it.
pub(crate) struct CoreAnswer {
    pub text: String,
    pub model: String,
    pub receipt_id: String,
}

/// Why no answer came back, as a short code for Wanda's own `reason`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CoreRefusal {
    /// Core is not running, or its socket refused this process.
    Unreachable,
    /// Core answered with an error: out of scope, paused, invalid input.
    Refused(String),
    /// The model ran but did not complete; the receipt's code says why.
    Failed(String),
    /// Nothing came back within the binding's deadline. The request is not replayed.
    Deadline,
}

/// The request Core receives. Wanda's context travels as the prompt, at Restricted;
/// Core's own recalled memory is left out, because Wanda already chose what is relevant.
pub(crate) fn request(
    binding: &CoreBinding,
    domain: &ContextDomain,
    model: &ModelId,
    prompt: String,
    max_output_tokens: u32,
) -> InferenceRequest {
    InferenceRequest {
        domain: domain.clone(),
        focus: binding.focus.clone(),
        privacy: PrivacyGrade::Restricted,
        prompt,
        evidence: vec![],
        runner: RunnerChoice::ApiPinned {
            model: model.to_string(),
        },
        max_output_tokens,
        budget: InferenceBudget::StrictTokens,
        recall_memory: false,
        reply_schema: None,
    }
}

/// Asks Core once. A connection that fails after the request was sent is not retried:
/// Core may have run it, and its receipt says so.
pub(crate) async fn ask(
    binding: &CoreBinding,
    request: InferenceRequest,
) -> Result<CoreAnswer, CoreRefusal> {
    let paths = binding
        .root
        .as_ref()
        .map(StatePaths::rooted)
        .unwrap_or_else(StatePaths::system);
    let mut stream = tokio::time::timeout(Duration::from_secs(2), endpoint::connect(&paths))
        .await
        .map_err(|_| CoreRefusal::Unreachable)?
        .map_err(|_| CoreRefusal::Unreachable)?;
    let pinned = match &request.runner {
        RunnerChoice::ApiPinned { model } => model.clone(),
        _ => return Err(CoreRefusal::Refused("unpinned Wanda request".into())),
    };
    let response = tokio::time::timeout(
        Duration::from_secs(u64::from(binding.timeout_seconds)),
        request_over(&mut stream, &ControlRequest::AiInfer { request }),
    )
    .await
    .map_err(|_| CoreRefusal::Deadline)?
    .map_err(|_| CoreRefusal::Unreachable)?;
    answer(response, &pinned)
}

/// Reads Core's reply. Only a completed receipt for the model Wanda pinned, with text,
/// is an answer.
pub(crate) fn answer(response: ControlResponse, pinned: &str) -> Result<CoreAnswer, CoreRefusal> {
    let outcome = match response {
        ControlResponse::AiResult { outcome } => outcome,
        ControlResponse::Error { message } => return Err(CoreRefusal::Refused(message)),
        _ => return Err(CoreRefusal::Refused("unexpected Core response".into())),
    };
    let receipt = &outcome.receipt;
    if receipt.status != InferenceStatus::Completed {
        return Err(CoreRefusal::Failed(
            receipt
                .error_code
                .clone()
                .unwrap_or_else(|| "not_completed".into()),
        ));
    }
    if receipt.model_id != pinned {
        return Err(CoreRefusal::Failed("model_differs_from_admission".into()));
    }
    let text = outcome
        .text
        .clone()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| CoreRefusal::Failed("empty_response".into()))?;
    Ok(CoreAnswer {
        text,
        model: receipt.model_id.clone(),
        receipt_id: receipt.receipt_id.clone(),
    })
}
