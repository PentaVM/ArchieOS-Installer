//! Roku External Control Protocol (ECP) adapter: plain HTTP on TCP 8060, no
//! credentials, served to anything on the LAN.
//!
//! Honesty notes, deliberately part of the module contract:
//! - After `PowerOff`, a Roku TV in deep sleep commonly stops answering ECP
//!   entirely. `observe` then fails with `Transport` and the engine records
//!   `Unavailable`/`Indeterminate` instead of `Verified`. That is intended: no
//!   postcondition is faked from a device that is not answering.
//! - Navigation (directions, select, back) and volume keys are intentionally
//!   absent. ECP does not report cursor focus or volume state, so their effect
//!   is not measurable; this engine does not actuate what it cannot verify.
//! - `query/device-info` carries a serial number, device names and network
//!   details. None of them are parsed, stored, or hashed into evidence — only
//!   `power-mode` is read.
use super::{body, client, endpoint, MAX_BODY};
use crate::{
    contracts::{
        validate_id, Command, Observation, ObservationOrigin, RemoteKeyName, ResourceState,
    },
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use reqwest::{Client, Response, Url};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Longest string we accept out of a device reply.
const MAX_TEXT: usize = 128;
/// ECP's fixed TCP port when the configured URL does not name one.
const ECP_PORT: u16 = 8060;

pub struct Roku {
    edge: String,
    adapter: String,
    /// resource_id → ECP base URL (`http://<private-ip>[:8060]`).
    endpoints: BTreeMap<String, Url>,
    client: Client,
}
impl std::fmt::Debug for Roku {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Roku")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("endpoints", &self.endpoints.len())
            .finish_non_exhaustive()
    }
}
impl Roku {
    pub fn new(
        edge: &str,
        adapter: &str,
        endpoints: BTreeMap<String, String>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        if endpoints.is_empty() || endpoints.len() > 1024 {
            return Err(AdapterError::Protocol);
        }
        let mut resolved = BTreeMap::new();
        for (resource, raw) in endpoints {
            validate_id(&resource).map_err(|_| AdapterError::Protocol)?;
            let mut url = endpoint(&raw, false)?;
            if url.path() != "/" {
                return Err(AdapterError::Protocol);
            }
            if url.port().is_none() {
                url.set_port(Some(ECP_PORT))
                    .map_err(|_| AdapterError::Protocol)?;
            }
            resolved.insert(resource, url);
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            endpoints: resolved,
            client: client()?,
        })
    }
    fn endpoint_of(&self, resource: &ResourceDescriptor) -> Result<&Url, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.endpoints
            .get(&resource.resource_id)
            .ok_or(AdapterError::Unsupported)
    }
    async fn get_xml(&self, base: &Url, path: &str) -> Result<String, AdapterError> {
        let url = base.join(path).map_err(|_| AdapterError::Protocol)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "text/xml").await?;
        String::from_utf8(bytes).map_err(|_| AdapterError::Protocol)
    }
}

/// Strip control characters and bound length; anything else is not evidence.
fn clean(raw: &str) -> Option<String> {
    let filtered: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT + 1)
        .collect();
    let trimmed = filtered.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TEXT {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
/// Inner text of the first `<name>…</name>` element, bounded.
fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let close = format!("</{name}>");
    let end = xml[start..].find(&close)? + start;
    clean(&xml[start..end])
}
/// The inside of the first `<name …>` open tag (attributes only). The name
/// must be followed by whitespace, `>` or `/` so `<apps>` cannot pass as
/// `<app`.
fn open_tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("<{name}");
    let mut rest = xml;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        let bounded = after
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || matches!(c, '>' | '/'));
        if bounded {
            let end = after.find('>')?;
            return Some(&after[..end]);
        }
        rest = after;
    }
    None
}
/// A `name="value"` or `name='value'` attribute out of an open tag, bounded.
/// The name must start at a whitespace boundary so `state=` cannot match
/// inside a longer attribute name.
fn attr(tag_text: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let mut rest = tag_text;
    while let Some(pos) = rest.find(&needle) {
        let boundary = pos == 0
            || rest.as_bytes()[pos - 1].is_ascii_whitespace()
            || rest.as_bytes()[pos - 1] == b'<';
        let after = &rest[pos + needle.len()..];
        if boundary {
            let quote = *after.as_bytes().first()?;
            if quote != b'"' && quote != b'\'' {
                return None;
            }
            let value = &after[1..];
            let end = value.find(quote as char)?;
            return clean(&value[..end]);
        }
        rest = after;
    }
    None
}

/// `query/device-info` → power. Only the documented Roku TV modes are
/// evidence; anything else, or a missing element, is a malformed reply.
fn parse_power(xml: &str) -> Result<bool, AdapterError> {
    match element(xml, "power-mode").as_deref() {
        Some("PowerOn") => Ok(true),
        Some("PowerOff") | Some("ReadyForSuspend") | Some("Headless") => Ok(false),
        _ => Err(AdapterError::Protocol),
    }
}
/// `query/active-app` → (showing_home, active_app id). No `<app>` element, an
/// `<app>` without attributes (a Roku on its home screen answers
/// `<app>Roku</app>`), or the home app itself means the home screen is
/// showing. An `<app>` with attributes but no `id` is malformed.
fn parse_active_app(xml: &str) -> Result<(bool, Option<String>), AdapterError> {
    if !xml.contains("<active-app") {
        return Err(AdapterError::Protocol);
    }
    match open_tag(xml, "app") {
        None => Ok((true, None)),
        Some(tag) if tag.trim().is_empty() => Ok((true, None)),
        Some(tag) => match attr(tag, "id") {
            None => Err(AdapterError::Protocol),
            Some(id) if id == "tvins.home" => Ok((true, None)),
            Some(id) => Ok((false, Some(id))),
        },
    }
}
/// `query/media-player` `state` attribute → playing. Anything unexpected is
/// "unknown", never an error: media playback is not always present.
fn parse_playing(xml: &str) -> Option<bool> {
    let tag = open_tag(xml, "player")?;
    match attr(tag, "state").as_deref() {
        Some("play") => Some(true),
        Some("pause") => Some(false),
        _ => None,
    }
}

/// keypress/launch replies are empty or tiny and usually untyped; bound the
/// read without requiring a content type.
async fn read_bounded(mut response: Response) -> Result<(), AdapterError> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(AdapterError::Authentication);
    }
    if !response.status().is_success() {
        return Err(AdapterError::Unavailable);
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(AdapterError::Protocol);
    }
    let mut size = 0usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AdapterError::Transport)?
    {
        size = size.saturating_add(chunk.len());
        if size > MAX_BODY {
            return Err(AdapterError::Protocol);
        }
    }
    Ok(())
}

#[async_trait]
impl EdgeAdapter for Roku {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    async fn observe(&mut self, resource: &ResourceDescriptor) -> Result<Observation, AdapterError> {
        let base = self.endpoint_of(resource)?.clone();
        let info = self.get_xml(&base, "query/device-info").await?;
        let power_on = parse_power(&info)?;
        let active = self.get_xml(&base, "query/active-app").await?;
        let (showing_home, active_app) = parse_active_app(&active)?;
        // A missing, failing or surprising media-player answer means "unknown",
        // not failure — playback is optional.
        let playing = match self.get_xml(&base, "query/media-player").await {
            Ok(player) => parse_playing(&player),
            Err(_) => None,
        };
        let state = ResourceState::Tv {
            power_on,
            showing_home,
            active_app,
            playing,
        };
        // Evidence digests the measured state alone. Identical reports keep an
        // identical evidence id rather than inventing a fresh measurement, and
        // device-identifying fields (serial, names, addresses) are never inputs.
        let report = serde_json::to_vec(&(&resource.resource_id, &state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id: format!("roku-{:x}", Sha256::digest(report)),
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
        let base = self.endpoint_of(resource)?.clone();
        let path = match command {
            Command::RemoteKey { key } => format!(
                "keypress/{}",
                match key {
                    RemoteKeyName::PowerOn => "PowerOn",
                    RemoteKeyName::PowerOff => "PowerOff",
                    RemoteKeyName::Home => "Home",
                    RemoteKeyName::Play => "Play",
                    RemoteKeyName::Pause => "Pause",
                }
            ),
            // Contract-validated app ids are [A-Za-z0-9._-] only, so the path
            // segment needs no percent-encoding.
            Command::AppLaunch { app_id } => format!("launch/{app_id}"),
            _ => return Err(AdapterError::Unsupported),
        };
        let url = base.join(&path).map_err(|_| AdapterError::Protocol)?;
        let response = self
            .client
            .post(url)
            .body(Vec::new())
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        read_bounded(response).await?;
        Ok(AdapterAck { accepted: true })
    }
}
