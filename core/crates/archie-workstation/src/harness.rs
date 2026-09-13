use archie_types::PrivacyGrade;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use thiserror::Error;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKind {
    Codex,
    Claude,
    Hermes,
    Uhp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSpec {
    pub id: String,
    pub kind: HarnessKind,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub endpoint: Option<String>,
}

impl HarnessSpec {
    pub fn official(kind: HarnessKind) -> Self {
        let (id, command) = match kind {
            HarnessKind::Codex => ("codex", Some("codex")),
            HarnessKind::Claude => ("claude", Some("claude")),
            HarnessKind::Hermes => ("hermes", Some("hermes")),
            HarnessKind::Uhp => ("uhp", None),
        };
        Self {
            id: id.into(),
            kind,
            command: command.map(str::to_string),
            args: vec![],
            endpoint: None,
        }
    }

    pub fn probe(&self) -> Result<String, HarnessError> {
        let command = self
            .command
            .as_deref()
            .ok_or(HarnessError::MissingCommand)?;
        let output = Command::new(command)
            .arg("--version")
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(HarnessError::ProcessFailure);
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .chars()
            .take(256)
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UhpRequest {
    pub model: String,
    pub input: String,
    pub metadata: UhpMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UhpMetadata {
    pub harness_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UhpResponse {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("harness command is not configured")]
    MissingCommand,
    #[error("harness process failed")]
    ProcessFailure,
    #[error("harness I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("UHP endpoint must be loopback HTTP")]
    InvalidEndpoint,
    #[error("private context cannot use a non-local harness")]
    PrivacyDenied,
    #[error("harness response exceeded its byte budget")]
    ResponseTooLarge,
    #[error("harness returned HTTP {0}")]
    Status(reqwest::StatusCode),
    #[error("harness transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("harness returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("harness configuration is invalid: {0}")]
    Config(#[from] toml::de::Error),
    #[error("harness configuration could not be encoded: {0}")]
    Encode(#[from] toml::ser::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HarnessFile {
    #[serde(default)]
    harness: Vec<HarnessSpec>,
}

pub fn load_harnesses(path: &Path) -> Result<Vec<HarnessSpec>, HarnessError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(toml::from_str::<HarnessFile>(&std::fs::read_to_string(path)?)?.harness)
}

pub fn save_harness(path: &Path, harness: HarnessSpec) -> Result<(), HarnessError> {
    let mut harnesses = load_harnesses(path)?;
    harnesses.retain(|item| item.id != harness.id);
    harnesses.push(harness);
    harnesses.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        toml::to_string_pretty(&HarnessFile { harness: harnesses })?,
    )?;
    Ok(())
}

#[derive(Clone)]
pub struct UhpClient {
    endpoint: Url,
    client: reqwest::Client,
}

impl UhpClient {
    pub fn loopback(endpoint: &str, timeout: Duration) -> Result<Self, HarnessError> {
        let mut endpoint = Url::parse(endpoint).map_err(|_| HarnessError::InvalidEndpoint)?;
        let loopback = endpoint
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if endpoint.scheme() != "http" || !loopback || timeout.is_zero() {
            return Err(HarnessError::InvalidEndpoint);
        }
        if !endpoint.path().ends_with('/') {
            endpoint.set_path(&format!("{}/", endpoint.path()));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { endpoint, client })
    }

    pub async fn invoke(
        &self,
        privacy: PrivacyGrade,
        request: &UhpRequest,
    ) -> Result<UhpResponse, HarnessError> {
        if !privacy.permits(archie_types::Locality::Local) {
            return Err(HarnessError::PrivacyDenied);
        }
        let response = self
            .client
            .post(
                self.endpoint
                    .join("responses")
                    .map_err(|_| HarnessError::InvalidEndpoint)?,
            )
            .json(request)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(HarnessError::Status(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(HarnessError::ResponseTooLarge);
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(HarnessError::ResponseTooLarge);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn cancel(&self, response_id: &str) -> Result<(), HarnessError> {
        if response_id.is_empty()
            || !response_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(HarnessError::InvalidEndpoint);
        }
        let url = self
            .endpoint
            .join(&format!("responses/{response_id}/cancel"))
            .map_err(|_| HarnessError::InvalidEndpoint)?;
        let response = self.client.post(url).send().await?;
        if !response.status().is_success() {
            return Err(HarnessError::Status(response.status()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let _ = socket.read(&mut bytes).await.unwrap();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/api/harness/v1/")
    }

    #[tokio::test]
    async fn uhp_invokes_a_real_loopback_server() {
        let client = UhpClient::loopback(
            &server(r#"{"id":"r1","status":"completed"}"#).await,
            Duration::from_secs(2),
        )
        .unwrap();
        let response = client
            .invoke(
                PrivacyGrade::Restricted,
                &UhpRequest {
                    model: "worker".into(),
                    input: "bounded capsule".into(),
                    metadata: UhpMetadata {
                        harness_id: "codex".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(response.status, "completed");
    }

    #[tokio::test]
    async fn privacy_secret_fails_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = UhpClient::loopback(
            &format!("http://{}/", listener.local_addr().unwrap()),
            Duration::from_millis(100),
        )
        .unwrap();
        let result = client
            .invoke(
                PrivacyGrade::Secret,
                &UhpRequest {
                    model: "worker".into(),
                    input: "never sent".into(),
                    metadata: UhpMetadata {
                        harness_id: "codex".into(),
                    },
                },
            )
            .await;
        assert!(matches!(result, Err(HarnessError::PrivacyDenied)));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn timeout_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _connection = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let client =
            UhpClient::loopback(&format!("http://{addr}/"), Duration::from_millis(20)).unwrap();
        let result = client
            .invoke(
                PrivacyGrade::Public,
                &UhpRequest {
                    model: "worker".into(),
                    input: "x".into(),
                    metadata: UhpMetadata {
                        harness_id: "codex".into(),
                    },
                },
            )
            .await;
        assert!(matches!(result, Err(HarnessError::Transport(_))));
    }

    #[test]
    fn harness_registration_persists_only_process_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("harnesses.toml");
        save_harness(&path, HarnessSpec::official(HarnessKind::Codex)).unwrap();
        let loaded = load_harnesses(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].command.as_deref(), Some("codex"));
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("token"));
    }
}
