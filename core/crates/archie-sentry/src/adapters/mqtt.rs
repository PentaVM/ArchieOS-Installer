//! Minimal MQTT 3.1.1 client over plaintext TCP for private-LAN brokers only.
//!
//! Scope: CONNECT (optional username/password), PUBLISH at QoS 0, SUBSCRIBE to
//! one configured topic and await one matching PUBLISH, DISCONNECT. `apply`
//! subscribes to the state topic before it publishes the command and waits,
//! bounded, for the device's live report, so the engine's observation that
//! follows is not racing the device. There is no
//! TLS: credentials and payloads cross the LAN in cleartext, so this adapter is
//! only acceptable on a trusted private segment, exactly like the other native
//! LAN adapters. TLS transport (mqtts, port 8883) is future work and requires a
//! TLS stack this crate does not currently depend on.
//!
//! Credentials arrive through the constructor config, never through URLs, and
//! are never logged or rendered in `Debug`. Packet parsing is strict: the
//! remaining-length varint is at most 4 bytes and the whole packet is capped at
//! 256 KiB; malformed, truncated or oversized frames are `AdapterError::Protocol`.
use crate::{
    contracts::{validate_id, Command, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, net::IpAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

/// Whole-packet cap; the remaining-length varint itself allows 256 MiB.
const MAX_PACKET: usize = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Awaiting a state publication is capped independently of Core's deadline so
/// standalone calls always terminate. Core's per-intent budget may cut earlier.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `apply` waits for the device's live state report after commanding.
const REPORT_WAIT: Duration = Duration::from_secs(3);

const PKT_CONNECT: u8 = 1;
const PKT_CONNACK: u8 = 2;
const PKT_PUBLISH: u8 = 3;
const PKT_SUBSCRIBE: u8 = 8;
const PKT_SUBACK: u8 = 9;
const PKT_DISCONNECT: u8 = 14;

/// How a resource's state payload maps onto a measurable `ResourceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MqttKind {
    Switch,
    Light,
    Climate,
}

/// Per-resource topic and payload configuration. Templates are literal strings
/// with one placeholder: `{percent}` for brightness (0-100), `{celsius}` for
/// temperature rendered with one decimal (e.g. `22.5`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttBinding {
    pub kind: MqttKind,
    pub command_topic: String,
    pub state_topic: String,
    pub payload_on: String,
    pub payload_off: String,
    pub brightness_template: String,
    pub temperature_template: String,
}
impl MqttBinding {
    pub fn switch(command_topic: &str, state_topic: &str, on: &str, off: &str) -> Self {
        Self {
            kind: MqttKind::Switch,
            command_topic: command_topic.into(),
            state_topic: state_topic.into(),
            payload_on: on.into(),
            payload_off: off.into(),
            brightness_template: "{percent}".into(),
            temperature_template: "{celsius}".into(),
        }
    }
    pub fn light(command_topic: &str, state_topic: &str, brightness_template: &str) -> Self {
        Self {
            kind: MqttKind::Light,
            brightness_template: brightness_template.into(),
            ..Self::switch(command_topic, state_topic, "ON", "OFF")
        }
    }
    pub fn climate(command_topic: &str, state_topic: &str, temperature_template: &str) -> Self {
        Self {
            kind: MqttKind::Climate,
            temperature_template: temperature_template.into(),
            ..Self::switch(command_topic, state_topic, "ON", "OFF")
        }
    }
}

pub struct Mqtt {
    edge: String,
    adapter: String,
    address: String,
    client_id: String,
    credentials: Option<(String, String)>,
    bindings: BTreeMap<String, MqttBinding>,
}
impl std::fmt::Debug for Mqtt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mqtt")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("address", &self.address)
            .field("credentials", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl Mqtt {
    /// `broker` is `"<ip>"` or `"<ip>:<port>"` (default 1883); the host must be
    /// a loopback or private IP literal, matching the LAN-only endpoint policy
    /// of the HTTP adapters. `credentials` is an optional (username, password)
    /// pair sent in CONNECT; it is never taken from a URL and never logged.
    pub fn new(
        edge: &str,
        adapter: &str,
        broker: &str,
        credentials: Option<(String, String)>,
        bindings: BTreeMap<String, MqttBinding>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        let address = broker_address(broker)?;
        if bindings.is_empty() || bindings.len() > 4096 {
            return Err(AdapterError::Protocol);
        }
        for (resource, binding) in &bindings {
            validate_id(resource).map_err(|_| AdapterError::Protocol)?;
            topic(&binding.command_topic)?;
            topic(&binding.state_topic)?;
            payload(&binding.payload_on)?;
            payload(&binding.payload_off)?;
            template(&binding.brightness_template, "{percent}")?;
            template(&binding.temperature_template, "{celsius}")?;
        }
        if let Some((username, password)) = &credentials {
            let valid = |s: &str, max: usize| {
                !s.is_empty() && s.len() <= max && !s.bytes().any(|b| b < 0x20 || b == 0x7f)
            };
            if !valid(username, 128) || !valid(password, 256) {
                return Err(AdapterError::Authentication);
            }
        }
        // Broker client ids are limited to 23 bytes by strict 3.1.1 servers.
        let digest = format!("{:x}", Sha256::digest(format!("{edge}\0{adapter}")));
        let client_id = format!("archie-{}", &digest[..16]);
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            address,
            client_id,
            credentials,
            bindings,
        })
    }
    fn binding(&self, resource: &ResourceDescriptor) -> Result<&MqttBinding, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.bindings
            .get(&resource.resource_id)
            .ok_or(AdapterError::Unsupported)
    }
    async fn connect(&self) -> Result<TcpStream, AdapterError> {
        let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(&self.address))
            .await
            .map_err(|_| AdapterError::Unavailable)?
            .map_err(|_| AdapterError::Transport)?;
        let packet = encode_connect(
            &self.client_id,
            self.credentials
                .as_ref()
                .map(|(u, p)| (u.as_str(), p.as_str())),
        )?;
        write_packet(&mut stream, &packet).await?;
        let (header, body) = read_packet(&mut stream).await?;
        if header >> 4 != PKT_CONNACK || body.len() != 2 {
            return Err(AdapterError::Protocol);
        }
        match body[1] {
            0 => Ok(stream),
            // Unacceptable protocol version / identifier rejected.
            1 | 2 => Err(AdapterError::Protocol),
            // Server unavailable.
            3 => Err(AdapterError::Unavailable),
            // Bad username or password / not authorised.
            4 | 5 => Err(AdapterError::Authentication),
            _ => Err(AdapterError::Protocol),
        }
    }
    async fn disconnect(&self, stream: &mut TcpStream) {
        let _ = stream.write_all(&encode_disconnect()).await;
    }
}

fn broker_address(input: &str) -> Result<String, AdapterError> {
    let (host, port) = match input.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, port.parse().ok()),
        _ => (input, Some(1883)),
    };
    let ip: IpAddr = host.parse().map_err(|_| AdapterError::Protocol)?;
    let permitted = match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
    };
    let port = port.filter(|p| *p != 0).ok_or(AdapterError::Protocol)?;
    if !permitted {
        return Err(AdapterError::Protocol);
    }
    Ok(format!("{ip}:{port}"))
}
/// Publish-topic rules: UTF-8, bounded, no wildcards, no control characters.
/// Subscribed state topics follow the same rules, so matching is exact.
fn topic(value: &str) -> Result<(), AdapterError> {
    if value.is_empty()
        || value.len() > 128
        || value.bytes().any(|b| b < 0x20 || b == 0x7f)
        || value.contains(['+', '#'])
    {
        return Err(AdapterError::Protocol);
    }
    Ok(())
}
fn payload(value: &str) -> Result<(), AdapterError> {
    if value.is_empty()
        || value.len() > 64
        || !value.is_ascii()
        || value.bytes().any(|b| b < 0x20 || b == 0x7f)
    {
        return Err(AdapterError::Protocol);
    }
    Ok(())
}
fn template(value: &str, placeholder: &str) -> Result<(), AdapterError> {
    payload(value)?;
    if value.matches(placeholder).count() != 1 {
        return Err(AdapterError::Protocol);
    }
    Ok(())
}

fn utf8(value: &str) -> Result<Vec<u8>, AdapterError> {
    let length = u16::try_from(value.len()).map_err(|_| AdapterError::Protocol)?;
    let mut bytes = length.to_be_bytes().to_vec();
    bytes.extend_from_slice(value.as_bytes());
    Ok(bytes)
}
fn encode_remaining_length(mut length: usize) -> Result<Vec<u8>, AdapterError> {
    if length > MAX_PACKET {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = Vec::new();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if length == 0 {
            return Ok(bytes);
        }
    }
}
fn frame(header: u8, body: &[u8]) -> Result<Vec<u8>, AdapterError> {
    let mut packet = vec![header];
    packet.extend_from_slice(&encode_remaining_length(body.len())?);
    packet.extend_from_slice(body);
    Ok(packet)
}

/// CONNECT for MQTT 3.1.1 with clean session and a 10 s keep-alive.
pub fn encode_connect(
    client_id: &str,
    credentials: Option<(&str, &str)>,
) -> Result<Vec<u8>, AdapterError> {
    let mut body = utf8("MQTT")?;
    body.push(4); // protocol level 3.1.1
    let mut flags = 0x02; // clean session
    if credentials.is_some() {
        flags |= 0x80 | 0x40; // username + password flags
    }
    body.push(flags);
    body.extend_from_slice(&10u16.to_be_bytes()); // keep-alive
    body.extend_from_slice(&utf8(client_id)?);
    if let Some((username, password)) = credentials {
        body.extend_from_slice(&utf8(username)?);
        body.extend_from_slice(&utf8(password)?);
    }
    frame(PKT_CONNECT << 4, &body)
}
/// QoS 0, not retained, no duplicate flag.
pub fn encode_publish(topic: &str, payload: &[u8]) -> Result<Vec<u8>, AdapterError> {
    let mut body = utf8(topic)?;
    body.extend_from_slice(payload);
    frame(PKT_PUBLISH << 4, &body)
}
pub fn encode_subscribe(packet_id: u16, topic: &str) -> Result<Vec<u8>, AdapterError> {
    if packet_id == 0 {
        return Err(AdapterError::Protocol);
    }
    let mut body = packet_id.to_be_bytes().to_vec();
    body.extend_from_slice(&utf8(topic)?);
    body.push(0); // requested QoS 0
    frame(PKT_SUBSCRIBE << 4 | 0x02, &body)
}
pub fn encode_disconnect() -> Vec<u8> {
    vec![PKT_DISCONNECT << 4, 0]
}

async fn write_packet(stream: &mut TcpStream, packet: &[u8]) -> Result<(), AdapterError> {
    timeout(IO_TIMEOUT, stream.write_all(packet))
        .await
        .map_err(|_| AdapterError::Unavailable)?
        .map_err(|_| AdapterError::Transport)
}
/// Read one packet with strict remaining-length parsing: at most 4 varint
/// bytes, total body bounded by MAX_PACKET; truncation is a protocol error
/// because a partial frame is unparseable, not merely absent.
async fn read_packet(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), AdapterError> {
    let mut first = [0u8; 1];
    timeout(IO_TIMEOUT, stream.read_exact(&mut first))
        .await
        .map_err(|_| AdapterError::Unavailable)?
        .map_err(|_| AdapterError::Protocol)?;
    let mut remaining = 0usize;
    let mut multiplier = 1usize;
    for _ in 0..4 {
        let mut byte = [0u8; 1];
        timeout(IO_TIMEOUT, stream.read_exact(&mut byte))
            .await
            .map_err(|_| AdapterError::Unavailable)?
            .map_err(|_| AdapterError::Protocol)?;
        remaining += usize::from(byte[0] & 0x7f) * multiplier;
        if byte[0] & 0x80 == 0 {
            if remaining > MAX_PACKET {
                return Err(AdapterError::Protocol);
            }
            let mut body = vec![0u8; remaining];
            timeout(IO_TIMEOUT, stream.read_exact(&mut body))
                .await
                .map_err(|_| AdapterError::Unavailable)?
                .map_err(|_| AdapterError::Protocol)?;
            return Ok((first[0], body));
        }
        multiplier *= 128;
    }
    Err(AdapterError::Protocol) // varint longer than 4 bytes
}

/// Topic and payload of a QoS 0 PUBLISH body. QoS > 0 carries a packet
/// identifier we never negotiated, so it is rejected rather than skipped.
fn parse_publish(header: u8, body: &[u8]) -> Result<(String, Vec<u8>), AdapterError> {
    if header >> 4 != PKT_PUBLISH || header & 0x06 != 0 {
        return Err(AdapterError::Protocol);
    }
    let length = usize::from(u16::from_be_bytes(
        body.get(..2)
            .and_then(|b| <[u8; 2]>::try_from(b).ok())
            .ok_or(AdapterError::Protocol)?,
    ));
    let end = 2 + length;
    if end > body.len() {
        return Err(AdapterError::Protocol);
    }
    let name = std::str::from_utf8(&body[2..end]).map_err(|_| AdapterError::Protocol)?;
    topic(name)?;
    Ok((name.to_owned(), body[end..].to_vec()))
}

fn render(binding: &MqttBinding, command: &Command) -> Result<Vec<u8>, AdapterError> {
    let text = match (binding.kind, command) {
        (MqttKind::Switch, Command::SwitchSet { on: true }) => binding.payload_on.clone(),
        (MqttKind::Switch, Command::SwitchSet { on: false }) => binding.payload_off.clone(),
        (MqttKind::Light, Command::LightSetBrightness { percent }) => binding
            .brightness_template
            .replace("{percent}", &percent.to_string()),
        (MqttKind::Climate, Command::ClimateSetTemperature { milli_celsius }) => {
            binding.temperature_template.replace(
                "{celsius}",
                &format!("{:.1}", *milli_celsius as f64 / 1000.0),
            )
        }
        _ => return Err(AdapterError::Unsupported),
    };
    payload(&text)?;
    Ok(text.into_bytes())
}

fn parse_state(binding: &MqttBinding, payload: &[u8]) -> Result<ResourceState, AdapterError> {
    let text = std::str::from_utf8(payload).map_err(|_| AdapterError::Protocol)?;
    match binding.kind {
        MqttKind::Switch if text == binding.payload_on => Ok(ResourceState::Switch { on: true }),
        MqttKind::Switch if text == binding.payload_off => Ok(ResourceState::Switch { on: false }),
        MqttKind::Switch => Err(AdapterError::Protocol),
        MqttKind::Light => {
            let percent: u8 = text.trim().parse().map_err(|_| AdapterError::Protocol)?;
            if percent > 100 {
                return Err(AdapterError::Protocol);
            }
            Ok(ResourceState::Light {
                on: percent > 0,
                brightness_percent: percent,
            })
        }
        MqttKind::Climate => {
            let celsius: f64 = text.trim().parse().map_err(|_| AdapterError::Protocol)?;
            if !celsius.is_finite() || !(5.0..=35.0).contains(&celsius) {
                return Err(AdapterError::Protocol);
            }
            Ok(ResourceState::Climate {
                target_milli_celsius: (celsius * 1000.0).round() as i32,
            })
        }
    }
}

#[async_trait]
impl EdgeAdapter for Mqtt {
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
        let binding = self.binding(resource)?.clone();
        let mut stream = self.connect().await?;
        let result = timeout(OBSERVE_TIMEOUT, async {
            write_packet(&mut stream, &encode_subscribe(1, &binding.state_topic)?).await?;
            let (header, body) = read_packet(&mut stream).await?;
            if header >> 4 != PKT_SUBACK
                || body.len() != 3
                || u16::from_be_bytes([body[0], body[1]]) != 1
            {
                return Err(AdapterError::Protocol);
            }
            if body[2] != 0 {
                // 0x80 = subscription refused; 1|2 = granted above our QoS 0.
                return Err(AdapterError::Unavailable);
            }
            loop {
                let (header, body) = read_packet(&mut stream).await?;
                let (topic, payload) = parse_publish(header, &body)?;
                if topic == binding.state_topic {
                    return Ok((topic, payload));
                }
                // A publication for another topic is ignored, not an error.
            }
        })
        .await
        .map_err(|_| AdapterError::Unavailable)?;
        self.disconnect(&mut stream).await;
        let (topic, payload) = result?;
        let state = parse_state(&binding, &payload)?;
        let mut evidence = topic.into_bytes();
        evidence.push(0);
        evidence.extend_from_slice(&payload);
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            // Identical reports keep one evidence identity; no freshness is invented.
            evidence_id: format!("mqtt-{:x}", Sha256::digest(evidence)),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        if !resource.capabilities.contains(&command.capability()) {
            return Err(AdapterError::Unsupported);
        }
        let binding = self.binding(resource)?.clone();
        let payload = render(&binding, command)?;
        let mut stream = self.connect().await?;
        let result = async {
            // Subscribe to the state topic before commanding, so the device's
            // own report of what it did cannot slip past between the PUBLISH
            // and the engine's observation. The SUBACK (and any retained,
            // pre-command state) is consumed first.
            write_packet(&mut stream, &encode_subscribe(1, &binding.state_topic)?).await?;
            let (header, body) = read_packet(&mut stream).await?;
            if header >> 4 != PKT_SUBACK
                || body.len() != 3
                || u16::from_be_bytes([body[0], body[1]]) != 1
            {
                return Err(AdapterError::Protocol);
            }
            if body[2] != 0 {
                return Err(AdapterError::Unavailable);
            }
            write_packet(
                &mut stream,
                &encode_publish(&binding.command_topic, &payload)?,
            )
            .await?;
            // Wait, bounded, for a live (not retained) report on the state
            // topic. It only synchronizes: whatever it says, the engine's own
            // observation afterwards is the evidence. No report in time is
            // not an error here; the observation then shows what is true.
            let _ = timeout(REPORT_WAIT, async {
                loop {
                    let (header, body) = read_packet(&mut stream).await?;
                    let retained = header & 0x01 != 0;
                    let (topic, _) = parse_publish(header, &body)?;
                    if !retained && topic == binding.state_topic {
                        return Ok::<_, AdapterError>(());
                    }
                }
            })
            .await;
            Ok::<_, AdapterError>(())
        }
        .await;
        self.disconnect(&mut stream).await;
        result?;
        Ok(AdapterAck { accepted: true })
    }
}
