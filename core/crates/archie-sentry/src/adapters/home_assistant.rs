use super::{body, client, endpoint};
use crate::{
    contracts::*,
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, Url,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub struct HomeAssistant {
    edge: String,
    adapter: String,
    base: Url,
    client: Client,
    authorization: HeaderValue,
    entities: BTreeMap<String, String>,
    climate_preflight: Option<(String, Command, Instant)>,
}
impl std::fmt::Debug for HomeAssistant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeAssistant")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("credentials", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl HomeAssistant {
    pub fn new(
        edge: &str,
        adapter: &str,
        base: &str,
        token: &str,
        entities: BTreeMap<String, String>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        let base = endpoint(base, true)?;
        if base.path() != "/"
            || token.is_empty()
            || token.len() > 4096
            || !token.bytes().all(|b| b.is_ascii_graphic())
            || entities.is_empty()
            || entities.len() > 4096
        {
            return Err(AdapterError::Protocol);
        }
        for (resource, entity) in &entities {
            validate_id(resource).map_err(|_| AdapterError::Protocol)?;
            let (domain, name) = entity.split_once('.').ok_or(AdapterError::Protocol)?;
            if !matches!(domain, "switch" | "light" | "media_player" | "climate")
                || name.is_empty()
                || name.len() > 100
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(AdapterError::Protocol);
            }
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AdapterError::Authentication)?;
        authorization.set_sensitive(true);
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            base,
            client: client()?,
            authorization,
            entities,
            climate_preflight: None,
        })
    }
    fn entity(&self, resource: &ResourceDescriptor) -> Result<&str, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.entities
            .get(&resource.resource_id)
            .map(String::as_str)
            .ok_or(AdapterError::Unsupported)
    }
    async fn service(&self, domain: &str, service: &str, data: Value) -> Result<(), AdapterError> {
        let bytes = serde_json::to_vec(&data).map_err(|_| AdapterError::Protocol)?;
        let response = self
            .client
            .post(
                self.base
                    .join(&format!("api/services/{domain}/{service}"))
                    .map_err(|_| AdapterError::Protocol)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Content-Type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "application/json").await?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| AdapterError::Protocol)?;
        if !value.is_array() {
            return Err(AdapterError::Protocol);
        }
        Ok(())
    }
    async fn require_celsius(&self) -> Result<(), AdapterError> {
        let response = self
            .client
            .get(
                self.base
                    .join("api/config")
                    .map_err(|_| AdapterError::Protocol)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "application/json").await?;
        let config: Value = serde_json::from_slice(&bytes).map_err(|_| AdapterError::Protocol)?;
        if config
            .get("unit_system")
            .and_then(|v| v.get("temperature"))
            .and_then(Value::as_str)
            != Some("°C")
        {
            return Err(AdapterError::Unsupported);
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct HaState {
    entity_id: String,
    state: String,
    attributes: Value,
    last_reported: Option<String>,
    last_updated: Option<String>,
}
#[async_trait]
impl EdgeAdapter for HomeAssistant {
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
        let entity = self.entity(resource)?.to_owned();
        // Refresh the integration rather than inventing freshness when reading HA's cache.
        self.service(
            "homeassistant",
            "update_entity",
            json!({"entity_id":entity}),
        )
        .await?;
        let response = self
            .client
            .get(
                self.base
                    .join(&format!("api/states/{entity}"))
                    .map_err(|_| AdapterError::Protocol)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let bytes = body(response, "application/json").await?;
        let value: HaState = serde_json::from_slice(&bytes).map_err(|_| AdapterError::Protocol)?;
        if value.entity_id != entity {
            return Err(AdapterError::Protocol);
        }
        let stamp = value
            .last_reported
            .or(value.last_updated)
            .ok_or(AdapterError::Protocol)?;
        let parsed =
            time::OffsetDateTime::parse(&stamp, &time::format_description::well_known::Rfc3339)
                .map_err(|_| AdapterError::Protocol)?;
        let millis = i64::try_from(parsed.unix_timestamp_nanos() / 1_000_000)
            .map_err(|_| AdapterError::Protocol)?;
        let domain = entity.split_once('.').ok_or(AdapterError::Protocol)?.0;
        let state = match (domain, value.state.as_str()) {
            ("switch", "on") => ResourceState::Switch { on: true },
            ("switch", "off") => ResourceState::Switch { on: false },
            ("media_player", "playing") => ResourceState::Media { playing: true },
            ("media_player", "paused") => ResourceState::Media { playing: false },
            ("light", "off") => ResourceState::Light {
                on: false,
                brightness_percent: 0,
            },
            ("light", "on") => {
                let brightness = value
                    .attributes
                    .get("brightness")
                    .and_then(Value::as_u64)
                    .filter(|b| *b <= 255)
                    .ok_or(AdapterError::Protocol)?;
                ResourceState::Light {
                    on: true,
                    brightness_percent: ((brightness * 100 + 127) / 255) as u8,
                }
            }
            ("climate", state) if !matches!(state, "unavailable" | "unknown") => {
                self.require_celsius().await?;
                let temperature = value
                    .attributes
                    .get("temperature")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && (5.0..=35.0).contains(n))
                    .ok_or(AdapterError::Protocol)?;
                ResourceState::Climate {
                    target_milli_celsius: (temperature * 1000.0).round() as i32,
                }
            }
            _ => return Err(AdapterError::Unavailable),
        };
        let report =
            serde_json::to_vec(&(&entity, millis, &state)).map_err(|_| AdapterError::Protocol)?;
        let evidence_id = format!("ha-{:x}", Sha256::digest(report));
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id,
            observed_at: Timestamp(millis),
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    async fn preflight(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<(), AdapterError> {
        self.climate_preflight = None;
        command.validate().map_err(|_| AdapterError::Protocol)?;
        if matches!(command, Command::ClimateSetTemperature { .. }) {
            if !self.entity(resource)?.starts_with("climate.")
                || !resource.capabilities.contains(&command.capability())
            {
                return Err(AdapterError::Unsupported);
            }
            self.require_celsius().await?;
            self.climate_preflight = Some((
                resource.resource_id.clone(),
                command.clone(),
                Instant::now(),
            ));
        }
        Ok(())
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
        let entity = self.entity(resource)?.to_owned();
        let domain = entity.split_once('.').ok_or(AdapterError::Protocol)?.0;
        let mut data = json!({"entity_id":entity});
        let service = match (domain, command) {
            ("switch", Command::SwitchSet { on: true }) => "turn_on",
            ("switch", Command::SwitchSet { on: false }) => "turn_off",
            ("media_player", Command::MediaPause) => "media_pause",
            ("media_player", Command::MediaPlay) => "media_play",
            ("light", Command::LightSetBrightness { percent: 0 }) => "turn_off",
            ("light", Command::LightSetBrightness { percent }) => {
                data["brightness_pct"] = json!(percent);
                "turn_on"
            }
            ("climate", Command::ClimateSetTemperature { milli_celsius }) => {
                let (id, prepared, at) = self
                    .climate_preflight
                    .take()
                    .ok_or(AdapterError::Unsupported)?;
                if id != resource.resource_id
                    || prepared != *command
                    || at.elapsed() > Duration::from_secs(5)
                {
                    return Err(AdapterError::Unsupported);
                }
                data["temperature"] = json!(*milli_celsius as f64 / 1000.0);
                "set_temperature"
            }
            _ => return Err(AdapterError::Unsupported),
        };
        self.service(domain, service, data).await?;
        Ok(AdapterAck { accepted: true })
    }
}
