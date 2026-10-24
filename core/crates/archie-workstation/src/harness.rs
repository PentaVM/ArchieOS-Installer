use archie_types::PrivacyGrade;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
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
        let spec = self.clone();
        // Preserve the synchronous API without nesting a runtime in CLI Tokio.
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(HarnessError::Io)?
                .block_on(spec.probe_bounded(Duration::from_secs(5)))
        })
        .join()
        .map_err(|_| HarnessError::ProcessFailure)?
    }

    pub async fn probe_bounded(&self, timeout: Duration) -> Result<String, HarnessError> {
        use tokio::io::AsyncReadExt;
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err(HarnessError::ProcessFailure);
        }
        let mut command = tokio::process::Command::new(
            self.command
                .as_deref()
                .ok_or(HarnessError::MissingCommand)?,
        );
        command
            .args(&self.args)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn()?;
        let mut stdout = child.stdout.take().ok_or(HarnessError::ProcessFailure)?;
        let result = tokio::time::timeout(timeout, async {
            let mut bytes = Vec::new();
            (&mut stdout).take(4097).read_to_end(&mut bytes).await?;
            if bytes.len() > 4096 {
                return Err(HarnessError::ResponseTooLarge);
            }
            if !child.wait().await?.success() {
                return Err(HarnessError::ProcessFailure);
            }
            Ok(String::from_utf8_lossy(&bytes)
                .trim()
                .chars()
                .take(256)
                .collect())
        })
        .await
        .map_err(|_| HarnessError::Timeout)
        .and_then(|v| v);
        if result.is_err() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        }
        result
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
    #[error("harness probe deadline expired")]
    Timeout,
    #[error("harness input is invalid or exceeds its byte budget")]
    InvalidRequest,
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
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
        if endpoint.scheme() != "http"
            || !loopback
            || timeout.is_zero()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
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
        // A loopback proxy proves transport placement, not downstream locality or
        // retention. Legacy UHP has no admitted backend descriptor: public only.
        if privacy != PrivacyGrade::Public {
            return Err(HarnessError::PrivacyDenied);
        }
        let valid_id = |id: &str| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
        };
        if request.input.len() > 64 * 1024
            || !valid_id(&request.model)
            || !valid_id(&request.metadata.harness_id)
        {
            return Err(HarnessError::InvalidRequest);
        }
        let mut response = self
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
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(HarnessError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
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
                PrivacyGrade::Public,
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
    async fn loopback_proxy_does_not_establish_local_inference_or_retention() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = UhpClient::loopback(
            &format!("http://{}/", listener.local_addr().unwrap()),
            Duration::from_millis(100),
        )
        .unwrap();
        for privacy in [
            PrivacyGrade::Internal,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
        ] {
            let result = client
                .invoke(
                    privacy,
                    &UhpRequest {
                        model: "cloud-worker".into(),
                        input: "not sent".into(),
                        metadata: UhpMetadata {
                            harness_id: "codex".into(),
                        },
                    },
                )
                .await;
            assert!(matches!(result, Err(HarnessError::PrivacyDenied)));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn proxy_endpoint_rejects_embedded_credentials_query_and_fragment() {
        for endpoint in [
            "http://user:password@localhost:8000/",
            "http://localhost:8000/?key=value",
            "http://localhost:8000/#fragment",
        ] {
            assert!(UhpClient::loopback(endpoint, Duration::from_secs(1)).is_err());
        }
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
