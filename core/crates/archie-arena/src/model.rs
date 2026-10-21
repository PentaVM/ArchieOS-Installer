//! Asking a model through the router and the veil, with fallback: the one door the arena's deciding,
//! judging and reflecting go through.

use archie_model_runtime::routed::{Question, Routed, RoutedError};
use archie_types::{ModelRole, PrivacyGrade};
use archie_vault::Veil;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where a question is asked directly when the router is not used: public data only.
const DIRECT_ENDPOINT: &str = "https://api.tokenfactory.nebius.com/v1/chat/completions";

pub struct Nebius {
    routed: Option<Arc<Routed>>,
    veil: Arc<Veil>,
    grade: PrivacyGrade,
    http: reqwest::Client,
    key: Option<String>,
    endpoint: String,
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
            endpoint: DIRECT_ENDPOINT.into(),
        })
    }

    /// Asks each model in turn until one answers within `timeout`. `images` are data URLs,
    /// sent after the text in the order given.
    ///
    /// Anything graded above public goes through the router and the veil and nowhere else:
    /// without a registry, with pictures (pixels cannot be veiled), or when no model the
    /// router allows answers, it is refused, never sent straight to the cloud. Only public
    /// data may take the direct request, and only when the router did not withhold it.
    pub async fn ask(
        &self,
        models: &[String],
        system: &str,
        text: &str,
        images: &[String],
        max_tokens: u32,
        timeout: Duration,
    ) -> Result<Answer, String> {
        let private = self.grade > PrivacyGrade::Public;
        if private && !images.is_empty() {
            return Err(format!(
                "withheld by privacy rules: {} data with a picture is not sent, a picture cannot be veiled",
                self.grade
            ));
        }
        match (&self.routed, images.is_empty()) {
            (Some(routed), true) => {
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
                    Err(err) if private || self.key.is_none() => return Err(err.to_string()),
                    Err(_) => {}
                }
            }
            (None, _) if private => {
                return Err(format!(
                    "withheld by privacy rules: {} data needs the model registry to decide where it may go, and none is loaded",
                    self.grade
                ));
            }
            _ => {}
        }

        // Public data only from here: with pictures, or when the router found no model.
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
                .post(&self.endpoint)
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

#[cfg(test)]
mod tests {
    use super::*;
    use archie_registry::ModelRegistry;
    use archie_types::{
        Availability, DataRetention, Locality, ModelDescriptor, ModelId, ProviderKind, TokenCost,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A loopback stand-in for the cloud that counts who reaches it and answers each.
    async fn cloud() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let reached = Arc::new(AtomicUsize::new(0));
        let counter = reached.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0; 64 * 1024];
                    let _ = stream.read(&mut buf).await;
                    let body = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"completion_tokens":1}}"#;
                    let _ = stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                });
            }
        });
        (url, reached)
    }

    /// A model on this machine whose server is not there: local inference fails.
    fn dead_local_model() -> Routed {
        let mut registry = ModelRegistry::default();
        registry
            .register(ModelDescriptor {
                id: ModelId::parse("local.qwen").unwrap(),
                display_name: "Qwen (local, down)".to_string(),
                provider: ProviderKind::OpenAiCompatible,
                locality: Locality::Local,
                node: None,
                context_window: 32_768,
                max_output_tokens: 4096,
                capabilities: vec![ModelRole::ReasoningWorker.capability()],
                max_privacy_grade: PrivacyGrade::Restricted,
                retention: DataRetention::Unknown,
                availability: Availability::Ready,
                cost: TokenCost::FREE,
                latency_ema_ms: Some(900),
                quality_tier: 40,
                required_accelerator_bytes: None,
                labels: BTreeMap::from([(
                    "base-url".to_string(),
                    "http://127.0.0.1:9/v1/".to_string(),
                )]),
            })
            .unwrap();
        Routed::new(registry)
    }

    fn nebius(routed: Option<Routed>, grade: PrivacyGrade, endpoint: String) -> Nebius {
        Nebius {
            routed: routed.map(Arc::new),
            veil: Arc::new(Veil::new()),
            grade,
            http: reqwest::Client::new(),
            key: Some("a-key-for-tests".into()),
            endpoint,
        }
    }

    async fn ask(nebius: &Nebius, images: &[String]) -> Result<Answer, String> {
        nebius
            .ask(
                &["some/model".to_string()],
                "you judge drawings",
                "is the cat drawn?",
                images,
                50,
                Duration::from_secs(5),
            )
            .await
    }

    const PICTURE: &str = "data:image/png;base64,iVBORw0KGgo=";

    #[tokio::test]
    async fn private_data_never_takes_the_direct_request() {
        for grade in [
            PrivacyGrade::Internal,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
        ] {
            let (url, reached) = cloud().await;
            // With a picture, with no registry, and with the local model down.
            let cases = [
                (nebius(None, grade, url.clone()), vec![PICTURE.to_string()]),
                (nebius(None, grade, url.clone()), vec![]),
                (nebius(Some(dead_local_model()), grade, url.clone()), vec![]),
                (
                    nebius(Some(dead_local_model()), grade, url.clone()),
                    vec![PICTURE.to_string()],
                ),
            ];
            for (i, (nebius, images)) in cases.iter().enumerate() {
                let refused = ask(nebius, images).await;
                assert!(refused.is_err(), "{grade} case {i}: {refused:?}");
            }
            assert_eq!(
                reached.load(Ordering::SeqCst),
                0,
                "{grade} reached the cloud"
            );
        }
    }

    /// The same loopback cloud does count a public request, so the zeros above mean something.
    #[tokio::test]
    async fn public_data_with_a_picture_takes_the_direct_request() {
        let (url, reached) = cloud().await;
        let answer = ask(
            &nebius(None, PrivacyGrade::Public, url),
            &[PICTURE.to_string()],
        )
        .await
        .unwrap();
        assert_eq!(answer.text, "ok");
        assert_eq!(reached.load(Ordering::SeqCst), 1);
    }
}
