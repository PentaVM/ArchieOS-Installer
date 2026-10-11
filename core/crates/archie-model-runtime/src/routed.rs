//! Asking whichever model the router chooses for a job.
//!
//! The callers that need reasoning (a planner for IRIS, the voice's answer, a critic) used
//! to name a model and carry their own client for one provider. Here they name the job
//! instead: a role, the privacy grade of what they are about to send, and the question.
//! The router chooses among the node's manifests, this module calls the choice through
//! the same boundary as every other call, and when that model fails the next eligible
//! one is tried. Which model answered comes back with the answer.
//!
//! # Private questions and models the node does not run
//!
//! [`Routed::ask_private`] is for text that may hold what must not be shown to a third
//! party: a key, an address, a name. When a model on the operator's own machines may
//! answer, it is asked as is. When only a cloud model can, the text is veiled first
//! ([`archie_vault::Veil`]): credentials, addresses and named entities leave as stand-ins,
//! the table that reverses them stays in this process, and the real values are put back
//! into the answer here. The cloud model writes the file or the letter and never holds
//! what goes in it.

use crate::{
    provider_model_id, ChatMessage, ChatRequest, InferenceClient, MessageRole, ProviderEndpoint,
    RuntimeError, VisionChatRequest, VisionImage,
};
use archie_registry::{ModelRegistry, ModelRegistryError};
use archie_router::{Router, RoutingRequest, Weights};
use archie_types::{Locality, ModelId, ModelRole, PrivacyGrade};
use archie_vault::veil::Kind;
use archie_vault::{Veil, Veiled};
use std::collections::BTreeMap;
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
    /// The question was not sent, and why.
    #[error("not sent: {0}")]
    Withheld(String),
}

/// Told to a model that is given veiled text, so it carries the stand-ins through.
const STAND_INS: &str = "\n\nWords such as SECRET_1, EMAIL_1, PHONE_1, CARD_1, RUT_1, PERSON_1 or COMPANY_1 in this \
conversation are stand-ins for real values you are not shown. Use each exactly as written wherever its value \
belongs. Do not change, expand, explain or guess them.";

/// How a private question is to be asked.
#[derive(Debug)]
pub enum Disclosure {
    /// As it is, of models on the operator's own machines.
    Kept { models: Vec<ModelId> },
    /// Veiled, of models that disclose to a third party.
    Veiled { models: Vec<ModelId>, veiled: Veiled },
}

/// The answer to a private question, with what may be written down about how it was got.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateAnswer {
    pub answered: Answered,
    /// Whether the text left the operator's machines, veiled.
    pub left_the_machine: bool,
    /// How many values of each kind were covered before it left.
    pub covered: BTreeMap<String, usize>,
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
        self.ask_among(self.candidates(question)?, question).await
    }

    /// Decides how a private question may be asked, without asking it.
    ///
    /// Models on the operator's own machines come first and are given the text as it is:
    /// veiling local inference costs accuracy and protects nothing. Only when none may
    /// answer is the text veiled for a cloud model, and then only if it is graded no
    /// higher than `sensitive`, carries no picture (pixels cannot be veiled), and nothing
    /// with a sensitive shape is left in it afterwards.
    pub fn disclosure(&self, question: &Question<'_>, veil: &Veil) -> Result<Disclosure, RoutedError> {
        if question.privacy == PrivacyGrade::Secret {
            return Err(RoutedError::Withheld("secret data reaches no model".into()));
        }
        let own: Vec<ModelId> = self
            .candidates(question)
            .unwrap_or_default()
            .into_iter()
            .filter(|model| self.registry.get(model).is_some_and(|m| m.locality != Locality::Cloud))
            .collect();
        if !own.is_empty() {
            return Ok(Disclosure::Kept { models: own });
        }
        if question.privacy > PrivacyGrade::Sensitive {
            return Err(RoutedError::Withheld(
                "restricted data does not leave this machine, and no model on it can answer".into(),
            ));
        }
        if question.image.is_some() {
            return Err(RoutedError::Withheld("a picture cannot be veiled, and no model on this machine can see".into()));
        }
        let veiled = veil.cover(&[question.system, question.user]);
        let residue = veiled.residue();
        if !residue.is_empty() {
            return Err(RoutedError::Withheld(format!("the veil left {} sensitive value(s) uncovered: {}", residue.len(), kinds(&residue))));
        }
        let outgoing = Question {
            role: question.role,
            privacy: PrivacyGrade::Public,
            system: &veiled.texts[0],
            user: &veiled.texts[1],
            image: None,
            max_tokens: question.max_tokens,
            timeout: question.timeout,
        };
        let models = self.candidates(&outgoing)?;
        Ok(Disclosure::Veiled { models, veiled })
    }

    /// Asks a question that may hold what a third party must not see. See the module's
    /// documentation for what leaves and what does not.
    pub async fn ask_private(&self, question: &Question<'_>, veil: &Veil) -> Result<PrivateAnswer, RoutedError> {
        match self.disclosure(question, veil)? {
            Disclosure::Kept { models } => Ok(PrivateAnswer {
                answered: self.ask_among(models, question).await?,
                left_the_machine: false,
                covered: BTreeMap::new(),
            }),
            Disclosure::Veiled { models, veiled } => {
                let system = format!("{}{STAND_INS}", veiled.texts[0]);
                let outgoing = Question {
                    role: question.role,
                    privacy: PrivacyGrade::Public,
                    system: &system,
                    user: &veiled.texts[1],
                    image: None,
                    max_tokens: question.max_tokens,
                    timeout: question.timeout,
                };
                let mut answered = self.ask_among(models, &outgoing).await?;
                answered.text = veiled.unveil.restore(&answered.text);
                Ok(PrivateAnswer { answered, left_the_machine: true, covered: veiled.unveil.covered() })
            }
        }
    }

    async fn ask_among(&self, models: Vec<ModelId>, question: &Question<'_>) -> Result<Answered, RoutedError> {
        let started = Instant::now();
        let mut failures = Vec::new();
        for model in models {
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

fn kinds(found: &[Kind]) -> String {
    let mut names: Vec<&str> = found.iter().map(|kind| kind.prefix()).collect();
    names.sort_unstable();
    names.dedup();
    names.join(", ").to_lowercase()
}
