//! Trusted operator configuration; secret contents never serialize into this type.
use archie_sentry::{
    adapters::{home_assistant::HomeAssistant, ipp::IppClient},
    contracts::{validate_id, CommandKind},
    engine::AdapterError,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SentrySection {
    pub printers: Vec<PrinterBinding>,
    pub operator_uids: Vec<u32>,
    pub home_assistant: Option<HomeAssistantBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterBinding {
    pub resource_id: String,
    pub endpoint: String,
    pub expected_uuid: String,
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
        }
        if let Some(ha) = &self.home_assistant {
            ha.validate().map_err(|_| invalid())?;
            for resource in ha.resources.iter().chain(&ha.telemetry_resources) {
                if !resources.insert(&resource.resource_id) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}
