//! Chromecast (CAST v2) over TLS on TCP 8009.
//!
//! The channel is the documented sender/receiver protocol: each frame is a
//! 4-byte big-endian length followed by a protobuf `CastMessage`
//! ([`wire::CastMessage`]); the payloads used here are JSON strings on three
//! namespaces — `tp.connection` (CONNECT/CLOSE), `tp.heartbeat` (PING/PONG)
//! and `receiver` (GET_STATUS, LAUNCH, STOP, SET_VOLUME → RECEIVER_STATUS).
//!
//! Every action is verified the way Roku and MQTT are: `apply` only sends the
//! request and waits for the receiver to answer that request id; the engine
//! then asks again with a fresh GET_STATUS and the postcondition is checked
//! against that RECEIVER_STATUS ([`ResourceState::Cast`]). A reply to LAUNCH
//! is an acknowledgement, never the evidence.
//!
//! ## What the TLS session verifies, exactly
//!
//! A Chromecast presents a self-signed certificate that chains to nothing a
//! client could check by name. This adapter does **not** accept any
//! certificate:
//!
//! - The leaf certificate's SHA-256 (over its DER bytes) must equal the
//!   fingerprint the operator pinned for that resource ([`CastDevice::certificate_sha256`]).
//!   Anything else fails the handshake and the operation returns
//!   `AdapterError::Authentication` before a single CAST byte is sent.
//! - The TLS handshake signature is verified against that certificate's
//!   public key (rustls with ring), so the peer holds the pinned key; a
//!   replayed certificate without the key does not complete the handshake.
//! - [`probe_certificate`] is the only way to learn a fingerprint. It
//!   completes a handshake, returns the fingerprint for the operator to
//!   compare and approve, and sends nothing. Nothing is trusted on first use
//!   in silence: the pin lives in configuration the operator wrote.
//!
//! What is **not** verified:
//!
//! - No chain, name, or validity dates: the pin is the whole trust anchor.
//! - The Cast device-authentication challenge (`tp.deviceauth`, a signature
//!   chained to Google's Cast root CA) is not performed. A pinned session
//!   proves "the same device the operator paired", not "a genuine Google
//!   device".
//! - Cast devices are reported to regenerate their TLS certificate
//!   periodically. When that happens the pin no longer matches and every
//!   operation fails closed with `Authentication` until the operator
//!   re-approves the new fingerprint. Not measured on real hardware.
//! - A certificate webpki cannot parse (for example X.509 v1) cannot complete
//!   the signature check and fails closed.
//!
//! Media play/pause needs the per-application media channel and is
//! `Unsupported` here. Nothing in this module has run against a physical
//! Chromecast: the tests talk to an in-process CAST v2 receiver over TLS.
use crate::{
    contracts::{validate_id, Command, CommandKind, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

/// CAST v2 wire format: the protobuf message and its length-prefixed frame.
pub mod wire {
    use crate::engine::AdapterError;
    use prost::Message;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    /// Largest frame a receiver accepts (64 KiB); larger is a protocol error.
    pub const MAX_FRAME: usize = 65_536;
    pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
    pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
    pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
    pub const SENDER: &str = "sender-0";
    pub const RECEIVER: &str = "receiver-0";
    /// `PayloadType.STRING`.
    pub const PAYLOAD_STRING: i32 = 0;
    /// `PayloadType.BINARY`.
    pub const PAYLOAD_BINARY: i32 = 1;

    /// `cast_channel.proto` `CastMessage` (proto2, all of it).
    #[derive(Clone, PartialEq, Message)]
    pub struct CastMessage {
        /// `ProtocolVersion.CASTV2_1_0` = 0.
        #[prost(int32, required, tag = "1")]
        pub protocol_version: i32,
        #[prost(string, required, tag = "2")]
        pub source_id: String,
        #[prost(string, required, tag = "3")]
        pub destination_id: String,
        #[prost(string, required, tag = "4")]
        pub namespace: String,
        #[prost(int32, required, tag = "5")]
        pub payload_type: i32,
        #[prost(string, optional, tag = "6")]
        pub payload_utf8: Option<String>,
        #[prost(bytes = "vec", optional, tag = "7")]
        pub payload_binary: Option<Vec<u8>>,
    }
    impl CastMessage {
        pub fn json(source: &str, destination: &str, namespace: &str, payload: String) -> Self {
            Self {
                protocol_version: 0,
                source_id: source.into(),
                destination_id: destination.into(),
                namespace: namespace.into(),
                payload_type: PAYLOAD_STRING,
                payload_utf8: Some(payload),
                payload_binary: None,
            }
        }
    }

    /// Length prefix plus encoded message.
    pub fn encode_frame(message: &CastMessage) -> Result<Vec<u8>, AdapterError> {
        let length = message.encoded_len();
        if length > MAX_FRAME {
            return Err(AdapterError::Protocol);
        }
        let mut out = Vec::with_capacity(4 + length);
        out.extend_from_slice(&(length as u32).to_be_bytes());
        message
            .encode(&mut out)
            .map_err(|_| AdapterError::Protocol)?;
        Ok(out)
    }
    pub async fn write_frame<W: AsyncWrite + Unpin>(
        writer: &mut W,
        message: &CastMessage,
    ) -> Result<(), AdapterError> {
        let frame = encode_frame(message)?;
        writer
            .write_all(&frame)
            .await
            .map_err(|_| AdapterError::Transport)?;
        writer.flush().await.map_err(|_| AdapterError::Transport)
    }
    /// One frame. The length is checked before any body byte is buffered.
    pub async fn read_frame<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> Result<CastMessage, AdapterError> {
        let mut prefix = [0u8; 4];
        reader
            .read_exact(&mut prefix)
            .await
            .map_err(|_| AdapterError::Transport)?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err(AdapterError::Protocol);
        }
        let mut body = vec![0u8; length];
        reader
            .read_exact(&mut body)
            .await
            .map_err(|_| AdapterError::Transport)?;
        CastMessage::decode(body.as_slice()).map_err(|_| AdapterError::Protocol)
    }
}

/// CAST v2 control port.
pub const CAST_PORT: u16 = 8009;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
/// Frames read while waiting for one reply; a chatty peer cannot hold us.
const MAX_FRAMES_PER_REPLY: usize = 64;
/// Applications listed in one RECEIVER_STATUS.
const MAX_APPLICATIONS: usize = 16;
const MAX_SESSION_ID: usize = 128;

/// A paired Chromecast: address from LAN discovery plus the certificate
/// fingerprint the operator approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastDevice {
    /// Stable Archie resource id for the device.
    pub resource_id: String,
    /// Friendly name, mDNS TXT `fn`.
    pub name: String,
    /// Model name, mDNS TXT `md` (e.g. "Chromecast", "Nest Hub").
    pub model: String,
    /// Device address from the discovery record; loopback or private only.
    pub address: IpAddr,
    /// CAST v2 control port, conventionally 8009.
    pub port: u16,
    /// Lowercase hex SHA-256 of the device's TLS certificate (DER), approved
    /// by the operator after [`probe_certificate`].
    pub certificate_sha256: String,
}
impl CastDevice {
    pub fn new(
        resource_id: &str,
        name: &str,
        model: &str,
        address: IpAddr,
        port: u16,
        certificate_sha256: &str,
    ) -> Result<Self, AdapterError> {
        validate_id(resource_id).map_err(|_| AdapterError::Protocol)?;
        let bounded =
            |s: &str| !s.is_empty() && s.len() <= 64 && !s.bytes().any(|b| b < 0x20 || b == 0x7f);
        if !bounded(name)
            || !bounded(model)
            || !permitted(address)
            || port == 0
            || parse_pin(certificate_sha256).is_none()
        {
            return Err(AdapterError::Protocol);
        }
        Ok(Self {
            resource_id: resource_id.into(),
            name: name.into(),
            model: model.into(),
            address,
            port,
            certificate_sha256: certificate_sha256.into(),
        })
    }
    fn socket(&self) -> SocketAddr {
        SocketAddr::new(self.address, self.port)
    }
}

fn permitted(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
    }
}
fn parse_pin(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Accepts exactly one leaf certificate (by SHA-256 of its DER) and verifies
/// the handshake signature with that certificate's key. In probe mode it
/// records the fingerprint instead of comparing it; the signature check stays.
#[derive(Debug)]
struct PinnedCertificate {
    pin: Option<[u8; 32]>,
    seen: Arc<Mutex<Option<[u8; 32]>>>,
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let digest: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(digest);
        }
        match self.pin {
            None => Ok(ServerCertVerified::assertion()),
            Some(pin) if pin == digest => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

type CastStream = tokio_rustls::client::TlsStream<TcpStream>;

async fn open_tls(
    socket: SocketAddr,
    pin: Option<[u8; 32]>,
) -> Result<(CastStream, Option<[u8; 32]>), AdapterError> {
    if !permitted(socket.ip()) {
        return Err(AdapterError::Protocol);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let seen = Arc::new(Mutex::new(None));
    let verifier = PinnedCertificate {
        pin,
        seen: seen.clone(),
        provider: provider.clone(),
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| AdapterError::Protocol)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(socket))
        .await
        .map_err(|_| AdapterError::Transport)?
        .map_err(|_| AdapterError::Transport)?;
    let _ = tcp.set_nodelay(true);
    let name = ServerName::IpAddress(socket.ip().into());
    let stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TlsConnector::from(Arc::new(config)).connect(name, tcp),
    )
    .await
    .map_err(|_| AdapterError::Transport)?
    .map_err(|error| {
        let pin_refused = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .is_some_and(|e| matches!(e, rustls::Error::InvalidCertificate(_)));
        if pin_refused {
            AdapterError::Authentication
        } else {
            AdapterError::Transport
        }
    })?;
    let fingerprint = *seen.lock().map_err(|_| AdapterError::Protocol)?;
    Ok((stream, fingerprint))
}

/// Complete one TLS handshake and return the lowercase hex SHA-256 of the
/// certificate the device presented, for the operator to compare and pin.
/// No CAST message is sent and nothing is stored or trusted by this call.
pub async fn probe_certificate(address: IpAddr, port: u16) -> Result<String, AdapterError> {
    let (mut stream, seen) = open_tls(SocketAddr::new(address, port), None).await?;
    let _ = stream.shutdown().await;
    seen.map(|d| to_hex(&d)).ok_or(AdapterError::Protocol)
}

/// One sender connection to `receiver-0`.
struct Channel<S> {
    stream: S,
    next_request: u32,
}
impl<S: AsyncRead + AsyncWrite + Unpin> Channel<S> {
    async fn open(stream: S) -> Result<Self, AdapterError> {
        let mut channel = Self {
            stream,
            next_request: 1,
        };
        channel
            .send(wire::NS_CONNECTION, json!({"type": "CONNECT"}))
            .await?;
        Ok(channel)
    }
    async fn send(&mut self, namespace: &str, payload: Value) -> Result<(), AdapterError> {
        let message =
            wire::CastMessage::json(wire::SENDER, wire::RECEIVER, namespace, payload.to_string());
        wire::write_frame(&mut self.stream, &message).await
    }
    /// Send a receiver request and return the reply carrying its request id.
    /// PINGs met on the way are answered with PONG; unsolicited status
    /// broadcasts and other namespaces are skipped.
    async fn request(&mut self, mut payload: Value) -> Result<Value, AdapterError> {
        let id = self.next_request;
        self.next_request = self.next_request.wrapping_add(1).max(1);
        payload["requestId"] = json!(id);
        self.send(wire::NS_RECEIVER, payload).await?;
        for _ in 0..MAX_FRAMES_PER_REPLY {
            let message = wire::read_frame(&mut self.stream).await?;
            if message.payload_type != wire::PAYLOAD_STRING {
                continue;
            }
            let Some(text) = message.payload_utf8.as_deref() else {
                return Err(AdapterError::Protocol);
            };
            let body: Value = serde_json::from_str(text).map_err(|_| AdapterError::Protocol)?;
            let kind = body.get("type").and_then(Value::as_str).unwrap_or("");
            match message.namespace.as_str() {
                wire::NS_HEARTBEAT if kind == "PING" => {
                    self.send(wire::NS_HEARTBEAT, json!({"type": "PONG"}))
                        .await?;
                }
                wire::NS_CONNECTION if kind == "CLOSE" => return Err(AdapterError::Unavailable),
                wire::NS_RECEIVER
                    if body.get("requestId").and_then(Value::as_u64) == Some(u64::from(id)) =>
                {
                    return Ok(body);
                }
                _ => {}
            }
        }
        Err(AdapterError::Protocol)
    }
    async fn close(mut self) {
        let _ = self
            .send(wire::NS_CONNECTION, json!({"type": "CLOSE"}))
            .await;
        let _ = self.stream.shutdown().await;
    }
}

/// What one RECEIVER_STATUS says, already validated.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiverStatus {
    state: ResourceState,
    /// Session of the running non-idle application, for STOP.
    session: Option<String>,
}

fn parse_status(reply: &Value) -> Result<ReceiverStatus, AdapterError> {
    match reply.get("type").and_then(Value::as_str) {
        Some("RECEIVER_STATUS") => {}
        Some("LAUNCH_ERROR") | Some("INVALID_REQUEST") => return Err(AdapterError::Unavailable),
        _ => return Err(AdapterError::Protocol),
    }
    let status = reply.get("status").ok_or(AdapterError::Protocol)?;
    let mut active_app = None;
    let mut session = None;
    if let Some(apps) = status.get("applications") {
        let apps = apps.as_array().ok_or(AdapterError::Protocol)?;
        if apps.len() > MAX_APPLICATIONS {
            return Err(AdapterError::Protocol);
        }
        for app in apps {
            let id = app
                .get("appId")
                .and_then(Value::as_str)
                .ok_or(AdapterError::Protocol)?;
            validate_id(id).map_err(|_| AdapterError::Protocol)?;
            let idle = match app.get("isIdleScreen") {
                None => false,
                Some(v) => v.as_bool().ok_or(AdapterError::Protocol)?,
            };
            if idle || active_app.is_some() {
                continue;
            }
            let sid = app
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or(AdapterError::Protocol)?;
            if sid.is_empty()
                || sid.len() > MAX_SESSION_ID
                || sid.bytes().any(|b| b.is_ascii_control())
            {
                return Err(AdapterError::Protocol);
            }
            active_app = Some(id.to_owned());
            session = Some(sid.to_owned());
        }
    }
    let (mut volume_percent, mut muted) = (None, None);
    if let Some(volume) = status.get("volume") {
        let fixed = volume.get("controlType").and_then(Value::as_str) == Some("fixed");
        if let Some(level) = volume.get("level") {
            let level = level.as_f64().ok_or(AdapterError::Protocol)?;
            if !level.is_finite() || !(0.0..=1.0).contains(&level) {
                return Err(AdapterError::Protocol);
            }
            if !fixed {
                volume_percent = Some((level * 100.0).round() as u8);
            }
        }
        if let Some(m) = volume.get("muted") {
            muted = Some(m.as_bool().ok_or(AdapterError::Protocol)?);
        }
    }
    let standby = match status.get("isStandBy") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or(AdapterError::Protocol)?),
    };
    Ok(ReceiverStatus {
        state: ResourceState::Cast {
            active_app,
            volume_percent,
            muted,
            standby,
        },
        session,
    })
}

pub struct Chromecast {
    edge: String,
    adapter: String,
    devices: BTreeMap<String, CastDevice>,
    /// Session observed by `preflight` for an AppStop, keyed by resource.
    stop_session: BTreeMap<String, String>,
}
impl std::fmt::Debug for Chromecast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chromecast")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("devices", &self.devices.len())
            .finish()
    }
}
impl Chromecast {
    /// Capabilities this adapter can verify against RECEIVER_STATUS.
    pub const CAPABILITIES: [CommandKind; 4] = [
        CommandKind::AppLaunch,
        CommandKind::AppStop,
        CommandKind::VolumeSet,
        CommandKind::MuteSet,
    ];
    pub fn new(edge: &str, adapter: &str, devices: Vec<CastDevice>) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        if devices.len() > 4096 {
            return Err(AdapterError::Protocol);
        }
        let mut map = BTreeMap::new();
        for device in devices {
            if map.insert(device.resource_id.clone(), device).is_some() {
                return Err(AdapterError::Protocol);
            }
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            devices: map,
            stop_session: BTreeMap::new(),
        })
    }
    /// The recorded descriptor for a paired device, if known.
    pub fn descriptor(&self, resource_id: &str) -> Option<&CastDevice> {
        self.devices.get(resource_id)
    }
    fn device_of(&self, resource: &ResourceDescriptor) -> Result<&CastDevice, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.devices
            .get(&resource.resource_id)
            .ok_or(AdapterError::Unsupported)
    }
    async fn connect(device: &CastDevice) -> Result<Channel<CastStream>, AdapterError> {
        let pin = parse_pin(&device.certificate_sha256).ok_or(AdapterError::Protocol)?;
        let (stream, _) = open_tls(device.socket(), Some(pin)).await?;
        Channel::open(stream).await
    }
    async fn status(device: &CastDevice) -> Result<ReceiverStatus, AdapterError> {
        let work = async {
            let mut channel = Self::connect(device).await?;
            let reply = channel.request(json!({"type": "GET_STATUS"})).await;
            channel.close().await;
            parse_status(&reply?)
        };
        tokio::time::timeout(OPERATION_TIMEOUT, work)
            .await
            .map_err(|_| AdapterError::Transport)?
    }
}

#[async_trait]
impl EdgeAdapter for Chromecast {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    async fn observe(
        &mut self,
        resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        let device = self.device_of(resource)?.clone();
        let status = Self::status(&device).await?;
        // Evidence digests the measured state alone; names, addresses and
        // session ids are not inputs.
        let report = serde_json::to_vec(&(&resource.resource_id, &status.state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id: format!("cast-{:x}", Sha256::digest(report)),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state: status.state,
        })
    }
    /// STOP names a session, so the session is read here, before Core's final
    /// check; nothing is actuated. If the session changes before `apply`, the
    /// receiver refuses or ignores the stale STOP and the postcondition shows it.
    async fn preflight(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<(), AdapterError> {
        self.stop_session.remove(&resource.resource_id);
        if matches!(command, Command::AppStop) {
            let device = self.device_of(resource)?.clone();
            let status = Self::status(&device).await?;
            let session = status.session.ok_or(AdapterError::Unavailable)?;
            self.stop_session
                .insert(resource.resource_id.clone(), session);
        }
        Ok(())
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        if !Self::CAPABILITIES.contains(&command.capability())
            || !resource.capabilities.contains(&command.capability())
        {
            return Err(AdapterError::Unsupported);
        }
        let device = self.device_of(resource)?.clone();
        let payload = match command {
            // Contract-validated ids are [A-Za-z0-9._-]; Cast app ids fit.
            Command::AppLaunch { app_id } => json!({"type": "LAUNCH", "appId": app_id}),
            Command::AppStop => {
                let session = self
                    .stop_session
                    .remove(&resource.resource_id)
                    .ok_or(AdapterError::Unavailable)?;
                json!({"type": "STOP", "sessionId": session})
            }
            Command::VolumeSet { percent } => {
                json!({"type": "SET_VOLUME", "volume": {"level": f64::from(*percent) / 100.0}})
            }
            Command::MuteSet { muted } => {
                json!({"type": "SET_VOLUME", "volume": {"muted": muted}})
            }
            _ => return Err(AdapterError::Unsupported),
        };
        let work = async {
            let mut channel = Self::connect(&device).await?;
            let reply = channel.request(payload).await;
            channel.close().await;
            // The acknowledgement must be a well-formed RECEIVER_STATUS for
            // this request; whether it shows the requested state is decided
            // by the engine's own observation afterwards, not here.
            parse_status(&reply?).map(|_| ())
        };
        tokio::time::timeout(OPERATION_TIMEOUT, work)
            .await
            .map_err(|_| AdapterError::Transport)??;
        Ok(AdapterAck { accepted: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    /// Golden bytes written by hand from the proto2 encoding rules, so the
    /// codec is not only checked against itself.
    #[test]
    fn connect_frame_matches_hand_encoded_protobuf() {
        let message = wire::CastMessage::json("s", "r", "n", "{}".into());
        let mut expected = vec![
            0x08, 0x00, // 1: protocol_version = 0
            0x12, 0x01, b's', // 2: source_id
            0x1a, 0x01, b'r', // 3: destination_id
            0x22, 0x01, b'n', // 4: namespace
            0x28, 0x00, // 5: payload_type = STRING
            0x32, 0x02, b'{', b'}', // 6: payload_utf8
        ];
        assert_eq!(message.encode_to_vec(), expected);
        let frame = wire::encode_frame(&message).unwrap();
        let mut prefixed = (expected.len() as u32).to_be_bytes().to_vec();
        prefixed.append(&mut expected);
        assert_eq!(frame, prefixed);
    }

    #[test]
    fn status_parsing_separates_idle_screen_and_quantizes_volume() {
        let reply = json!({"type":"RECEIVER_STATUS","requestId":1,"status":{
            "applications":[
                {"appId":"E8C28D3C","isIdleScreen":true,"sessionId":"idle"},
                {"appId":"CC1AD845","isIdleScreen":false,"sessionId":"s-1"}],
            "volume":{"controlType":"attenuation","level":0.35,"muted":false},
            "isStandBy":false}});
        let status = parse_status(&reply).unwrap();
        assert_eq!(
            status.state,
            ResourceState::Cast {
                active_app: Some("CC1AD845".into()),
                volume_percent: Some(35),
                muted: Some(false),
                standby: Some(false),
            }
        );
        assert_eq!(status.session.as_deref(), Some("s-1"));
        let fixed = json!({"type":"RECEIVER_STATUS","status":{
            "volume":{"controlType":"fixed","level":1.0,"muted":false}}});
        assert!(matches!(
            parse_status(&fixed).unwrap().state,
            ResourceState::Cast {
                volume_percent: None,
                active_app: None,
                ..
            }
        ));
        for bad in [
            json!({"type":"RECEIVER_STATUS"}),
            json!({"type":"RECEIVER_STATUS","status":{"volume":{"level":1.5}}}),
            json!({"type":"RECEIVER_STATUS","status":{"applications":[{"appId":"bad id"}]}}),
            json!({"type":"RECEIVER_STATUS","status":{"applications":[{"appId":"A"}]}}),
            json!({"type":"MEDIA_STATUS","status":{}}),
        ] {
            assert_eq!(parse_status(&bad), Err(AdapterError::Protocol), "{bad}");
        }
        assert_eq!(
            parse_status(&json!({"type":"LAUNCH_ERROR","reason":"NOT_FOUND"})),
            Err(AdapterError::Unavailable)
        );
    }

    #[test]
    fn pins_are_lowercase_sha256_hex_only() {
        let good = "ab".repeat(32);
        assert!(parse_pin(&good).is_some());
        assert!(parse_pin(&"AB".repeat(32)).is_none());
        assert!(parse_pin(&"ab".repeat(31)).is_none());
        assert!(parse_pin(&format!("{}zz", "ab".repeat(31))).is_none());
        assert_eq!(to_hex(&parse_pin(&good).unwrap()), good);
    }
}
