//! The cloud repository the nodes share (a private Hugging Face model repository): the board
//! and the packages. Reached with `curl`, which every Windows 10+ and Linux machine has, so
//! the node carries no TLS stack of its own. The token is handed to `curl` on its standard
//! input, never on a command line (where any process list shows it) and never in a file.
//!
//! Every write is one commit on top of the revision the node read. The Hub refuses a
//! commit whose parent is no longer the head, which is what keeps two nodes from
//! overwriting each other's changes to the board.

use base64::Engine;
use std::io::Write;
use std::process::{Command, Stdio};

/// Packages travel inline in the commit; larger files would need the Hub's LFS upload.
pub const MAX_INLINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// Someone committed since this node read the board: read it again and retry.
    Moved,
    NotFound,
    TooLarge(usize),
    Failed(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Moved => f.write_str("the board changed since it was read: read it again and retry"),
            Self::NotFound => f.write_str("not found in the repository"),
            Self::TooLarge(bytes) => write!(f, "file of {bytes} bytes is larger than an inline commit takes"),
            Self::Failed(message) => write!(f, "hub: {message}"),
        }
    }
}

/// What a node needs from the shared repository.
pub trait Hub {
    /// The head revision of the main branch.
    fn head(&self) -> Result<String, HubError>;
    fn read(&self, path: &str) -> Result<Vec<u8>, HubError>;
    /// One commit of `files` on top of `parent`, authored by this node's token.
    fn commit(&self, summary: &str, files: &[(String, Vec<u8>)], parent: &str) -> Result<String, HubError>;
}

pub struct HuggingFace {
    pub repo: String,
    token: String,
    pub endpoint: String,
}

impl HuggingFace {
    pub fn new(repo: &str, token: String) -> Self {
        Self { repo: repo.into(), token, endpoint: "https://huggingface.co".into() }
    }

    /// `curl` with the authorization header read from its standard input.
    fn curl(&self, args: &[&str], body: Option<&[u8]>) -> Result<(u16, Vec<u8>), HubError> {
        let mut command = Command::new("curl");
        command.args(["-sS", "-L", "--max-time", "600", "-H", "@-", "-w", "\n%{http_code}"]);
        let body_file = match body {
            Some(bytes) => {
                let file = tempfile_with(bytes).map_err(|e| HubError::Failed(e.to_string()))?;
                command.arg("--data-binary").arg(format!("@{}", file.display()));
                Some(file)
            }
            None => None,
        };
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| HubError::Failed(format!("curl: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = writeln!(stdin, "Authorization: Bearer {}", self.token);
        }
        let output = child.wait_with_output().map_err(|e| HubError::Failed(e.to_string()));
        if let Some(file) = body_file {
            let _ = std::fs::remove_file(file);
        }
        let output = output?;
        let mut stdout = output.stdout;
        let split = stdout.iter().rposition(|b| *b == b'\n').ok_or_else(|| {
            HubError::Failed(String::from_utf8_lossy(&output.stderr).chars().take(200).collect())
        })?;
        let code = std::str::from_utf8(&stdout[split + 1..]).ok().and_then(|c| c.trim().parse().ok()).unwrap_or(0);
        stdout.truncate(split);
        Ok((code, stdout))
    }
}

fn tempfile_with(bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    let name = format!("archie-skill-node-{}-{}.ndjson", std::process::id(),
                       std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// The body of a Hub commit: a header line, then one line per file, base64 inline.
pub fn commit_body(summary: &str, files: &[(String, Vec<u8>)], parent: &str) -> Result<Vec<u8>, HubError> {
    let mut lines = vec![serde_json::json!({"key": "header",
                                            "value": {"summary": summary, "description": "", "parentCommit": parent}})];
    for (path, bytes) in files {
        if bytes.len() > MAX_INLINE_BYTES {
            return Err(HubError::TooLarge(bytes.len()));
        }
        lines.push(serde_json::json!({"key": "file", "value": {
            "path": path, "encoding": "base64",
            "content": base64::engine::general_purpose::STANDARD.encode(bytes)}}));
    }
    let mut body = Vec::new();
    for line in lines {
        body.extend_from_slice(line.to_string().as_bytes());
        body.push(b'\n');
    }
    Ok(body)
}

impl Hub for HuggingFace {
    fn head(&self) -> Result<String, HubError> {
        let url = format!("{}/api/models/{}/revision/main", self.endpoint, self.repo);
        let (code, body) = self.curl(&[&url], None)?;
        if code != 200 {
            return Err(HubError::Failed(format!("revision: http {code}")));
        }
        let value: serde_json::Value = serde_json::from_slice(&body).map_err(|e| HubError::Failed(e.to_string()))?;
        value["sha"].as_str().map(str::to_string).ok_or_else(|| HubError::Failed("revision has no sha".into()))
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, HubError> {
        let url = format!("{}/{}/resolve/main/{}", self.endpoint, self.repo, path);
        match self.curl(&[&url], None)? {
            (200, body) => Ok(body),
            (404, _) => Err(HubError::NotFound),
            (code, _) => Err(HubError::Failed(format!("read {path}: http {code}"))),
        }
    }

    fn commit(&self, summary: &str, files: &[(String, Vec<u8>)], parent: &str) -> Result<String, HubError> {
        let body = commit_body(summary, files, parent)?;
        let url = format!("{}/api/models/{}/commit/main", self.endpoint, self.repo);
        match self.curl(&["-X", "POST", "-H", "Content-Type: application/x-ndjson", &url], Some(&body))? {
            (200, reply) => {
                let value: serde_json::Value = serde_json::from_slice(&reply).unwrap_or_default();
                Ok(value["commitOid"].as_str().unwrap_or_default().to_string())
            }
            // The parent is no longer the head: another node committed first.
            (409 | 412, _) => Err(HubError::Moved),
            (code, reply) => Err(HubError::Failed(format!(
                "commit: http {code}: {}", String::from_utf8_lossy(&reply).chars().take(200).collect::<String>()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_names_its_parent_and_carries_files_inline() {
        let body = commit_body("csgo candidate", &[("board.json".into(), b"{}".to_vec())], "abc123").unwrap();
        let lines: Vec<serde_json::Value> = String::from_utf8(body).unwrap().lines()
            .map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["value"]["parentCommit"], "abc123");
        assert_eq!(lines[1]["value"]["path"], "board.json");
        assert_eq!(lines[1]["value"]["content"], "e30=");
        let big = vec![0u8; MAX_INLINE_BYTES + 1];
        assert_eq!(commit_body("x", &[("p".into(), big)], "a"), Err(HubError::TooLarge(MAX_INLINE_BYTES + 1)));
    }
}
