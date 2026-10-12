//! iPhone companion adapter — the honest model. Apple's sandbox means there
//! is no remote control of iOS: no app may lock the phone, read another
//! app's data, change system settings directly, or observe the real
//! foreground app. What exists instead is an owner-installed companion app
//! that receives intents over local HTTPS and executes the surfaces Apple
//! does allow: opening URLs and app URL schemes, and running the owner's
//! Shortcuts (`shortcuts://`), which are the real control surface (a
//! Shortcut can toggle device settings, act on HomeKit accessories, or
//! prepare messages the user confirms on screen).
//!
//! Server side, this adapter speaks a tiny JSON protocol to the companion:
//!
//! * `POST /v1/intent` — `{"id", "kind": "open_url"|"shortcut", "target",
//!   "input"?}`. The app answers `{"id", "status": "completed"|"rejected"}`:
//!   `completed` is the dispatch ack (for a Shortcut, that the
//!   `shortcuts://` URL was handed to the system — what the Shortcut then
//!   does is between the user and iOS); `rejected` means the app refused,
//!   surfaced as `Unavailable`.
//! * `GET /v1/state` — the app's own report: `battery_percent`,
//!   `volume_percent` (readable via AVAudioSession), `focus_active` (when
//!   the owner granted focus-status permission), and `last_action`, the
//!   intent id the app last completed.
//!
//! Contract mapping, stated plainly:
//! * `AppLaunch { app_id }` resolves through the per-device action table to
//!   a configured URL or Shortcut name — the command never carries a URL,
//!   so nothing operator-unreviewed is ever opened. The verification field
//!   `foreground_app` is the companion's `last_action` report: what the
//!   *app* last did, NOT the phone's real foreground app, which iOS never
//!   discloses. The docs (`docs/IOS-COMPANION.md`) say this to the owner.
//! * `VolumeSet` exists only when the device config names a `volume_shortcut`
//!   (an owner-created Shortcut taking the percent as input); the
//!   postcondition is the app's AVAudioSession read-back. Without one, or
//!   without a read-back, it is `Unsupported`.
//! * `SessionLock` is always `Unsupported`. iOS cannot be locked remotely,
//!   and this adapter says so rather than pretending.
//!
//! Transport honesty: the endpoint must be a private/loopback IP (see
//! [`super::endpoint`]); HTTPS outside loopback requires the companion's
//! pinned certificate in configuration, and that certificate is then the
//! *only* trust root for the connection. An unreachable app is a plain
//! `Transport`/`Unavailable` error — never a cached or invented state.

use super::{body, endpoint};
use crate::{
    contracts::{validate_id, Command, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, Url,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

/// What the companion executes for an `AppLaunch`'s `app_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IosAction {
    /// A URL the companion opens: an app's URL scheme (`music://`,
    /// `shortcuts://run-shortcut?name=...`) or an https link.
    OpenUrl(String),
    /// The name of an owner-created Shortcut the companion runs.
    Shortcut(String),
}
impl IosAction {
    fn validate(&self) -> Result<(), AdapterError> {
        let value = match self {
            Self::OpenUrl(url) => {
                // scheme ":" rest; scheme per RFC 3986, no whitespace or
                // control bytes anywhere. The URL is opened on the phone, so
                // javascript:/data: would be meaningless here but harmless;
                // the owner configured it either way.
                let (scheme, _) = url.split_once(':').ok_or(AdapterError::Protocol)?;
                if scheme.is_empty()
                    || scheme.len() > 32
                    || !scheme.bytes().enumerate().all(|(i, b)| {
                        b.is_ascii_lowercase()
                            || (i > 0 && (b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')))
                    })
                {
                    return Err(AdapterError::Protocol);
                }
                url
            }
            Self::Shortcut(name) => name,
        };
        if value.is_empty() || value.len() > 512 || value.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(AdapterError::Protocol);
        }
        Ok(())
    }
}

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IosDevice {
    /// `app_id` → the action the companion executes for it. The table is
    /// also the allowlist: an `app_id` that is not here is `Unsupported`.
    pub actions: BTreeMap<String, IosAction>,
    /// Name of an owner-created Shortcut that takes a number (percent) as
    /// input and sets the volume. `VolumeSet` exists only when this is set.
    pub volume_shortcut: Option<String>,
}

/// The companion's answer to an intent.
#[derive(Debug, Deserialize)]
struct IntentAck {
    id: String,
    status: String,
}

/// The companion's state report. All fields optional: the app reports what
/// iOS lets it know, and nothing else is invented.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionReport {
    pub battery_percent: Option<u8>,
    pub volume_percent: Option<u8>,
    pub focus_active: Option<bool>,
    /// The intent id the companion last completed — what the app did, not
    /// the phone's real foreground app (iOS does not disclose that).
    pub last_action: Option<String>,
}

/// The server side of the companion channel: one adapter, many iPhones.
pub struct IosCompanion {
    edge: String,
    adapter: String,
    base: Url,
    client: Client,
    authorization: HeaderValue,
    devices: BTreeMap<String, IosDevice>,
    observations: u64,
}
impl std::fmt::Debug for IosCompanion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IosCompanion")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("base", &self.base.as_str())
            .field("credentials", &"[redacted]")
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}
impl IosCompanion {
    /// `base` is the companion's URL (e.g. `https://192.168.1.70:8443`),
    /// `token` the bearer token the app expects, `pinned_certificate_pem`
    /// the companion's self-signed certificate. Outside loopback, HTTPS
    /// without a pinned certificate is refused — an unpinned "encrypted"
    /// channel on a LAN is not authentication.
    pub fn new(
        edge: &str,
        adapter: &str,
        base: &str,
        token: &str,
        pinned_certificate_pem: Option<&str>,
        devices: BTreeMap<String, IosDevice>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        let base = endpoint(base, true)?;
        if base.path() != "/"
            || token.is_empty()
            || token.len() > 4096
            || !token.bytes().all(|b| b.is_ascii_graphic())
            || devices.is_empty()
            || devices.len() > 16
        {
            return Err(AdapterError::Protocol);
        }
        let loopback = base
            .host_str()
            .map(|h| h.trim_matches(['[', ']']) == "127.0.0.1" || h == "[::1]" || h == "::1")
            .unwrap_or(false);
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5));
        match pinned_certificate_pem {
            Some(pem) => {
                // reqwest's from_pem is lenient (it can accept input without
                // a single certificate block), so the envelope is checked
                // here first: a pinned certificate must actually look like
                // one, bounded, with base64 content only.
                let valid_envelope = pem.len() <= 8192
                    && pem.contains("-----BEGIN CERTIFICATE-----")
                    && pem.contains("-----END CERTIFICATE-----");
                if !valid_envelope {
                    return Err(AdapterError::Protocol);
                }
                let certificate = reqwest::Certificate::from_pem(pem.as_bytes())
                    .map_err(|_| AdapterError::Protocol)?;
                builder = builder
                    .tls_built_in_root_certs(false)
                    .add_root_certificate(certificate);
            }
            // HTTPS to a non-loopback peer without a pinned certificate is
            // refused; http off-loopback is already refused by `endpoint`.
            None if base.scheme() == "https" && !loopback => return Err(AdapterError::Protocol),
            None => {}
        }
        let client = builder.build().map_err(|_| AdapterError::Transport)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AdapterError::Authentication)?;
        authorization.set_sensitive(true);
        for (resource, device) in &devices {
            validate_id(resource).map_err(|_| AdapterError::Protocol)?;
            if device.actions.len() > 256 {
                return Err(AdapterError::Protocol);
            }
            for (app_id, action) in &device.actions {
                validate_id(app_id).map_err(|_| AdapterError::Protocol)?;
                action.validate()?;
            }
            if let Some(name) = &device.volume_shortcut {
                if name.is_empty()
                    || name.len() > 128
                    || name.bytes().any(|b| b < 0x20 || b == 0x7f)
                {
                    return Err(AdapterError::Protocol);
                }
            }
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            base,
            client,
            authorization,
            devices,
            observations: 0,
        })
    }
    fn device(&self, resource: &ResourceDescriptor) -> Result<&IosDevice, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.devices
            .get(&resource.resource_id)
            .ok_or(AdapterError::Unsupported)
    }
    /// POST one intent; the companion's ack is the dispatch postcondition.
    /// A `completed` ack whose echoed id differs is a protocol violation,
    /// not a success.
    async fn intent(&self, payload: serde_json::Value) -> Result<AdapterAck, AdapterError> {
        let id = payload
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(AdapterError::Protocol)?
            .to_string();
        let bytes = serde_json::to_vec(&payload).map_err(|_| AdapterError::Protocol)?;
        let response = self
            .client
            .post(
                self.base
                    .join("v1/intent")
                    .map_err(|_| AdapterError::Protocol)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Content-Type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "application/json").await?;
        let ack: IntentAck = serde_json::from_slice(&bytes).map_err(|_| AdapterError::Protocol)?;
        if ack.id != id {
            return Err(AdapterError::Protocol);
        }
        match ack.status.as_str() {
            "completed" => Ok(AdapterAck { accepted: true }),
            // The app itself refused (e.g. the Shortcut does not exist).
            "rejected" => Err(AdapterError::Unavailable),
            _ => Err(AdapterError::Protocol),
        }
    }
    /// The companion's current state report.
    async fn report(&self) -> Result<CompanionReport, AdapterError> {
        let response = self
            .client
            .get(
                self.base
                    .join("v1/state")
                    .map_err(|_| AdapterError::Protocol)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "application/json").await?;
        let report: CompanionReport =
            serde_json::from_slice(&bytes).map_err(|_| AdapterError::Protocol)?;
        if let Some(last) = &report.last_action {
            validate_id(last).map_err(|_| AdapterError::Protocol)?;
        }
        Ok(report)
    }
}

#[async_trait]
impl EdgeAdapter for IosCompanion {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    /// The companion app's own report, mapped honestly onto the Host state:
    /// reachable/awake mean the app answered; `session_locked` is always
    /// None (iOS never tells us, and we never guess); `foreground_app` is
    /// the app's last completed intent, not the real foreground app.
    async fn observe(
        &mut self,
        resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        self.device(resource)?;
        let report = self.report().await?;
        let state = ResourceState::Host {
            reachable: true,
            awake: Some(true),
            session_locked: None,
            volume_percent: report.volume_percent,
            foreground_app: report.last_action.clone(),
        };
        let now = Timestamp::now();
        self.observations = self.observations.saturating_add(1);
        let evidence =
            serde_json::to_vec(&(&resource.resource_id, &report, now.0, self.observations))
                .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id: format!("ios-{:x}", Sha256::digest(evidence)),
            observed_at: now,
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
        let device = self.device(resource)?;
        match command {
            Command::AppLaunch { app_id } => {
                let action = device
                    .actions
                    .get(app_id)
                    .ok_or(AdapterError::Unsupported)?;
                let payload = match action {
                    IosAction::OpenUrl(url) => {
                        json!({"id": app_id, "kind": "open_url", "target": url})
                    }
                    IosAction::Shortcut(name) => {
                        json!({"id": app_id, "kind": "shortcut", "target": name})
                    }
                };
                self.intent(payload).await
            }
            Command::VolumeSet { percent } => {
                let name = device
                    .volume_shortcut
                    .as_ref()
                    .ok_or(AdapterError::Unsupported)?;
                self.intent(json!({
                    "id": "volume",
                    "kind": "shortcut",
                    "target": name,
                    "input": percent,
                }))
                .await
            }
            // SessionLock: iOS cannot be locked remotely. Everything else is
            // not a phone command this channel carries. Said plainly, never
            // attempted.
            _ => Err(AdapterError::Unsupported),
        }
    }
}
