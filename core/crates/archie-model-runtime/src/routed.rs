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
    Veiled {
        models: Vec<ModelId>,
        veiled: Veiled,
    },
}

/// How a chat request (potentially streaming with tools) is to be asked.
#[derive(Debug)]
pub enum ChatDisclosure {
    /// As it is, of models on the operator's own machines.
    Kept {
        models: Vec<ModelId>,
        request: crate::ToolChatRequest,
    },
    /// Veiled, of models that disclose to a third party.
    Veiled {
        models: Vec<ModelId>,
        request: crate::ToolChatRequest,
        unveil: archie_vault::veil::Unveil,
    },
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
    min_quality_tier: u8,
}

impl Routed {
    pub fn new(registry: ModelRegistry) -> Self {
        Self {
            registry,
            router: Router::new(Weights::default()),
            min_quality_tier: 0,
        }
    }

    /// Operator-configured quality floor, using the existing router's admission.
    pub fn requiring_quality(mut self, tier: u8) -> Self {
        self.min_quality_tier = tier.min(100);
        self
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
        .requiring(question.role.capability())
        .min_quality(self.min_quality_tier);
        if question.image.is_some() {
            request = request.requiring(ModelRole::VisionModel.capability());
        }
        self.callable(&request)
    }

    /// The eligible models for `request` that can be called, best first.
    fn callable(&self, request: &RoutingRequest) -> Result<Vec<ModelId>, RoutedError> {
        let outcome = self.router.route(&self.registry, request);
        let mut eligible: Vec<_> = outcome
            .considered()
            .iter()
            .filter(|c| c.is_eligible())
            .collect();
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
    pub fn disclosure(
        &self,
        question: &Question<'_>,
        veil: &Veil,
    ) -> Result<Disclosure, RoutedError> {
        if question.privacy == PrivacyGrade::Secret {
            return Err(RoutedError::Withheld("secret data reaches no model".into()));
        }
        let own: Vec<ModelId> = self
            .candidates(question)
            .unwrap_or_default()
            .into_iter()
            .filter(|model| {
                self.registry
                    .get(model)
                    .is_some_and(|m| m.locality != Locality::Cloud)
            })
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
            return Err(RoutedError::Withheld(
                "a picture cannot be veiled, and no model on this machine can see".into(),
            ));
        }
        let veiled = veil.cover(&[question.system, question.user]);
        let residue = veiled.residue();
        if !residue.is_empty() {
            return Err(RoutedError::Withheld(format!(
                "the veil left {} sensitive value(s) uncovered: {}",
                residue.len(),
                kinds(&residue)
            )));
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

    /// Decides how a tool chat request may be asked, as [`Routed::disclosure`] does for a
    /// question: as it is to a model on the operator's own machines, else veiled for a cloud
    /// model when graded no higher than `sensitive`.
    ///
    /// Everything that leaves is covered: message text, and every string inside the
    /// arguments of the tools the assistant called. Ids and tool names are protocol and
    /// stay as they are. The tool definitions are the program's own words; one that shows
    /// a sensitive shape is not rewritten but withheld.
    pub fn disclose_chat(
        &self,
        request: &crate::ToolChatRequest,
        privacy: PrivacyGrade,
        role: ModelRole,
        veil: &Veil,
    ) -> Result<ChatDisclosure, RoutedError> {
        use crate::ToolChatMessage as Message;

        if privacy == PrivacyGrade::Secret {
            return Err(RoutedError::Withheld("secret data reaches no model".into()));
        }

        let total_chars: usize = request
            .messages
            .iter()
            .map(|message| match message {
                Message::System(t) | Message::User(t) => t.len(),
                Message::Assistant {
                    content,
                    tool_calls,
                } => {
                    content.as_ref().map_or(0, String::len)
                        + tool_calls.iter().map(|c| c.arguments.len()).sum::<usize>()
                }
                Message::Tool { content, .. } => content.len(),
            })
            .sum();
        let input_tokens = (total_chars / 3).max(1) as u32;
        let routing = |grade| {
            RoutingRequest::new(grade, input_tokens, request.max_tokens)
                .requiring(role.capability())
        };

        // No model at this grade is no reason to refuse: the veiled request below is
        // public, and a cloud model may take it.
        let own: Vec<ModelId> = self
            .callable(&routing(privacy))
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                self.registry
                    .get(m)
                    .is_some_and(|d| d.locality != Locality::Cloud)
            })
            .collect();
        if !own.is_empty() {
            return Ok(ChatDisclosure::Kept {
                models: own,
                request: request.clone(),
            });
        }

        if privacy > PrivacyGrade::Sensitive {
            return Err(RoutedError::Withheld(
                "restricted data does not leave this machine, and no model on it can answer".into(),
            ));
        }

        for tool in &request.tools {
            let parameters = tool.parameters.to_string();
            if shows_sensitive(veil, &[&tool.name, &tool.description, &parameters]) {
                return Err(RoutedError::Withheld(format!(
                    "the definition of tool {} holds a sensitive value",
                    tool.name
                )));
            }
        }

        // Every text that leaves, in the order it is put back below.
        let mut arguments: Vec<Option<serde_json::Value>> = Vec::new();
        let mut leaving: Vec<String> = Vec::new();
        for message in &request.messages {
            match message {
                Message::System(t) | Message::User(t) => leaving.push(t.clone()),
                Message::Assistant {
                    content,
                    tool_calls,
                } => {
                    leaving.extend(content.iter().cloned());
                    for call in tool_calls {
                        match serde_json::from_str::<serde_json::Value>(&call.arguments) {
                            Ok(value) => {
                                let mut keys = Vec::new();
                                json_keys(&value, &mut keys);
                                let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                                if shows_sensitive(veil, &keys) {
                                    return Err(RoutedError::Withheld(format!(
                                        "the arguments of a call to {} hold a sensitive value \
                                         where only a name may stand",
                                        call.name
                                    )));
                                }
                                json_strings(&value, &mut leaving);
                                arguments.push(Some(value));
                            }
                            // Not JSON: covered as the text it is.
                            Err(_) => {
                                leaving.push(call.arguments.clone());
                                arguments.push(None);
                            }
                        }
                    }
                }
                Message::Tool { content, .. } => leaving.push(content.clone()),
            }
        }

        let texts: Vec<&str> = leaving.iter().map(String::as_str).collect();
        let veiled = veil.cover(&texts);
        let residue = veiled.residue();
        if !residue.is_empty() {
            return Err(RoutedError::Withheld(format!(
                "the veil left {} sensitive value(s) uncovered: {}",
                residue.len(),
                kinds(&residue)
            )));
        }

        let Veiled { texts, unveil } = veiled;
        let mut covered = texts.into_iter();
        let mut arguments = arguments.into_iter();
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        let mut system_found = false;
        for message in &request.messages {
            messages.push(match message {
                Message::System(_) => {
                    let mut s = covered.next().unwrap_or_default();
                    if !system_found {
                        s.push_str(STAND_INS);
                        system_found = true;
                    }
                    Message::System(s)
                }
                Message::User(_) => Message::User(covered.next().unwrap_or_default()),
                Message::Assistant {
                    content,
                    tool_calls,
                } => {
                    let content = content.as_ref().map(|_| covered.next().unwrap_or_default());
                    let tool_calls = tool_calls
                        .iter()
                        .map(|call| {
                            let arguments = match arguments.next().flatten() {
                                Some(mut value) => {
                                    put_json_strings(&mut value, &mut covered);
                                    value.to_string()
                                }
                                None => covered.next().unwrap_or_default(),
                            };
                            crate::ToolCall {
                                id: call.id.clone(),
                                name: call.name.clone(),
                                arguments,
                            }
                        })
                        .collect();
                    Message::Assistant {
                        content,
                        tool_calls,
                    }
                }
                Message::Tool { call_id, .. } => Message::Tool {
                    call_id: call_id.clone(),
                    content: covered.next().unwrap_or_default(),
                },
            });
        }
        if !system_found {
            messages.insert(0, Message::System(STAND_INS.trim_start().to_string()));
        }

        let models = self.callable(&routing(PrivacyGrade::Public))?;
        let mut veiled_request = request.clone();
        veiled_request.messages = messages;
        Ok(ChatDisclosure::Veiled {
            models,
            request: veiled_request,
            unveil,
        })
    }

    /// Asks a question that may hold what a third party must not see. See the module's
    /// documentation for what leaves and what does not.
    pub async fn ask_private(
        &self,
        question: &Question<'_>,
        veil: &Veil,
    ) -> Result<PrivateAnswer, RoutedError> {
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
                Ok(PrivateAnswer {
                    answered,
                    left_the_machine: true,
                    covered: veiled.unveil.covered(),
                })
            }
        }
    }

    async fn ask_among(
        &self,
        models: Vec<ModelId>,
        question: &Question<'_>,
    ) -> Result<Answered, RoutedError> {
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

    async fn ask_one(
        &self,
        model: &ModelId,
        question: &Question<'_>,
        timeout: Duration,
    ) -> Result<Answered, RuntimeError> {
        let descriptor = self
            .registry
            .get(model)
            .ok_or(RuntimeError::ProviderMismatch)?;
        let endpoint =
            ProviderEndpoint::for_model(descriptor)?.with_limits(timeout, 8 * 1024 * 1024)?;
        let client = InferenceClient::new(endpoint)?;
        let asked = Instant::now();
        let response = match &question.image {
            Some(image) => {
                let request = VisionChatRequest {
                    instruction: question.system,
                    context: question.user,
                    // The same borrowed picture: a keyframe is deliberately not cloneable.
                    image: VisionImage {
                        mime: image.mime,
                        bytes: image.bytes,
                        width: image.width,
                        height: image.height,
                    },
                    max_tokens: question.max_tokens,
                };
                client
                    .vision_chat(descriptor, question.privacy, &request)
                    .await?
            }
            None => {
                let request = ChatRequest {
                    model: provider_model_id(descriptor)?.to_string(),
                    messages: vec![
                        ChatMessage {
                            role: MessageRole::System,
                            content: question.system.to_string(),
                        },
                        ChatMessage {
                            role: MessageRole::User,
                            content: question.user.to_string(),
                        },
                    ],
                    max_tokens: Some(question.max_tokens),
                    temperature: Some(0.2),
                    stream: false,
                };
                client.chat(descriptor, question.privacy, &request).await?
            }
        };
        let text = response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .unwrap_or_default();
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

/// Whether the veil would change any of `texts`, or still finds something in them.
fn shows_sensitive(veil: &Veil, texts: &[&str]) -> bool {
    let veiled = veil.cover(texts);
    veiled
        .texts
        .iter()
        .zip(texts)
        .any(|(after, before)| after != before)
        || !veiled.residue().is_empty()
}

/// The object keys anywhere in `value`.
fn json_keys(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                out.push(key.clone());
                json_keys(inner, out);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| json_keys(v, out)),
        _ => {}
    }
}

/// The string values anywhere in `value`, in the order [`put_json_strings`] puts them back.
fn json_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Object(map) => map.values().for_each(|v| json_strings(v, out)),
        serde_json::Value::Array(items) => items.iter().for_each(|v| json_strings(v, out)),
        _ => {}
    }
}

fn put_json_strings(value: &mut serde_json::Value, texts: &mut impl Iterator<Item = String>) {
    match value {
        serde_json::Value::String(text) => *text = texts.next().unwrap_or_default(),
        serde_json::Value::Object(map) => map.values_mut().for_each(|v| put_json_strings(v, texts)),
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|v| put_json_strings(v, texts))
        }
        _ => {}
    }
}

fn kinds(found: &[Kind]) -> String {
    let mut names: Vec<&str> = found.iter().map(|kind| kind.prefix()).collect();
    names.sort_unstable();
    names.dedup();
    names.join(", ").to_lowercase()
}
