//! Chromecast (CAST v2) — an explicitly stated non-implementation.
//!
//! CAST v2 speaks length-prefixed protobuf frames over a TLS session on TCP
//! port 8009, authenticated by the device certificate presented during the
//! mDNS-advertised handshake. This crate has no TLS client stack for raw
//! sockets (`tokio-rustls` is not a dependency of archie-sentry; `reqwest`'s
//! rustls only covers HTTPS) and no protobuf codec, so neither observation
//! (receiver status is a protobuf message) nor control (launch/play/pause) can
//! be implemented honestly here. Rather than staying silent — which would read
//! as an oversight — this module records the decision as code: it keeps typed
//! [`CastDevice`] descriptors from LAN discovery and answers every `observe`
//! and `apply` with `AdapterError::Unsupported`, documented by
//! [`Chromecast::UNSUPPORTED_REASON`].
//!
//! If a TLS stack and a protobuf codec ever enter the dependency tree, the
//! postcondition story also needs design: CAST reports `playerState` and volume
//! per session, so `MediaPause`/`MediaPlay` would be verifiable, while
//! `AppLaunch` has no stable postcondition (app ids are session-scoped).
use crate::{
    contracts::{validate_id, Command, Observation},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::IpAddr};

/// A discovered Chromecast, described by data the LAN inventory already holds
/// (mDNS service instance plus TXT fields); nothing is probed by this module.
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
}
impl CastDevice {
    pub fn new(
        resource_id: &str,
        name: &str,
        model: &str,
        address: IpAddr,
        port: u16,
    ) -> Result<Self, AdapterError> {
        validate_id(resource_id).map_err(|_| AdapterError::Protocol)?;
        let bounded = |s: &str| {
            !s.is_empty() && s.len() <= 64 && !s.bytes().any(|b| b < 0x20 || b == 0x7f)
        };
        let permitted = match address {
            IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        };
        if !bounded(name) || !bounded(model) || !permitted || port == 0 {
            return Err(AdapterError::Protocol);
        }
        Ok(Self {
            resource_id: resource_id.into(),
            name: name.into(),
            model: model.into(),
            address,
            port,
        })
    }
}

pub struct Chromecast {
    edge: String,
    adapter: String,
    devices: BTreeMap<String, CastDevice>,
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
    /// Why every operation returns `AdapterError::Unsupported`. Surfaced so
    /// receipts and operator tooling can state the reason instead of a bare code.
    pub const UNSUPPORTED_REASON: &'static str = "CAST v2 requires TLS plus protobuf on port 8009; \
         archie-sentry has neither a raw-TLS stack (tokio-rustls is not a dependency) \
         nor a protobuf codec, so cast observation and control are out of scope";
    pub fn new(
        edge: &str,
        adapter: &str,
        devices: Vec<CastDevice>,
    ) -> Result<Self, AdapterError> {
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
        })
    }
    /// The recorded descriptor for a discovered device, if known.
    pub fn descriptor(&self, resource_id: &str) -> Option<&CastDevice> {
        self.devices.get(resource_id)
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
    /// Unsupported: receiver status is a protobuf message inside a TLS session
    /// this crate cannot open; see [`Chromecast::UNSUPPORTED_REASON`].
    async fn observe(
        &mut self,
        _resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        Err(AdapterError::Unsupported)
    }
    /// Unsupported for every command on every resource, known or not: see
    /// [`Chromecast::UNSUPPORTED_REASON`].
    async fn apply(
        &mut self,
        _resource: &ResourceDescriptor,
        _command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        Err(AdapterError::Unsupported)
    }
}
