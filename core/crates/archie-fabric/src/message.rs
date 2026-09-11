//! Fabric wire messages.
//!
//! The message set is deliberately small. v0.1 needs exactly enough to answer
//! "who else is out there, and what can they do" — discovery, capability exchange, and
//! liveness. Task dispatch rides on top of this later and does not require new
//! envelope semantics.

use archie_types::{NodeDescriptor, NodeId};
use serde::{Deserialize, Serialize};

/// What one node says to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FabricMessage {
    /// Periodic "I exist, and here is what I can do".
    ///
    /// Carries the full descriptor rather than a pointer to one, so a peer that joins
    /// mid-conversation becomes useful after a single packet instead of needing a
    /// round trip.
    Announce { descriptor: Box<NodeDescriptor> },

    /// "Everyone announce yourselves." Sent on join so a new node does not wait a full
    /// announce interval to see the fabric.
    Probe,

    /// Ask one specific node for a fresh descriptor.
    DescribeRequest { target: NodeId },

    /// Reply to [`FabricMessage::DescribeRequest`].
    DescribeResponse { descriptor: Box<NodeDescriptor> },

    /// Graceful departure. Lets peers mark the node gone immediately rather than
    /// waiting for it to time out.
    Goodbye { reason: String },
}

impl FabricMessage {
    /// Short name for logs and metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Announce { .. } => "announce",
            Self::Probe => "probe",
            Self::DescribeRequest { .. } => "describe_request",
            Self::DescribeResponse { .. } => "describe_response",
            Self::Goodbye { .. } => "goodbye",
        }
    }

    /// The descriptor this message carries, if any.
    pub fn descriptor(&self) -> Option<&NodeDescriptor> {
        match self {
            Self::Announce { descriptor } | Self::DescribeResponse { descriptor } => {
                Some(descriptor)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kinds_are_distinct() {
        let kinds = [
            FabricMessage::Probe.kind(),
            FabricMessage::Goodbye {
                reason: "shutdown".into(),
            }
            .kind(),
            FabricMessage::DescribeRequest {
                target: NodeId::from_public_key_bytes(&[1u8; 32]),
            }
            .kind(),
        ];
        let unique: std::collections::HashSet<_> = kinds.iter().collect();
        assert_eq!(unique.len(), kinds.len());
    }

    #[test]
    fn probe_round_trips() {
        let json = serde_json::to_string(&FabricMessage::Probe).unwrap();
        assert_eq!(
            serde_json::from_str::<FabricMessage>(&json).unwrap(),
            FabricMessage::Probe
        );
    }

    #[test]
    fn goodbye_round_trips_with_its_reason() {
        let message = FabricMessage::Goodbye {
            reason: "operator stopped archied".into(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<FabricMessage>(&json).unwrap(),
            message
        );
    }

    #[test]
    fn messages_without_a_descriptor_report_none() {
        assert!(FabricMessage::Probe.descriptor().is_none());
        assert!(FabricMessage::Goodbye { reason: "x".into() }
            .descriptor()
            .is_none());
    }
}
