//! Two households, one network.
//!
//! `membership.rs` checks that the certificate logic is right. This checks that it is
//! actually *wired in* — that a stranger's announcement is refused by the same code path
//! a real packet takes, rather than by a rule nobody calls.
//!
//! The distinction matters more than it sounds. Every existing fabric test builds its
//! nodes under one owner, so all of them would still pass if the admission check were
//! removed entirely. The scenario nobody had was the one that actually happens: someone
//! else installs ArchieOS on the same wire.

use archie_fabric::{
    Enrolment, Fabric, FabricConfig, FabricEvent, FabricMessage, NodeIdentity, OwnerIdentity,
    RejectReason, SignedEnvelope,
};
use archie_observability::Metrics;
use archie_types::{
    Capability, CpuInfo, MemoryInfo, NodeDescriptor, NodeHealth, NodeLoad, NodeProfile, Timestamp,
};
use std::sync::Arc;

use archie_fabric::transport::{Inbound, LoopbackBus, PeerAddress};

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

struct Household {
    owner: OwnerIdentity,
}

impl Household {
    fn new() -> Self {
        Self {
            owner: OwnerIdentity::generate(),
        }
    }

    fn node(&self, bus: &LoopbackBus, hostname: &str) -> Arc<Fabric> {
        let identity = Arc::new(NodeIdentity::generate());
        let enrolment = Enrolment::new(
            &identity.public_key_hex(),
            self.owner.admit(&identity.public_key_hex(), 1),
        )
        .expect("own owner admits own node");
        let descriptor = descriptor_for(&identity, hostname);
        Arc::new(Fabric::new(
            identity,
            enrolment,
            Arc::new(bus.endpoint()),
            descriptor,
            FabricConfig::default(),
            Metrics::new(),
        ))
    }

    /// An announcement as this household's node would put it on the wire.
    fn announcement(&self, hostname: &str) -> Vec<u8> {
        let identity = NodeIdentity::generate();
        let descriptor = descriptor_for(&identity, hostname);
        let membership = self.owner.admit(&identity.public_key_hex(), 1);
        SignedEnvelope::seal(
            &identity,
            Some(&membership),
            &FabricMessage::Announce {
                descriptor: Box::new(descriptor),
            },
        )
        .expect("seal")
        .to_bytes()
        .expect("encode")
    }
}

/// The control. Without this the refusals below could be refusing everything.
#[tokio::test]
async fn a_node_of_the_same_household_is_discovered() {
    let bus = LoopbackBus::new();
    let home = Household::new();
    let listener = home.node(&bus, "listener");

    let event = listener
        .handle_inbound(Inbound {
            bytes: home.announcement("sibling"),
            from: PeerAddress::Loopback(1),
        })
        .await;

    assert!(
        matches!(event, FabricEvent::PeerDiscovered(_)),
        "got {event:?}"
    );
    assert_eq!(listener.peer_count().await, 1);
}

/// The requirement, through the real packet path.
///
/// Nelson's node and a guest's node, on one wire, each announcing honestly. Neither has
/// been configured to know the other exists, and neither joins the other.
#[tokio::test]
async fn a_neighbours_install_does_not_join_this_fabric() {
    let bus = LoopbackBus::new();
    let nelson = Household::new();
    let guest = Household::new();

    let nelsons_node = nelson.node(&bus, "atlas");
    let guests_node = guest.node(&bus, "their-laptop");

    let event = nelsons_node
        .handle_inbound(Inbound {
            bytes: guest.announcement("their-laptop"),
            from: PeerAddress::Loopback(2),
        })
        .await;
    assert!(
        matches!(event, FabricEvent::Rejected(RejectReason::NotAMember(_))),
        "a guest install joined the owner's fabric: {event:?}"
    );
    assert_eq!(nelsons_node.peer_count().await, 0);

    // And the other way, which is the half a one-sided allowlist would miss.
    let event = guests_node
        .handle_inbound(Inbound {
            bytes: nelson.announcement("atlas"),
            from: PeerAddress::Loopback(3),
        })
        .await;
    assert!(
        matches!(event, FabricEvent::Rejected(RejectReason::NotAMember(_))),
        "the owner's fabric was joined by a guest node: {event:?}"
    );
    assert_eq!(guests_node.peer_count().await, 0);
}

/// A perfectly genuine node that nobody invited.
///
/// This is what the fabric used to accept: the signature verifies, the descriptor is
/// consistent, the protocol version matches. Everything about it is authentic. It was
/// simply never admitted, and authentication was never the same question as admission.
#[tokio::test]
async fn an_uninvited_node_is_refused_however_genuine_it_is() {
    let bus = LoopbackBus::new();
    let home = Household::new();
    let listener = home.node(&bus, "listener");

    let stranger = NodeIdentity::generate();
    let envelope = SignedEnvelope::seal(
        &stranger,
        None,
        &FabricMessage::Announce {
            descriptor: Box::new(descriptor_for(&stranger, "stranger")),
        },
    )
    .expect("seal")
    .to_bytes()
    .expect("encode");

    let event = listener
        .handle_inbound(Inbound {
            bytes: envelope,
            from: PeerAddress::Loopback(4),
        })
        .await;

    assert!(
        matches!(event, FabricEvent::Rejected(RejectReason::NotAMember(_))),
        "got {event:?}"
    );
    assert_eq!(listener.peer_count().await, 0);
}

/// A machine that has been installed but not yet told what it belongs to.
#[tokio::test]
async fn an_unenrolled_node_talks_to_nobody() {
    let bus = LoopbackBus::new();
    let home = Household::new();

    let identity = Arc::new(NodeIdentity::generate());
    let fresh = Fabric::new(
        identity.clone(),
        Enrolment::none(),
        Arc::new(bus.endpoint()),
        descriptor_for(&identity, "fresh-install"),
        FabricConfig::default(),
        Metrics::new(),
    );

    assert!(!fresh.is_enrolled());
    assert_eq!(fresh.fabric_fingerprint(), None);

    // Even a valid member of a real household is refused, because this node has no
    // household to compare it against. Belonging to nothing is not belonging to
    // everything.
    let event = fresh
        .handle_inbound(Inbound {
            bytes: home.announcement("atlas"),
            from: PeerAddress::Loopback(5),
        })
        .await;
    assert!(
        matches!(event, FabricEvent::Rejected(RejectReason::NotAMember(_))),
        "got {event:?}"
    );
    assert_eq!(fresh.peer_count().await, 0);
}

/// Membership travels on every message, not only on announcements, so a probe from a
/// stranger cannot make us announce ourselves to them.
#[tokio::test]
async fn a_stranger_cannot_probe_us_into_answering() {
    let bus = LoopbackBus::new();
    let home = Household::new();
    let listener = home.node(&bus, "listener");

    let stranger = NodeIdentity::generate();
    let probe = SignedEnvelope::seal(&stranger, None, &FabricMessage::Probe)
        .expect("seal")
        .to_bytes()
        .expect("encode");

    let event = listener
        .handle_inbound(Inbound {
            bytes: probe,
            from: PeerAddress::Loopback(6),
        })
        .await;

    assert!(
        matches!(event, FabricEvent::Rejected(RejectReason::NotAMember(_))),
        "a probe is a request to be told about us, and answering a stranger's is how a \
         scan of the subnet becomes an inventory: {event:?}"
    );
}

/// Two nodes admitted by the same owner report the same fabric, so an operator can see
/// that two installs meant to be together are together — and that two meant to be apart
/// are apart.
#[tokio::test]
async fn nodes_show_which_fabric_they_are_in() {
    let bus = LoopbackBus::new();
    let home = Household::new();
    let guest = Household::new();

    let a = home.node(&bus, "a");
    let b = home.node(&bus, "b");
    let theirs = guest.node(&bus, "theirs");

    assert_eq!(a.fabric_fingerprint(), b.fabric_fingerprint());
    assert!(a.fabric_fingerprint().is_some());
    assert_ne!(a.fabric_fingerprint(), theirs.fabric_fingerprint());
}
