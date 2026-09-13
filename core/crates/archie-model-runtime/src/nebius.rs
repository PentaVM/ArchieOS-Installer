use archie_types::{PrivacyGrade, RuntimeError, Locality};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

pub struct NebiusClient {
    client: Client,
    base_url: String,
    api_key: String,
}

#[derive(Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
pub struct Choice {
    pub message: ChatMessage,
}

impl NebiusClient {
    pub fn new() -> Result<Self, RuntimeError> {
        let api_key = env::var("NEBIUS_TOKEN_FACTORY_KEY")
            .or_else(|_| env::var("NEBIUS_API_KEY"))
            .map_err(|_| RuntimeError::CloudCredentialRequired)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|_| RuntimeError::InvalidEndpoint)?;

        Ok(Self {
            client,
            base_url: "https://api.tokenfactory.nebius.com/v1".to_string(),
            api_key,
        })
    }

    pub async fn chat(&self, req: ChatCompletionRequest, grade: PrivacyGrade) -> Result<ChatCompletionResponse, RuntimeError> {
        if grade == PrivacyGrade::Secret {
            return Err(RuntimeError::PrivacyViolation);
        }

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.client.post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .map_err(|_| RuntimeError::Network)?;

        match resp.status() {
            StatusCode::OK => {
                let json = resp.json().await.map_err(|_| RuntimeError::InvalidEndpoint)?;
                Ok(json)
            }
            StatusCode::TOO_MANY_REQUESTS => Err(RuntimeError::RateLimit),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(RuntimeError::CloudCredentialRequired),
            _ => Err(RuntimeError::Network),
        }
    }
}
