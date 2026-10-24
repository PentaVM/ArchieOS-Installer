//! One feeder from every Sentry and Wanda producer to Cerebra, shaped by a
//! [`FocusProfile`].
//!
//! The producers stay where they are and keep their discipline — validation,
//! content-derived references, sealing at `restricted`, source timestamps:
//! [`crate::sentry_bridge`] for netwatch alerts, [`crate::wanda_bridge`] for
//! the owner's chats and notifications, and [`record_device_events`] here for
//! device changes. Each record becomes a [`FeedItem`] (a reference, its
//! source, its own time and a weight), and [`plan`] decides, under one
//! profile, which items enter the [`PillarContext`], in what order, and which
//! are dropped and why. [`consider_plan`] runs one Cerebra pass over it.
//!
//! What a profile cannot do, by construction:
//!
//! - Cross domains. The plan holds one [`ContextDomain`]; an item whose
//!   reference lies outside it is dropped as [`DropReason::OtherDomain`], and
//!   a work profile cannot be planned in a personal domain (or the reverse).
//! - Lower a grade. The pass observation is graded `restricted` by
//!   [`PillarContext::observation`]; on a node without a local model the pass
//!   is refused and that refusal is the outcome.
//! - Invent content. Items are references to entries the producers already
//!   sealed; the feeder stores nothing except device records it produces.
//!
//! Weights are criteria, not measurements: alert severity maps to 30/60/90
//! (a hypothesis counts half), device changes to 50/40/20, a message to its
//! Wanda salience, a chat aggregate to ten points per message (capped).
//!
//! [`collect_from_vault`] rebuilds the items from the vault after a restart.
//! It has to read each entry to weigh it, and every read is a disclosure in
//! the vault's audit log under the actor `focus-feeder`; the in-memory path
//! ([`FeedItem::from_alert`] and friends) reads nothing.
use crate::focus::{FeedSource, FocusProfile, ProfileError};
use crate::pass::{Cerebra, CerebraError, Intent, Pass};
use crate::pillars::{Pillar, PillarContext, PillarError};
use crate::sentry_bridge::{
    classify, content_id, AlertOrigin, AlertRecord, AlertSeverity, PassRefusal,
};
use crate::wanda_bridge::{Category, ChatPlatform, ChatRecord, MessageRecord};
use crate::{Observation, Trigger};
use archie_registry::ModelRegistry;
use archie_types::{PrivacyGrade, TraceId};
use archie_vault::{ContextDomain, DomainError, EntryClass, Vault, VaultError, VaultRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Most items one plan considers (before the profile's budget applies).
pub const MAX_FEED_ITEMS: usize = 4096;
/// Most device events one call records.
pub const MAX_DEVICE_EVENTS: usize = 256;
const MAX_DEVICE_LEN: usize = 64;
const MAX_DETAIL_LEN: usize = 256;
const DEVICE_FORMAT: &str = "archie-sentry-device-v1";
const ACTOR: &str = "focus-feeder";

#[derive(Debug, thiserror::Error)]
pub enum FeederError {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error("the focus profile does not run in this kind of domain")]
    ProfileDomain,
    #[error(
        "more than {MAX_FEED_ITEMS} feed items, or more than {MAX_DEVICE_EVENTS} device events"
    )]
    TooMany,
    #[error("invalid device event: {0}")]
    InvalidDevice(&'static str),
    #[error("a different record already sits at {reference}")]
    ReferenceCollision { reference: VaultRef },
    #[error(transparent)]
    Pillar(#[from] PillarError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error(transparent)]
    Cerebra(#[from] CerebraError),
}

// ---------------------------------------------------------------------------
// Device changes: the producer that did not exist yet
// ---------------------------------------------------------------------------

/// What happened to a device, as the unified device view reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceChange {
    Appeared,
    AddressChanged,
    Disappeared,
}

impl DeviceChange {
    fn weight(self) -> u8 {
        match self {
            Self::Appeared => 50,
            Self::AddressChanged => 40,
            Self::Disappeared => 20,
        }
    }
}

/// One device change. `device` is a hardware address, never a person.
#[derive(Debug, Clone)]
pub struct DeviceEvent {
    /// The source's own time, milliseconds.
    pub at_ms: u64,
    pub device: String,
    pub change: DeviceChange,
    pub detail: String,
    pub origin: AlertOrigin,
}

#[derive(Serialize, Deserialize)]
struct DeviceContent {
    format: String,
    at_ms: u64,
    device: String,
    change: DeviceChange,
    origin: String,
    detail: String,
}

/// A device change sealed in the vault.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    reference: VaultRef,
    at_ms: i64,
    change: DeviceChange,
    origin: AlertOrigin,
}

impl DeviceRecord {
    pub fn reference(&self) -> &VaultRef {
        &self.reference
    }
    pub fn at_ms(&self) -> i64 {
        self.at_ms
    }
    pub fn change(&self) -> DeviceChange {
        self.change
    }
}

fn bounded(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

/// Validate, seal (restricted, at the event's own time) and reference device
/// changes under `<domain>/sentry/devices/<content id>`. A replay lands on
/// the same reference and is stored once; a batch with one invalid event
/// stores nothing.
pub fn record_device_events(
    vault: &mut Vault,
    domain: &ContextDomain,
    events: &[DeviceEvent],
) -> Result<Vec<DeviceRecord>, FeederError> {
    if events.len() > MAX_DEVICE_EVENTS {
        return Err(FeederError::TooMany);
    }
    let mut shaped = Vec::with_capacity(events.len());
    for event in events {
        if !bounded(&event.device, MAX_DEVICE_LEN) {
            return Err(FeederError::InvalidDevice("device"));
        }
        if !bounded(&event.detail, MAX_DETAIL_LEN) {
            return Err(FeederError::InvalidDevice("detail"));
        }
        let at_ms = i64::try_from(event.at_ms).map_err(|_| FeederError::InvalidDevice("at_ms"))?;
        let content = serde_json::to_string(&DeviceContent {
            format: DEVICE_FORMAT.into(),
            at_ms: event.at_ms,
            device: event.device.clone(),
            change: event.change,
            origin: event.origin.as_str().into(),
            detail: event.detail.clone(),
        })
        .map_err(|_| FeederError::InvalidDevice("encoding"))?;
        let reference = domain.reference(&format!(
            "sentry/devices/{}",
            content_id(content.as_bytes())
        ))?;
        shaped.push((
            DeviceRecord {
                reference,
                at_ms,
                change: event.change,
                origin: event.origin,
            },
            content,
        ));
    }
    let mut out: Vec<DeviceRecord> = Vec::new();
    for (record, content) in shaped {
        if out.iter().any(|r| r.reference == record.reference) {
            continue;
        }
        if vault.contains(&record.reference) {
            let existing = vault
                .disclose(
                    ACTOR,
                    &record.reference,
                    PrivacyGrade::Restricted,
                    record.at_ms,
                )
                .ok()
                .and_then(|d| d.text().map(str::to_owned));
            if existing.as_deref() != Some(content.as_str()) {
                return Err(FeederError::ReferenceCollision {
                    reference: record.reference,
                });
            }
        } else {
            vault.store(
                ACTOR,
                record.reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                &content,
                record.at_ms,
                None,
            )?;
        }
        out.push(record);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// One reference the feeder may hand to Cerebra. Carries no content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    pub source: FeedSource,
    pub reference: VaultRef,
    /// The source's own time; 0 is the explicit unknown.
    pub at_ms: i64,
    /// 0..=100, see the module's weight table.
    pub weight: u8,
    /// Dominant Wanda category, when the item is a message or notification.
    pub category: Option<Category>,
}

fn severity_weight(severity: AlertSeverity, origin: AlertOrigin) -> u8 {
    let base = match severity {
        AlertSeverity::Notice => 30,
        AlertSeverity::Warning => 60,
        AlertSeverity::High => 90,
    };
    match origin {
        AlertOrigin::Measurement => base,
        AlertOrigin::Hypothesis => base / 2,
    }
}

impl FeedItem {
    pub fn from_alert(record: &AlertRecord) -> Self {
        Self {
            source: FeedSource::NetwatchAlert,
            reference: record.reference().clone(),
            at_ms: record.at_ms(),
            weight: severity_weight(record.severity(), record.origin()),
            category: None,
        }
    }
    pub fn from_device(record: &DeviceRecord) -> Self {
        let weight = match record.origin {
            AlertOrigin::Measurement => record.change.weight(),
            AlertOrigin::Hypothesis => record.change.weight() / 2,
        };
        Self {
            source: FeedSource::Device,
            reference: record.reference.clone(),
            at_ms: record.at_ms,
            weight,
            category: None,
        }
    }
    pub fn from_message(record: &MessageRecord) -> Self {
        let source = match record.platform() {
            ChatPlatform::Notification(_) => FeedSource::WandaNotification,
            _ => FeedSource::WandaMessage,
        };
        Self {
            source,
            reference: record.reference().clone(),
            at_ms: record
                .sent_at()
                .and_then(|at| i64::try_from(at).ok())
                .unwrap_or(0),
            weight: record.vector().salience().min(100),
            category: record.vector().dominant().map(|(c, _)| c),
        }
    }
    pub fn from_chat(record: &ChatRecord) -> Self {
        Self {
            source: FeedSource::WandaChat,
            reference: record.reference().clone(),
            at_ms: 0,
            weight: (record.message_count().saturating_mul(10)).min(100) as u8,
            category: None,
        }
    }
    pub fn from_iris_episode(record: &crate::iris_bridge::IrisEpisodeRecord) -> Self {
        Self {
            source: FeedSource::IrisEpisode,
            reference: record.reference.clone(),
            at_ms: record.at_ms,
            weight: record.weight,
            category: None,
        }
    }
    pub fn from_forge_artifact(record: &crate::forge_bridge::ForgeArtifactRecord) -> Self {
        Self {
            source: FeedSource::ForgeArtifact,
            reference: record.reference.clone(),
            at_ms: record.at_ms,
            weight: record.weight,
            category: None,
        }
    }
}

fn category_from_label(label: &str) -> Option<Category> {
    Category::ALL.into_iter().find(|c| c.label() == label)
}

/// Rebuild the feed items of one domain from the vault, newest first,
/// skipping entries sealed before `since_ms`. Each candidate is disclosed
/// (and audited) once to read its weight; anything that does not parse as one
/// of the known record formats is skipped, never guessed.
pub fn collect_from_vault(
    vault: &mut Vault,
    domain: &ContextDomain,
    since_ms: i64,
    now_ms: i64,
) -> Vec<FeedItem> {
    let mut candidates: Vec<(VaultRef, i64)> = vault
        .sealed_entries()
        .into_iter()
        .filter(|e| domain.contains(&e.reference) && e.created_at_ms >= since_ms)
        .map(|e| (e.reference, e.created_at_ms))
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates.truncate(MAX_FEED_ITEMS);
    let base = domain.path().len() + 1;
    let mut items = Vec::new();
    for (reference, at_ms) in candidates {
        let relative = &reference.path()[base..];
        let kind = if relative.starts_with("sentry/alerts/") {
            FeedSource::NetwatchAlert
        } else if relative.starts_with("sentry/devices/") {
            FeedSource::Device
        } else if relative.starts_with("wanda/messages/") {
            FeedSource::WandaMessage
        } else if relative.starts_with("wanda/chats/") {
            FeedSource::WandaChat
        } else if relative.starts_with("vault/documents/") {
            FeedSource::VaultDocument
        } else if relative.starts_with("iris/episodes/") {
            FeedSource::IrisEpisode
        } else if relative.starts_with("forge/artifacts/") {
            FeedSource::ForgeArtifact
        } else {
            continue;
        };
        let Some(text) = vault
            .disclose(ACTOR, &reference, PrivacyGrade::Restricted, now_ms)
            .ok()
            .and_then(|d| d.text().map(str::to_owned))
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let str_of = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let item = match kind {
            FeedSource::NetwatchAlert => {
                let severity = match str_of("severity") {
                    "notice" => AlertSeverity::Notice,
                    "warning" => AlertSeverity::Warning,
                    "high" => AlertSeverity::High,
                    _ => continue,
                };
                let origin = match str_of("origin") {
                    "measurement" => AlertOrigin::Measurement,
                    "hypothesis" => AlertOrigin::Hypothesis,
                    _ => continue,
                };
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight: severity_weight(severity, origin),
                    category: None,
                }
            }
            FeedSource::Device => {
                let Ok(content) = serde_json::from_value::<DeviceContent>(value.clone()) else {
                    continue;
                };
                if content.format != DEVICE_FORMAT {
                    continue;
                }
                let weight = if content.origin == "measurement" {
                    content.change.weight()
                } else {
                    content.change.weight() / 2
                };
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight,
                    category: None,
                }
            }
            FeedSource::WandaMessage | FeedSource::WandaNotification => {
                let source = if str_of("platform") == "notification" {
                    FeedSource::WandaNotification
                } else {
                    FeedSource::WandaMessage
                };
                let Some(salience) = value.get("salience").and_then(|v| v.as_u64()) else {
                    continue;
                };
                // The stored vector lists categories by score; the dominant one
                // is the highest score, ties broken by Category::ALL order.
                let mut best: Option<(Category, u64)> = None;
                if let Some(vector) = value.get("vector").and_then(|v| v.as_array()) {
                    for entry in vector {
                        let Some(category) = entry
                            .get("category")
                            .and_then(|c| c.as_str())
                            .and_then(category_from_label)
                        else {
                            continue;
                        };
                        let score = entry.get("score").and_then(|s| s.as_u64()).unwrap_or(0);
                        let rank = |c: Category| Category::ALL.iter().position(|x| *x == c);
                        let better = match best {
                            None => score > 0,
                            Some((current, top)) => {
                                score > top || (score == top && rank(category) < rank(current))
                            }
                        };
                        if better {
                            best = Some((category, score));
                        }
                    }
                }
                FeedItem {
                    source,
                    reference,
                    at_ms,
                    weight: salience.min(100) as u8,
                    category: best.map(|(c, _)| c),
                }
            }
            FeedSource::VaultDocument => {
                let Ok(content) =
                    serde_json::from_value::<crate::vault_bridge::DocumentContent>(value.clone())
                else {
                    continue;
                };
                if content.format != crate::vault_bridge::DOCUMENT_FORMAT {
                    continue;
                }
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight: content.kind.weight(),
                    category: None,
                }
            }
            FeedSource::IrisEpisode => {
                let Ok(content) =
                    serde_json::from_value::<crate::iris_bridge::IrisEpisodeContent>(value.clone())
                else {
                    continue;
                };
                if content.format != crate::iris_bridge::EPISODE_FORMAT {
                    continue;
                }
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight: content.weight(),
                    category: None,
                }
            }
            FeedSource::ForgeArtifact => {
                let Ok(content) = serde_json::from_value::<crate::forge_bridge::ForgeArtifactContent>(
                    value.clone(),
                ) else {
                    continue;
                };
                if content.format != crate::forge_bridge::ARTIFACT_FORMAT {
                    continue;
                }
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight: content.weight(),
                    category: None,
                }
            }
            FeedSource::WandaChat => {
                let Some(count) = value.get("message_count").and_then(|v| v.as_u64()) else {
                    continue;
                };
                FeedItem {
                    source: kind,
                    reference,
                    at_ms,
                    weight: count.saturating_mul(10).min(100) as u8,
                    category: None,
                }
            }
        };
        items.push(item);
    }
    items
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// Why an item did not reach Cerebra under this profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// The profile has no rule for this source.
    NotInProfile,
    /// Weight below the rule's threshold.
    BelowWeight,
    /// The message's dominant category is not one the rule admits.
    Category,
    /// The reference belongs to another domain.
    OtherDomain,
    /// Admitted, but the profile's budget was already full of higher priorities.
    Budget,
    /// The same reference appeared twice; the first one stands.
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    pub reference: VaultRef,
    pub source: FeedSource,
    pub priority: u8,
    pub weight: u8,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub reference: VaultRef,
    pub source: FeedSource,
    pub reason: DropReason,
}

/// One profile's decision over a set of items: the context Cerebra will see,
/// in priority order, and everything it will not, with the reason.
#[derive(Debug, Clone)]
pub struct FeedPlan {
    profile: String,
    context: PillarContext,
    admitted: Vec<Admitted>,
    dropped: Vec<Dropped>,
}

impl FeedPlan {
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn context(&self) -> &PillarContext {
        &self.context
    }
    /// Highest priority first.
    pub fn admitted(&self) -> &[Admitted] {
        &self.admitted
    }
    pub fn dropped(&self) -> &[Dropped] {
        &self.dropped
    }
    /// The pass observation: graded restricted by the pillar context, stamped
    /// with the newest admitted item's own time. `None` when nothing was admitted.
    pub fn observation(&self) -> Option<Observation> {
        if self.admitted.is_empty() {
            return None;
        }
        let at_ms = self.admitted.iter().map(|a| a.at_ms).max().unwrap_or(0);
        Some(self.context.observation(
            Trigger::SystemEvent {
                source: format!("archie-feeder:{}", self.profile),
            },
            &format!("focus-{}", self.profile),
            at_ms,
        ))
    }
}

fn pillar_of(source: FeedSource) -> Pillar {
    match source {
        FeedSource::NetwatchAlert | FeedSource::Device => Pillar::Sentry,
        FeedSource::WandaMessage | FeedSource::WandaNotification | FeedSource::WandaChat => {
            Pillar::Wanda
        }
        FeedSource::VaultDocument => Pillar::Vault,
        FeedSource::IrisEpisode => Pillar::Iris,
        FeedSource::ForgeArtifact => Pillar::Forge,
    }
}

/// Decide, under `profile`, what of `items` enters one pillar context in
/// `domain`. Deterministic: the same items give the same plan.
pub fn plan(
    profile: &FocusProfile,
    domain: &ContextDomain,
    task_id: &str,
    items: &[FeedItem],
) -> Result<FeedPlan, FeederError> {
    profile.validate()?;
    if !profile.scope.accepts(domain) {
        return Err(FeederError::ProfileDomain);
    }
    if items.len() > MAX_FEED_ITEMS {
        return Err(FeederError::TooMany);
    }
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut dropped = Vec::new();
    for item in items {
        let drop = |reason| Dropped {
            reference: item.reference.clone(),
            source: item.source,
            reason,
        };
        if !seen.insert(item.reference.clone()) {
            dropped.push(drop(DropReason::Duplicate));
            continue;
        }
        if !domain.contains(&item.reference) {
            dropped.push(drop(DropReason::OtherDomain));
            continue;
        }
        let Some(rule) = profile.rule(item.source) else {
            dropped.push(drop(DropReason::NotInProfile));
            continue;
        };
        if let Some(categories) = &rule.categories {
            if !item.category.is_some_and(|c| categories.contains(&c)) {
                dropped.push(drop(DropReason::Category));
                continue;
            }
        }
        if item.weight < rule.min_weight {
            dropped.push(drop(DropReason::BelowWeight));
            continue;
        }
        candidates.push(Admitted {
            reference: item.reference.clone(),
            source: item.source,
            priority: rule.priority,
            weight: item.weight,
            at_ms: item.at_ms,
        });
    }
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(b.weight.cmp(&a.weight))
            .then(b.at_ms.cmp(&a.at_ms))
            .then_with(|| a.reference.cmp(&b.reference))
    });
    let overflow = candidates.split_off(candidates.len().min(profile.max_items));
    dropped.extend(overflow.into_iter().map(|a| Dropped {
        reference: a.reference,
        source: a.source,
        reason: DropReason::Budget,
    }));
    for admitted in &candidates {
        context.include(pillar_of(admitted.source), admitted.reference.clone())?;
    }
    Ok(FeedPlan {
        profile: profile.name.clone(),
        context,
        admitted: candidates,
        dropped,
    })
}

/// What one focused pass came to.
pub enum FeedOutcome {
    /// Nothing passed the profile: no pass, no trace. Not an error.
    Nothing {
        plan: FeedPlan,
    },
    Reasoning {
        pass: Pass,
        plan: FeedPlan,
    },
    /// Refused, and the refusal is the recorded outcome (e.g. no local model
    /// may see restricted context).
    Refused {
        pass: Pass,
        refusal: PassRefusal,
        plan: FeedPlan,
    },
}

impl std::fmt::Debug for FeedOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing { plan } => write!(f, "FeedOutcome::Nothing({})", plan.profile),
            Self::Reasoning { pass, plan } => f
                .debug_struct("FeedOutcome::Reasoning")
                .field("profile", &plan.profile)
                .field("trace_id", &pass.trace.trace_id)
                .finish_non_exhaustive(),
            Self::Refused {
                pass,
                refusal,
                plan,
            } => f
                .debug_struct("FeedOutcome::Refused")
                .field("profile", &plan.profile)
                .field("trace_id", &pass.trace.trace_id)
                .field("refusal", refusal)
                .finish_non_exhaustive(),
        }
    }
}

/// One Cerebra pass over a plan.
pub fn consider_plan(
    cerebra: &Cerebra,
    vault: &mut Vault,
    models: &ModelRegistry,
    plan: FeedPlan,
    trace_id: TraceId,
    now_ms: i64,
) -> Result<FeedOutcome, FeederError> {
    let Some(observation) = plan.observation() else {
        return Ok(FeedOutcome::Nothing { plan });
    };
    let pass = cerebra.consider(vault, models, &observation, trace_id, now_ms)?;
    Ok(match &pass.intent {
        Intent::Reason { .. } => FeedOutcome::Reasoning { pass, plan },
        Intent::StaySilent { reason } => FeedOutcome::Refused {
            refusal: classify(reason),
            pass,
            plan,
        },
    })
}
