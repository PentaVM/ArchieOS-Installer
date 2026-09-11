//! The vault itself: encrypted entries, an access log, and one narrow door for
//! credentials.
//!
//! # The grade is authenticated, not merely recorded
//!
//! Every entry is sealed with its own reference, class and grade as additional
//! authenticated data. That makes the grade part of what the ciphertext *is*: an
//! attacker (or a bug) that rewrites `grade: secret` to `grade: public` in the stored
//! record does not obtain a readable public entry, it obtains a decryption failure.
//! A grade held only in a plaintext field beside the ciphertext would be a label; this
//! one is a lock.

use crate::reference::{EntryClass, VaultRef};
use archie_types::PrivacyGrade;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

/// The key that opens a vault.
///
/// Held in memory only as long as the vault is open, and wiped on drop. Deriving one
/// from a passphrase is [`VaultKey::derive`]; tests and provisioning use
/// [`VaultKey::from_bytes`].
pub struct VaultKey {
    bytes: Zeroizing<[u8; 32]>,
}

impl VaultKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// Derive a key from a passphrase with PBKDF2-HMAC-SHA256.
    ///
    /// The iteration count is deliberately not a parameter: a caller that can lower it
    /// will, on the machine where it matters most.
    pub fn derive(passphrase: &[u8], salt: &[u8]) -> Result<Self, VaultError> {
        use std::num::NonZeroU32;
        if salt.len() < 16 {
            return Err(VaultError::WeakSalt);
        }
        let mut out = [0u8; 32];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(600_000).expect("nonzero"),
            salt,
            passphrase,
            &mut out,
        );
        let key = Self::from_bytes(out);
        out.zeroize();
        Ok(key)
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(redacted)")
    }
}

/// One sealed record as it sits at rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedEntry {
    pub reference: VaultRef,
    pub class: EntryClass,
    pub grade: PrivacyGrade,
    pub created_at_ms: i64,
    /// `None` never expires.
    pub expires_at_ms: Option<i64>,
    #[serde(with = "hex_bytes")]
    nonce: Vec<u8>,
    #[serde(with = "hex_bytes")]
    ciphertext: Vec<u8>,
}

impl SealedEntry {
    fn expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|at| now_ms >= at)
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd-length hex"));
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect()
    }
}

/// What a caller gets back when it asks for an entry.
///
/// A [`PrivacyGrade::Secret`] entry never yields its content here, whatever the caller
/// intends to do with it. The only route to the bytes is [`Vault::broker_credential`],
/// which hands them to the outbound request rather than to the caller's context.
#[derive(Debug, Clone, PartialEq)]
pub enum Disclosure {
    /// The entry's content, cleared for disclosure at the requested grade.
    Content {
        reference: VaultRef,
        class: EntryClass,
        grade: PrivacyGrade,
        text: String,
    },
    /// A secret exists here. This is what a model is allowed to know about it.
    CredentialReference {
        reference: VaultRef,
        class: EntryClass,
    },
}

impl Disclosure {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Content { text, .. } => Some(text),
            Self::CredentialReference { .. } => None,
        }
    }

    pub fn reference(&self) -> &VaultRef {
        match self {
            Self::Content { reference, .. } | Self::CredentialReference { reference, .. } => {
                reference
            }
        }
    }
}

/// A credential on its way to an outbound request and nowhere else.
///
/// It cannot be cloned, does not implement `Display`, prints as redacted under
/// `Debug`, does not serialise, and yields its bytes exactly once through
/// [`SealedCredential::expose`], which consumes it. None of that stops a determined
/// caller from writing the string somewhere foolish — it stops the accidental cases,
/// which are the ones that actually happen: a credential landing in a log line, a
/// trace payload, or a struct that later grows a `Serialize`.
pub struct SealedCredential {
    reference: VaultRef,
    secret: Zeroizing<String>,
}

impl SealedCredential {
    pub fn reference(&self) -> &VaultRef {
        &self.reference
    }

    /// Yield the secret at the execution boundary, consuming this handle.
    pub fn expose(self) -> Zeroizing<String> {
        self.secret
    }
}

impl fmt::Debug for SealedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealedCredential({}, redacted)", self.reference)
    }
}

/// What happened, for the record. Never what was in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at_ms: i64,
    pub actor: String,
    pub action: AuditAction,
    pub reference: VaultRef,
    pub grade: PrivacyGrade,
    pub granted: bool,
    /// Stable machine-readable reason, present when `granted` is false.
    pub refusal: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditAction {
    Store,
    Disclose,
    BrokerCredential,
    Remove,
}

/// Reasons an access was refused. Stable strings: they end up in audit records.
pub mod refusals {
    pub const NOT_FOUND: &str = "entry_not_found";
    pub const EXPIRED: &str = "entry_expired";
    pub const GRADE_EXCEEDS_REQUEST: &str = "entry_grade_exceeds_permitted";
    pub const SECRET_NOT_DISCLOSED_AS_CONTENT: &str = "secret_never_disclosed_as_content";
    pub const NOT_A_CREDENTIAL: &str = "entry_is_not_a_credential";
}

/// The private store.
pub struct Vault {
    key: LessSafeKey,
    rng: SystemRandom,
    entries: BTreeMap<VaultRef, SealedEntry>,
    audit: Vec<AuditEntry>,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("entries", &self.entries.len())
            .field("audit_records", &self.audit.len())
            .finish_non_exhaustive()
    }
}

impl Vault {
    pub fn open(key: &VaultKey) -> Result<Self, VaultError> {
        let unbound =
            UnboundKey::new(&AES_256_GCM, key.bytes.as_ref()).map_err(|_| VaultError::Crypto)?;
        Ok(Self {
            key: LessSafeKey::new(unbound),
            rng: SystemRandom::new(),
            entries: BTreeMap::new(),
            audit: Vec::new(),
        })
    }

    /// Load sealed records produced by a previous run. Contents stay sealed until read.
    pub fn load(key: &VaultKey, sealed: Vec<SealedEntry>) -> Result<Self, VaultError> {
        let mut vault = Self::open(key)?;
        for entry in sealed {
            vault.entries.insert(entry.reference.clone(), entry);
        }
        Ok(vault)
    }

    /// The sealed records, for persistence. Still encrypted.
    pub fn sealed_entries(&self) -> Vec<SealedEntry> {
        self.entries.values().cloned().collect()
    }

    pub fn audit_log(&self) -> &[AuditEntry] {
        &self.audit
    }

    pub fn contains(&self, reference: &VaultRef) -> bool {
        self.entries.contains_key(reference)
    }

    /// Store `content`, sealed under `grade`.
    ///
    /// A credential class cannot be stored below [`PrivacyGrade::Secret`]; passing a
    /// lower grade is an error rather than a silent upgrade, because the caller and
    /// this function disagree about what is being stored and one of them is wrong.
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &mut self,
        actor: &str,
        reference: VaultRef,
        class: EntryClass,
        grade: PrivacyGrade,
        content: &str,
        now_ms: i64,
        expires_at_ms: Option<i64>,
    ) -> Result<(), VaultError> {
        if class.is_always_secret() && grade != PrivacyGrade::Secret {
            return Err(VaultError::CredentialMustBeSecret);
        }
        if expires_at_ms.is_some_and(|at| at <= now_ms) {
            return Err(VaultError::AlreadyExpired);
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|_| VaultError::Crypto)?;
        let mut buffer = content.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(aad(&reference, class, grade)),
                &mut buffer,
            )
            .map_err(|_| VaultError::Crypto)?;

        self.entries.insert(
            reference.clone(),
            SealedEntry {
                reference: reference.clone(),
                class,
                grade,
                created_at_ms: now_ms,
                expires_at_ms,
                nonce: nonce_bytes.to_vec(),
                ciphertext: buffer,
            },
        );
        self.record(
            actor,
            AuditAction::Store,
            &reference,
            grade,
            true,
            None,
            now_ms,
        );
        Ok(())
    }

    /// Read an entry for disclosure at no more than `permitted` sensitivity.
    ///
    /// Secrets come back as [`Disclosure::CredentialReference`] — a refusal shaped like
    /// an answer, because the caller genuinely needs to know the credential exists in
    /// order to name it in a tool call.
    pub fn disclose(
        &mut self,
        actor: &str,
        reference: &VaultRef,
        permitted: PrivacyGrade,
        now_ms: i64,
    ) -> Result<Disclosure, VaultError> {
        let entry = match self.entries.get(reference) {
            Some(entry) => entry.clone(),
            None => {
                self.refuse(
                    actor,
                    AuditAction::Disclose,
                    reference,
                    permitted,
                    refusals::NOT_FOUND,
                    now_ms,
                );
                return Err(VaultError::NotFound);
            }
        };
        if entry.expired(now_ms) {
            self.refuse(
                actor,
                AuditAction::Disclose,
                reference,
                entry.grade,
                refusals::EXPIRED,
                now_ms,
            );
            return Err(VaultError::Expired);
        }
        if entry.grade == PrivacyGrade::Secret {
            // Logged as a refusal even though a value is returned: what the caller asked
            // for (content) was not what it received.
            self.refuse(
                actor,
                AuditAction::Disclose,
                reference,
                entry.grade,
                refusals::SECRET_NOT_DISCLOSED_AS_CONTENT,
                now_ms,
            );
            return Ok(Disclosure::CredentialReference {
                reference: entry.reference,
                class: entry.class,
            });
        }
        if entry.grade > permitted {
            self.refuse(
                actor,
                AuditAction::Disclose,
                reference,
                entry.grade,
                refusals::GRADE_EXCEEDS_REQUEST,
                now_ms,
            );
            return Err(VaultError::GradeExceedsPermitted {
                entry: entry.grade,
                permitted,
            });
        }

        let text = self.unseal(&entry)?;
        self.record(
            actor,
            AuditAction::Disclose,
            reference,
            entry.grade,
            true,
            None,
            now_ms,
        );
        Ok(Disclosure::Content {
            reference: entry.reference,
            class: entry.class,
            grade: entry.grade,
            text,
        })
    }

    /// Hand a credential to the execution boundary.
    ///
    /// This is the only function in the crate that returns secret material, and what it
    /// returns cannot be cloned, printed or serialised. Callers on the context path do
    /// not call it; the outbound request does, immediately before signing or sending.
    pub fn broker_credential(
        &mut self,
        actor: &str,
        reference: &VaultRef,
        now_ms: i64,
    ) -> Result<SealedCredential, VaultError> {
        let entry = match self.entries.get(reference) {
            Some(entry) => entry.clone(),
            None => {
                self.refuse(
                    actor,
                    AuditAction::BrokerCredential,
                    reference,
                    PrivacyGrade::Secret,
                    refusals::NOT_FOUND,
                    now_ms,
                );
                return Err(VaultError::NotFound);
            }
        };
        if entry.expired(now_ms) {
            self.refuse(
                actor,
                AuditAction::BrokerCredential,
                reference,
                entry.grade,
                refusals::EXPIRED,
                now_ms,
            );
            return Err(VaultError::Expired);
        }
        if !entry.class.is_always_secret() {
            self.refuse(
                actor,
                AuditAction::BrokerCredential,
                reference,
                entry.grade,
                refusals::NOT_A_CREDENTIAL,
                now_ms,
            );
            return Err(VaultError::NotACredential);
        }

        let secret = Zeroizing::new(self.unseal(&entry)?);
        self.record(
            actor,
            AuditAction::BrokerCredential,
            reference,
            entry.grade,
            true,
            None,
            now_ms,
        );
        Ok(SealedCredential {
            reference: entry.reference,
            secret,
        })
    }

    pub fn remove(&mut self, actor: &str, reference: &VaultRef, now_ms: i64) -> bool {
        match self.entries.remove(reference) {
            Some(entry) => {
                self.record(
                    actor,
                    AuditAction::Remove,
                    reference,
                    entry.grade,
                    true,
                    None,
                    now_ms,
                );
                true
            }
            None => {
                self.refuse(
                    actor,
                    AuditAction::Remove,
                    reference,
                    PrivacyGrade::Public,
                    refusals::NOT_FOUND,
                    now_ms,
                );
                false
            }
        }
    }

    /// Drop every entry whose expiry has passed. Returns how many went.
    pub fn evict_expired(&mut self, actor: &str, now_ms: i64) -> usize {
        let stale: Vec<VaultRef> = self
            .entries
            .values()
            .filter(|e| e.expired(now_ms))
            .map(|e| e.reference.clone())
            .collect();
        for reference in &stale {
            self.entries.remove(reference);
            self.record(
                actor,
                AuditAction::Remove,
                reference,
                PrivacyGrade::Secret,
                true,
                Some(refusals::EXPIRED),
                now_ms,
            );
        }
        stale.len()
    }

    fn unseal(&self, entry: &SealedEntry) -> Result<String, VaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        if entry.nonce.len() != NONCE_LEN {
            return Err(VaultError::Crypto);
        }
        nonce.copy_from_slice(&entry.nonce);
        let mut buffer = entry.ciphertext.clone();
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(&entry.reference, entry.class, entry.grade)),
                &mut buffer,
            )
            .map_err(|_| VaultError::Crypto)?;
        String::from_utf8(plain.to_vec()).map_err(|_| VaultError::Crypto)
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        actor: &str,
        action: AuditAction,
        reference: &VaultRef,
        grade: PrivacyGrade,
        granted: bool,
        refusal: Option<&'static str>,
        at_ms: i64,
    ) {
        self.audit.push(AuditEntry {
            at_ms,
            actor: actor.to_string(),
            action,
            reference: reference.clone(),
            grade,
            granted,
            refusal,
        });
    }

    fn refuse(
        &mut self,
        actor: &str,
        action: AuditAction,
        reference: &VaultRef,
        grade: PrivacyGrade,
        reason: &'static str,
        at_ms: i64,
    ) {
        self.record(actor, action, reference, grade, false, Some(reason), at_ms);
    }
}

/// Additional authenticated data: the entry's identity and grade.
fn aad(reference: &VaultRef, class: EntryClass, grade: PrivacyGrade) -> Vec<u8> {
    format!("{reference}\u{1f}{class}\u{1f}{grade}").into_bytes()
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VaultError {
    #[error("no such vault entry")]
    NotFound,
    #[error("vault entry has expired")]
    Expired,
    #[error("entry is graded {entry} but only {permitted} was permitted")]
    GradeExceedsPermitted {
        entry: PrivacyGrade,
        permitted: PrivacyGrade,
    },
    #[error("a credential class must be stored as secret")]
    CredentialMustBeSecret,
    #[error("entry is not a credential and cannot be brokered")]
    NotACredential,
    #[error("expiry is in the past")]
    AlreadyExpired,
    #[error("salt must be at least 16 bytes")]
    WeakSalt,
    #[error("vault cryptography failed")]
    Crypto,
}
