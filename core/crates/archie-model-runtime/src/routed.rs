//! Asking whichever model the router chooses for a job.
//!
//! The callers that need reasoning (a planner for IRIS, the voice's answer, a critic) used
//! to name a model and carry their own client for one provider. Here they name the job
//! instead: a role, the privacy grade of what they are about to send, and the question.
//! The router chooses among the node's manifests, this module calls the choice through
//! the same boundary as every other call, and when that model fails the next eligible
//! one is tried. Which model answered comes back with the answer.

use crate::{
    provider_model_id, ChatMessage, ChatRequest, InferenceClient, MessageRole, ProviderEndpoint,
    RuntimeError, VisionChatRequest, VisionImage,
};
use archie_registry::{ModelRegistry, ModelRegistryError};
use archie_router::{Router, RoutingRequest, Weights};
use archie_types::{ModelId, ModelRole, PrivacyGrade};
use std::path::Path;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoutedError {
    #[error(transparent)]
    Registry(#[from] ModelRegistryError),
    /// The router's own words for why nothing may serve the request.
    #[error("no model may answer this: {0}")]
    NoModel(String),
    /// Every eligible model was tried; each one's failure, in the order tried.
    #[error("no model answered: {}", .0.iter().map(|(model, why)| format!("{model}: {why}")).collect::<Vec<_>>().join("; "))]
    AllFailed(Vec<(ModelId, String)>),
}

/// What is asked.
pub struct Question<'a> {
    pub role: ModelRole,
    /// The grade of everything in `system`, `user` and `image`.
    pub privacy: PrivacyGrade,
    pub system: &'a str,
    pub user: &'a str,
    /// A picture to look at. A model that cannot see is not chosen when one is given.
    pub image: Option<VisionImage<'a>>,
    pub max_tokens: u32,
    /// For every model tried together, not each.
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    pub model: ModelId,
    pub text: String,
    pub seconds: f32,
    pub completion_tokens: u64,
}

/// The node's models and the router over them.
pub struct Routed {
    registry: ModelRegistry,
    router: Router,
}

impl Routed {
    pub fn new(registry: ModelRegistry) -> Self {
        Self { registry, router: Router::new(Weights::default()) }
    }

    /// From the node's model manifests, e.g. `/etc/archie/models`.
    pub fn from_dir(models: impl AsRef<Path>) -> Result<Self, RoutedError> {
        Ok(Self::new(ModelRegistry::load_from_dir(models)?))
    }

    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// The models that may answer, best first. A cloud model whose key is not in the
    /// environment is left out: it is registered and cannot be called.
    pub fn candidates(&self, question: &Question<'_>) -> Result<Vec<ModelId>, RoutedError> {
        let mut request = RoutingRequest::new(
            question.privacy,
            ((question.system.len() + question.user.len()) / 3) as u32,
            question.max_tokens,
        )
        .requiring(question.role.capability());
        if question.image.is_some() {
            request = request.requiring(ModelRole::VisionModel.capability());
        }
        let outcome = self.router.route(&self.registry, &request);
        let mut eligible: Vec<_> = outcome.considered().iter().filter(|c| c.is_eligible()).collect();
        eligible.sort_by(|a, b| b.score.total_cmp(&a.score));
        let callable: Vec<ModelId> = eligible
            .into_iter()
            .filter(|candidate| {
                self.registry
                    .get(&candidate.model)
                    .and_then(|descriptor| ProviderEndpoint::for_model(descriptor).ok())
                    .is_some_and(|endpoint| endpoint.is_configured())
            })
            .map(|candidate| candidate.model.clone())
            .collect();
        if callable.is_empty() {
            return Err(RoutedError::NoModel(outcome.explain()));
        }
        Ok(callable)
    }

    pub async fn ask(&self, question: &Question<'_>) -> Result<Answered, RoutedError> {
        let started = Instant::now();
        let mut failures = Vec::new();
        for model in self.candidates(question)? {
            let left = question.timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                failures.push((model, "no time left".to_string()));
                continue;
            }
            match self.ask_one(&model, question, left).await {
                Ok(answered) => return Ok(answered),
                Err(error) => failures.push((model, error.to_string())),
            }
        }
        Err(RoutedError::AllFailed(failures))
    }

    async fn ask_one(&self, model: &ModelId, question: &Question<'_>, timeout: Duration) -> Result<Answered, RuntimeError> {
        let descriptor = self.registry.get(model).ok_or(RuntimeError::ProviderMismatch)?;
        let endpoint = ProviderEndpoint::for_model(descriptor)?.with_limits(timeout, 8 * 1024 * 1024)?;
        let client = InferenceClient::new(endpoint)?;
        let asked = Instant::now();
        let response = match &question.image {
            Some(image) => {
                let request = VisionChatRequest {
                    instruction: question.system,
                    context: question.user,
                    // The same borrowed picture: a keyframe is deliberately not cloneable.
                    image: VisionImage { mime: image.mime, bytes: image.bytes, width: image.width, height: image.height },
                    max_tokens: question.max_tokens,
                };
                client.vision_chat(descriptor, question.privacy, &request).await?
            }
            None => {
                let request = ChatRequest {
                    model: provider_model_id(descriptor)?.to_string(),
                    messages: vec![
                        ChatMessage { role: MessageRole::System, content: question.system.to_string() },
                        ChatMessage { role: MessageRole::User, content: question.user.to_string() },
                    ],
                    max_tokens: Some(question.max_tokens),
                    temperature: Some(0.2),
                    stream: false,
                };
                client.chat(descriptor, question.privacy, &request).await?
            }
        };
        let text = response.choices.into_iter().next().map(|choice| choice.message.content).unwrap_or_default();
        if text.trim().is_empty() {
            return Err(RuntimeError::EmptyCompletion);
        }
        Ok(Answered {
            model: model.clone(),
            text,
            seconds: asked.elapsed().as_secs_f32(),
            completion_tokens: response.usage.map_or(0, |usage| usage.completion_tokens),
        })
    }
}
