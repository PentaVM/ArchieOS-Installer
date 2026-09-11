//! Identifiers.
//!
//! # Why `NodeId` is derived, not assigned
//!
//! A node's identity is the SHA-256 of its Ed25519 public key. It is *derived*, never
//! chosen. This matters for the fabric: any peer that receives a signed announcement can
//! recompute the sender's `NodeId` from the enclosed public key and confirm it matches
//! the claimed id. A malicious node therefore cannot impersonate another node's id
//! without also holding that node's private key. Had we let nodes pick their own UUID,
//! id spoofing would be free.

use crate::error::{Result, TypeError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// A node's stable identity: `sha256(domain_tag || ed25519_public_key)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId([u8; 32]);

impl NodeId {
    /// Derive the id that necessarily corresponds to this public key.
    pub fn from_public_key_bytes(public_key: &[u8; 32]) -> Self {
        let mut hasher = Sha256::new();
        // Domain separation: hashing a bare key would let the same bytes be reused as
        // a different kind of identifier elsewhere in the protocol.
        hasher.update(b"archieos-node-id-v1");
        hasher.update(public_key);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Self(out)
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Short form for logs and CLI tables. Never use this for equality.
    pub fn short(&self) -> String {
        hex::encode(&self.0[..6])
    }

    /// Confirm this id really is the one implied by `public_key`.
    pub fn matches_public_key(&self, public_key: &[u8; 32]) -> bool {
        *self == Self::from_public_key_bytes(public_key)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl FromStr for NodeId {
    type Err = TypeError;

    fn from_str(value: &str) -> Result<Self> {
        let raw = hex::decode(value).map_err(|_| TypeError::InvalidNodeId {
            value: value.to_string(),
            reason: "node id must be hex",
        })?;
        let bytes: [u8; 32] = raw.try_into().map_err(|_| TypeError::InvalidNodeId {
            value: value.to_string(),
            reason: "node id must be 32 bytes (64 hex chars)",
        })?;
        Ok(Self(bytes))
    }
}

impl Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// Macro for the UUID-backed identifiers. They differ only in name; giving each its own
/// type stops a `TaskId` from being passed where an `AgentId` is expected.
macro_rules! uuid_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = TypeError;

            fn from_str(value: &str) -> Result<Self> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| TypeError::InvalidNodeId {
                        value: value.to_string(),
                        reason: "expected a UUID",
                    })
            }
        }
    };
}

uuid_id!(TaskId, "Identifies a node in the persistent task DAG.");
uuid_id!(
    TraceId,
    "Correlates every record produced by one user-visible intent."
);
uuid_id!(
    AgentId,
    "Identifies a logical agent, independent of which worker runs it."
);
uuid_id!(LeaseId, "Identifies an issued capability lease.");
uuid_id!(JobId, "Identifies a scheduler work item.");

/// A dotted, lowercase name such as `cognition.researcher`.
///
/// Shared by module and model identifiers so both get the same validation and the same
/// guarantee: safe to embed in a path, a topic name, or a filename without escaping.
fn validate_dotted(value: &str) -> std::result::Result<(), &'static str> {
    if value.is_empty() {
        return Err("must not be empty");
    }
    if value.len() > 128 {
        return Err("must be at most 128 characters");
    }
    for segment in value.split('.') {
        if segment.is_empty() {
            return Err("segments must not be empty");
        }
        let mut chars = segment.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {}
            _ => return Err("each segment must start with a lowercase letter"),
        }
        for c in chars {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                return Err("segments allow only [a-z0-9_-]");
            }
        }
    }
    Ok(())
}

macro_rules! dotted_id {
    ($name:ident, $err:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_dotted(&value).map_err(|reason| TypeError::$err {
                    value: value.clone(),
                    reason,
                })?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = TypeError;
            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = TypeError;
            fn try_from(value: String) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }
    };
}

dotted_id!(
    ModuleId,
    InvalidModuleId,
    "Identifies an installed Archie module, e.g. `cognition.critic`."
);
dotted_id!(
    ModelId,
    InvalidModelId,
    "Identifies a model within its provider, e.g. `local.qwen3-8b-q4`."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_is_derived_from_key() {
        let key = [7u8; 32];
        let id = NodeId::from_public_key_bytes(&key);
        assert!(id.matches_public_key(&key));
        assert!(!id.matches_public_key(&[8u8; 32]));
    }

    #[test]
    fn node_id_derivation_is_stable() {
        assert_eq!(
            NodeId::from_public_key_bytes(&[1u8; 32]),
            NodeId::from_public_key_bytes(&[1u8; 32])
        );
    }

    #[test]
    fn node_id_round_trips_through_text() {
        let id = NodeId::from_public_key_bytes(&[3u8; 32]);
        assert_eq!(NodeId::from_str(&id.to_string()).unwrap(), id);
    }

    #[test]
    fn node_id_rejects_wrong_length() {
        assert!(NodeId::from_str("abcd").is_err());
        assert!(NodeId::from_str("zzzz").is_err());
    }

    #[test]
    fn node_id_serde_round_trip() {
        let id = NodeId::from_public_key_bytes(&[9u8; 32]);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<NodeId>(&json).unwrap(), id);
    }

    #[test]
    fn dotted_ids_accept_valid_names() {
        assert!(ModuleId::parse("cognition.critic").is_ok());
        assert!(ModuleId::parse("core.archie").is_ok());
        assert!(ModelId::parse("local.qwen3-8b_q4").is_ok());
        assert!(ModuleId::parse("single").is_ok());
    }

    #[test]
    fn dotted_ids_reject_invalid_names() {
        for bad in [
            "",
            "Cognition.Critic",
            "1bad.name",
            "trailing.",
            ".leading",
            "a..b",
            "has space",
        ] {
            assert!(
                ModuleId::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn uuid_ids_display_as_uuid() {
        assert_eq!(TaskId::new().to_string().len(), 36);
    }
}
