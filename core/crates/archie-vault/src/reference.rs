//! Vault references, and what kind of thing sits behind one.
//!
//! References were in this codebase long before the vault was. `archie-memory`'s
//! correction records store `raw_correction_ref`, `archie-core` returns cortex
//! trajectories carrying `vault:responses/0001`, and a control-plane test already
//! asserts that those references are stripped out of anything a client can read. The
//! discipline existed; the thing being referred to did not. This module parses what
//! the rest of the system has been writing.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A reference to something held in the vault.
///
/// Rendered as `vault:<scope>/<name>` — the form already used across the codebase.
/// `vault://<scope>/<name>` parses to the same reference, because both spellings are
/// in circulation and a reference that is equal to itself under a different spelling
/// is the kind of thing that silently duplicates an audit record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VaultRef {
    path: String,
}

impl VaultRef {
    /// Longest reference accepted. Matches `archie_memory`'s reference bound, so a
    /// reference that round-trips through a cognitive event is never truncated.
    pub const MAX_LEN: usize = 192;

    pub fn parse(text: &str) -> Result<Self, RefError> {
        let rest = text
            .strip_prefix("vault://")
            .or_else(|| text.strip_prefix("vault:"))
            .ok_or(RefError::NotAVaultRef)?;
        Self::from_path(rest)
    }

    /// Build from the path part alone, e.g. `github/main`.
    pub fn from_path(path: &str) -> Result<Self, RefError> {
        if path.is_empty() || path.len() > Self::MAX_LEN {
            return Err(RefError::Length);
        }
        if !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
        {
            return Err(RefError::Charset);
        }
        // `..` and empty segments would let one reference name another entry's slot.
        if path.starts_with('/')
            || path.ends_with('/')
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            return Err(RefError::Shape);
        }
        Ok(Self {
            path: path.to_string(),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The first path segment, by convention the owning area (`github`, `responses`).
    pub fn scope(&self) -> &str {
        self.path.split('/').next().unwrap_or(&self.path)
    }
}

impl fmt::Display for VaultRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vault:{}", self.path)
    }
}

impl TryFrom<String> for VaultRef {
    type Error = RefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<VaultRef> for String {
    fn from(value: VaultRef) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    #[error("not a vault reference: expected a `vault:` prefix")]
    NotAVaultRef,
    #[error("vault reference is empty or longer than 192 bytes")]
    Length,
    #[error("vault reference may contain only alphanumerics and `._-/`")]
    Charset,
    #[error("vault reference has an empty, relative or unterminated path segment")]
    Shape,
}

/// What sort of material an entry holds.
///
/// The class exists to decide a *default* grade, so that storing a credential under a
/// permissive grade has to be a typed, deliberate act rather than an omission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryClass {
    /// Passwords, API keys, private keys, raw tokens.
    Credential,
    /// Who the operator is: names, addresses, identifiers.
    Identity,
    /// Things the operator told Archie that are true and private.
    PrivateFact,
    /// Retained conversational or episodic material.
    PrivateMemory,
    /// Files the operator put under Archie's care.
    Document,
    /// How the operator likes things done.
    Preference,
    /// Grants, scopes, consent records.
    Permission,
    /// Credentials belonging to a device rather than to a person.
    DeviceCredential,
}

impl EntryClass {
    /// The grade an entry of this class carries unless it is explicitly raised.
    ///
    /// Credentials are [`PrivacyGrade::Secret`] and cannot be stored lower — see
    /// [`crate::VaultEntry`]. The rest default to [`PrivacyGrade::Restricted`]: private
    /// material stays on this machine until someone decides otherwise for a reason.
    pub fn default_grade(&self) -> archie_types::PrivacyGrade {
        use archie_types::PrivacyGrade;
        match self {
            Self::Credential | Self::DeviceCredential => PrivacyGrade::Secret,
            Self::Identity
            | Self::PrivateFact
            | Self::PrivateMemory
            | Self::Document
            | Self::Preference
            | Self::Permission => PrivacyGrade::Restricted,
        }
    }

    /// Whether this class may only ever be [`PrivacyGrade::Secret`].
    pub fn is_always_secret(&self) -> bool {
        matches!(self, Self::Credential | Self::DeviceCredential)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Credential => "credential",
            Self::Identity => "identity",
            Self::PrivateFact => "private-fact",
            Self::PrivateMemory => "private-memory",
            Self::Document => "document",
            Self::Preference => "preference",
            Self::Permission => "permission",
            Self::DeviceCredential => "device-credential",
        }
    }
}

impl fmt::Display for EntryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::PrivacyGrade;

    #[test]
    fn both_spellings_in_circulation_parse_to_one_reference() {
        let short = VaultRef::parse("vault:github/main").unwrap();
        let long = VaultRef::parse("vault://github/main").unwrap();
        assert_eq!(short, long);
        // And render back into the form the existing records use.
        assert_eq!(short.to_string(), "vault:github/main");
    }

    #[test]
    fn references_already_written_by_other_crates_parse() {
        for existing in [
            "vault:corrections/0001",
            "vault:responses/0001",
            "vault:objective",
        ] {
            assert!(VaultRef::parse(existing).is_ok(), "{existing}");
        }
    }

    #[test]
    fn traversal_and_empty_segments_are_refused() {
        for bad in [
            "vault:../etc/shadow",
            "vault:github/../aws/key",
            "vault:/leading",
            "vault:trailing/",
            "vault:double//slash",
            "vault:.",
            "vault:",
        ] {
            assert!(VaultRef::parse(bad).is_err(), "{bad} must not parse");
        }
    }

    #[test]
    fn a_plain_string_is_not_a_vault_reference() {
        assert_eq!(
            VaultRef::parse("github/main").unwrap_err(),
            RefError::NotAVaultRef
        );
    }

    #[test]
    fn credential_classes_default_to_secret() {
        assert_eq!(EntryClass::Credential.default_grade(), PrivacyGrade::Secret);
        assert_eq!(
            EntryClass::DeviceCredential.default_grade(),
            PrivacyGrade::Secret
        );
        assert!(EntryClass::Credential.is_always_secret());
        assert!(!EntryClass::Document.is_always_secret());
    }

    #[test]
    fn reference_round_trips_through_json_as_one_string() {
        let r = VaultRef::parse("vault:github/main").unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"vault:github/main\"");
        assert_eq!(serde_json::from_str::<VaultRef>(&json).unwrap(), r);
    }
}
