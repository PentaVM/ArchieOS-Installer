//! Asking a model through the router and the veil, with fallback: the one door the arena's deciding,
//! judging and reflecting go through.

use archie_model_runtime::routed::{Question, Routed, RoutedError};
use archie_types::{ModelRole, PrivacyGrade};
use archie_vault::Veil;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Nebius {
    routed: Option<Arc<Routed>>,
    veil: Arc<Veil>,
    grade: PrivacyGrade,
    http: reqwest::Client,
    key: Option<String>,
}

/// What a model answered, and how long it took.
#[derive(Debug, Clone)]
pub struct Answer {
    pub model: String,
    pub text: String,
    pub seconds: f32,
    pub completion_tokens: u64,
}

impl Nebius {
    /// On `NEBIUS_TOKEN_FACTORY_KEY`, the conversation's own key.
    pub fn from_environment() -> Result<Self, String> {
        Self::from_environment_with_routing(None, PrivacyGrade::Public)
    }

    /// Loads models from `models_dir` (or `ARCHIE_MODELS_DIR`, or `/etc/archie/models`),
    /// routing questions and veiling private text.
    pub fn from_environment_with_routing(
        models_dir: Option<&str>,
        grade: PrivacyGrade,
    ) -> Result<Self, String> {
        let key = std::env::var("NEBIUS_TOKEN_FACTORY_KEY").ok();
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;

        let dir = models_dir
            .map(str::to_string)
            .or_else(|| std::env::var("ARCHIE_MODELS_DIR").ok())
            .unwrap_or_else(|| "/etc/archie/models".into());

        let routed = if std::path::Path::new(&dir).is_dir() {
            Routed::from_dir(&dir).ok().map(Arc::new)
        } else {
            None
        };

        if routed.is_none() && key.is_none() {
            return Err(
                "neither models directory nor NEBIUS_TOKEN_FACTORY_KEY is available".into(),
            );
        }

        Ok(Self {
            routed,
            veil: Arc::new(Veil::new()),
            grade,
            http,
            key,
        })
    }

    /// Asks each model in turn until one answers within `timeout`. `images` are data URLs,
    /// sent after the text in the order given.
    pub async fn ask(
        &self,
        models: &[String],
        system: &str,
        text: &str,
        images: &[String],
        max_tokens: u32,
        timeout: Duration,
    ) -> Result<Answer, String> {
        if images.is_empty() {
            if let Some(routed) = &self.routed {
                let question = Question {
                    role: ModelRole::ReasoningWorker,
                    privacy: self.grade,
                    system,
                    user: text,
                    image: None,
                    max_tokens,
                    timeout,
                };
                match routed.ask_private(&question, &self.veil).await {
                    Ok(ans) => {
                        return Ok(Answer {
                            model: ans.answered.model.to_string(),
                            text: ans.answered.text,
                            seconds: ans.answered.seconds,
                            completion_tokens: ans.answered.completion_tokens,
                        });
                    }
                    Err(RoutedError::Withheld(why)) => {
                        return Err(format!("withheld by privacy rules: {why}"));
                    }
                    Err(RoutedError::NoModel(why)) => {
                        if self.key.is_none() {
                            return Err(format!("no model may answer: {why}"));
                        }
                    }
                    Err(err) => {
                        if self.key.is_none() {
                            return Err(err.to_string());
                        }
                    }
                }
            }
        }

        // When images are present or when direct provider endpoint is used with key:
        let key = match &self.key {
            Some(k) => k,
            None => {
                return Err(
                    "NEBIUS_TOKEN_FACTORY_KEY is not set and no model on this machine answered"
                        .into(),
                );
            }
        };

        let mut content = vec![json!({"type": "text", "text": text})];
        content.extend(
            images
                .iter()
                .map(|url| json!({"type": "image_url", "image_url": {"url": url}})),
        );
        let started = Instant::now();
        let mut last = String::from("no model to ask");
        for model in models {
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            let body = json!({
                "model": model, "temperature": 0.2, "max_tokens": max_tokens,
                "messages": [{"role": "system", "content": system}, {"role": "user", "content": content}],
            });
            let asked = Instant::now();
            let response = self
                .http
                .post("https://api.tokenfactory.nebius.com/v1/chat/completions")
                .bearer_auth(key)
                .timeout(left)
                .json(&body)
                .send()
                .await;
            let value: serde_json::Value = match response {
                Ok(response) => match response.error_for_status() {
                    Ok(response) => match response.json().await {
                        Ok(value) => value,
                        Err(e) => {
                            last = format!("{model}: {e}");
                            continue;
                        }
                    },
                    Err(e) => {
                        last = format!(
                            "{model}: {}",
                            e.status().map_or("failed".into(), |s| s.to_string())
                        );
                        continue;
                    }
                },
                Err(e) => {
                    last = format!(
                        "{model}: {}",
                        if e.is_timeout() {
                            "no answer in time".into()
                        } else {
                            e.to_string()
                        }
                    );
                    continue;
                }
            };
            let text = value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if text.trim().is_empty() {
                last = format!("{model}: an empty answer");
                continue;
            }
            return Ok(Answer {
                model: model.clone(),
                text,
                seconds: asked.elapsed().as_secs_f32(),
                completion_tokens: value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            });
        }
        Err(last)
    }
}
