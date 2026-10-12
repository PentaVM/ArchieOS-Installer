//! The bridge from the owner's own chats to Wanda, the social-interpretation pillar.
//!
//! Sibling of [`crate::sentry_bridge`]: where that bridge feeds netwatch alerts to the
//! Sentry pillar, this one turns chat exports and notification dumps into categorized
//! context for [`Pillar::Wanda`] — validated, deduplicated by content-derived
//! reference, sealed in the vault at [`PrivacyGrade::Restricted`], and referenced from
//! one [`PillarContext`]. The discipline is the same: source timestamps only, bounded
//! everything, typed refusals, no panics, and a `Debug` that never prints what people
//! said.
//!
//! # Sources supported today
//!
//! - **WhatsApp export `.txt`** (`parse_whatsapp_export`): the file WhatsApp itself
//!   writes from *Export chat*. Both timestamp layouts (`[dd/mm/yyyy, hh:mm:ss] Name:
//!   message` and `dd/mm/yyyy, hh:mm - Name: message`) are read; dates are read as
//!   day-first, the owner's locale. System lines (encryption notices, group events) are
//!   skipped and counted; `<Media omitted>` / `image omitted` markers become a flag, not
//!   text; continuation lines are joined into their message.
//! - **Instagram data-export `messages_*.json`** (`parse_instagram_export`): the
//!   per-conversation file from Instagram's data download. Plain `content`, `share`
//!   variants (link/share text), and media-only messages (photos/videos/audio) are
//!   handled. Mojibake note: IG exports are often latin1-misdecoded UTF-8 (`Ã©` for
//!   `é`). This crate has no repair helper, and repair is ambiguous, so text is kept as
//!   lossy UTF-8 exactly as decoded — the assumption is documented, not hidden.
//! - **Notification-dump JSONL** (`parse_notification_dump`): one JSON object per line
//!   (`{package, title, text, posted_at_ms}`), the shape an Android notification
//!   listener or an `adb shell dumpsys notification` exporter emits. Duplicates of
//!   `(package, posted_at_ms, text)` are folded and counted.
//!
//! # What is NOT here
//!
//! No live WhatsApp reading. The chats are end-to-end encrypted; the only honest
//! on-device paths are the export files and the notification stream, and those are the
//! paths above. Nothing contacts a cloud, nothing scrapes another device, and nothing
//! here runs an LLM — categorization is a static rule table. The semantic embedding of
//! the text comes later from IRIS/JEPA; the [`ContextVector`] this module produces is
//! the deterministic, inspectable interim that Wanda consumes until then.
//!
//! # Privacy stance
//!
//! This is the owner's own data, read on his own devices, stored locally at
//! [`PrivacyGrade::Restricted`] (`docs/PENDIENTES.md` §3–4). Raw phone numbers are
//! never stored: a sender that looks like a number becomes a salted local hash
//! ([`SenderIdentity::from_number`]). `sha2` is not a dependency of this crate, so the
//! hash is the same dual-FNV content id [`crate::sentry_bridge`] uses, keyed with the
//! salt — a pseudonym, not a defence: phone numbers are low-entropy, and anyone holding
//! the salt and the vault can brute-force them. The salt lives in local config, never
//! in a record. Raw text stays out of the vault below the salience threshold; at or
//! above it, a digest plus a bounded excerpt (≤280 chars) is stored instead of the
//! message.

use crate::pillars::{Pillar, PillarContext, PillarError, MAX_CONTEXT_REFERENCES};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, DomainError, EntryClass, Vault, VaultError, VaultRef};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// The most messages one batch may carry. Each message takes one pillar reference and
/// each distinct chat one more; 32 + 32 is the pillar context's budget.
pub const MAX_MESSAGES_PER_BATCH: usize = MAX_CONTEXT_REFERENCES / 2;

/// A message at or above this salience leaves a digest and a bounded excerpt in the
/// vault; below it, only metadata does.
pub const EXCERPT_SALIENCE_THRESHOLD: u8 = 70;

const MAX_CHAT_ID_CHARS: usize = 64;
const MAX_DISPLAY_NAME_CHARS: usize = 64;
const MAX_PACKAGE_CHARS: usize = 128;
const MAX_TEXT_CHARS: usize = 2000;
const MAX_SALT_BYTES: usize = 128;
const MAX_EXCERPT_CHARS: usize = 280;
const MAX_CHAT_SENDERS: usize = 16;
const MIN_NUMBER_DIGITS: usize = 6;
const MESSAGE_FORMAT: &str = "archie-wanda-message-v1";
const CHAT_FORMAT: &str = "archie-wanda-chat-v1";
const SENDER_HASH_FORMAT: &str = "wanda-sender-v1";
const ACTOR: &str = "wanda-bridge";

// ---------------------------------------------------------------------------
// Normalized input model
// ---------------------------------------------------------------------------

/// Where a message came from. `Notification` carries the Android package (or exporter
/// label) of the app that posted it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChatPlatform {
    Whatsapp,
    Instagram,
    Notification(String),
}

impl ChatPlatform {
    /// A notification source, validated: a package is a bounded label without control
    /// characters.
    pub fn notification(package: impl Into<String>) -> Result<Self, WandaError> {
        let package = package.into();
        if package.is_empty()
            || package.chars().count() > MAX_PACKAGE_CHARS
            || package.bytes().any(|b| b.is_ascii_control())
        {
            return Err(WandaError::InvalidMessage {
                field: "package",
                reason: "a package is a bounded label without control characters",
            });
        }
        Ok(Self::Notification(package))
    }

    /// The stable label stored in records.
    pub fn label(&self) -> &str {
        match self {
            Self::Whatsapp => "whatsapp",
            Self::Instagram => "instagram",
            Self::Notification(_) => "notification",
        }
    }

    fn package(&self) -> Option<&str> {
        match self {
            Self::Notification(package) => Some(package),
            _ => None,
        }
    }
}

/// Who sent a message. A display name is kept as-is; a phone number is only ever its
/// salted hash — the raw number is dropped at construction and there is no code path
/// that stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderIdentity {
    /// A display name, bounded and free of control characters.
    Display(String),
    /// The salted local hash of a phone number (32 hex chars). Not cryptography: see
    /// the module's privacy note.
    NumberHash(String),
}

impl SenderIdentity {
    /// A sender known by name.
    pub fn display(name: impl Into<String>) -> Result<Self, WandaError> {
        let name = strip_directional_marks(name.into().trim()).to_string();
        if name.is_empty()
            || name.chars().count() > MAX_DISPLAY_NAME_CHARS
            || name.bytes().any(|b| b.is_ascii_control())
        {
            return Err(WandaError::InvalidMessage {
                field: "sender",
                reason: "a display name is bounded text without control characters",
            });
        }
        Ok(Self::Display(name))
    }

    /// A sender known by number. The number is normalized to its digits, hashed with
    /// the salt, and forgotten; fewer than six digits is not a number.
    pub fn from_number(number: &str, salt: &str) -> Result<Self, WandaError> {
        validate_salt(salt)?;
        let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < MIN_NUMBER_DIGITS {
            return Err(WandaError::InvalidMessage {
                field: "sender",
                reason: "a number sender carries at least six digits",
            });
        }
        let mut material =
            Vec::with_capacity(SENDER_HASH_FORMAT.len() + salt.len() + digits.len() + 2);
        material.extend_from_slice(SENDER_HASH_FORMAT.as_bytes());
        material.push(0);
        material.extend_from_slice(salt.as_bytes());
        material.push(0);
        material.extend_from_slice(digits.as_bytes());
        Ok(Self::NumberHash(content_id(&material)))
    }

    /// A sender as an export wrote it: if it looks like a phone number (only digits,
    /// spaces and `+()-`, six digits or more) it is hashed; otherwise it is a display
    /// name.
    pub fn from_export(raw: &str, salt: &str) -> Result<Self, WandaError> {
        let looks_numeric = raw
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '+' | '(' | ')' | '-'))
            && raw.chars().filter(|c| c.is_ascii_digit()).count() >= MIN_NUMBER_DIGITS;
        if looks_numeric {
            Self::from_number(raw, salt)
        } else {
            Self::display(raw)
        }
    }

    /// The stored form: the name itself, or the hash. Never the raw number.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Display(name) => name,
            Self::NumberHash(hash) => hash,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Display(_) => "display",
            Self::NumberHash(_) => "number-hash",
        }
    }
}

fn validate_salt(salt: &str) -> Result<(), WandaError> {
    if salt.is_empty() || salt.len() > MAX_SALT_BYTES {
        return Err(WandaError::InvalidSalt);
    }
    Ok(())
}

/// One message, normalized. Construction is the validation boundary: fields are
/// private, the constructors check them, and what a constructor accepted is what the
/// vault can hold.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    platform: ChatPlatform,
    chat_id: String,
    sender: SenderIdentity,
    /// The source's own timestamp, milliseconds. `None` is the explicit missing
    /// marker; the bridge never substitutes its own clock.
    sent_at: Option<u64>,
    /// Normalized: directional marks dropped, control characters folded or removed,
    /// bounded to [`MAX_TEXT_CHARS`].
    text: String,
    media_omitted: bool,
}

impl ChatMessage {
    /// A text message. The text is normalized (control characters folded to spaces or
    /// dropped) and must fit the bound; an empty message is not a message — media-only
    /// messages use [`ChatMessage::media`].
    pub fn new(
        platform: ChatPlatform,
        chat_id: impl Into<String>,
        sender: SenderIdentity,
        sent_at: Option<u64>,
        text: impl Into<String>,
    ) -> Result<Self, WandaError> {
        let text = normalize_text(&text.into());
        if text.is_empty() {
            return Err(WandaError::InvalidMessage {
                field: "text",
                reason: "an empty message is a media message or nothing",
            });
        }
        Self::checked(platform, chat_id.into(), sender, sent_at, text, false)
    }

    /// A media-only message (`<Media omitted>` and friends): the marker becomes this
    /// flag and the text stays empty.
    pub fn media(
        platform: ChatPlatform,
        chat_id: impl Into<String>,
        sender: SenderIdentity,
        sent_at: Option<u64>,
    ) -> Result<Self, WandaError> {
        Self::checked(
            platform,
            chat_id.into(),
            sender,
            sent_at,
            String::new(),
            true,
        )
    }

    fn checked(
        platform: ChatPlatform,
        chat_id: String,
        sender: SenderIdentity,
        sent_at: Option<u64>,
        text: String,
        media_omitted: bool,
    ) -> Result<Self, WandaError> {
        let invalid = |field: &'static str, reason: &'static str| WandaError::InvalidMessage {
            field,
            reason,
        };
        if chat_id.is_empty()
            || chat_id.chars().count() > MAX_CHAT_ID_CHARS
            || chat_id.bytes().any(|b| b.is_ascii_control())
        {
            return Err(invalid(
                "chat_id",
                "a chat id is bounded text without control characters",
            ));
        }
        match sent_at {
            Some(0) => {
                return Err(invalid(
                    "sent_at",
                    "a missing source time is the explicit None marker",
                ))
            }
            Some(at) if i64::try_from(at).is_err() => {
                return Err(invalid(
                    "sent_at",
                    "the source time does not fit the trace's clock",
                ))
            }
            _ => {}
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(invalid("text", "a message is bounded to 2000 characters"));
        }
        Ok(Self {
            platform,
            chat_id,
            sender,
            sent_at,
            text,
            media_omitted,
        })
    }

    pub fn platform(&self) -> &ChatPlatform {
        &self.platform
    }
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }
    pub fn sender(&self) -> &SenderIdentity {
        &self.sender
    }
    /// The source's own time; `None` means the source did not say.
    pub fn sent_at(&self) -> Option<u64> {
        self.sent_at
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn media_omitted(&self) -> bool {
        self.media_omitted
    }
}

/// WhatsApp exports sprinkle U+200E/U+200F around timestamps and markers.
fn strip_directional_marks(text: &str) -> &str {
    text.trim_matches(|c| c == '\u{200e}' || c == '\u{200f}')
}

/// The normalization every message text passes through: directional marks dropped,
/// newlines and tabs folded to single spaces, other control characters removed.
fn normalize_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for c in raw.chars() {
        match c {
            '\u{200e}' | '\u{200f}' => {}
            '\n' | '\r' | '\t' => pending_space = true,
            c if c.is_control() => {}
            c => {
                if pending_space && !out.is_empty() && c != ' ' {
                    out.push(' ');
                }
                pending_space = false;
                out.push(c);
            }
        }
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Categorizer — a static rule table, deterministic, no model in this crate
// ---------------------------------------------------------------------------

/// The fixed dimensions of a [`ContextVector`]. Order is the tie-break priority of
/// [`ContextVector::dominant`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Social,
    Urgency,
    Work,
    Personal,
    Question,
    Planning,
    Finance,
    Family,
    Greeting,
    News,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Self::Social,
        Self::Urgency,
        Self::Work,
        Self::Personal,
        Self::Question,
        Self::Planning,
        Self::Finance,
        Self::Family,
        Self::Greeting,
        Self::News,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Social => "social",
            Self::Urgency => "urgency",
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Question => "question",
            Self::Planning => "planning",
            Self::Finance => "finance",
            Self::Family => "family",
            Self::Greeting => "greeting",
            Self::News => "news",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|c| *c == self)
            .expect("ALL holds every category")
    }
}

/// One keyword rule. A keyword with a space matches as a substring of the lowercased
/// text; a bare word matches a whole alphanumeric token, so `mom` does not fire on
/// `momento`. Tuning the categorizer is editing this table — it is data.
struct Rule {
    category: Category,
    keyword: &'static str,
    weight: u8,
}

const RULES: &[Rule] = &[
    // Social
    Rule {
        category: Category::Social,
        keyword: "jaja",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "haha",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "lol",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "fiesta",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "party",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "salimos",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "cervezas",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "beers",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "cumpleaños",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "birthday",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "amigo",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "amiga",
        weight: 40,
    },
    Rule {
        category: Category::Social,
        keyword: "friend",
        weight: 40,
    },
    // Urgency
    Rule {
        category: Category::Urgency,
        keyword: "urgente",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "urgent",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "ahora mismo",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "right now",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "asap",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "emergencia",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "emergency",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "inmediatamente",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "immediately",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "cuanto antes",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "prisa",
        weight: 40,
    },
    Rule {
        category: Category::Urgency,
        keyword: "deadline",
        weight: 40,
    },
    // Work
    Rule {
        category: Category::Work,
        keyword: "trabajo",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "work",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "reunión",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "meeting",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "proyecto",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "project",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "cliente",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "client",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "oficina",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "office",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "informe",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "report",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "presentación",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "presentation",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "entrevista",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "interview",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "jefe",
        weight: 40,
    },
    Rule {
        category: Category::Work,
        keyword: "boss",
        weight: 40,
    },
    // Personal
    Rule {
        category: Category::Personal,
        keyword: "me siento",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "i feel",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "secreto",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "secret",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "entre tú y yo",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "between us",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "te cuento",
        weight: 40,
    },
    Rule {
        category: Category::Personal,
        keyword: "en confianza",
        weight: 40,
    },
    // Question (a question mark adds 50 more, see categorize)
    Rule {
        category: Category::Question,
        keyword: "qué",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "cómo",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "cuándo",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "dónde",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "quién",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "cuál",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "what",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "how",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "when",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "where",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "who",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "which",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "why",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "puedes",
        weight: 40,
    },
    Rule {
        category: Category::Question,
        keyword: "sabes",
        weight: 40,
    },
    // Planning
    Rule {
        category: Category::Planning,
        keyword: "mañana",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "tomorrow",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "plan",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "quedamos",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "nos vemos",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "see you",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "reserva",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "booking",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "cita",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "appointment",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "finde",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "weekend",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "viaje",
        weight: 40,
    },
    Rule {
        category: Category::Planning,
        keyword: "trip",
        weight: 40,
    },
    // Finance
    Rule {
        category: Category::Finance,
        keyword: "dinero",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "money",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "pago",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "payment",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "pagar",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "factura",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "invoice",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "banco",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "bank",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "precio",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "price",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "deuda",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "debt",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "transferencia",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "transfer",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "bizum",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "sueldo",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "salary",
        weight: 40,
    },
    Rule {
        category: Category::Finance,
        keyword: "alquiler",
        weight: 40,
    },
    // Family
    Rule {
        category: Category::Family,
        keyword: "mamá",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "mom",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "papá",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "dad",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "madre",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "padre",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "hermano",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "hermana",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "brother",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "sister",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "abuela",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "abuelo",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "grandma",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "grandpa",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "tío",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "tía",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "uncle",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "aunt",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "hijo",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "hija",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "familia",
        weight: 40,
    },
    Rule {
        category: Category::Family,
        keyword: "family",
        weight: 40,
    },
    // Greeting
    Rule {
        category: Category::Greeting,
        keyword: "hola",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "hello",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "hey",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "buenos días",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "good morning",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "buenas tardes",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "good afternoon",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "buenas noches",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "good night",
        weight: 40,
    },
    Rule {
        category: Category::Greeting,
        keyword: "qué tal",
        weight: 40,
    },
    // News
    Rule {
        category: Category::News,
        keyword: "noticias",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "news",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "artículo",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "article",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "vídeo",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "video",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "viral",
        weight: 40,
    },
    Rule {
        category: Category::News,
        keyword: "viste",
        weight: 40,
    },
];

/// The categorized shape of one message: ten integer scores in `0..=100`, a salience
/// in `0..=100`, and the dominant category. This is what Wanda consumes; the semantic
/// embedding comes later from IRIS/JEPA and does not replace this vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextVector {
    scores: [u8; 10],
    salience: u8,
}

impl ContextVector {
    /// Score one message. Pure and deterministic: same text, same vector.
    pub fn categorize(text: &str, media_omitted: bool) -> Self {
        let lower = text.to_lowercase();
        let tokens: HashSet<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        let mut scores = [0u32; 10];
        for rule in RULES {
            let hit = if rule.keyword.contains(' ') {
                lower.contains(rule.keyword)
            } else {
                tokens.contains(rule.keyword)
            };
            if hit {
                scores[rule.category.index()] += u32::from(rule.weight);
            }
        }
        if lower.contains('?') || lower.contains('¿') {
            scores[Category::Question.index()] += 50;
        }
        let mut clamped = [0u8; 10];
        for (i, score) in scores.iter().enumerate() {
            clamped[i] = (*score).min(100) as u8;
        }
        // Salience heuristic: urgency and questions dominate, finance and personal
        // follow, planning counts a little, length a token amount. All integer; the
        // weights live here so they can be tuned in one place.
        let weighted = (30 * u32::from(clamped[Category::Urgency.index()])
            + 25 * u32::from(clamped[Category::Question.index()])
            + 20 * u32::from(clamped[Category::Finance.index()])
            + 15 * u32::from(clamped[Category::Personal.index()])
            + 10 * u32::from(clamped[Category::Planning.index()]))
            / 100;
        let length_bonus = u32::try_from(text.chars().count() / 100)
            .unwrap_or(10)
            .min(10);
        let mut salience = (weighted + length_bonus).min(100);
        if media_omitted {
            salience /= 2;
        }
        Self {
            scores: clamped,
            salience: salience as u8,
        }
    }

    pub fn score(&self, category: Category) -> u8 {
        self.scores[category.index()]
    }

    pub fn salience(&self) -> u8 {
        self.salience
    }

    /// The highest-scoring category and its score; ties resolve by [`Category::ALL`]
    /// order, and a message with no signal at all has no dominant category.
    pub fn dominant(&self) -> Option<(Category, u8)> {
        let mut best: Option<(Category, u8)> = None;
        for category in Category::ALL {
            let score = self.score(category);
            if score > 0 && best.is_none_or(|(_, top)| score > top) {
                best = Some((category, score));
            }
        }
        best
    }
}

#[derive(Serialize)]
struct VectorEntry<'a> {
    category: &'a str,
    score: u8,
}

impl ContextVector {
    fn entries(&self) -> Vec<VectorEntry<'static>> {
        Category::ALL
            .iter()
            .map(|c| VectorEntry {
                category: c.label(),
                score: self.score(*c),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Parsers — pure functions, typed errors, no panics
// ---------------------------------------------------------------------------

/// What a parser made of a file. Malformed lines and records are counted and skipped;
/// a structurally unreadable file is a [`ParseError`].
#[derive(Debug, Default)]
pub struct ParseOutcome {
    pub messages: Vec<ChatMessage>,
    /// System lines (encryption notices, group events) skipped on purpose.
    pub system_lines: usize,
    /// Messages whose content was a media marker.
    pub media_omitted: usize,
    /// Lines or records that could not be read and were skipped.
    pub malformed: usize,
    /// Duplicate notifications folded by (package, posted_at_ms, text).
    pub duplicates: usize,
}

/// Why a file could not be parsed at all. Malformed pieces inside a readable file are
/// counts on [`ParseOutcome`], not errors.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the file held no parseable messages ({malformed} malformed, {system} system lines)")]
    NoMessages { malformed: usize, system: usize },
    #[error("the export is not readable as its format: {reason}")]
    MalformedDocument { reason: &'static str },
}

fn finish(outcome: ParseOutcome) -> Result<ParseOutcome, ParseError> {
    if outcome.messages.is_empty() {
        return Err(ParseError::NoMessages {
            malformed: outcome.malformed,
            system: outcome.system_lines,
        });
    }
    Ok(outcome)
}

/// Howard Hinnant's days-from-civil: a date to days since the epoch, with no clock and
/// no dependency. Validity is checked by the caller.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `dd/mm/yyyy` (day-first, the owner's locale; two-digit years are 2000s) plus
/// `hh:mm[:ss]` to epoch milliseconds.
fn parse_timestamp(date: &str, time: &str) -> Option<u64> {
    let date: Vec<&str> = date.split('/').collect();
    let [day, month, year] = <[&str; 3]>::try_from(date.as_slice()).ok()?;
    let day: u32 = day.parse().ok()?;
    let month: u32 = month.parse().ok()?;
    let mut year: i64 = year.parse().ok()?;
    if (0..100).contains(&year) {
        year += 2000;
    }
    if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let time: Vec<&str> = time.split(':').collect();
    if time.len() < 2 || time.len() > 3 {
        return None;
    }
    let hour: u64 = time[0].parse().ok()?;
    let minute: u64 = time[1].parse().ok()?;
    let second: u64 = if time.len() == 3 {
        time[2].parse().ok()?
    } else {
        0
    };
    if hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let ms = days.checked_mul(86_400_000)?
        + (hour * 3_600_000 + minute * 60_000 + second * 1_000) as i64;
    u64::try_from(ms).ok()
}

/// The known shapes of WhatsApp system lines; matched against the sender candidate, so
/// `Alice: I added sugar` stays a message while `Alice added Bob` is counted a system
/// line.
const WHATSAPP_SYSTEM_MARKERS: &[&str] = &[
    "end-to-end encrypted",
    "cifrado de extremo a extremo",
    "created group",
    "creó el grupo",
    "changed the subject",
    "cambió el asunto",
    "changed this group's icon",
    "security code changed",
    "código de seguridad",
    " added ",
    " removed ",
    " left",
    "joined using",
    "pinned a message",
    "this message was deleted",
    "eliminó este mensaje",
    "disappearing messages",
    "mensajes temporales",
];

const WHATSAPP_MEDIA_MARKERS: &[&str] = &[
    "<media omitted>",
    "<multimedia omitido>",
    "image omitted",
    "video omitted",
    "audio omitted",
    "sticker omitted",
    "gif omitted",
    "document omitted",
    "contact card omitted",
];

/// One new-message header: timestamp and the remainder after it.
fn whatsapp_header(line: &str) -> Option<(u64, &str)> {
    let line = strip_directional_marks(line.trim_start());
    // Layout one: `[dd/mm/yyyy, hh:mm:ss] rest`.
    if let Some(body) = line.strip_prefix('[') {
        let (head, rest) = body.split_once("] ")?;
        let (date, time) = head.split_once(", ")?;
        return parse_timestamp(date, time).map(|at| (at, rest));
    }
    // Layout two: `dd/mm/yyyy, hh:mm - rest`.
    let (head, rest) = line.split_once(" - ")?;
    let (date, time) = head.split_once(", ")?;
    parse_timestamp(date, time).map(|at| (at, rest))
}

/// Parse a WhatsApp export `.txt`. `chat_id` names the conversation (the export file
/// does not); the salt keys sender hashes. Day-first dates, system lines skipped and
/// counted, media markers flagged, continuation lines joined.
pub fn parse_whatsapp_export(
    chat_id: &str,
    text: &str,
    salt: &str,
) -> Result<ParseOutcome, ParseError> {
    let mut outcome = ParseOutcome::default();
    // A message in progress: sender, time, media flag, accumulated body lines.
    let mut pending: Option<(SenderIdentity, u64, bool, Vec<&str>)> = None;
    let flush = |outcome: &mut ParseOutcome,
                 pending: &mut Option<(SenderIdentity, u64, bool, Vec<&str>)>| {
        let Some((sender, at, is_media, lines)) = pending.take() else {
            return;
        };
        let body = lines.join("\n");
        let message = if is_media {
            ChatMessage::media(ChatPlatform::Whatsapp, chat_id, sender, Some(at))
        } else {
            ChatMessage::new(ChatPlatform::Whatsapp, chat_id, sender, Some(at), body)
        };
        match message {
            Ok(message) => {
                if message.media_omitted() {
                    outcome.media_omitted += 1;
                }
                outcome.messages.push(message);
            }
            // A message that survives the export but not the bounds (overlong, bad
            // chat id) is malformed input, counted — never a panic.
            Err(_) => outcome.malformed += 1,
        }
    };
    for line in text.trim_start_matches('\u{feff}').lines() {
        if let Some((at, rest)) = whatsapp_header(line) {
            flush(&mut outcome, &mut pending);
            let rest = strip_directional_marks(rest);
            let sender_ok = match rest.split_once(": ") {
                Some((sender, _)) => {
                    let lower = sender.to_lowercase();
                    !sender.is_empty()
                        && sender.chars().count() <= MAX_DISPLAY_NAME_CHARS
                        && !WHATSAPP_SYSTEM_MARKERS.iter().any(|m| lower.contains(m))
                }
                None => false,
            };
            if !sender_ok {
                outcome.system_lines += 1;
                continue;
            }
            let (sender, body) = rest.split_once(": ").expect("sender_ok implies a colon");
            let body = strip_directional_marks(body);
            let is_media = WHATSAPP_MEDIA_MARKERS.contains(&body.to_lowercase().as_str());
            match SenderIdentity::from_export(sender, salt) {
                Ok(sender) => pending = Some((sender, at, is_media, vec![body])),
                Err(_) => outcome.malformed += 1,
            }
        } else if line.trim().is_empty() {
            // Blank lines carry nothing and are not counted against the file.
        } else if let Some((_, _, _, lines)) = pending.as_mut() {
            lines.push(line);
        } else {
            outcome.malformed += 1;
        }
    }
    flush(&mut outcome, &mut pending);
    finish(outcome)
}

/// Parse one Instagram data-export `messages_*.json`. The chat id is derived from the
/// participant list, salted, so re-importing the same conversation lands on the same
/// chat. Mojibake is kept as decoded (see the module note) — this parser assumes the
/// text is already lossy UTF-8.
pub fn parse_instagram_export(json: &str, salt: &str) -> Result<ParseOutcome, ParseError> {
    validate_salt(salt).map_err(|_| ParseError::MalformedDocument {
        reason: "the salt is missing or oversized",
    })?;
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|_| ParseError::MalformedDocument {
            reason: "not valid JSON",
        })?;
    let messages =
        root.get("messages")
            .and_then(|m| m.as_array())
            .ok_or(ParseError::MalformedDocument {
                reason: "no messages array",
            })?;
    let mut participants: Vec<String> = root
        .get("participants")
        .and_then(|p| p.as_array())
        .map(|p| {
            p.iter()
                .filter_map(|one| one.get("name")?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    participants.sort();
    let mut chat_material = b"wanda-ig-chat-v1\0".to_vec();
    chat_material.extend_from_slice(salt.as_bytes());
    for name in &participants {
        chat_material.push(0);
        chat_material.extend_from_slice(name.as_bytes());
    }
    let chat_id = format!("ig-{}", &content_id(&chat_material)[..16]);

    let mut outcome = ParseOutcome::default();
    for entry in messages {
        let parsed = (|| {
            let sender = entry.get("sender_name")?.as_str()?;
            let at = entry.get("timestamp_ms")?.as_u64()?;
            let content = entry.get("content").and_then(|c| c.as_str());
            let share_text = entry.get("share").and_then(|s| {
                s.get("share_text")
                    .and_then(|t| t.as_str())
                    .or_else(|| s.get("link").and_then(|l| l.as_str()))
            });
            let has_media = entry.get("share").is_some() && share_text.is_none()
                || ["photos", "videos", "audio_files", "gifs"]
                    .iter()
                    .any(|key| {
                        entry
                            .get(key)
                            .and_then(|m| m.as_array())
                            .is_some_and(|a| !a.is_empty())
                    });
            let sender = SenderIdentity::display(sender).ok()?;
            let sent_at = if at == 0 { None } else { Some(at) };
            if let Some(text) = content.or(share_text) {
                ChatMessage::new(
                    ChatPlatform::Instagram,
                    chat_id.clone(),
                    sender,
                    sent_at,
                    text,
                )
                .ok()
            } else if has_media {
                ChatMessage::media(ChatPlatform::Instagram, chat_id.clone(), sender, sent_at).ok()
            } else {
                None
            }
        })();
        match parsed {
            Some(message) => {
                if message.media_omitted() {
                    outcome.media_omitted += 1;
                }
                outcome.messages.push(message);
            }
            None => outcome.malformed += 1,
        }
    }
    finish(outcome)
}

/// Parse a notification dump: one JSON object per line, `{package, title, text,
/// posted_at_ms}`, the shape an Android notification listener or an `adb shell dumpsys
/// notification` exporter emits. The package is the chat id; the title is the sender
/// (hashed when it looks like a number). Exact duplicates of `(package, posted_at_ms,
/// text)` are folded and counted.
pub fn parse_notification_dump(jsonl: &str, salt: &str) -> Result<ParseOutcome, ParseError> {
    let mut outcome = ParseOutcome::default();
    let mut seen: HashSet<(String, u64, String)> = HashSet::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed = (|| {
            let entry: serde_json::Value = serde_json::from_str(line).ok()?;
            let package = entry.get("package")?.as_str()?;
            let at = entry.get("posted_at_ms")?.as_u64()?;
            let text = entry.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if text.trim().is_empty() {
                return None;
            }
            let title = entry
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let key = (package.to_string(), at, content_id(text.as_bytes()));
            if !seen.insert(key) {
                return Some(None); // a duplicate, not malformed
            }
            let sender = SenderIdentity::from_export(title, salt).ok()?;
            let platform = ChatPlatform::notification(package).ok()?;
            let sent_at = if at == 0 { None } else { Some(at) };
            ChatMessage::new(platform, package, sender, sent_at, text)
                .ok()
                .map(Some)
        })();
        match parsed {
            Some(Some(message)) => outcome.messages.push(message),
            Some(None) => outcome.duplicates += 1,
            None => outcome.malformed += 1,
        }
    }
    finish(outcome)
}

// ---------------------------------------------------------------------------
// The bridge: validate → dedupe → vault store → PillarContext include
// ---------------------------------------------------------------------------

/// Why a batch, a message or a hash was refused. Input arrives over files and wires;
/// nothing here panics on it.
#[derive(Debug, thiserror::Error)]
pub enum WandaError {
    #[error("a message batch holds {found} messages; the most one batch may carry is {cap}")]
    BatchTooLarge { found: usize, cap: usize },
    #[error("invalid message: {field}: {reason}")]
    InvalidMessage {
        field: &'static str,
        reason: &'static str,
    },
    #[error("the sender-hash salt is missing or oversized")]
    InvalidSalt,
    #[error("a different record already holds {reference}")]
    ReferenceCollision { reference: VaultRef },
    #[error("pillar context refused the message: {0}")]
    Pillar(#[from] PillarError),
    #[error("the message's domain could not hold its reference: {0}")]
    Domain(#[from] DomainError),
    #[error("the vault refused the message record: {0}")]
    Vault(#[from] VaultError),
}

/// FNV-1a over the content, twice from different bases — the same content id
/// [`crate::sentry_bridge`] uses. Not cryptography: a dedup key and a path component,
/// stable across compiler versions, and a collision is caught by comparing the full
/// stored content rather than trusted.
fn content_id(bytes: &[u8]) -> String {
    const fn fnv1a(bytes: &[u8], basis: u64) -> u64 {
        let mut hash = basis;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            index += 1;
        }
        hash
    }
    let first = fnv1a(bytes, 0xcbf2_9ce4_8422_2325);
    let second = fnv1a(bytes, 0x9e37_79b9_7f4a_7c15);
    format!("{first:016x}{second:016x}")
}

/// One validated message, stored in the vault and referenced from the pillar context.
///
/// `Debug` shows identifiers and scores only: the excerpt is the owner's business, not
/// a log line's.
pub struct MessageRecord {
    reference: VaultRef,
    platform: ChatPlatform,
    chat_id: String,
    sender: SenderIdentity,
    sent_at: Option<u64>,
    vector: ContextVector,
    media_omitted: bool,
    excerpt: Option<String>,
    content: String,
}

impl std::fmt::Debug for MessageRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageRecord")
            .field("reference", &self.reference)
            .field("platform", &self.platform.label())
            .field("chat_id", &self.chat_id)
            .field("sender_kind", &self.sender.kind())
            .field("sent_at", &self.sent_at)
            .field("salience", &self.vector.salience())
            .field("media_omitted", &self.media_omitted)
            .finish_non_exhaustive()
    }
}

impl MessageRecord {
    pub fn reference(&self) -> &VaultRef {
        &self.reference
    }
    pub fn platform(&self) -> &ChatPlatform {
        &self.platform
    }
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }
    /// The stored sender form: a display name or a number hash, never a raw number.
    pub fn sender(&self) -> &SenderIdentity {
        &self.sender
    }
    /// The message's own time, never the bridge's. `None` is the explicit missing
    /// marker.
    pub fn sent_at(&self) -> Option<u64> {
        self.sent_at
    }
    pub fn vector(&self) -> &ContextVector {
        &self.vector
    }
    pub fn media_omitted(&self) -> bool {
        self.media_omitted
    }
    /// The bounded excerpt, present only at or above [`EXCERPT_SALIENCE_THRESHOLD`].
    pub fn excerpt(&self) -> Option<&str> {
        self.excerpt.as_deref()
    }
    /// The stored record, verbatim. Metadata, scores and at most one bounded excerpt.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// One chat's aggregate over the batch: counts, the dominant vector, and the digests
/// of its messages — never transcripts.
pub struct ChatRecord {
    reference: VaultRef,
    platform: ChatPlatform,
    chat_id: String,
    message_count: usize,
    content: String,
}

impl std::fmt::Debug for ChatRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatRecord")
            .field("reference", &self.reference)
            .field("platform", &self.platform.label())
            .field("chat_id", &self.chat_id)
            .field("message_count", &self.message_count)
            .finish_non_exhaustive()
    }
}

impl ChatRecord {
    pub fn reference(&self) -> &VaultRef {
        &self.reference
    }
    pub fn platform(&self) -> &ChatPlatform {
        &self.platform
    }
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }
    pub fn message_count(&self) -> usize {
        self.message_count
    }
    pub fn content(&self) -> &str {
        &self.content
    }
}

#[derive(Serialize)]
struct MessageContent<'a> {
    format: &'static str,
    platform: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<&'a str>,
    chat_id: &'a str,
    sender: &'a str,
    sender_kind: &'static str,
    // No skip: `null` is the explicit missing marker, recorded as such.
    sent_at: Option<u64>,
    media_omitted: bool,
    text_len: usize,
    digest: String,
    vector: Vec<VectorEntry<'a>>,
    salience: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    excerpt: Option<&'a str>,
}

#[derive(Serialize)]
struct ChatContent<'a> {
    format: &'static str,
    platform: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<&'a str>,
    chat_id: &'a str,
    message_count: usize,
    media_omitted_count: usize,
    first_sent_at: Option<u64>,
    last_sent_at: Option<u64>,
    senders: Vec<&'a str>,
    vector: Vec<VectorEntry<'a>>,
    salience_mean: u8,
    salience_max: u8,
    high_salience_count: usize,
    message_digests: Vec<&'a str>,
}

fn encode<T: Serialize>(value: &T) -> Result<String, WandaError> {
    serde_json::to_string(value).map_err(|_| WandaError::InvalidMessage {
        field: "content",
        reason: "the record could not be encoded",
    })
}

/// Shape one message into the record the vault will hold. Pure: nothing is stored
/// here, so a bad batch leaves the vault untouched.
fn message_record(
    domain: &ContextDomain,
    message: &ChatMessage,
) -> Result<MessageRecord, WandaError> {
    let vector = ContextVector::categorize(&message.text, message.media_omitted);
    let excerpt = if vector.salience() >= EXCERPT_SALIENCE_THRESHOLD && !message.media_omitted {
        Some(
            message
                .text
                .chars()
                .take(MAX_EXCERPT_CHARS)
                .collect::<String>(),
        )
    } else {
        None
    };
    let content = MessageContent {
        format: MESSAGE_FORMAT,
        platform: message.platform.label(),
        package: message.platform.package(),
        chat_id: &message.chat_id,
        sender: message.sender.as_str(),
        sender_kind: message.sender.kind(),
        sent_at: message.sent_at,
        media_omitted: message.media_omitted,
        text_len: message.text.chars().count(),
        digest: content_id(message.text.as_bytes()),
        vector: vector.entries(),
        salience: vector.salience(),
        excerpt: excerpt.as_deref(),
    };
    let content = encode(&content)?;
    let reference = domain.reference(&format!(
        "wanda/messages/{}",
        content_id(content.as_bytes())
    ))?;
    Ok(MessageRecord {
        reference,
        platform: message.platform.clone(),
        chat_id: message.chat_id.clone(),
        sender: message.sender.clone(),
        sent_at: message.sent_at,
        vector,
        media_omitted: message.media_omitted,
        excerpt,
        content,
    })
}

/// Aggregate one chat's message records into the chat record. Pure, like
/// [`message_record`]; the digests name the messages rather than quoting them.
fn chat_record(
    domain: &ContextDomain,
    platform: &ChatPlatform,
    chat_id: &str,
    records: &[&MessageRecord],
) -> Result<ChatRecord, WandaError> {
    let message_count = records.len();
    let count = message_count as u32;
    let mut sums = [0u32; 10];
    let mut salience_sum = 0u32;
    let mut salience_max = 0u8;
    let mut high = 0usize;
    let mut media = 0usize;
    let mut senders: Vec<&str> = Vec::new();
    let mut times: Vec<u64> = Vec::new();
    for record in records {
        for category in Category::ALL {
            sums[category.index()] += u32::from(record.vector.score(category));
        }
        salience_sum += u32::from(record.vector.salience());
        salience_max = salience_max.max(record.vector.salience());
        if record.vector.salience() >= EXCERPT_SALIENCE_THRESHOLD {
            high += 1;
        }
        if record.media_omitted {
            media += 1;
        }
        senders.push(record.sender.as_str());
        if let Some(at) = record.sent_at {
            times.push(at);
        }
    }
    senders.sort_unstable();
    senders.dedup();
    senders.truncate(MAX_CHAT_SENDERS);
    times.sort_unstable();
    let mean_scores: Vec<VectorEntry<'static>> = Category::ALL
        .iter()
        .map(|c| VectorEntry {
            category: c.label(),
            score: (sums[c.index()] / count).min(100) as u8,
        })
        .collect();
    let digests: Vec<String> = records.iter().map(|record| content_id_of(record)).collect();
    let content = ChatContent {
        format: CHAT_FORMAT,
        platform: platform.label(),
        package: platform.package(),
        chat_id,
        message_count,
        media_omitted_count: media,
        first_sent_at: times.first().copied(),
        last_sent_at: times.last().copied(),
        senders,
        vector: mean_scores,
        salience_mean: (salience_sum / count).min(100) as u8,
        salience_max,
        high_salience_count: high,
        message_digests: digests.iter().map(String::as_str).collect(),
    };
    let content = encode(&content)?;
    let reference = domain.reference(&format!("wanda/chats/{}", content_id(content.as_bytes())))?;
    Ok(ChatRecord {
        reference,
        platform: platform.clone(),
        chat_id: chat_id.to_string(),
        message_count,
        content,
    })
}

/// The message's content id, which is also the last component of its reference path.
fn content_id_of(record: &MessageRecord) -> String {
    record
        .reference
        .path()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A batch of messages, validated, stored and collected into one pillar context.
pub struct ChatBatch {
    context: PillarContext,
    messages: Vec<MessageRecord>,
    chats: Vec<ChatRecord>,
}

impl std::fmt::Debug for ChatBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatBatch")
            .field("context", &self.context)
            .field("messages", &self.messages.len())
            .field("chats", &self.chats.len())
            .finish()
    }
}

impl ChatBatch {
    pub fn context(&self) -> &PillarContext {
        &self.context
    }
    pub fn messages(&self) -> &[MessageRecord] {
        &self.messages
    }
    pub fn chats(&self) -> &[ChatRecord] {
        &self.chats
    }
}

/// What a batch came to. An empty batch is typed [`ChatBatchOutcome::NoMessages`]:
/// nothing stored, no context, not an error.
pub enum ChatBatchOutcome {
    NoMessages,
    Recorded(ChatBatch),
}

impl std::fmt::Debug for ChatBatchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMessages => f.write_str("ChatBatchOutcome::NoMessages"),
            Self::Recorded(batch) => f
                .debug_tuple("ChatBatchOutcome::Recorded")
                .field(batch)
                .finish(),
        }
    }
}

impl ChatBatchOutcome {
    /// The recorded batch, when there was anything to record.
    pub fn batch(&self) -> Option<&ChatBatch> {
        match self {
            Self::NoMessages => None,
            Self::Recorded(batch) => Some(batch),
        }
    }
}

/// Validate, store and collect one batch of messages into a [`PillarContext`] tagged
/// [`Pillar::Wanda`].
///
/// Everything is validated and shaped before the vault is touched: a batch whose
/// records collide stores nothing. Identical messages — a replayed export, a repeated
/// notification pull — land on one content-derived reference and are stored once; a
/// different record already sitting at that reference is a
/// [`WandaError::ReferenceCollision`], never a silent overwrite. Stored entries carry
/// the messages' own times; a message with no source time is sealed at `0`, the
/// explicit unknown — the bridge has no clock of its own.
pub fn record_chat_context(
    vault: &mut Vault,
    domain: &ContextDomain,
    task_id: &str,
    messages: &[ChatMessage],
) -> Result<ChatBatchOutcome, WandaError> {
    if messages.is_empty() {
        return Ok(ChatBatchOutcome::NoMessages);
    }
    if messages.len() > MAX_MESSAGES_PER_BATCH {
        return Err(WandaError::BatchTooLarge {
            found: messages.len(),
            cap: MAX_MESSAGES_PER_BATCH,
        });
    }
    // Phase one: shape every message, dedupe by the content-derived reference.
    let mut records: Vec<MessageRecord> = Vec::new();
    for message in messages {
        let record = message_record(domain, message)?;
        match records
            .iter()
            .find(|existing| existing.reference == record.reference)
        {
            Some(existing) if existing.content == record.content => continue,
            Some(_) => {
                return Err(WandaError::ReferenceCollision {
                    reference: record.reference,
                })
            }
            None => records.push(record),
        }
    }
    // Phase two: aggregate per chat, grouped by platform and chat id.
    let mut groups: BTreeMap<(String, String), Vec<&MessageRecord>> = BTreeMap::new();
    let mut platforms: BTreeMap<(String, String), ChatPlatform> = BTreeMap::new();
    for record in &records {
        let key = (record.platform.label().to_string(), record.chat_id.clone());
        platforms
            .entry(key.clone())
            .or_insert_with(|| record.platform.clone());
        groups.entry(key).or_default().push(record);
    }
    let mut chats: Vec<ChatRecord> = Vec::new();
    for (key, group) in &groups {
        let platform = platforms.get(key).expect("a group implies its platform");
        chats.push(chat_record(domain, platform, &key.1, group)?);
    }
    // Phase three: store and include. A replayed record is already there with the same
    // content, which is fine; anything else under its reference is a collision.
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    for record in &records {
        store_once(
            vault,
            &record.reference,
            &record.content,
            source_ms(record.sent_at),
        )?;
        context.include(Pillar::Wanda, record.reference.clone())?;
    }
    for chat in &chats {
        store_once(vault, &chat.reference, &chat.content, 0)?;
        context.include(Pillar::Wanda, chat.reference.clone())?;
    }
    Ok(ChatBatchOutcome::Recorded(ChatBatch {
        context,
        messages: records,
        chats,
    }))
}

/// The timestamp a record is sealed at: the source's own, or 0 as the explicit
/// unknown.
fn source_ms(sent_at: Option<u64>) -> i64 {
    sent_at.and_then(|at| i64::try_from(at).ok()).unwrap_or(0)
}

/// Store a record unless the vault already holds exactly it at its reference.
fn store_once(
    vault: &mut Vault,
    reference: &VaultRef,
    content: &str,
    at_ms: i64,
) -> Result<(), WandaError> {
    if vault.contains(reference) {
        let existing = vault
            .disclose(ACTOR, reference, PrivacyGrade::Restricted, at_ms)
            .ok()
            .and_then(|disclosure| disclosure.text().map(str::to_owned));
        match existing {
            Some(text) if text == content => Ok(()),
            _ => Err(WandaError::ReferenceCollision {
                reference: reference.clone(),
            }),
        }
    } else {
        vault
            .store(
                ACTOR,
                reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                content,
                at_ms,
                None,
            )
            .map_err(WandaError::Vault)
    }
}
