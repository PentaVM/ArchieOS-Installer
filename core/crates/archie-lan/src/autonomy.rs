//! Observation history and scheduling, never device authority.
use crate::inventory::{Device, Inventory, FORMAT};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub max_devices: usize,
    pub retention_seconds: u64,
    pub light_seconds: u64,
    pub deep_seconds: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            max_devices: 4096,
            retention_seconds: 86400,
            light_seconds: 30,
            deep_seconds: 600,
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        if !(1..=4096).contains(&self.max_devices)
            || !(1..=2_592_000).contains(&self.retention_seconds)
            || !(1..=3600).contains(&self.light_seconds)
            || self.deep_seconds < self.light_seconds
            || self.deep_seconds > 86400
        {
            return Err("unsafe autonomous monitor settings".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    device: Device,
    first_known_unix: u64,
    last_response_unix: Option<u64>,
    last_cache_unix: Option<u64>,
    #[serde(default)]
    last_neighbor_mac: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    format: String,
    #[serde(default)]
    scope: Option<String>,
    records: BTreeMap<String, Record>,
    last_success_unix: Option<u64>,
    last_deep_unix: Option<u64>,
    failures: u32,
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct Controller {
    settings: Settings,
    state: State,
}

fn cache_only(device: &Device) -> bool {
    !device.seen_by.is_empty()
        && device
            .seen_by
            .iter()
            .all(|s| matches!(s.as_str(), "neighbor-cache" | "history-cache"))
}

fn valid_device(device: &Device) -> Result<(), String> {
    let ip = device
        .address
        .parse::<std::net::IpAddr>()
        .map_err(|_| "invalid history device address")?;
    if !crate::is_lan(ip)
        || device.services.len() > 64
        || device.seen_by.len() > 8
        || device.state.len() > 64
    {
        return Err("history device outside scope or field bounds".into());
    }
    let fields = [&device.name, &device.model, &device.manufacturer]
        .into_iter()
        .chain(device.class.iter())
        .chain(device.services.iter())
        .chain(device.seen_by.iter())
        .chain(device.state.iter().flat_map(|(key, value)| [key, value]));
    if fields
        .into_iter()
        .any(|s| s.chars().count() > 256 || s.chars().any(char::is_control))
    {
        return Err("history contains an invalid field".into());
    }
    Ok(())
}

impl Controller {
    pub fn new(settings: Settings) -> Result<Self, String> {
        settings.validate()?;
        Ok(Self {
            settings,
            state: State {
                format: "archie-sentry-history-v1".into(),
                scope: None,
                records: BTreeMap::new(),
                last_success_unix: None,
                last_deep_unix: None,
                failures: 0,
                last_error: None,
            },
        })
    }
    pub fn restore(settings: Settings, bytes: &[u8]) -> Result<Self, String> {
        settings.validate()?;
        if bytes.len() > MAX_BYTES {
            return Err("history exceeds 8 MiB".into());
        }
        let state: State = serde_json::from_slice(bytes).map_err(|_| "invalid history JSON")?;
        if state.format != "archie-sentry-history-v1"
            || state.records.len() > settings.max_devices
            || state
                .last_error
                .as_ref()
                .is_some_and(|s| s.chars().count() > 200 || s.chars().any(char::is_control))
            || state
                .scope
                .as_ref()
                .is_some_and(|s| s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err("unsupported or oversized history".into());
        }
        for (key, record) in &state.records {
            if *key != record.device.address {
                return Err("history identity/address mismatch".into());
            }
            valid_device(&record.device)?;
            if record
                .last_neighbor_mac
                .as_ref()
                .is_some_and(|mac| mac.chars().count() > 256 || mac.chars().any(char::is_control))
            {
                return Err("invalid history neighbor metadata".into());
            }
        }
        Ok(Self { settings, state })
    }
    pub fn set_scope(&mut self, scope: &str) -> Result<(), String> {
        if scope.is_empty() || scope.len() > 512 || scope.chars().any(char::is_control) {
            return Err("invalid monitor interface scope".into());
        }
        if self.state.scope.as_deref() != Some(scope) {
            self.state.records.clear();
            self.state.last_success_unix = None;
            self.state.last_deep_unix = None;
            self.state.scope = Some(scope.into());
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(&self.state).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("history exceeds 8 MiB".into());
        }
        Ok(bytes)
    }
    pub fn deep_due(&self, now: u64) -> bool {
        self.state
            .last_deep_unix
            .is_none_or(|last| now >= last && now - last >= self.settings.deep_seconds)
    }
    pub fn record_failure(&mut self, error: &str) -> Result<u64, String> {
        self.state.failures = self.state.failures.saturating_add(1);
        self.state.last_error = Some(
            error
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect(),
        );
        Ok(self
            .settings
            .light_seconds
            .saturating_mul(1u64 << self.state.failures.saturating_sub(1).min(8))
            .min(300.max(self.settings.light_seconds)))
    }
    pub fn record_success(
        &mut self,
        inventory: &Inventory,
        deep: bool,
    ) -> Result<Inventory, String> {
        if inventory.format != FORMAT || inventory.devices.len() > self.settings.max_devices {
            return Err("invalid live inventory".into());
        }
        for device in &inventory.devices {
            valid_device(device)?;
        }
        let now = inventory.scanned_unix;
        let mut next = self.state.clone();
        for device in &inventory.devices {
            if next.records.len() >= self.settings.max_devices
                && !next.records.contains_key(&device.address)
            {
                let oldest = next
                    .records
                    .iter()
                    .min_by_key(|(_, r)| r.last_response_unix.unwrap_or(r.first_known_unix))
                    .map(|(key, _)| key.clone());
                if let Some(key) = oldest {
                    next.records.remove(&key);
                }
            }
            let record = next
                .records
                .entry(device.address.clone())
                .or_insert_with(|| Record {
                    device: device.clone(),
                    first_known_unix: now,
                    last_response_unix: None,
                    last_cache_unix: None,
                    last_neighbor_mac: None,
                });
            if cache_only(device) {
                let changed_mac = device
                    .state
                    .get("neighbor.mac")
                    .zip(
                        record
                            .last_neighbor_mac
                            .as_ref()
                            .or_else(|| record.device.state.get("neighbor.mac")),
                    )
                    .is_some_and(|(current, previous)| current != previous);
                if changed_mac {
                    // A reused IP is not evidence that the old named peer remains.
                    record.device = device.clone();
                    record.first_known_unix = now;
                    record.last_response_unix = None;
                }
                record.last_cache_unix = Some(now);
                if record.last_response_unix.is_none() {
                    record.device = device.clone();
                }
            } else {
                record.last_response_unix = Some(now);
                record.device = device.clone();
            }
            if let Some(mac) = device.state.get("neighbor.mac") {
                record.last_neighbor_mac = Some(mac.clone());
            }
        }
        next.records.retain(|_, record| {
            let anchor = record.last_response_unix.unwrap_or(record.first_known_unix);
            now.saturating_sub(anchor) <= self.settings.retention_seconds
        });
        let mut devices = Vec::new();
        for record in next.records.values() {
            let current = inventory
                .devices
                .iter()
                .find(|d| d.address == record.device.address);
            let mut device = if let Some(current) = current {
                current.clone()
            } else {
                record.device.clone()
            };
            if current.is_none() || current.is_some_and(cache_only) {
                device.name.clone_from(&record.device.name);
                device.model.clone_from(&record.device.model);
                device.manufacturer.clone_from(&record.device.manufacturer);
                device.class.clone_from(&record.device.class);
                device.services.clone_from(&record.device.services);
                // Old power/app/session state is never represented as current.
                device.state.clear();
                if let Some(current) = current {
                    for key in ["neighbor.mac", "neighbor.status", "neighbor.source"] {
                        if let Some(value) = current.state.get(key) {
                            device.state.insert(key.into(), value.clone());
                        }
                    }
                } else if let Some(mac) = record
                    .last_neighbor_mac
                    .as_ref()
                    .or_else(|| record.device.state.get("neighbor.mac"))
                {
                    device
                        .state
                        .insert("history.neighbor_mac".into(), mac.clone());
                }
                device.seen_by = vec!["history-cache".into()];
                device.state.insert(
                    "history.observation_status".into(),
                    if current.is_some() {
                        "cached"
                    } else {
                        "not-seen"
                    }
                    .into(),
                );
            }
            if cache_only(&device) && device.state.len() < 64 {
                if let Some(last) = record.last_response_unix {
                    device
                        .state
                        .insert("history.last_response_unix".into(), last.to_string());
                }
            }
            if cache_only(&device) && device.state.len() < 64 {
                if let Some(last) = record.last_cache_unix {
                    device
                        .state
                        .insert("history.last_cache_unix".into(), last.to_string());
                }
            }
            devices.push(device);
        }
        next.last_success_unix = Some(now);
        if deep {
            next.last_deep_unix = Some(now);
        }
        next.failures = 0;
        next.last_error = None;
        let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("history exceeds 8 MiB".into());
        }
        self.state = next;
        Ok(Inventory {
            format: FORMAT,
            scanned_unix: now,
            devices,
        })
    }
    pub fn health(&self, next_retry_seconds: u64) -> serde_json::Value {
        serde_json::json!({"format":"archie-sentry-health-v1","known_devices":self.state.records.len(),
            "last_success_unix":self.state.last_success_unix,"last_deep_unix":self.state.last_deep_unix,
            "consecutive_failures":self.state.failures,"last_error":self.state.last_error,
            "next_retry_seconds":next_retry_seconds})
    }
}
