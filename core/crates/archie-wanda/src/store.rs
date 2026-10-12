use crate::{
    config::{bounded_id, SourceBinding, WandaConfig},
    types::{Actor, Coverage, EventSource, IngestReport, SourceView, StoredEvent, WandaEvent},
    WandaError, MAX_BATCH, MAX_CONTEXT, MAX_REQUEST_BYTES,
};
use archie_cerebra::vault_io::{open_vault, persist_vault, VaultGuard};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, EntryClass};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub struct WandaStore {
    config: WandaConfig,
    operator_digest: [u8; 32],
    sources: Vec<(SourceBinding, [u8; 32])>,
}
impl std::fmt::Debug for WandaStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WandaStore(redacted)")
    }
}
impl WandaStore {
    pub fn new(config: WandaConfig) -> Result<Self, WandaError> {
        config.validate()?;
        let mut sources: Vec<(SourceBinding, [u8; 32])> = Vec::new();
        for source in config.sources.iter().filter(|s| s.enabled) {
            let token = match &source.token_file {
                Some(path) => read_token(path)?,
                None => std::env::var(source.token_env.as_ref().ok_or(WandaError::Unauthorized)?)
                    .map_err(|_| WandaError::Unauthorized)?,
            };
            if !(16..=4096).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(WandaError::Unauthorized);
            }
            let hash = digest(token.as_bytes());
            if sources.iter().any(|(_, h)| same(&hash, h)) {
                return Err(WandaError::Config("source tokens must differ".into()));
            }
            sources.push((source.clone(), hash));
        }
        // One OS transaction protects token and Vault key creation across processes.
        let _guard =
            open_vault(&config.state_dir.join("cerebra/vault")).map_err(|_| WandaError::Storage)?;
        if !config.operator_token_file.exists() {
            let mut raw = [0u8; 32];
            SystemRandom::new()
                .fill(&mut raw)
                .map_err(|_| WandaError::Storage)?;
            let token = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
            if let Some(parent) = config.operator_token_file.parent() {
                std::fs::create_dir_all(parent).map_err(|_| WandaError::Storage)?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options
                .open(&config.operator_token_file)
                .map_err(|_| WandaError::Storage)?;
            file.write_all(token.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| WandaError::Storage)?;
        }
        let operator_digest = digest(read_token(&config.operator_token_file)?.as_bytes());
        if sources.iter().any(|(_, h)| same(&operator_digest, h)) {
            return Err(WandaError::Config(
                "owner and source tokens must differ".into(),
            ));
        }
        Ok(Self {
            config,
            operator_digest,
            sources,
        })
    }
    pub fn config(&self) -> &WandaConfig {
        &self.config
    }
    pub fn vault_path(&self) -> PathBuf {
        self.config.state_dir.join("cerebra/vault")
    }
    pub fn owner_authenticated(&self, token: &str) -> bool {
        token.len() <= 4096 && same(&self.operator_digest, &digest(token.as_bytes()))
    }
    pub fn require_domain(&self, domain: &ContextDomain) -> Result<(), WandaError> {
        if self.config.domains.iter().any(|b| &b.domain == domain) {
            Ok(())
        } else {
            Err(WandaError::UnknownDomain)
        }
    }
    fn purge(&self, vault: &mut VaultGuard, now_ms: i64) -> bool {
        let stale: Vec<_> = vault
            .sealed_entries()
            .into_iter()
            .filter(|e| {
                self.config.domains.iter().any(|b| {
                    ["events", "query", "graph"].iter().any(|kind| e.reference.path()
                        .starts_with(&format!("{}/wanda/{kind}/", b.domain.path())))
                }) && e.expires_at_ms.is_some_and(|at| now_ms >= at)
            })
            .map(|e| e.reference)
            .collect();
        for reference in &stale {
            vault.remove("wanda-retention", reference, now_ms);
        }
        !stale.is_empty()
    }
    pub fn ingest(
        &self,
        token: &str,
        events: &[WandaEvent],
        now_ms: i64,
    ) -> Result<IngestReport, WandaError> {
        if token.len() > 4096 {
            return Err(WandaError::Unauthorized);
        }
        let hash = digest(token.as_bytes());
        let source = self
            .sources
            .iter()
            .find(|(_, h)| same(&hash, h))
            .map(|(s, _)| s)
            .ok_or(WandaError::Unauthorized)?;
        if events.len() > MAX_BATCH
            || serde_json::to_vec(events)
                .map_err(|_| WandaError::Storage)?
                .len()
                > MAX_REQUEST_BYTES
        {
            return Err(WandaError::TooLarge);
        }
        let mut report = IngestReport::default();
        let mut valid = Vec::new();
        for event in events {
            match self.validate_event(source, event, now_ms) {
                Ok(domain) => valid.push((domain, event)),
                Err(reason) => {
                    report.rejected += 1;
                    if !report.reasons.iter().any(|r| r == reason) {
                        report.reasons.push(reason.into());
                    }
                }
            }
        }
        if valid.is_empty() {
            return Ok(report);
        }
        let dir = self.vault_path();
        let mut vault = open_vault(&dir).map_err(|_| WandaError::Storage)?;
        self.purge(&mut vault, now_ms);
        for (domain, event) in valid {
            let key = serde_json::to_vec(&(
                &event.device_id,
                event.source,
                &event.chat_id,
                &event.event_id,
            ))
            .map_err(|_| WandaError::Storage)?;
            let id = digest(&key)
                .iter()
                .take(16)
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let reference = domain
                .reference(&format!("wanda/events/{id}"))
                .map_err(|_| WandaError::Storage)?;
            let stored = StoredEvent {
                domain: domain.clone(),
                evidence_ref: reference.clone(),
                event: event.clone(),
            };
            if vault.contains(&reference) {
                let old = vault
                    .disclose("wanda-ingest", &reference, PrivacyGrade::Restricted, now_ms)
                    .map_err(|_| WandaError::Storage)?;
                let mut previous: StoredEvent =
                    serde_json::from_str(old.text().ok_or(WandaError::Storage)?)
                        .map_err(|_| WandaError::Storage)?;
                let mut prior = previous.event.clone();
                prior.observed_at_ms = 0;
                let mut incoming = event.clone();
                incoming.observed_at_ms = 0;
                if previous.domain != domain
                    || previous.evidence_ref != reference
                    || serde_json::to_vec(&prior).ok() != serde_json::to_vec(&incoming).ok()
                {
                    report.rejected += 1;
                    if !report.reasons.iter().any(|r| r == "event_id_collision") {
                        report.reasons.push("event_id_collision".into());
                    }
                    continue;
                }
                let expiry = vault
                    .sealed_entries()
                    .into_iter()
                    .find(|e| e.reference == reference)
                    .and_then(|e| e.expires_at_ms)
                    .ok_or(WandaError::Storage)?;
                previous.event.observed_at_ms =
                    previous.event.observed_at_ms.max(event.observed_at_ms);
                vault
                    .store(
                        "wanda-repost",
                        reference,
                        EntryClass::PrivateFact,
                        PrivacyGrade::Restricted,
                        &serde_json::to_string(&previous).map_err(|_| WandaError::Storage)?,
                        now_ms,
                        Some(expiry),
                    )
                    .map_err(|_| WandaError::Storage)?;
                report.duplicates += 1;
                continue;
            }
            let expiry = event
                .observed_at_ms
                .saturating_add(i64::from(self.config.retention_hours) * 3_600_000);
            vault
                .store(
                    "wanda-ingest",
                    reference,
                    EntryClass::PrivateFact,
                    PrivacyGrade::Restricted,
                    &serde_json::to_string(&stored).map_err(|_| WandaError::Storage)?,
                    now_ms,
                    Some(expiry),
                )
                .map_err(|_| WandaError::Storage)?;
            report.accepted += 1;
        }
        persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
        Ok(report)
    }
    fn validate_event(
        &self,
        source: &SourceBinding,
        event: &WandaEvent,
        now_ms: i64,
    ) -> Result<ContextDomain, &'static str> {
        if event.device_id != source.device_id || event.source != source.source {
            return Err("source_binding_mismatch");
        }
        if event.actor != Actor::ConfirmedOwner
            && !(source.allow_presumed_owner && event.actor == Actor::PresumedOwner)
        {
            return Err("owner_attribution_required");
        }
        let coverage = match event.source {
            EventSource::AndroidNotification => Coverage::NotificationPreview,
            EventSource::WhatsappVisibleDom => Coverage::VisibleChat,
            EventSource::IrisSemantic => Coverage::SemanticObservation,
        };
        if event.coverage != coverage {
            return Err("coverage_mismatch");
        }
        if event.schema_version != 1
            || [
                &event.event_id,
                &event.device_id,
                &event.chat_id,
                &event.sender,
            ]
            .iter()
            .any(|s| !bounded_id(s))
            || event
                .source_message_id
                .as_ref()
                .is_some_and(|s| !bounded_id(s))
            || event.monitor_id.as_ref().is_some_and(|s| !bounded_id(s))
            || event.window_id.as_ref().is_some_and(|s| !bounded_id(s))
            || (event.source != EventSource::IrisSemantic
                && (event.monitor_id.is_some() || event.window_id.is_some()))
        {
            return Err("invalid_identifiers");
        }
        if event.text.trim().is_empty()
            || event.text.chars().count() > 2000
            || event
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
        {
            return Err("invalid_text");
        }
        if event.observed_at_ms < 0
            || event.observed_at_ms > now_ms.saturating_add(60_000)
            || event
                .sent_at_ms
                .is_some_and(|t| t < 0 || t > event.observed_at_ms.saturating_add(60_000))
            || event
                .observed_at_ms
                .saturating_add(i64::from(self.config.retention_hours) * 3_600_000)
                <= now_ms
        {
            return Err("stale_or_invalid_time");
        }
        Ok(source
            .chat_domains
            .get(&event.chat_id)
            .unwrap_or(&source.domain)
            .clone())
    }
    pub fn events(
        &self,
        domain: &ContextDomain,
        limit: usize,
        now_ms: i64,
    ) -> Result<Vec<StoredEvent>, WandaError> {
        self.require_domain(domain)?;
        if limit > MAX_CONTEXT {
            return Err(WandaError::TooLarge);
        }
        let dir = self.vault_path();
        let mut vault = open_vault(&dir).map_err(|_| WandaError::Storage)?;
        if self.purge(&mut vault, now_ms) {
            persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
        }
        let prefix = format!("{}/wanda/events/", domain.path());
        let mut entries: Vec<_> = vault
            .sealed_entries()
            .into_iter()
            .filter(|e| e.reference.path().starts_with(&prefix))
            .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.created_at_ms));
        let mut events = Vec::new();
        for entry in entries.into_iter().take(limit) {
            let content = vault
                .disclose(
                    "wanda-query",
                    &entry.reference,
                    PrivacyGrade::Restricted,
                    now_ms,
                )
                .map_err(|_| WandaError::Storage)?;
            let stored: StoredEvent =
                serde_json::from_str(content.text().ok_or(WandaError::Storage)?)
                    .map_err(|_| WandaError::Storage)?;
            if &stored.domain != domain || stored.evidence_ref != entry.reference {
                return Err(WandaError::Storage);
            }
            events.push(stored);
        }
        Ok(events)
    }
    pub fn sources(
        &self,
        domain: &ContextDomain,
        now_ms: i64,
    ) -> Result<Vec<SourceView>, WandaError> {
        let events = self.events(domain, MAX_CONTEXT, now_ms)?;
        let mut views = Vec::new();
        for (source, _) in &self.sources {
            if &source.domain != domain && !source.chat_domains.values().any(|d| d == domain) {
                continue;
            }
            let latest = events
                .iter()
                .filter(|e| {
                    e.event.device_id == source.device_id && e.event.source == source.source
                })
                .max_by_key(|e| e.event.observed_at_ms);
            views.push(SourceView {
                device_id: source.device_id.clone(),
                source: source.source,
                coverage: latest.map(|e| e.event.coverage),
                last_observed_at_ms: latest.map(|e| e.event.observed_at_ms),
            });
        }
        Ok(views)
    }
    pub fn domains(&self) -> Vec<ContextDomain> {
        self.config
            .domains
            .iter()
            .map(|b| b.domain.clone())
            .collect()
    }
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    // HMAC verification supplies ring's supported constant-time comparison.
    let key_a = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, a);
    let key_b = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b);
    let tag = ring::hmac::sign(&key_b, b"archie-wanda-token-comparison-v1");
    ring::hmac::verify(&key_a, b"archie-wanda-token-comparison-v1", tag.as_ref()).is_ok()
}
pub(crate) fn read_token(path: &Path) -> Result<String, WandaError> {
    let file = std::fs::File::open(path).map_err(|_| WandaError::Unauthorized)?;
    if !file.metadata().map_err(|_| WandaError::Storage)?.is_file() {
        return Err(WandaError::Unauthorized);
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| WandaError::Storage)?;
    let token = std::str::from_utf8(&bytes)
        .map_err(|_| WandaError::Unauthorized)?
        .trim();
    if bytes.len() > 4096
        || !(16..=4096).contains(&token.len())
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(WandaError::Unauthorized);
    }
    Ok(token.to_string())
}
