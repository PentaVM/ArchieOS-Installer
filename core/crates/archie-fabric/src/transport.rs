//! Transport abstraction.
//!
//! The fabric does not care how bytes move. [`Transport`] is the seam, and v0.1 ships
//! two implementations:
//!
//! * [`LoopbackTransport`] — an in-process bus. Lets an integration test run several
//!   complete Archie nodes inside one test binary with no sockets and no timing races.
//! * [`UdpTransport`] — real UDP over a real network.
//!
//! # On Zenoh
//!
//! The original specification named Zenoh as the preferred fabric transport. It is not
//! implemented in v0.1. Rather than ship a stub that pretends to work, the seam is
//! defined here so a `ZenohTransport` becomes an additional implementation of this
//! trait and nothing above it changes. See `docs/FABRIC_PROTOCOL.md` for exactly what
//! that task involves.

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::{broadcast, Mutex as AsyncMutex};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("transport is closed")]
    Closed,

    #[error("no peer address known for this transport")]
    NoRoute,

    #[error("message exceeds the transport's datagram limit: {size} bytes")]
    TooLarge { size: usize },
}

pub type Result<T> = std::result::Result<T, TransportError>;

/// Where a message came from, or is going to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PeerAddress {
    /// An endpoint on the in-process bus.
    Loopback(u64),
    Udp(SocketAddr),
}

impl fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loopback(id) => write!(f, "loopback:{id}"),
            Self::Udp(addr) => write!(f, "udp:{addr}"),
        }
    }
}

/// A received datagram, before any parsing or verification.
#[derive(Debug, Clone)]
pub struct Inbound {
    pub bytes: Vec<u8>,
    pub from: PeerAddress,
}

/// How bytes reach other nodes.
///
/// Implementations must be safe to share: the fabric holds one and uses it from both
/// its announce loop and its receive loop concurrently.
#[async_trait]
pub trait Transport: Send + Sync + fmt::Debug {
    /// Human-readable name, for logs and `archie status`.
    fn name(&self) -> &'static str;

    /// This endpoint's own address, so it can recognise and drop its own traffic.
    fn local_address(&self) -> PeerAddress;

    /// Send to every peer reachable on this transport.
    async fn broadcast(&self, bytes: &[u8]) -> Result<()>;

    /// Send to one specific peer.
    async fn send_to(&self, bytes: &[u8], peer: &PeerAddress) -> Result<()>;

    /// Await the next inbound datagram.
    ///
    /// Cancellation-safe: the fabric selects over this and a shutdown signal.
    async fn recv(&self) -> Result<Inbound>;
}

// --------------------------------------------------------------------- loopback

/// A shared in-process message bus.
///
/// Every [`LoopbackTransport`] attached to one bus can reach every other. This is what
/// makes the "node A discovers node B" integration test deterministic — no ports, no
/// firewall, no waiting on a network stack.
#[derive(Debug, Clone)]
pub struct LoopbackBus {
    sender: broadcast::Sender<(u64, Arc<Vec<u8>>)>,
    next_id: Arc<AtomicU64>,
    /// Endpoints that have been partitioned away from each other, for failure testing.
    partitions: Arc<Mutex<HashMap<u64, Vec<u64>>>>,
}

impl Default for LoopbackBus {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopbackBus {
    pub fn new() -> Self {
        // Capacity is generous: a slow receiver lagging is a test bug we want to see as
        // a dropped message rather than as a deadlock.
        let (sender, _) = broadcast::channel(1024);
        Self {
            sender,
            next_id: Arc::new(AtomicU64::new(1)),
            partitions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Attach a new endpoint.
    pub fn endpoint(&self) -> LoopbackTransport {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        LoopbackTransport {
            id,
            bus: self.clone(),
            receiver: AsyncMutex::new(self.sender.subscribe()),
        }
    }

    /// Stop delivering messages between `a` and `b`, in both directions.
    ///
    /// Used to test that the fabric degrades sanely when the network splits, rather
    /// than assuming every peer stays reachable forever.
    pub fn partition(&self, a: u64, b: u64) {
        if let Ok(mut map) = self.partitions.lock() {
            map.entry(a).or_default().push(b);
            map.entry(b).or_default().push(a);
        }
    }

    pub fn heal(&self) {
        if let Ok(mut map) = self.partitions.lock() {
            map.clear();
        }
    }

    fn is_partitioned(&self, from: u64, to: u64) -> bool {
        self.partitions
            .lock()
            .map(|map| map.get(&from).is_some_and(|blocked| blocked.contains(&to)))
            .unwrap_or(false)
    }
}

/// One endpoint on a [`LoopbackBus`].
#[derive(Debug)]
pub struct LoopbackTransport {
    id: u64,
    bus: LoopbackBus,
    receiver: AsyncMutex<broadcast::Receiver<(u64, Arc<Vec<u8>>)>>,
}

impl LoopbackTransport {
    pub fn id(&self) -> u64 {
        self.id
    }
}

#[async_trait]
impl Transport for LoopbackTransport {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn local_address(&self) -> PeerAddress {
        PeerAddress::Loopback(self.id)
    }

    async fn broadcast(&self, bytes: &[u8]) -> Result<()> {
        // A send with no subscribers is not an error: a single-node fabric is valid.
        let _ = self.bus.sender.send((self.id, Arc::new(bytes.to_vec())));
        Ok(())
    }

    async fn send_to(&self, bytes: &[u8], _peer: &PeerAddress) -> Result<()> {
        // The bus is a shared medium; receivers filter by sender. Unicast is modelled
        // as a broadcast the others ignore, which is enough for v0.1 semantics.
        self.broadcast(bytes).await
    }

    async fn recv(&self) -> Result<Inbound> {
        let mut receiver = self.receiver.lock().await;
        loop {
            match receiver.recv().await {
                Ok((sender_id, bytes)) => {
                    if sender_id == self.id {
                        continue; // never hear ourselves
                    }
                    if self.bus.is_partitioned(sender_id, self.id) {
                        continue;
                    }
                    return Ok(Inbound {
                        bytes: bytes.as_ref().clone(),
                        from: PeerAddress::Loopback(sender_id),
                    });
                }
                // Lagged means this endpoint fell behind. Skipping is correct: fabric
                // messages are periodic, so the next announce re-conveys the state.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Err(TransportError::Closed),
            }
        }
    }
}

// -------------------------------------------------------------------------- udp

/// Real UDP.
///
/// Discovery targets are explicit: a multicast group, a directed broadcast address, or
/// a static list of seed peers. Making them explicit rather than assuming multicast
/// means the fabric still forms on networks where multicast is filtered — which is most
/// corporate and cloud networks.
#[derive(Debug)]
pub struct UdpTransport {
    socket: tokio::net::UdpSocket,
    local: SocketAddr,
    targets: Vec<SocketAddr>,
}

impl UdpTransport {
    /// Bind and prepare to send to `targets`.
    pub async fn bind(bind_addr: SocketAddr, targets: Vec<SocketAddr>) -> Result<Self> {
        let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
        socket.set_broadcast(true).ok();
        let local = socket.local_addr()?;
        Ok(Self {
            socket,
            local,
            targets,
        })
    }

    /// Join an IPv4 multicast group so this node hears fabric traffic sent to it.
    ///
    /// Returns an error the caller may choose to tolerate: multicast being unavailable
    /// should degrade the fabric to seed-based discovery, not stop the daemon.
    pub fn join_multicast(
        &self,
        group: std::net::Ipv4Addr,
        interface: std::net::Ipv4Addr,
    ) -> Result<()> {
        self.socket.join_multicast_v4(group, interface)?;
        self.socket.set_multicast_loop_v4(true)?;
        Ok(())
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Add a peer discovered at runtime as a future unicast target.
    pub fn targets(&self) -> &[SocketAddr] {
        &self.targets
    }
}

#[async_trait]
impl Transport for UdpTransport {
    fn name(&self) -> &'static str {
        "udp"
    }

    fn local_address(&self) -> PeerAddress {
        PeerAddress::Udp(self.local)
    }

    async fn broadcast(&self, bytes: &[u8]) -> Result<()> {
        if self.targets.is_empty() {
            return Err(TransportError::NoRoute);
        }
        // One unreachable target must not stop the others: a fabric with a dead seed
        // should still form among the live peers.
        let mut last_error = None;
        let mut delivered = false;
        for target in &self.targets {
            match self.socket.send_to(bytes, target).await {
                Ok(_) => delivered = true,
                Err(e) => last_error = Some(e),
            }
        }
        if delivered {
            Ok(())
        } else {
            Err(last_error
                .map(TransportError::Io)
                .unwrap_or(TransportError::NoRoute))
        }
    }

    async fn send_to(&self, bytes: &[u8], peer: &PeerAddress) -> Result<()> {
        let PeerAddress::Udp(addr) = peer else {
            return Err(TransportError::NoRoute);
        };
        self.socket.send_to(bytes, addr).await?;
        Ok(())
    }

    async fn recv(&self) -> Result<Inbound> {
        // Sized to the envelope limit; anything larger is truncated by the OS and will
        // fail signature verification, which is the correct outcome.
        let mut buffer = vec![0u8; crate::envelope::MAX_ENVELOPE_BYTES];
        let (len, from) = self.socket.recv_from(&mut buffer).await?;
        buffer.truncate(len);
        Ok(Inbound {
            bytes: buffer,
            from: PeerAddress::Udp(from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    const SHORT: Duration = Duration::from_millis(500);

    #[tokio::test]
    async fn loopback_delivers_between_endpoints() {
        let bus = LoopbackBus::new();
        let a = bus.endpoint();
        let b = bus.endpoint();

        a.broadcast(b"hello").await.unwrap();
        let received = timeout(SHORT, b.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"hello");
        assert_eq!(received.from, a.local_address());
    }

    #[tokio::test]
    async fn loopback_never_echoes_to_the_sender() {
        let bus = LoopbackBus::new();
        let a = bus.endpoint();
        let _b = bus.endpoint();

        a.broadcast(b"mine").await.unwrap();
        // If a node heard its own announce it would register itself as a peer.
        assert!(timeout(Duration::from_millis(120), a.recv()).await.is_err());
    }

    #[tokio::test]
    async fn loopback_reaches_every_other_endpoint() {
        let bus = LoopbackBus::new();
        let a = bus.endpoint();
        let b = bus.endpoint();
        let c = bus.endpoint();

        a.broadcast(b"fanout").await.unwrap();
        for endpoint in [&b, &c] {
            let received = timeout(SHORT, endpoint.recv()).await.unwrap().unwrap();
            assert_eq!(received.bytes, b"fanout");
        }
    }

    #[tokio::test]
    async fn a_single_endpoint_broadcast_is_not_an_error() {
        let bus = LoopbackBus::new();
        let lonely = bus.endpoint();
        // A one-node fabric is a legitimate state, not a failure.
        assert!(lonely.broadcast(b"anyone there").await.is_ok());
    }

    #[tokio::test]
    async fn a_partition_blocks_delivery_in_both_directions() {
        let bus = LoopbackBus::new();
        let a = bus.endpoint();
        let b = bus.endpoint();
        bus.partition(a.id(), b.id());

        a.broadcast(b"blocked").await.unwrap();
        assert!(timeout(Duration::from_millis(120), b.recv()).await.is_err());

        b.broadcast(b"also blocked").await.unwrap();
        assert!(timeout(Duration::from_millis(120), a.recv()).await.is_err());
    }

    #[tokio::test]
    async fn healing_a_partition_restores_delivery() {
        let bus = LoopbackBus::new();
        let a = bus.endpoint();
        let b = bus.endpoint();
        bus.partition(a.id(), b.id());
        a.broadcast(b"lost").await.unwrap();
        assert!(timeout(Duration::from_millis(100), b.recv()).await.is_err());

        bus.heal();
        a.broadcast(b"delivered").await.unwrap();
        let received = timeout(SHORT, b.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"delivered");
    }

    #[tokio::test]
    async fn udp_delivers_between_two_real_sockets() {
        // Bind ephemeral ports, then point each at the other.
        let a_probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let a_addr = a_probe.local_addr().unwrap();
        let b_probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b_addr = b_probe.local_addr().unwrap();
        drop(a_probe);
        drop(b_probe);

        let a = UdpTransport::bind(a_addr, vec![b_addr]).await.unwrap();
        let b = UdpTransport::bind(b_addr, vec![a_addr]).await.unwrap();

        a.broadcast(b"over the wire").await.unwrap();
        let received = timeout(SHORT, b.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"over the wire");
        assert_eq!(received.from, PeerAddress::Udp(a_addr));
    }

    #[tokio::test]
    async fn udp_unicast_reaches_a_specific_peer() {
        let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b_addr = probe.local_addr().unwrap();
        drop(probe);

        let a = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), vec![])
            .await
            .unwrap();
        let b = UdpTransport::bind(b_addr, vec![]).await.unwrap();

        a.send_to(b"direct", &PeerAddress::Udp(b_addr))
            .await
            .unwrap();
        let received = timeout(SHORT, b.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"direct");
    }

    #[tokio::test]
    async fn udp_broadcast_without_targets_reports_no_route() {
        let a = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), vec![])
            .await
            .unwrap();
        assert!(matches!(
            a.broadcast(b"nowhere").await.unwrap_err(),
            TransportError::NoRoute
        ));
    }

    #[tokio::test]
    async fn udp_survives_one_dead_target_among_several() {
        let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let live_addr = probe.local_addr().unwrap();
        drop(probe);
        let live = UdpTransport::bind(live_addr, vec![]).await.unwrap();

        // 127.0.0.1:1 is reserved and nothing listens there.
        let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let sender = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), vec![dead, live_addr])
            .await
            .unwrap();

        sender.broadcast(b"partial").await.unwrap();
        let received = timeout(SHORT, live.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"partial");
    }

    #[tokio::test]
    async fn transports_are_usable_behind_a_trait_object() {
        // The point of the seam: the fabric holds `Arc<dyn Transport>`, so adding a
        // Zenoh implementation later changes nothing above this line.
        let bus = LoopbackBus::new();
        let a: Arc<dyn Transport> = Arc::new(bus.endpoint());
        let b: Arc<dyn Transport> = Arc::new(bus.endpoint());
        assert_eq!(a.name(), "loopback");

        a.broadcast(b"dyn").await.unwrap();
        let received = timeout(SHORT, b.recv()).await.unwrap().unwrap();
        assert_eq!(received.bytes, b"dyn");
    }
}
