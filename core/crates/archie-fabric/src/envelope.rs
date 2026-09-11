//! Signed envelopes.
//!
//! Every byte that crosses the fabric is wrapped in a [`SignedEnvelope`]. There is no
//! unauthenticated message type, because "just this one control message is unsigned" is
//! how a fabric ends up trusting whatever is shouting loudest on the LAN.
//!
//! # Why the payload travels as opaque bytes
//!
//! The envelope carries the serialized payload verbatim and signs *those exact bytes*.
//! If it instead carried a structured payload and the verifier re-serialized it to
//! check the signature, then any difference in map ordering, float formatting, or
//! serde version between two nodes would produce a valid message that fails
//! verification — or worse, a payload that verifies but decodes differently from what
//! was signed. Signing the received bytes removes the whole class of problem.

use crate::identity::{self, IdentityError, NodeIdentity};
use crate::membership::Membership;
use crate::message::FabricMessage;
use archie_types::{NodeId, FABRIC_PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Domain separator. Prevents a signature made for some other ArchieOS purpose from
/// being replayed as a fabric message.
const SIGNING_DOMAIN: &[u8] = b"archieos-fabric-envelope-v1";

/// The largest envelope accepted from the network.
///
/// A descriptor with many volumes and interfaces is a few kilobytes; 64 KiB leaves
/// generous headroom while staying inside a single UDP datagram's practical limit and
/// bounding what a hostile sender can make us allocate.
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("unsupported fabric protocol version {found} (this node speaks {expected})")]
    ProtocolMismatch { found: u16, expected: u16 },

    #[error("envelope too large: {size} bytes exceeds the {MAX_ENVELOPE_BYTES} byte limit")]
    TooLarge { size: usize },

    #[error("malformed envelope: {0}")]
    Malformed(String),

    #[error(transparent)]
    Identity(#[from] IdentityError),
}

pub type Result<T> = std::result::Result<T, EnvelopeError>;

/// An authenticated message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEnvelope {
    /// Wire protocol version. Checked before anything else is parsed.
    pub protocol_version: u16,
    /// Who claims to have sent this.
    pub sender: NodeId,
    /// Hex Ed25519 public key. Must derive to `sender`.
    pub public_key: String,
    /// The certificate that admitted this sender to a fabric.
    ///
    /// Deliberately outside the signed bytes. It is signed by the *owner*, not by the
    /// sender, and it names the node key it admits — so a node cannot present another
    /// node's certificate without also being able to sign as that node, which is the
    /// thing it cannot do. Signing it again here would buy nothing and would mean an
    /// owner reissuing a membership invalidated every envelope already in flight.
    ///
    /// Optional on the wire so an unenrolled node can still be *heard* refusing, rather
    /// than producing packets that fail to parse. It is not optional in effect: a
    /// receiver with no certificate to check rejects the sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<Membership>,

    /// The JSON-encoded [`FabricMessage`], signed verbatim.
    pub payload: String,
    /// Hex Ed25519 signature over the canonical signing bytes.
    pub signature: String,
}

impl SignedEnvelope {
    /// Wrap and sign a message.
    ///
    /// `membership` is an explicit argument rather than something the envelope reaches
    /// for, so that every place a message is created has to answer the question "on
    /// whose authority". A `None` here is a node saying it belongs to nothing, which is
    /// a real answer and one every receiver will refuse.
    pub fn seal(
        identity: &NodeIdentity,
        membership: Option<&Membership>,
        message: &FabricMessage,
    ) -> Result<Self> {
        let payload = serde_json::to_string(message)
            .map_err(|e| EnvelopeError::Malformed(format!("cannot encode payload: {e}")))?;
        let sender = identity.node_id();
        let signing_bytes = signing_bytes(FABRIC_PROTOCOL_VERSION, &sender, payload.as_bytes());
        let signature = identity.sign(&signing_bytes);

        Ok(Self {
            protocol_version: FABRIC_PROTOCOL_VERSION,
            sender,
            public_key: identity.public_key_hex(),
            membership: membership.cloned(),
            payload,
            signature: hex::encode(signature),
        })
    }

    /// Verify and unwrap.
    ///
    /// Checks, in order: protocol version, key/id binding, signature, then payload
    /// decoding. Cheap structural checks come before cryptography so that garbage
    /// traffic costs a hostile sender more than it costs us.
    pub fn open(&self) -> Result<FabricMessage> {
        if self.protocol_version != FABRIC_PROTOCOL_VERSION {
            return Err(EnvelopeError::ProtocolMismatch {
                found: self.protocol_version,
                expected: FABRIC_PROTOCOL_VERSION,
            });
        }

        let public_key = identity::public_key_from_hex(&self.public_key)?;

        let signature_bytes = hex::decode(&self.signature)
            .map_err(|e| EnvelopeError::Malformed(format!("signature is not hex: {e}")))?;
        let signature: [u8; 64] = signature_bytes
            .try_into()
            .map_err(|_| EnvelopeError::Malformed("signature must be 64 bytes".into()))?;

        let signing_bytes =
            signing_bytes(self.protocol_version, &self.sender, self.payload.as_bytes());
        identity::verify_signed_by(&self.sender, &public_key, &signing_bytes, &signature)?;

        serde_json::from_str(&self.payload)
            .map_err(|e| EnvelopeError::Malformed(format!("cannot decode payload: {e}")))
    }

    /// Serialize for transmission.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)
            .map_err(|e| EnvelopeError::Malformed(format!("cannot encode envelope: {e}")))?;
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeError::TooLarge { size: bytes.len() });
        }
        Ok(bytes)
    }

    /// Parse a received datagram.
    ///
    /// The size check happens before parsing so that an oversized message is rejected
    /// without being allocated into a structure first.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(EnvelopeError::TooLarge { size: bytes.len() });
        }
        serde_json::from_slice(bytes)
            .map_err(|e| EnvelopeError::Malformed(format!("cannot decode envelope: {e}")))
    }
}

/// Build the exact byte string that gets signed.
///
/// Length-prefixing every field means no combination of field values can be rearranged
/// into a different message with the same signing bytes.
fn signing_bytes(protocol_version: u16, sender: &NodeId, payload: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(SIGNING_DOMAIN.len() + 2 + 32 + 8 + payload.len());
    buffer.extend_from_slice(SIGNING_DOMAIN);
    buffer.extend_from_slice(&protocol_version.to_be_bytes());
    buffer.extend_from_slice(sender.as_bytes());
    buffer.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    buffer.extend_from_slice(payload);
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{
        Capability, CpuInfo, MemoryInfo, NodeDescriptor, NodeHealth, NodeLoad, NodeProfile,
        Timestamp,
    };

    fn descriptor(identity: &NodeIdentity) -> NodeDescriptor {
        NodeDescriptor {
            node_id: identity.node_id(),
            public_key: identity.public_key_hex(),
            hostname: "test".into(),
            profile: NodeProfile::DevVm,
            archieos_version: "0.1.0".into(),
            os: "Arch Linux".into(),
            kernel: "6.10.0".into(),
            architecture: "x86_64".into(),
            cpu: CpuInfo {
                model: "Test".into(),
                architecture: "x86_64".into(),
                physical_cores: 4,
                logical_cores: 8,
                base_frequency_mhz: 3000,
            },
            memory: MemoryInfo {
                total_bytes: 16 << 30,
                available_bytes: 8 << 30,
                swap_total_bytes: 0,
            },
            accelerators: vec![],
            storage: vec![],
            networks: vec![],
            capabilities: [Capability::known("compute.cpu")].into_iter().collect(),
            loaded_models: vec![],
            modules: vec![],
            health: NodeHealth::Ok,
            load: NodeLoad::default(),
            announced_at: Timestamp::now(),
        }
    }

    #[test]
    fn a_sealed_envelope_opens_to_the_original_message() {
        let identity = NodeIdentity::generate();
        let message = FabricMessage::Announce {
            descriptor: Box::new(descriptor(&identity)),
        };
        let envelope = SignedEnvelope::seal(&identity, None, &message).unwrap();
        assert_eq!(envelope.open().unwrap(), message);
    }

    #[test]
    fn envelope_survives_a_byte_round_trip() {
        let identity = NodeIdentity::generate();
        let envelope = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        let bytes = envelope.to_bytes().unwrap();
        let parsed = SignedEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, envelope);
        assert_eq!(parsed.open().unwrap(), FabricMessage::Probe);
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        let identity = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        envelope.payload = serde_json::to_string(&FabricMessage::Goodbye {
            reason: "injected".into(),
        })
        .unwrap();
        assert!(matches!(
            envelope.open().unwrap_err(),
            EnvelopeError::Identity(IdentityError::BadSignature)
        ));
    }

    #[test]
    fn swapping_in_another_nodes_key_is_rejected() {
        // Mallory intercepts Alice's envelope and substitutes her own key, hoping the
        // receiver checks only that the signature matches the key presented.
        let alice = NodeIdentity::generate();
        let mallory = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(&alice, None, &FabricMessage::Probe).unwrap();
        envelope.public_key = mallory.public_key_hex();
        assert!(matches!(
            envelope.open().unwrap_err(),
            EnvelopeError::Identity(IdentityError::IdKeyMismatch)
        ));
    }

    #[test]
    fn claiming_another_senders_id_is_rejected() {
        let alice = NodeIdentity::generate();
        let mallory = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(&mallory, None, &FabricMessage::Probe).unwrap();
        envelope.sender = alice.node_id();
        assert!(matches!(
            envelope.open().unwrap_err(),
            EnvelopeError::Identity(IdentityError::IdKeyMismatch)
        ));
    }

    #[test]
    fn a_replayed_signature_from_a_different_payload_is_rejected() {
        let identity = NodeIdentity::generate();
        let probe = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        let mut goodbye = SignedEnvelope::seal(
            &identity,
            None,
            &FabricMessage::Goodbye {
                reason: "bye".into(),
            },
        )
        .unwrap();
        goodbye.signature = probe.signature.clone();
        assert!(goodbye.open().is_err());
    }

    #[test]
    fn a_wrong_protocol_version_is_refused_before_anything_else() {
        let identity = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        envelope.protocol_version = 999;
        assert!(matches!(
            envelope.open().unwrap_err(),
            EnvelopeError::ProtocolMismatch {
                found: 999,
                expected: _
            }
        ));
    }

    #[test]
    fn a_malformed_signature_is_reported_not_panicked_on() {
        let identity = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        envelope.signature = "zz".into();
        assert!(matches!(
            envelope.open().unwrap_err(),
            EnvelopeError::Malformed(_)
        ));

        let mut short = SignedEnvelope::seal(&identity, None, &FabricMessage::Probe).unwrap();
        short.signature = hex::encode([0u8; 10]);
        assert!(matches!(
            short.open().unwrap_err(),
            EnvelopeError::Malformed(_)
        ));
    }

    #[test]
    fn oversized_input_is_rejected_without_parsing() {
        let huge = vec![b'x'; MAX_ENVELOPE_BYTES + 1];
        assert!(matches!(
            SignedEnvelope::from_bytes(&huge).unwrap_err(),
            EnvelopeError::TooLarge { .. }
        ));
    }

    #[test]
    fn garbage_bytes_do_not_panic() {
        for junk in [
            b"".as_slice(),
            b"{",
            b"not json at all",
            &[0xFF, 0xFE, 0x00],
        ] {
            assert!(SignedEnvelope::from_bytes(junk).is_err());
        }
    }

    #[test]
    fn signing_bytes_are_unambiguous_across_field_boundaries() {
        // Without length prefixing, a payload could be crafted so that moving bytes
        // between fields produced identical signing input.
        let sender = NodeId::from_public_key_bytes(&[1u8; 32]);
        let a = signing_bytes(1, &sender, b"ab");
        let b = signing_bytes(1, &sender, b"a");
        assert_ne!(a, b);
        assert_ne!(
            signing_bytes(1, &sender, b"x"),
            signing_bytes(2, &sender, b"x")
        );
    }
}
