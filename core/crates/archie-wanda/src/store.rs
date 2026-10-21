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

/// Each domain's social state and cursor as this service last loaded or saved them.
pub(crate) type SocialCache = std::collections::BTreeMap<
    ContextDomain,
    (archie_wanda_lncm::SocialState, crate::social::Cursor),
>;

#[derive(Clone)]
pub struct WandaStore {
    config: WandaConfig,
    operator_digest: [u8; 32],
    sources: Vec<(SourceBinding, [u8; 32])>,
    lncm: Option<std::sync::Arc<crate::social::LncmRuntime>>,
    /// Only this service writes the social states (`social::save`), so what it saved is what
    /// the Vault holds: a question reads the state from here instead of opening and parsing
    /// the whole Vault, every message in it included.
    social: std::sync::Arc<std::sync::Mutex<SocialCache>>,
    /// Held while a domain's state is read, changed and saved, so the reader, an edit and a
    /// query's inline read never interleave.
    social_write: std::sync::Arc<std::sync::Mutex<()>>,
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
            // Configured chat ids are matched in their canonical form.
            let mut source = source.clone();
            source.chat_domains = source
                .chat_domains
                .into_iter()
                .filter_map(|(chat, domain)| {
                    crate::chat::routing_key(&chat).map(|key| (key, domain))
                })
                .collect();
            sources.push((source, hash));
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
        // A configured expert must open, and every file of it must match its manifest;
        // otherwise Wanda refuses to start rather than serve weights nobody measured.
        let lncm = match &config.lncm {
            Some(binding) => Some(std::sync::Arc::new(crate::social::LncmRuntime::load(
                binding,
            )?)),
            None => None,
        };
        Ok(Self {
            config,
            operator_digest,
            sources,
            lncm,
            social: Default::default(),
            social_write: Default::default(),
        })
    }
    /// The local expert, when one is configured.
    pub fn lncm(&self) -> Option<&crate::social::LncmRuntime> {
        self.lncm.as_deref()
    }
    pub(crate) fn social_cache(&self) -> &std::sync::Mutex<SocialCache> {
        &self.social
    }
    pub(crate) fn social_write(&self) -> &std::sync::Mutex<()> {
        &self.social_write
    }
    /// Runs Vault and filesystem work off the async runtime. The Vault lock waits up to
    /// seconds for another process, and persisting fsyncs; neither may hold a runtime
    /// worker that is serving other requests.
    pub async fn blocking<T, F>(&self, work: F) -> Result<T, WandaError>
    where
        T: Send + 'static,
        F: FnOnce(&WandaStore) -> Result<T, WandaError> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || work(&store))
            .await
            .map_err(|_| WandaError::Storage)?
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
                    ["events", "query", "graph"].iter().any(|kind| {
                        e.reference
                            .path()
                            .starts_with(&format!("{}/wanda/{kind}/", b.domain.path()))
                    })
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
        let mut sightings: std::collections::BTreeMap<ContextDomain, i64> = Default::default();
        for (domain, event) in &valid {
            let seen = sightings
                .entry(domain.clone())
                .or_insert(event.observed_at_ms);
            *seen = (*seen).max(event.observed_at_ms);
        }
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
                let previous: StoredEvent =
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
                // The first record stands as it was. Rewriting it would stamp an old
                // message as newly stored and push genuinely recent evidence out of the
                // window a query reads.
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
        // A repost leaves its event as first stored, but it still shows the source is alive.
        for (domain, observed) in sightings {
            self.record_sighting(&mut vault, source, &domain, observed, now_ms)?;
        }
        persist_vault(&dir, &vault).map_err(|_| WandaError::Storage)?;
        Ok(report)
    }
    /// Where the latest sighting of a source is kept within one domain.
    fn sighting_ref(
        source: &SourceBinding,
        domain: &ContextDomain,
    ) -> Result<archie_vault::VaultRef, WandaError> {
        let key = serde_json::to_vec(&(&source.device_id, source.source))
            .map_err(|_| WandaError::Storage)?;
        let id = digest(&key)
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        domain
            .reference(&format!("wanda/sources/{id}"))
            .map_err(|_| WandaError::Storage)
    }
    fn sighting(
        vault: &mut VaultGuard,
        reference: &archie_vault::VaultRef,
        now_ms: i64,
    ) -> Option<i64> {
        if !vault.contains(reference) {
            return None;
        }
        let content = vault
            .disclose("wanda-source", reference, PrivacyGrade::Restricted, now_ms)
            .ok()?;
        content.text()?.parse().ok()
    }
    fn record_sighting(
        &self,
        vault: &mut VaultGuard,
        source: &SourceBinding,
        domain: &ContextDomain,
        observed: i64,
        now_ms: i64,
    ) -> Result<(), WandaError> {
        let reference = Self::sighting_ref(source, domain)?;
        if Self::sighting(vault, &reference, now_ms).is_some_and(|known| known >= observed) {
            return Ok(());
        }
        vault
            .store(
                "wanda-source",
                reference,
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                &observed.to_string(),
                now_ms,
                None,
            )
            .map_err(|_| WandaError::Storage)
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
        // Only a stable conversation id routes to another domain; a display title (which
        // anyone in a group can change) always stays in the source's own domain.
        Ok(crate::chat::routing_key(&event.chat_id)
            .and_then(|key| source.chat_domains.get(&key))
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
        let mut vault = open_vault(&self.vault_path()).map_err(|_| WandaError::Storage)?;
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
            let sighting = Self::sighting(&mut vault, &Self::sighting_ref(source, domain)?, now_ms);
            views.push(SourceView {
                device_id: source.device_id.clone(),
                source: source.source,
                coverage: latest.map(|e| e.event.coverage),
                last_observed_at_ms: latest.map(|e| e.event.observed_at_ms).max(sighting),
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
