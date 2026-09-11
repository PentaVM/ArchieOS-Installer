//! Discovery and the peer table.
//!
//! # What "discovery" means here
//!
//! A node periodically broadcasts a signed [`FabricMessage::Announce`] carrying its
//! full descriptor. Receivers verify the signature, check that the descriptor is
//! internally consistent and that it actually describes the sender, then record the
//! peer. There is no central registry and no join protocol: a node that starts talking
//! is discovered, and one that stops talking eventually times out.
//!
//! # What this layer does NOT decide
//!
//! Discovery establishes *who said something*, never *how much they should be trusted*.
//! A verified peer is merely authenticated. Whether it may be scheduled work, or
//! receive sensitive data, is a policy decision made above this layer against the
//! pinned trust state in `archie-state`. Keeping those apart is what stops "I heard it
//! on the LAN" from becoming "I trust it".

use crate::envelope::{EnvelopeError, SignedEnvelope};
use crate::identity::NodeIdentity;
use crate::membership::{Enrolment, MembershipError};
use crate::message::FabricMessage;
use crate::transport::{Inbound, PeerAddress, Transport};
use archie_observability::Metrics;
use archie_types::{NodeDescriptor, NodeHealth, NodeId, Timestamp};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum FabricError {
    #[error("transport error: {0}")]
    Transport(#[from] crate::transport::TransportError),

    #[error("envelope error: {0}")]
    Envelope(#[from] EnvelopeError),
}

pub type Result<T> = std::result::Result<T, FabricError>;

/// Why an inbound message was discarded.
///
/// Returned rather than merely logged so that tests can assert on the *reason*, and so
/// `archie status` can show whether the fabric is seeing hostile or merely stale traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// Failed signature or id/key binding.
    Unauthenticated(String),
    /// Our own announcement, looped back.
    SelfOrigin,
    /// The descriptor describes a node other than the signer.
    DescriptorSenderMismatch,
    /// The descriptor contradicts itself.
    InconsistentDescriptor(Vec<String>),
    /// Undecodable bytes.
    Malformed(String),
    /// Speaks a protocol version we do not.
    ProtocolMismatch { found: u16 },

    /// Genuine, well-formed, and not a member of this fabric.
    ///
    /// Separate from [`RejectReason::Unauthenticated`] because the two mean opposite
    /// things to whoever reads the counter. An unauthenticated message is someone
    /// failing to prove who they are; this is someone proving it perfectly well and
    /// still not being ours. On a street where two households both run ArchieOS, this
    /// counter climbing is the system working.
    NotAMember(MembershipError),
}

impl RejectReason {
    /// Short code for metrics, so cardinality stays bounded.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated(_) => "unauthenticated",
            Self::SelfOrigin => "self_origin",
            Self::DescriptorSenderMismatch => "descriptor_sender_mismatch",
            Self::InconsistentDescriptor(_) => "inconsistent_descriptor",
            Self::Malformed(_) => "malformed",
            Self::ProtocolMismatch { .. } => "protocol_mismatch",
            Self::NotAMember(_) => "not_a_member",
        }
    }
}

/// What handling an inbound message produced.
#[derive(Debug, Clone, PartialEq)]
pub enum FabricEvent {
    /// A peer we had never seen.
    PeerDiscovered(Box<PeerEntry>),
    /// A peer we already knew, with a refreshed descriptor.
    PeerUpdated(Box<PeerEntry>),
    /// A peer said goodbye.
    PeerDeparted { node_id: NodeId, reason: String },
    /// A peer asked everyone to announce.
    ProbeReceived { from: NodeId },
    /// The message was discarded.
    Rejected(RejectReason),
}

/// A peer as the fabric currently sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerEntry {
    pub node_id: NodeId,
    pub public_key: String,
    pub descriptor: NodeDescriptor,
    pub address: PeerAddress,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    /// Set when the last announcement's signature verified. Always true for entries in
    /// the table — unverified messages never create one.
    pub authenticated: bool,
}

impl PeerEntry {
    /// Whether the peer has gone quiet for longer than `timeout_ms`.
    pub fn is_stale(&self, now: Timestamp, timeout_ms: i64) -> bool {
        self.last_seen.elapsed_to(now) > timeout_ms
    }

    /// Health as this node should treat it, accounting for silence.
    ///
    /// A peer that has stopped announcing is `Unreachable` regardless of the health it
    /// last claimed — its own opinion is stale by definition.
    pub fn effective_health(&self, now: Timestamp, timeout_ms: i64) -> NodeHealth {
        if self.is_stale(now, timeout_ms) {
            NodeHealth::Unreachable
        } else {
            self.descriptor.health
        }
    }
}

/// Fabric tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FabricConfig {
    /// How often to re-announce.
    pub announce_interval_ms: i64,
    /// Silence after which a peer is considered unreachable.
    ///
    /// Must be a multiple of the announce interval so a single dropped packet does not
    /// evict a healthy peer.
    pub peer_timeout_ms: i64,
    /// Drop peers entirely after this much silence, so the table does not grow without
    /// bound on a network where nodes come and go.
    pub peer_forget_ms: i64,
}

impl Default for FabricConfig {
    fn default() -> Self {
        Self {
            announce_interval_ms: 5_000,
            peer_timeout_ms: 20_000,
            peer_forget_ms: 3_600_000,
        }
    }
}

impl FabricConfig {
    /// Check the relationships between the intervals hold.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.announce_interval_ms <= 0 {
            return Err("announce_interval_ms must be positive".into());
        }
        if self.peer_timeout_ms < self.announce_interval_ms * 2 {
            return Err(
                "peer_timeout_ms must be at least twice announce_interval_ms, or one lost \
                 packet evicts a healthy peer"
                    .into(),
            );
        }
        if self.peer_forget_ms < self.peer_timeout_ms {
            return Err("peer_forget_ms must not be shorter than peer_timeout_ms".into());
        }
        Ok(())
    }
}

/// The fabric: identity + transport + peer table.
#[derive(Debug)]
pub struct Fabric {
    identity: Arc<NodeIdentity>,
    /// Which fabric this node belongs to, and the certificate proving it.
    ///
    /// Not optional-by-omission: [`Enrolment::none`] is the explicit state of a node
    /// nobody has admitted anywhere, and it talks to nobody.
    enrolment: Enrolment,
    transport: Arc<dyn Transport>,
    descriptor: RwLock<NodeDescriptor>,
    peers: RwLock<HashMap<NodeId, PeerEntry>>,
    config: FabricConfig,
    metrics: Metrics,
}

impl Fabric {
    pub fn new(
        identity: Arc<NodeIdentity>,
        enrolment: Enrolment,
        transport: Arc<dyn Transport>,
        descriptor: NodeDescriptor,
        config: FabricConfig,
        metrics: Metrics,
    ) -> Self {
        Self {
            identity,
            enrolment,
            transport,
            descriptor: RwLock::new(descriptor),
            peers: RwLock::new(HashMap::new()),
            config,
            metrics,
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.identity.node_id()
    }

    /// A short form of the fabric this node belongs to, or `None` when it belongs to
    /// none. Shown by `archie status` so two installs meant to be separate can be seen
    /// to be separate.
    pub fn fabric_fingerprint(&self) -> Option<String> {
        self.enrolment.fabric_fingerprint()
    }

    pub fn is_enrolled(&self) -> bool {
        self.enrolment.is_enrolled()
    }

    pub fn transport_name(&self) -> &'static str {
        self.transport.name()
    }

    pub fn config(&self) -> FabricConfig {
        self.config
    }

    /// Replace the descriptor published in future announcements.
    pub async fn update_descriptor(&self, descriptor: NodeDescriptor) {
        *self.descriptor.write().await = descriptor;
    }

    /// Broadcast this node's descriptor.
    pub async fn announce(&self) -> Result<()> {
        let descriptor = self.descriptor.read().await.clone();
        let message = FabricMessage::Announce {
            descriptor: Box::new(descriptor),
        };
        self.send(&message).await?;
        self.metrics.count("fabric.announce.sent");
        Ok(())
    }

    /// Ask every peer to announce itself now.
    pub async fn probe(&self) -> Result<()> {
        self.send(&FabricMessage::Probe).await?;
        self.metrics.count("fabric.probe.sent");
        Ok(())
    }

    /// Announce departure so peers do not wait for a timeout.
    pub async fn goodbye(&self, reason: impl Into<String>) -> Result<()> {
        self.send(&FabricMessage::Goodbye {
            reason: reason.into(),
        })
        .await
    }

    async fn send(&self, message: &FabricMessage) -> Result<()> {
        let envelope = SignedEnvelope::seal(&self.identity, self.enrolment.membership(), message)?;
        let bytes = envelope.to_bytes()?;
        self.transport.broadcast(&bytes).await?;
        Ok(())
    }

    /// Await and process one inbound message.
    pub async fn receive_once(&self) -> Result<FabricEvent> {
        let inbound = self.transport.recv().await?;
        Ok(self.handle_inbound(inbound).await)
    }

    /// Verify and apply one datagram.
    ///
    /// Never returns an error for hostile input: a bad packet is a [`FabricEvent::Rejected`],
    /// because one malformed datagram must not be able to stop the receive loop.
    pub async fn handle_inbound(&self, inbound: Inbound) -> FabricEvent {
        self.metrics.count("fabric.inbound.total");

        let envelope = match SignedEnvelope::from_bytes(&inbound.bytes) {
            Ok(envelope) => envelope,
            Err(e) => return self.reject(RejectReason::Malformed(e.to_string())),
        };

        // Drop our own traffic before spending a signature verification on it.
        if envelope.sender == self.identity.node_id() {
            return self.reject(RejectReason::SelfOrigin);
        }

        // Admission before authentication, and deliberately in that order.
        //
        // Verifying the envelope is elliptic-curve work; comparing two owner keys is a
        // string comparison. The ordinary case on a shared network is a *neighbour's*
        // fabric announcing itself — not an attack, just someone else's house — and that
        // has to cost us a comparison rather than a signature check, or being next door
        // to another ArchieOS install becomes a way to spend our CPU.
        //
        // Checking membership first is safe because the certificate names the node key it
        // admits, and a sender who has not got that key still cannot produce an envelope
        // that opens below.
        if let Err(error) = self
            .enrolment
            .admits(&envelope.public_key, envelope.membership.as_ref())
        {
            return self.reject(RejectReason::NotAMember(error));
        }

        let message = match envelope.open() {
            Ok(message) => message,
            Err(EnvelopeError::ProtocolMismatch { found, .. }) => {
                return self.reject(RejectReason::ProtocolMismatch { found })
            }
            Err(EnvelopeError::Identity(e)) => {
                return self.reject(RejectReason::Unauthenticated(e.to_string()))
            }
            Err(e) => return self.reject(RejectReason::Malformed(e.to_string())),
        };

        match message {
            FabricMessage::Announce { descriptor }
            | FabricMessage::DescribeResponse { descriptor } => {
                self.apply_descriptor(*descriptor, &envelope, inbound.from)
                    .await
            }
            FabricMessage::Probe => {
                self.metrics.count("fabric.probe.received");
                FabricEvent::ProbeReceived {
                    from: envelope.sender,
                }
            }
            FabricMessage::DescribeRequest { .. } => {
                // Answered by the caller, which owns the send side.
                FabricEvent::ProbeReceived {
                    from: envelope.sender,
                }
            }
            FabricMessage::Goodbye { reason } => {
                self.peers.write().await.remove(&envelope.sender);
                self.update_peer_gauge().await;
                self.metrics.count("fabric.peer.departed");
                FabricEvent::PeerDeparted {
                    node_id: envelope.sender,
                    reason,
                }
            }
        }
    }

    async fn apply_descriptor(
        &self,
        descriptor: NodeDescriptor,
        envelope: &SignedEnvelope,
        address: PeerAddress,
    ) -> FabricEvent {
        // A signed message proves who sent it. It does not prove the descriptor inside
        // describes the sender — without this check, a valid peer could announce a
        // fabricated descriptor for a *different* node id and poison the topology.
        if descriptor.node_id != envelope.sender {
            return self.reject(RejectReason::DescriptorSenderMismatch);
        }

        let problems = descriptor.self_consistency_problems();
        if !problems.is_empty() {
            return self.reject(RejectReason::InconsistentDescriptor(problems));
        }

        let now = Timestamp::now();
        let mut peers = self.peers.write().await;
        let is_new = !peers.contains_key(&descriptor.node_id);
        let first_seen = peers
            .get(&descriptor.node_id)
            .map(|p| p.first_seen)
            .unwrap_or(now);

        let entry = PeerEntry {
            node_id: descriptor.node_id,
            public_key: envelope.public_key.clone(),
            descriptor,
            address,
            first_seen,
            last_seen: now,
            authenticated: true,
        };
        peers.insert(entry.node_id, entry.clone());
        drop(peers);

        self.update_peer_gauge().await;
        if is_new {
            self.metrics.count("fabric.peer.discovered");
            FabricEvent::PeerDiscovered(Box::new(entry))
        } else {
            self.metrics.count("fabric.peer.updated");
            FabricEvent::PeerUpdated(Box::new(entry))
        }
    }

    fn reject(&self, reason: RejectReason) -> FabricEvent {
        self.metrics
            .increment(&format!("fabric.rejected.{}", reason.code()), 1);
        FabricEvent::Rejected(reason)
    }

    async fn update_peer_gauge(&self) {
        let count = self.peers.read().await.len() as i64;
        self.metrics.set_gauge("fabric.peers", count);
    }

    /// Every known peer, ordered by when it was first seen.
    pub async fn peers(&self) -> Vec<PeerEntry> {
        let mut peers: Vec<PeerEntry> = self.peers.read().await.values().cloned().collect();
        peers.sort_by_key(|p| (p.first_seen.as_millis(), p.node_id.to_string()));
        peers
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    pub async fn peer(&self, node_id: &NodeId) -> Option<PeerEntry> {
        self.peers.read().await.get(node_id).cloned()
    }

    /// Peers currently considered reachable.
    pub async fn reachable_peers(&self) -> Vec<PeerEntry> {
        let now = Timestamp::now();
        self.peers()
            .await
            .into_iter()
            .filter(|p| !p.is_stale(now, self.config.peer_timeout_ms))
            .collect()
    }

    /// Drop peers that have been silent past the forget threshold.
    ///
    /// Returns those removed. Peers between `peer_timeout_ms` and `peer_forget_ms` are
    /// kept but reported unreachable — a node rebooting should not have to be
    /// rediscovered from scratch.
    pub async fn forget_stale_peers(&self) -> Vec<NodeId> {
        let now = Timestamp::now();
        let mut peers = self.peers.write().await;
        let forgotten: Vec<NodeId> = peers
            .values()
            .filter(|p| p.is_stale(now, self.config.peer_forget_ms))
            .map(|p| p.node_id)
            .collect();
        for id in &forgotten {
            peers.remove(id);
        }
        drop(peers);
        if !forgotten.is_empty() {
            self.update_peer_gauge().await;
        }
        forgotten
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::LoopbackBus;
    use archie_types::{Capability, CpuInfo, MemoryInfo, NodeLoad, NodeProfile};
    use std::time::Duration;
    use tokio::time::timeout;

    const SHORT: Duration = Duration::from_millis(500);

    fn descriptor_for(identity: &NodeIdentity, hostname: &str) -> NodeDescriptor {
        NodeDescriptor {
            node_id: identity.node_id(),
            public_key: identity.public_key_hex(),
            hostname: hostname.into(),
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

    /// One owner for the whole test module, so every fabric built here is in the same
    /// household. Tests about *rejection for other reasons* need their sender admitted
    /// first, or admission would reject it earlier and they would pass for the wrong
    /// reason.
    fn test_owner() -> &'static crate::membership::OwnerIdentity {
        static OWNER: std::sync::OnceLock<crate::membership::OwnerIdentity> =
            std::sync::OnceLock::new();
        OWNER.get_or_init(crate::membership::OwnerIdentity::generate)
    }

    fn membership_for(identity: &NodeIdentity) -> crate::membership::Membership {
        test_owner().admit(&identity.public_key_hex(), 1)
    }

    fn enrol(identity: &NodeIdentity) -> Enrolment {
        Enrolment::new(&identity.public_key_hex(), membership_for(identity))
            .expect("the owner admitted this exact node")
    }

    fn fabric_on(bus: &LoopbackBus, hostname: &str) -> Arc<Fabric> {
        let identity = Arc::new(NodeIdentity::generate());
        let descriptor = descriptor_for(&identity, hostname);
        let enrolment = enrol(&identity);
        Arc::new(Fabric::new(
            identity,
            enrolment,
            Arc::new(bus.endpoint()),
            descriptor,
            FabricConfig::default(),
            Metrics::new(),
        ))
    }

    #[tokio::test]
    async fn a_node_discovers_a_peer_that_announces() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "node-a");
        let b = fabric_on(&bus, "node-b");

        a.announce().await.unwrap();
        let event = timeout(SHORT, b.receive_once()).await.unwrap().unwrap();

        match event {
            FabricEvent::PeerDiscovered(entry) => {
                assert_eq!(entry.node_id, a.node_id());
                assert_eq!(entry.descriptor.hostname, "node-a");
                assert!(entry.authenticated);
            }
            other => panic!("expected discovery, got {other:?}"),
        }
        assert_eq!(b.peer_count().await, 1);
    }

    #[tokio::test]
    async fn a_second_announcement_updates_rather_than_duplicates() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "node-a");
        let b = fabric_on(&bus, "node-b");

        a.announce().await.unwrap();
        let first = timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert!(matches!(first, FabricEvent::PeerDiscovered(_)));

        a.announce().await.unwrap();
        let second = timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert!(matches!(second, FabricEvent::PeerUpdated(_)));

        assert_eq!(b.peer_count().await, 1);
    }

    #[tokio::test]
    async fn first_seen_is_preserved_across_updates() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "node-a");
        let b = fabric_on(&bus, "node-b");

        a.announce().await.unwrap();
        timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        let first_seen = b.peer(&a.node_id()).await.unwrap().first_seen;

        tokio::time::sleep(Duration::from_millis(5)).await;
        a.announce().await.unwrap();
        timeout(SHORT, b.receive_once()).await.unwrap().unwrap();

        let entry = b.peer(&a.node_id()).await.unwrap();
        assert_eq!(entry.first_seen, first_seen);
        assert!(entry.last_seen >= first_seen);
    }

    #[tokio::test]
    async fn a_node_never_discovers_itself() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "node-a");
        // A second endpoint keeps the bus alive but ignores traffic.
        let _b = bus.endpoint();

        a.announce().await.unwrap();
        // The loopback transport already filters self-traffic; assert the peer table
        // agrees, since a node listing itself as a peer would double-count resources.
        assert_eq!(a.peer_count().await, 0);
    }

    #[tokio::test]
    async fn three_nodes_all_discover_each_other() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "a");
        let b = fabric_on(&bus, "b");
        let c = fabric_on(&bus, "c");

        for node in [&a, &b, &c] {
            node.announce().await.unwrap();
        }
        // Each node should see the two announcements that were not its own.
        for node in [&a, &b, &c] {
            for _ in 0..2 {
                timeout(SHORT, node.receive_once()).await.unwrap().unwrap();
            }
            assert_eq!(node.peer_count().await, 2, "each node sees both others");
        }
    }

    #[tokio::test]
    async fn goodbye_removes_the_peer_immediately() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "a");
        let b = fabric_on(&bus, "b");

        a.announce().await.unwrap();
        timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert_eq!(b.peer_count().await, 1);

        a.goodbye("operator stopped archied").await.unwrap();
        let event = timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert!(matches!(event, FabricEvent::PeerDeparted { .. }));
        assert_eq!(b.peer_count().await, 0);
    }

    #[tokio::test]
    async fn a_probe_is_recognised() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "a");
        let b = fabric_on(&bus, "b");

        a.probe().await.unwrap();
        let event = timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert_eq!(event, FabricEvent::ProbeReceived { from: a.node_id() });
    }

    #[tokio::test]
    async fn a_descriptor_describing_a_different_node_is_rejected() {
        // The signature is valid, but the payload claims to describe someone else.
        // Accepting it would let any authenticated peer inject fabricated nodes.
        let bus = LoopbackBus::new();
        let attacker = fabric_on(&bus, "attacker");
        let victim = fabric_on(&bus, "victim");

        let other = NodeIdentity::generate();
        let forged = descriptor_for(&other, "ghost-node");
        let message = FabricMessage::Announce {
            descriptor: Box::new(forged),
        };
        // Seal with the attacker's own key — genuinely signed, wrong contents, and now
        // also genuinely a member of this fabric. Being invited is not permission to
        // describe somebody else.
        let attacker_key = NodeIdentity::from_private_bytes(&[3u8; 32]);
        let envelope = SignedEnvelope::seal(
            &attacker_key,
            Some(&membership_for(&attacker_key)),
            &message,
        )
        .unwrap();
        let event = victim
            .handle_inbound(Inbound {
                bytes: envelope.to_bytes().unwrap(),
                from: PeerAddress::Loopback(99),
            })
            .await;

        assert_eq!(
            event,
            FabricEvent::Rejected(RejectReason::DescriptorSenderMismatch)
        );
        assert_eq!(victim.peer_count().await, 0);
        let _ = attacker;
    }

    #[tokio::test]
    async fn an_inconsistent_descriptor_is_rejected() {
        let bus = LoopbackBus::new();
        let victim = fabric_on(&bus, "victim");

        let liar = NodeIdentity::generate();
        let mut descriptor = descriptor_for(&liar, "liar");
        // Claim more free memory than exists.
        descriptor.memory.available_bytes = descriptor.memory.total_bytes + 1;
        let envelope = SignedEnvelope::seal(
            &liar,
            Some(&membership_for(&liar)),
            &FabricMessage::Announce {
                descriptor: Box::new(descriptor),
            },
        )
        .unwrap();

        let event = victim
            .handle_inbound(Inbound {
                bytes: envelope.to_bytes().unwrap(),
                from: PeerAddress::Loopback(7),
            })
            .await;
        assert!(matches!(
            event,
            FabricEvent::Rejected(RejectReason::InconsistentDescriptor(_))
        ));
        assert_eq!(victim.peer_count().await, 0);
    }

    #[tokio::test]
    async fn a_tampered_envelope_is_rejected_as_unauthenticated() {
        let bus = LoopbackBus::new();
        let victim = fabric_on(&bus, "victim");

        let sender = NodeIdentity::generate();
        let mut envelope = SignedEnvelope::seal(
            &sender,
            Some(&membership_for(&sender)),
            &FabricMessage::Probe,
        )
        .unwrap();
        envelope.payload = serde_json::to_string(&FabricMessage::Goodbye {
            reason: "injected".into(),
        })
        .unwrap();

        let event = victim
            .handle_inbound(Inbound {
                bytes: envelope.to_bytes().unwrap(),
                from: PeerAddress::Loopback(7),
            })
            .await;
        assert!(matches!(
            event,
            FabricEvent::Rejected(RejectReason::Unauthenticated(_))
        ));
    }

    #[tokio::test]
    async fn garbage_traffic_does_not_stop_the_receive_loop() {
        let bus = LoopbackBus::new();
        let victim = fabric_on(&bus, "victim");
        let noisy = bus.endpoint();
        let good = fabric_on(&bus, "good");

        // Junk first...
        noisy.broadcast(b"not an envelope at all").await.unwrap();
        let event = timeout(SHORT, victim.receive_once())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            FabricEvent::Rejected(RejectReason::Malformed(_))
        ));

        // ...then a legitimate announcement still works.
        good.announce().await.unwrap();
        let event = timeout(SHORT, victim.receive_once())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, FabricEvent::PeerDiscovered(_)));
    }

    #[tokio::test]
    async fn a_partitioned_peer_becomes_unreachable_without_being_forgotten() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "a");
        let b = fabric_on(&bus, "b");

        a.announce().await.unwrap();
        timeout(SHORT, b.receive_once()).await.unwrap().unwrap();

        let entry = b.peer(&a.node_id()).await.unwrap();
        let much_later = Timestamp::now().plus_millis(60_000);

        assert!(entry.is_stale(much_later, b.config().peer_timeout_ms));
        assert_eq!(
            entry.effective_health(much_later, b.config().peer_timeout_ms),
            NodeHealth::Unreachable,
            "a silent peer is unreachable regardless of the health it last claimed"
        );
        // Still in the table: a rebooting node should not need rediscovering.
        assert_eq!(b.peer_count().await, 1);
        assert!(b.forget_stale_peers().await.is_empty());
    }

    #[tokio::test]
    async fn reachable_peers_excludes_silent_ones() {
        let bus = LoopbackBus::new();
        let a = fabric_on(&bus, "a");
        let b = fabric_on(&bus, "b");
        a.announce().await.unwrap();
        timeout(SHORT, b.receive_once()).await.unwrap().unwrap();
        assert_eq!(b.reachable_peers().await.len(), 1);
    }

    #[tokio::test]
    async fn metrics_record_discovery_and_rejection() {
        let metrics = Metrics::new();
        let bus = LoopbackBus::new();
        let identity = Arc::new(NodeIdentity::generate());
        let descriptor = descriptor_for(&identity, "metered");
        let enrolment = enrol(&identity);
        let fabric = Fabric::new(
            identity,
            enrolment,
            Arc::new(bus.endpoint()),
            descriptor,
            FabricConfig::default(),
            metrics.clone(),
        );
        let other = fabric_on(&bus, "other");

        other.announce().await.unwrap();
        timeout(SHORT, fabric.receive_once())
            .await
            .unwrap()
            .unwrap();

        let noisy = bus.endpoint();
        noisy.broadcast(b"junk").await.unwrap();
        timeout(SHORT, fabric.receive_once())
            .await
            .unwrap()
            .unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.counter("fabric.peer.discovered"), 1);
        assert_eq!(snapshot.counter("fabric.rejected.malformed"), 1);
        assert_eq!(snapshot.gauge("fabric.peers"), 1);
    }

    #[test]
    fn default_config_is_internally_consistent() {
        assert!(FabricConfig::default().validate().is_ok());
    }

    #[test]
    fn a_timeout_shorter_than_two_announce_intervals_is_rejected() {
        let config = FabricConfig {
            announce_interval_ms: 5_000,
            peer_timeout_ms: 6_000,
            peer_forget_ms: 60_000,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn forget_must_not_precede_timeout() {
        let config = FabricConfig {
            announce_interval_ms: 1_000,
            peer_timeout_ms: 10_000,
            peer_forget_ms: 5_000,
        };
        assert!(config.validate().is_err());
    }
}
