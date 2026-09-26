//! Asking a model on Nebius Token Factory, with pictures: the one door the arena's deciding,
//! judging and reflecting go through.

use serde_json::json;
use std::time::{Duration, Instant};

pub struct Nebius {
    http: reqwest::Client,
    key: String,
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
        let key = std::env::var("NEBIUS_TOKEN_FACTORY_KEY").map_err(|_| "NEBIUS_TOKEN_FACTORY_KEY is not set")?;
        Ok(Self {
            http: reqwest::Client::builder().build().map_err(|e| e.to_string())?,
            key: key.trim().to_string(),
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
        let mut content = vec![json!({"type": "text", "text": text})];
        content.extend(images.iter().map(|url| json!({"type": "image_url", "image_url": {"url": url}})));
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
                .bearer_auth(&self.key)
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
                        last = format!("{model}: {}", e.status().map_or("failed".into(), |s| s.to_string()));
                        continue;
                    }
                },
                Err(e) => {
                    last = format!("{model}: {}", if e.is_timeout() { "no answer in time".into() } else { e.to_string() });
                    continue;
                }
            };
            let text = value["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
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
