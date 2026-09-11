//! Versioned handoff between the native wizard and the privileged installer.
//! Personal preferences are not credentials and never confer fabric membership.
use crate::{
    mapdata::{self, MapChoice, MapRegion},
    Answers,
};
use archie_types::NodeProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerPreferences {
    pub name: String,
    pub nickname: String,
    pub address_as: String,
    pub ai_alias: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    pub source: String,
    pub accuracy_m: Option<f64>,
    pub observed_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PerceptionConsent {
    pub camera: bool,
    pub microphone: bool,
    pub face_enrolment: bool,
    pub voice_enrolment: bool,
    pub hand_enrolment: bool,
    pub require_phone_approval: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapPackage {
    pub region: String,
    pub url: String,
    pub destination: String,
    pub partition: Option<String>,
    #[serde(default)]
    pub mirrors: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub hostname: String,
    pub username: String,
    pub owner: OwnerPreferences,
    pub location: Option<Location>,
    pub perception: PerceptionConsent,
    pub map: Option<MapPackage>,
}
impl Profile {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("profile exceeds 64 KiB".into());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "invalid wizard profile")?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported profile version".into());
        }
        Answers::new(
            &self.hostname,
            &self.username,
            NodeProfile::Workstation,
            true,
        )
        .map_err(|e| e.to_string())?;
        for value in [
            &self.owner.name,
            &self.owner.nickname,
            &self.owner.address_as,
            &self.owner.ai_alias,
        ] {
            if value.trim().is_empty()
                || value.chars().count() > 80
                || value.chars().any(char::is_control)
            {
                return Err("owner names and AI alias require 1..80 printable characters".into());
            }
        }
        if let Some(location) = &self.location {
            if !location.latitude.is_finite()
                || !(-90.0..=90.0).contains(&location.latitude)
                || !location.longitude.is_finite()
                || !(-180.0..=180.0).contains(&location.longitude)
                || !["operator", "node", "gnss"].contains(&location.source.as_str())
                || location.observed_at_ms <= 0
                || location
                    .accuracy_m
                    .is_some_and(|v| !v.is_finite() || v < 0.0)
            {
                return Err("invalid location or provenance".into());
            }
        }
        let consent = &self.perception;
        if ((consent.face_enrolment || consent.hand_enrolment) && !consent.camera)
            || (consent.voice_enrolment && !consent.microphone)
        {
            return Err("enrolment requires consent to its sensor".into());
        }
        if let Some(map) = &self.map {
            if map.mirrors.len() > 8 || map.mirrors.iter().any(|url| !mapdata::safe_mirror(url)) {
                return Err("at most eight credential-free HTTPS mirrors are allowed".into());
            }
            if map
                .partition
                .as_ref()
                .is_some_and(|p| !mapdata::safe_device(p))
            {
                return Err("invalid map partition device".into());
            }
            if !mapdata::safe_source(&map.url) || !mapdata::safe_destination(&map.destination) {
                return Err("invalid map source or persistent destination".into());
            }
        }
        Ok(())
    }
    pub fn map_choice(&self) -> MapChoice {
        self.map
            .as_ref()
            .map_or(MapChoice::Skip, |m| MapChoice::Download {
                region: MapRegion::new(&m.region, &m.url),
                destination: m.destination.clone().into(),
            })
    }
    /// TOML quoted strings use the JSON string escaping subset accepted here.
    /// Validation excludes control characters; names cannot inject TOML sections.
    pub fn owner_toml(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "\n[owner]\nname = {}\nnickname = {}\naddress_as = {}\nai_alias = {}\n",
            serde_json::to_string(&self.owner.name)?,
            serde_json::to_string(&self.owner.nickname)?,
            serde_json::to_string(&self.owner.address_as)?,
            serde_json::to_string(&self.owner.ai_alias)?
        ))
    }
    /// Stored privately. No recordings, templates, passwords or enrolment-success flags.
    pub fn preferences_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
