//! A local `llama-server`'s context, kept instead of recomputed (§9 of
//! `_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`).
//!
//! The LLM is Wanda's bottleneck (405 s to prefill 2k tokens on the CPU, measured) and
//! llama.cpp answers `cache_reuse is not supported` for the hybrid Bonsai-2 model, so the
//! context it already computed has to be kept whole:
//!
//! - [`PromptLayout`] orders a prompt so the part that changes least comes first: system,
//!   the durable state summary, the task's context bundle, then the turn. The digest of
//!   everything but the turn names the KV state, so the same prefix finds the same state.
//! - [`SlotStore`] saves a slot's state to the server's `--slot-save-path` and restores it
//!   (`POST /slots/{id}?action=save|restore|erase`), so switching conversation or task
//!   restores instead of prefilling again. Whether llama.cpp's slot file carries the
//!   recurrent (DeltaNet) state of the hybrid layers is not verified here; restore's
//!   `n_restored` against the prompt's length is the check to run on the machine.
//!
//! Only a loopback server is accepted: slot files hold the whole context, and file names
//! are plain names, so nothing outside the save directory is named.

use crate::{ChatMessage, MessageRole, RuntimeError};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// A prompt in the order that keeps its prefix stable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptLayout {
    pub system: String,
    /// What persists across turns: Wanda's social state summary, the owner's preferences.
    pub durable_state: String,
    /// Cortex's `ContextBundle` for the task, as rendered.
    pub task_context: String,
    pub turn: String,
}

impl PromptLayout {
    /// The messages, prefix first. Empty parts are left out.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut prefix = Vec::new();
        for part in [&self.system, &self.durable_state, &self.task_context] {
            if !part.is_empty() {
                prefix.push(part.as_str());
            }
        }
        let mut out = Vec::new();
        if !prefix.is_empty() {
            out.push(ChatMessage {
                role: MessageRole::System,
                content: prefix.join("\n\n"),
            });
        }
        out.push(ChatMessage {
            role: MessageRole::User,
            content: self.turn.clone(),
        });
        out
    }

    /// SHA-256 of everything but the turn: equal digests share a KV prefix.
    pub fn prefix_digest(&self) -> String {
        let mut digest = Sha256::new();
        for part in [&self.system, &self.durable_state, &self.task_context] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        hex::encode(digest.finalize())
    }

    /// The slot file for this prefix.
    pub fn slot_name(&self) -> SlotName {
        SlotName(format!("prefix-{}.bin", &self.prefix_digest()[..32]))
    }
}

/// A slot file name: letters, digits, `.`, `_` and `-`, not starting with `.`, at most 96.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SlotName(String);

impl SlotName {
    pub fn new(name: &str) -> Result<Self, RuntimeError> {
        let plain = !name.is_empty()
            && name.len() <= 96
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if plain {
            Ok(Self(name.to_owned()))
        } else {
            Err(RuntimeError::InvalidRequest)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the server reports for a save or a restore.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SlotReport {
    pub id_slot: u32,
    #[serde(default)]
    pub filename: Option<String>,
    /// Tokens written (save).
    #[serde(default)]
    pub n_saved: Option<u64>,
    /// Tokens read back (restore).
    #[serde(default)]
    pub n_restored: Option<u64>,
    #[serde(default)]
    pub n_erased: Option<u64>,
}

/// Slot save, restore and erase on a loopback `llama-server`.
#[derive(Debug, Clone)]
pub struct SlotStore {
    base: Url,
    client: Client,
}

const MAX_REPORT_BYTES: usize = 64 * 1024;

impl SlotStore {
    /// `base_url` is the server's root, e.g. `http://127.0.0.1:8081/`.
    pub fn new(base_url: &str) -> Result<Self, RuntimeError> {
        let base = Url::parse(base_url).map_err(|_| RuntimeError::InvalidEndpoint)?;
        if base.username() != ""
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(RuntimeError::InvalidEndpoint);
        }
        let loopback = base
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1"));
        if !loopback || base.scheme() != "http" {
            return Err(RuntimeError::InsecureEndpoint);
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { base, client })
    }

    async fn call(
        &self,
        slot: u32,
        action: &str,
        name: Option<&SlotName>,
    ) -> Result<SlotReport, RuntimeError> {
        let mut url = self
            .base
            .join(&format!("slots/{slot}"))
            .map_err(|_| RuntimeError::InvalidEndpoint)?;
        url.query_pairs_mut().append_pair("action", action);
        let body = match name {
            Some(name) => serde_json::json!({ "filename": name.as_str() }),
            None => serde_json::json!({}),
        };
        let response = self.client.post(url).json(&body).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(RuntimeError::ResponseTooLarge);
        }
        if !status.is_success() {
            return Err(RuntimeError::HttpStatus(status));
        }
        let report: SlotReport = serde_json::from_slice(&bytes)?;
        if report.id_slot != slot {
            return Err(RuntimeError::InvalidRequest);
        }
        Ok(report)
    }

    pub async fn save(&self, slot: u32, name: &SlotName) -> Result<SlotReport, RuntimeError> {
        self.call(slot, "save", Some(name)).await
    }

    pub async fn restore(&self, slot: u32, name: &SlotName) -> Result<SlotReport, RuntimeError> {
        self.call(slot, "restore", Some(name)).await
    }

    pub async fn erase(&self, slot: u32) -> Result<SlotReport, RuntimeError> {
        self.call(slot, "erase", None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_turn_does_not_change_the_prefix() {
        let mut layout = PromptLayout {
            system: "Sos Wanda.".into(),
            durable_state: "Ana: amiga, prefiere audios.".into(),
            task_context: "Reunión el viernes.".into(),
            turn: "¿Qué le respondo?".into(),
        };
        let digest = layout.prefix_digest();
        let messages = layout.messages();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].content.starts_with("Sos Wanda."));
        assert_eq!(messages[1].content, "¿Qué le respondo?");
        layout.turn = "Otra cosa".into();
        assert_eq!(layout.prefix_digest(), digest);
        layout.durable_state.push('!');
        assert_ne!(layout.prefix_digest(), digest);
        // Moving text between parts is another prefix.
        let a = PromptLayout {
            system: "ab".into(),
            durable_state: "c".into(),
            ..Default::default()
        };
        let b = PromptLayout {
            system: "a".into(),
            durable_state: "bc".into(),
            ..Default::default()
        };
        assert_ne!(a.prefix_digest(), b.prefix_digest());
        assert!(SlotName::new(layout.slot_name().as_str()).is_ok());
    }

    #[test]
    fn slot_names_stay_inside_the_save_directory() {
        for bad in ["", "../x.bin", "a/b", ".hidden", "x y", &"a".repeat(97)] {
            assert!(SlotName::new(bad).is_err(), "{bad:?}");
        }
        assert!(SlotName::new("wanda-c1.bin").is_ok());
    }

    #[test]
    fn only_a_loopback_server_is_accepted() {
        assert!(SlotStore::new("http://127.0.0.1:8081/").is_ok());
        assert!(SlotStore::new("http://localhost:8081/").is_ok());
        assert!(SlotStore::new("http://[::1]:8081/").is_ok());
        assert!(SlotStore::new("http://192.168.1.4:8081/").is_err());
        assert!(SlotStore::new("https://example.com/").is_err());
    }

    #[tokio::test]
    async fn save_and_restore_speak_llama_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for reply in [
                r#"{"id_slot":2,"filename":"wanda-c1.bin","n_saved":1745,"n_written":14309796}"#,
                r#"{"id_slot":2,"filename":"wanda-c1.bin","n_restored":1745,"n_read":14309796}"#,
                r#"{"id_slot":3,"n_restored":1}"#,
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0u8; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                seen.push(String::from_utf8_lossy(&buffer[..n]).to_string());
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            seen
        });
        let store = SlotStore::new(&format!("http://127.0.0.1:{port}/")).unwrap();
        let name = SlotName::new("wanda-c1.bin").unwrap();
        assert_eq!(store.save(2, &name).await.unwrap().n_saved, Some(1745));
        assert_eq!(
            store.restore(2, &name).await.unwrap().n_restored,
            Some(1745)
        );
        assert!(
            store.restore(2, &name).await.is_err(),
            "a report for another slot"
        );
        let seen = server.await.unwrap();
        assert!(seen[0].starts_with("POST /slots/2?action=save "));
        assert!(seen[0].contains(r#"{"filename":"wanda-c1.bin"}"#));
        assert!(seen[1].starts_with("POST /slots/2?action=restore "));
    }
}
