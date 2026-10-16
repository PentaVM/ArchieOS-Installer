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
    /// What the last deep pass saw of this device through [`DEEP_SOURCES`], if anything.
    #[serde(default)]
    deep: Option<Device>,
}

impl Record {
    /// The last time anything placed this device here: an answer, a cache entry, or its
    /// first appearance. Retention runs from this.
    fn anchor(&self) -> u64 {
        self.last_response_unix
            .unwrap_or(self.first_known_unix)
            .max(self.last_cache_unix.unwrap_or(0))
    }
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
    archie_types::lan::cache_only(&device.seen_by)
}

/// Sources only a deep pass runs. What they saw stands until the next deep pass, so a
/// light pass in between neither loses it nor makes devices and their fields flicker.
pub const DEEP_SOURCES: [&str; 2] = ["tcp-presence", "mdns-sd"];

fn deep_key(key: &str) -> bool {
    key == "open_tcp_ports" || key.starts_with("mdns.")
}

/// The part of a device a deep source vouched for: who it is, the deep sources that saw
/// it and their fields. Never its power, app or session state, which only a pass that
/// heard it now may report.
fn deep_part(device: &Device) -> Option<Device> {
    let seen_by: Vec<String> = device
        .seen_by
        .iter()
        .filter(|s| DEEP_SOURCES.contains(&s.as_str()))
        .cloned()
        .collect();
    (!seen_by.is_empty()).then(|| Device {
        address: device.address.clone(),
        class: device.class.clone(),
        name: device.name.clone(),
        manufacturer: device.manufacturer.clone(),
        model: device.model.clone(),
        services: device.services.clone(),
        seen_by,
        state: device
            .state
            .iter()
            .filter(|(key, _)| deep_key(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    })
}

/// A light pass's device with the last deep pass's evidence added back.
fn with_deep(mut device: Device, deep: &Device) -> Device {
    for source in &deep.seen_by {
        if !device.seen_by.contains(source) && device.seen_by.len() < 8 {
            device.seen_by.push(source.clone());
        }
    }
    for (key, value) in &deep.state {
        if device.state.len() < 64 {
            device
                .state
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    for service in &deep.services {
        if device.services.len() < 64 && !device.services.contains(service) {
            device.services.push(service.clone());
        }
    }
    if device.class.is_none() {
        device.class.clone_from(&deep.class);
    }
    for (slot, value) in [
        (&mut device.name, &deep.name),
        (&mut device.model, &deep.model),
        (&mut device.manufacturer, &deep.manufacturer),
    ] {
        if slot.is_empty() {
            slot.clone_from(value);
        }
    }
    device
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
            if let Some(deep) = &record.deep {
                if deep.address != *key {
                    return Err("history identity/address mismatch".into());
                }
                valid_device(deep)?;
            }
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
            // A recorded time ahead of now means the clock stepped back. A small step
            // (within one deep interval) is waited out; a larger one runs the deep pass
            // rather than suppress it until wall time catches up, hours or years later.
            .is_none_or(|last| {
                if now >= last {
                    now - last >= self.settings.deep_seconds
                } else {
                    last - now > self.settings.deep_seconds
                }
            })
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
        // Times ahead of now were written before the clock stepped back. They are read as
        // now, so retention and scheduling measure from something that has happened.
        for record in next.records.values_mut() {
            record.first_known_unix = record.first_known_unix.min(now);
            record.last_response_unix = record.last_response_unix.map(|t| t.min(now));
            record.last_cache_unix = record.last_cache_unix.map(|t| t.min(now));
        }
        next.last_success_unix = next.last_success_unix.map(|t| t.min(now));
        next.last_deep_unix = next.last_deep_unix.map(|t| t.min(now));
        let live: BTreeMap<&str, &Device> = inventory
            .devices
            .iter()
            .map(|device| (device.address.as_str(), device))
            .collect();
        // Room for the newcomers is made once, from the records this pass did not see,
        // oldest first; the live inventory itself is never larger than the limit.
        let incoming = live
            .keys()
            .filter(|address| !next.records.contains_key(**address))
            .count();
        let overflow = (next.records.len() + incoming).saturating_sub(self.settings.max_devices);
        if overflow > 0 {
            let mut unseen: Vec<(u64, String)> = next
                .records
                .iter()
                .filter(|(address, _)| !live.contains_key(address.as_str()))
                .map(|(address, record)| (record.anchor(), address.clone()))
                .collect();
            unseen.sort();
            for (_, address) in unseen.into_iter().take(overflow) {
                next.records.remove(&address);
            }
        }
        for device in &inventory.devices {
            let record = next
                .records
                .entry(device.address.clone())
                .or_insert_with(|| Record {
                    device: device.clone(),
                    first_known_unix: now,
                    last_response_unix: None,
                    last_cache_unix: None,
                    last_neighbor_mac: None,
                    deep: None,
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
                    record.deep = None;
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
            if deep {
                record.deep = deep_part(device);
            }
        }
        if deep {
            for (address, record) in &mut next.records {
                if !live.contains_key(address.as_str()) {
                    record.deep = None;
                }
            }
        }
        let retention = self.settings.retention_seconds;
        next.records.retain(|address, record| {
            live.contains_key(address.as_str()) || now - record.anchor() <= retention
        });
        let mut devices = Vec::new();
        for (address, record) in &next.records {
            // On a light pass, what the last deep pass saw still stands.
            let carried = if deep { None } else { record.deep.as_ref() };
            let current = match (live.get(address.as_str()), carried) {
                (Some(live), Some(deep)) => Some(with_deep((*live).clone(), deep)),
                (Some(live), None) => Some((*live).clone()),
                (None, Some(deep)) => Some(deep.clone()),
                (None, None) => None,
            };
            let mut device = current.clone().unwrap_or_else(|| record.device.clone());
            if current.as_ref().is_none_or(cache_only) {
                device.name.clone_from(&record.device.name);
                device.model.clone_from(&record.device.model);
                device.manufacturer.clone_from(&record.device.manufacturer);
                device.class.clone_from(&record.device.class);
                device.services.clone_from(&record.device.services);
                // Old power/app/session state is never represented as current.
                device.state.clear();
                if let Some(current) = &current {
                    for key in ["neighbor.mac", "neighbor.source"] {
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
            // Sources in one order, whichever pass listed them first.
            device.seen_by.sort();
            device.seen_by.dedup();
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
