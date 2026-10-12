//! Trusted operator configuration; secret contents never serialize into this type.
use archie_sentry::{
    adapters::{
        home_assistant::HomeAssistant,
        ipp::{IppAdapter, IppClient},
        mqtt::{Mqtt, MqttBinding, MqttKind},
        onvif::OnvifProbe,
        roku::Roku,
    },
    contracts::{validate_id, CommandKind},
    engine::AdapterError,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SentrySection {
    pub printers: Vec<PrinterBinding>,
    pub operator_uids: Vec<u32>,
    pub home_assistant: Option<HomeAssistantBinding>,
    /// Roku ECP televisions/actuators. Paired devices only; see the registry wiring.
    pub roku: Vec<RokuBinding>,
    /// One LAN MQTT broker and its actuable resources.
    pub mqtt: Option<MqttSection>,
    /// ONVIF cameras: observation-only telemetry, never actuable resources.
    pub onvif_cameras: Vec<OnvifCameraBinding>,
    /// Local IRIS context-feed defaults for `archie context-feed`.
    pub context_feed: Option<ContextFeedSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterBinding {
    pub resource_id: String,
    /// The printer URI (`ipp://`, `ipps://` or their HTTP forms).
    pub endpoint: String,
    pub expected_uuid: String,
    /// Absolute spool directory for reviewed PrintSubmit jobs. A printer
    /// without a spool directory stays query-only and never becomes actuable.
    #[serde(default)]
    pub spool_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeAssistantBinding {
    pub endpoint: String,
    pub token_file: String,
    pub resources: Vec<HomeAssistantResource>,
    /// Observation-only bindings. They never enter the actuator registry.
    #[serde(default)]
    pub telemetry_resources: Vec<HomeAssistantResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeAssistantResource {
    pub resource_id: String,
    pub entity: String,
}
impl HomeAssistantResource {
    pub fn capabilities(&self) -> Result<Vec<CommandKind>, AdapterError> {
        match self.entity.split_once('.').map(|(domain, _)| domain) {
            Some("switch") => Ok(vec![CommandKind::SwitchSet]),
            Some("light") => Ok(vec![CommandKind::LightSetBrightness]),
            Some("media_player") => Ok(vec![CommandKind::MediaPause, CommandKind::MediaPlay]),
            Some("climate") => Ok(vec![CommandKind::ClimateSetTemperature]),
            _ => Err(AdapterError::Unsupported),
        }
    }
}
impl HomeAssistantBinding {
    fn entities(&self) -> BTreeMap<String, String> {
        self.resources
            .iter()
            .chain(&self.telemetry_resources)
            .map(|r| (r.resource_id.clone(), r.entity.clone()))
            .collect()
    }
    pub fn validate(&self) -> Result<(), AdapterError> {
        let count = self.resources.len() + self.telemetry_resources.len();
        if count == 0
            || count > 64
            || self.entities().len() != count
            || self.token_file.len() > 512
            || !Path::new(&self.token_file).is_absolute()
        {
            return Err(AdapterError::Protocol);
        }
        for resource in &self.resources {
            resource.capabilities()?;
        }
        HomeAssistant::new(
            "local-core",
            "home-assistant",
            &self.endpoint,
            "validation-placeholder",
            self.entities(),
        )?;
        Ok(())
    }
    /// Opens a fresh token for each operation. No live token is kept in config/audit.
    pub fn adapter(&self) -> Result<HomeAssistant, AdapterError> {
        self.validate()?;
        let token = read_private_token(Path::new(&self.token_file))?;
        HomeAssistant::new(
            "local-core",
            "home-assistant",
            &self.endpoint,
            token.trim(),
            self.entities(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RokuBinding {
    pub resource_id: String,
    /// ECP base URL, e.g. `http://192.168.1.40` (port defaults to 8060).
    pub endpoint: String,
}
impl RokuBinding {
    /// Every configured Roku resource accepts exactly these commands.
    pub fn capabilities() -> Vec<CommandKind> {
        vec![CommandKind::RemoteKey, CommandKind::AppLaunch]
    }
}

/// One LAN MQTT broker with its actuable resources. Secrets are references,
/// never inline values: `password_ref` names `env:VARIABLE` or an absolute
/// private file (same rules as the Home Assistant token file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttSection {
    /// `"<ip>"` or `"<ip>:<port>"`; loopback or private IP literals only.
    pub broker: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password_ref: Option<String>,
    pub resources: BTreeMap<String, MqttBinding>,
}
impl MqttSection {
    fn credentials_shape(&self) -> Result<(), AdapterError> {
        // The adapter speaks CONNECT with a username/password pair or none;
        // a half-configured credential is an operator mistake, not a mode.
        match (&self.username, &self.password_ref) {
            (Some(_), Some(reference)) => validate_secret_ref(reference),
            (None, None) => Ok(()),
            _ => Err(AdapterError::Authentication),
        }
    }
    pub fn validate(&self) -> Result<(), AdapterError> {
        self.credentials_shape()?;
        let placeholder = self
            .username
            .as_ref()
            .map(|username| (username.clone(), "validation-placeholder".to_owned()));
        Mqtt::new(
            "local-core",
            "mqtt",
            &self.broker,
            placeholder,
            self.resources.clone(),
        )?;
        Ok(())
    }
    /// Resolves the referenced password for this one operation. No live
    /// credential is kept in config or audit.
    pub fn adapter(&self) -> Result<Mqtt, AdapterError> {
        self.validate()?;
        let credentials = match (&self.username, &self.password_ref) {
            (Some(username), Some(reference)) => {
                Some((username.clone(), read_secret_ref(reference)?))
            }
            _ => None,
        };
        Mqtt::new(
            "local-core",
            "mqtt",
            &self.broker,
            credentials,
            self.resources.clone(),
        )
    }
}
/// The actuable capability an MQTT binding kind carries.
pub fn mqtt_capabilities(binding: &MqttBinding) -> Vec<CommandKind> {
    match binding.kind {
        MqttKind::Switch => vec![CommandKind::SwitchSet],
        MqttKind::Light => vec![CommandKind::LightSetBrightness],
        MqttKind::Climate => vec![CommandKind::ClimateSetTemperature],
    }
}

/// An observation-only ONVIF camera. Cameras are telemetry resources and never
/// enter the actuable registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnvifCameraBinding {
    pub resource_id: String,
    /// Device service URL, e.g. `http://192.168.1.20/onvif/device_service`.
    pub endpoint: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password_ref: Option<String>,
}
impl OnvifCameraBinding {
    fn validate(&self) -> Result<(), AdapterError> {
        validate_id(&self.resource_id).map_err(|_| AdapterError::Protocol)?;
        match (&self.username, &self.password_ref) {
            (Some(_), Some(reference)) => validate_secret_ref(reference)?,
            (None, None) => {}
            _ => return Err(AdapterError::Authentication),
        }
        let placeholder = self
            .username
            .as_ref()
            .map(|username| (username.clone(), "validation-placeholder".to_owned()));
        OnvifProbe::new(&self.endpoint, placeholder)?;
        Ok(())
    }
}

fn default_poll_ms() -> u64 {
    5000
}
/// Local context-feed defaults. Both directories must be absolute; the poll
/// cadence is bounded so a typo cannot spin or stall the loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFeedSection {
    pub watchdog_dir: String,
    pub out_dir: String,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
}
impl ContextFeedSection {
    fn validate(&self) -> Result<(), AdapterError> {
        let dir =
            |value: &str| value.len() <= 512 && !value.is_empty() && Path::new(value).is_absolute();
        if !dir(&self.watchdog_dir) || !dir(&self.out_dir) || !(500..=60000).contains(&self.poll_ms)
        {
            return Err(AdapterError::Protocol);
        }
        Ok(())
    }
}

/// A secret reference is `env:VARIABLE` or an absolute path to a private file.
/// Inline secrets do not exist in this configuration surface by construction.
fn validate_secret_ref(reference: &str) -> Result<(), AdapterError> {
    if let Some(variable) = reference.strip_prefix("env:") {
        let valid = !variable.is_empty()
            && variable.len() <= 64
            && variable
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && variable
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_');
        return if valid {
            Ok(())
        } else {
            Err(AdapterError::Authentication)
        };
    }
    if reference.len() <= 512 && Path::new(reference).is_absolute() {
        Ok(())
    } else {
        Err(AdapterError::Authentication)
    }
}
fn read_secret_ref(reference: &str) -> Result<String, AdapterError> {
    validate_secret_ref(reference)?;
    if let Some(variable) = reference.strip_prefix("env:") {
        return std::env::var(variable).map_err(|_| AdapterError::Authentication);
    }
    read_private_token(Path::new(reference))
}

#[cfg(target_os = "linux")]
fn read_private_token(path: &Path) -> Result<String, AdapterError> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| AdapterError::Authentication)?;
    let metadata = file.metadata().map_err(|_| AdapterError::Authentication)?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() > 4096 {
        return Err(AdapterError::Authentication);
    }
    let mut token = String::new();
    file.take(4097)
        .read_to_string(&mut token)
        .map_err(|_| AdapterError::Authentication)?;
    if token.len() > 4096 {
        return Err(AdapterError::Authentication);
    }
    Ok(token)
}
#[cfg(not(target_os = "linux"))]
fn read_private_token(_: &Path) -> Result<String, AdapterError> {
    Err(AdapterError::Authentication)
}

impl SentrySection {
    pub fn validate(&self) -> Result<(), crate::config::ConfigError> {
        let invalid = || crate::config::ConfigError::Invalid("invalid Sentry configuration".into());
        if self.printers.len() > 64
            || self.operator_uids.len() > 32
            || self.operator_uids.iter().collect::<BTreeSet<_>>().len() != self.operator_uids.len()
            || self.roku.len() > 64
            || self.onvif_cameras.len() > 64
        {
            return Err(invalid());
        }
        let mut resources = BTreeSet::new();
        for printer in &self.printers {
            validate_id(&printer.resource_id).map_err(|_| invalid())?;
            if !resources.insert(&printer.resource_id) {
                return Err(invalid());
            }
            IppClient::new(&printer.endpoint).map_err(|_| invalid())?;
            let raw = printer
                .expected_uuid
                .strip_prefix("urn:uuid:")
                .ok_or_else(invalid)?;
            if raw.len() != 36
                || !raw.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                    }
                })
            {
                return Err(invalid());
            }
            // A spool directory makes the printer actuable; probe the exact
            // adapter construction the execution path will use.
            if let Some(spool) = &printer.spool_dir {
                if spool.len() > 512 {
                    return Err(invalid());
                }
                IppAdapter::new(
                    "local-core",
                    "ipp",
                    &printer.endpoint,
                    &printer.resource_id,
                    PathBuf::from(spool),
                )
                .map_err(|_| invalid())?;
            }
        }
        if let Some(ha) = &self.home_assistant {
            ha.validate().map_err(|_| invalid())?;
            for resource in ha.resources.iter().chain(&ha.telemetry_resources) {
                if !resources.insert(&resource.resource_id) {
                    return Err(invalid());
                }
            }
        }
        if !self.roku.is_empty() {
            let mut endpoints = BTreeMap::new();
            for roku in &self.roku {
                validate_id(&roku.resource_id).map_err(|_| invalid())?;
                if roku.endpoint.len() > 512 || !resources.insert(&roku.resource_id) {
                    return Err(invalid());
                }
                endpoints.insert(roku.resource_id.clone(), roku.endpoint.clone());
            }
            // Construction probe: endpoint policy and bounds live in the adapter.
            Roku::new("local-core", "roku", endpoints).map_err(|_| invalid())?;
        }
        if let Some(mqtt) = &self.mqtt {
            mqtt.validate().map_err(|_| invalid())?;
            for resource_id in mqtt.resources.keys() {
                if !resources.insert(resource_id) {
                    return Err(invalid());
                }
            }
        }
        for camera in &self.onvif_cameras {
            camera.validate().map_err(|_| invalid())?;
            if !resources.insert(&camera.resource_id) {
                return Err(invalid());
            }
        }
        if let Some(feed) = &self.context_feed {
            feed.validate().map_err(|_| invalid())?;
        }
        Ok(())
    }

    /// The actuable route of a configured resource: (adapter_id, capabilities).
    /// Telemetry-only resources (HA telemetry, ONVIF cameras) are absent here
    /// by construction, and a printer without a spool directory is query-only.
    pub fn actuable_route(&self, resource_id: &str) -> Option<(&'static str, Vec<CommandKind>)> {
        if let Some(ha) = &self.home_assistant {
            if let Some(resource) = ha.resources.iter().find(|r| r.resource_id == resource_id) {
                return resource.capabilities().ok().map(|c| ("home-assistant", c));
            }
        }
        if self.roku.iter().any(|r| r.resource_id == resource_id) {
            return Some(("roku", RokuBinding::capabilities()));
        }
        if self
            .printers
            .iter()
            .any(|p| p.resource_id == resource_id && p.spool_dir.is_some())
        {
            return Some(("ipp", vec![CommandKind::PrintSubmit]));
        }
        if let Some(mqtt) = &self.mqtt {
            if let Some(binding) = mqtt.resources.get(resource_id) {
                return Some(("mqtt", mqtt_capabilities(binding)));
            }
        }
        None
    }
}
