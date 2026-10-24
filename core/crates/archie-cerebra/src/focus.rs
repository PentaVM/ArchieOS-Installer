//! Focus profiles: what one aspect of attention admits, at what priority, and
//! what it drops.
//!
//! A profile never widens anything. It cannot cross domains — a work profile
//! only ever sees `work/…` references, a personal profile only `personal/…` —
//! and it cannot lower a grade: everything the feeder hands Cerebra stays
//! `restricted`, as the producers sealed it. A profile is only a filter and an
//! order over references that already exist.
use crate::wanda_bridge::Category;
use archie_vault::ContextDomain;
use std::collections::BTreeSet;

/// Where a feed item came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeedSource {
    /// An `archie-netwatch` alert recorded by [`crate::sentry_bridge`].
    NetwatchAlert,
    /// A device change recorded by [`crate::feeder::record_device_events`].
    Device,
    /// One chat message from the owner's own exports, via [`crate::wanda_bridge`].
    WandaMessage,
    /// One notification from the owner's own devices, via [`crate::wanda_bridge`].
    WandaNotification,
    /// One chat's aggregate (counts and digests, never transcripts).
    WandaChat,
    /// A document the owner put under Archie's care, via [`crate::vault_bridge`].
    VaultDocument,
    /// A bounded visual interaction episode receipt, via [`crate::iris_bridge`].
    IrisEpisode,
    /// A build, test, patch or review receipt, via [`crate::forge_bridge`].
    ForgeArtifact,
}

impl FeedSource {
    pub const ALL: [FeedSource; 8] = [
        Self::NetwatchAlert,
        Self::Device,
        Self::WandaMessage,
        Self::WandaNotification,
        Self::WandaChat,
        Self::VaultDocument,
        Self::IrisEpisode,
        Self::ForgeArtifact,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::NetwatchAlert => "netwatch-alert",
            Self::Device => "device",
            Self::WandaMessage => "wanda-message",
            Self::WandaNotification => "wanda-notification",
            Self::WandaChat => "wanda-chat",
            Self::VaultDocument => "vault-document",
            Self::IrisEpisode => "iris-episode",
            Self::ForgeArtifact => "forge-artifact",
        }
    }
}

/// Which domain a profile may be planned in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainScope {
    /// Only `personal/…` domains.
    Personal,
    /// Only `work/…` domains.
    Work,
    /// Either kind; the plan still holds exactly one domain.
    Either,
}

impl DomainScope {
    pub fn accepts(self, domain: &ContextDomain) -> bool {
        match self {
            Self::Personal => !domain.is_work(),
            Self::Work => domain.is_work(),
            Self::Either => true,
        }
    }
}

/// One source's admission rule inside a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusRule {
    pub source: FeedSource,
    /// Items below this weight (0..=100) are dropped.
    pub min_weight: u8,
    /// Higher goes first; ties break on weight, then recency.
    pub priority: u8,
    /// For Wanda messages and notifications: only these dominant categories.
    /// `None` admits every category, including messages with none.
    pub categories: Option<BTreeSet<Category>>,
}

impl FocusRule {
    pub fn new(source: FeedSource, min_weight: u8, priority: u8) -> Self {
        Self {
            source,
            min_weight: min_weight.min(100),
            priority,
            categories: None,
        }
    }
    pub fn only(mut self, categories: &[Category]) -> Self {
        self.categories = Some(categories.iter().copied().collect());
        self
    }
}

/// A concentration profile: a name, the domains it may run in, one rule per
/// admitted source and a budget. Sources without a rule are dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusProfile {
    pub name: String,
    pub scope: DomainScope,
    pub rules: Vec<FocusRule>,
    /// At most this many references reach Cerebra (≤ the pillar budget).
    pub max_items: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("unknown focus profile")]
    Unknown,
    #[error("a focus profile needs a bounded name, at least one rule, one rule per source and a budget within the pillar context")]
    Invalid,
}

impl FocusProfile {
    /// The built-in profiles: `pantalla-1`, `trabajo`, `personal`, `red`.
    pub const PRESETS: [&'static str; 4] = ["pantalla-1", "trabajo", "personal", "red"];

    pub fn preset(name: &str) -> Result<Self, ProfileError> {
        use FeedSource::*;
        let profile = match name {
            // What may surface on the main screen, which other people can see:
            // network problems worth interrupting for and device changes. No
            // chat or notification content, ever.
            "pantalla-1" => Self {
                name: name.into(),
                scope: DomainScope::Either,
                rules: vec![
                    FocusRule::new(NetwatchAlert, 60, 90),
                    FocusRule::new(Device, 40, 50),
                ],
                max_items: 8,
            },
            // Work: messages that read as work, questions, planning or urgency;
            // only high-severity network alerts interrupt.
            "trabajo" => Self {
                name: name.into(),
                scope: DomainScope::Work,
                rules: vec![
                    FocusRule::new(WandaMessage, 0, 70).only(&[
                        Category::Work,
                        Category::Urgency,
                        Category::Question,
                        Category::Planning,
                    ]),
                    FocusRule::new(WandaNotification, 10, 60)
                        .only(&[Category::Work, Category::Urgency]),
                    FocusRule::new(WandaChat, 0, 30),
                    FocusRule::new(NetwatchAlert, 90, 80),
                    // The work's own evidence: the brief and its restrictions first,
                    // followed by Forge receipts and IRIS episodes.
                    FocusRule::new(VaultDocument, 0, 85),
                    FocusRule::new(ForgeArtifact, 0, 75),
                    FocusRule::new(IrisEpisode, 0, 65),
                ],
                max_items: 24,
            },
            // Wanda salience is a low scale (one "urgente" scores about 12), so
            // notification thresholds sit at 10: below it is noise such as a
            // greeting. Messages are filtered by category, not by salience.
            // Personal: the owner's own chats and notifications first, then
            // high-severity alerts on the home network.
            "personal" => Self {
                name: name.into(),
                scope: DomainScope::Personal,
                rules: vec![
                    FocusRule::new(WandaMessage, 0, 70),
                    FocusRule::new(WandaNotification, 10, 60),
                    FocusRule::new(WandaChat, 0, 40),
                    FocusRule::new(NetwatchAlert, 90, 50),
                    FocusRule::new(VaultDocument, 0, 45),
                    FocusRule::new(IrisEpisode, 0, 35),
                ],
                max_items: 32,
            },
            // The network: every alert and every device change, nothing social.
            "red" => Self {
                name: name.into(),
                scope: DomainScope::Either,
                rules: vec![
                    FocusRule::new(NetwatchAlert, 0, 90),
                    FocusRule::new(Device, 0, 70),
                ],
                max_items: crate::pillars::MAX_CONTEXT_REFERENCES,
            },
            _ => return Err(ProfileError::Unknown),
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        let mut seen = BTreeSet::new();
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self.rules.is_empty()
            || self.max_items == 0
            || self.max_items > crate::pillars::MAX_CONTEXT_REFERENCES
            || !self.rules.iter().all(|r| seen.insert(r.source))
        {
            return Err(ProfileError::Invalid);
        }
        Ok(())
    }

    pub fn rule(&self, source: FeedSource) -> Option<&FocusRule> {
        self.rules.iter().find(|r| r.source == source)
    }
}
