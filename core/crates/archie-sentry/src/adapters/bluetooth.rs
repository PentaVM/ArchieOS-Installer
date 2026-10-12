//! Bluetooth adapter: BLE and Bluetooth Classic devices through the system
//! `bluetoothctl` binary (BlueZ, Linux), always via [`CommandRunner`] with an
//! absolute configured path. There is never a shell: every argument is a
//! separate argv element, so a value can never become syntax. Device
//! addresses, characteristic UUIDs and GATT payloads come from operator
//! configuration and are validated before they ever reach an argv.
//!
//! Supported contract commands:
//! - `SwitchSet` on a target with a configured [`BleSwitch`]: the configured
//!   on/off byte payload is written to `write_uuid`
//!   (`gatt.select-attribute <write_uuid>` then `gatt.write <bytes...>`).
//!   Postcondition, measured by `observe`: `gatt.read` on `read_uuid` returns
//!   exactly the configured on or off payload. A value that matches neither
//!   is `Protocol` - the adapter never guesses which state bytes mean.
//!
//! Connection management (not contract commands; provisioning-level helpers,
//! each with its own read-back):
//! - `scan(seconds)`: bounded discovery, `bluetoothctl --timeout <s> scan on`.
//! - `pair` / `connect` / `disconnect`: the action is followed by
//!   `bluetoothctl info <address>` and only accepted when the tool itself
//!   reports `Paired: yes` / `Connected: yes|no`. A reply without that marker
//!   is `Unavailable`, never assumed.
//!
//! Honesty notes, deliberately part of the module contract:
//! - Anything without a read-back is `Unsupported`: sensor telemetry,
//!   beacon data, brightness/color writes and every command kind besides
//!   `SwitchSet` on a switch-configured target. A target without a `switch`
//!   block is reachable through the connection helpers but never actuable.
//! - `gatt.read` output formats vary across BlueZ releases (inline hex,
//!   `0xNN` tokens, or a hexdump block with an ASCII gutter). The parser
//!   accepts 2-digit hex tokens after the `Value:` marker and fails with
//!   `Protocol` on anything else; a release that prints something different
//!   verifies nothing.
//! - Reading or writing a characteristic requires a connected, paired link.
//!   `observe`/`apply` connect first when `info` reports disconnected - the
//!   same documented pre-action an adb network target gets - but they never
//!   pair: pairing is an explicit operator call.
//! - BlueZ is a Linux stack. The constructor accepts any absolute path like
//!   the other process adapters, but on a host without bluetoothctl the
//!   runner's existence check refuses before any process exists.

use super::process::CommandRunner;
use crate::{
    contracts::{validate_id, Command, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

/// info/connect/disconnect round trips.
const BT_SHORT: Duration = Duration::from_secs(5);
/// A BLE link establishment can take several seconds; still bounded.
const BT_CONNECT: Duration = Duration::from_secs(15);
/// select-attribute + read/write against one characteristic.
const BT_GATT: Duration = Duration::from_secs(8);
/// Discovery results are bounded; a scan is a survey, not a census.
const MAX_SCAN: usize = 64;
/// GATT on/off payloads are small by nature; 32 bytes is generous.
const MAX_PAYLOAD: usize = 32;

/// A device's address: six hex octets joined by colons. Either case is
/// accepted on input; argv always carries BlueZ's canonical uppercase.
pub fn parse_address(text: &str) -> Option<String> {
    let octets: Vec<&str> = text.split(':').collect();
    if octets.len() != 6 {
        return None;
    }
    if !octets
        .iter()
        .all(|o| o.len() == 2 && o.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(octets.join(":").to_ascii_uppercase())
}

/// A characteristic UUID: the 4-hex-digit short form or the full
/// 36-character lowercase form (`00002a19-0000-1000-8000-00805f9b34fb`).
pub fn valid_uuid(text: &str) -> bool {
    if text.len() == 4 {
        return text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    }
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}

/// A configured GATT payload: even-length hex, at least one byte, bounded.
pub fn parse_hex_payload(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() % 2 != 0 || text.len() / 2 > MAX_PAYLOAD {
        return None;
    }
    if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some(bytes)
}

/// GATT wiring for a switch-like characteristic pair. `on`/`off` are hex
/// byte strings (e.g. `"01"`/`"00"`); the read-back must equal one of them
/// exactly, or the state is honestly unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BleSwitch {
    /// Characteristic the state is read back from after acting.
    pub read_uuid: String,
    /// Characteristic the on/off payload is written to.
    pub write_uuid: String,
    /// Hex bytes that mean and request "on".
    pub on: String,
    /// Hex bytes that mean and request "off".
    pub off: String,
}

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BleTarget {
    /// Device address (`AA:BB:CC:DD:EE:FF`); case-insensitive on input.
    pub address: String,
    /// Without this block the target is connection-managed but never actuable.
    #[serde(default)]
    pub switch: Option<BleSwitch>,
}

/// What `bluetoothctl info <address>` said. Fields that did not parse are
/// unknown and verify nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkInfo {
    pub paired: Option<bool>,
    pub connected: Option<bool>,
}

fn yes_no(line: &str, key: &str) -> Option<bool> {
    let trimmed = line.trim_start();
    let value = trimmed.strip_prefix(key)?.trim();
    match value {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// `bluetoothctl info` output: `Paired:` and `Connected:` markers only.
/// A missing device prints "Device <address> not available" and yields all
/// unknowns - which then fails honestly at the caller.
pub fn parse_info(output: &str) -> LinkInfo {
    let mut info = LinkInfo::default();
    for line in output.lines() {
        if info.paired.is_none() {
            info.paired = yes_no(line, "Paired:");
        }
        if info.connected.is_none() {
            info.connected = yes_no(line, "Connected:");
        }
    }
    info
}

/// Discovery lines (`[NEW] Device AA:BB:CC:DD:EE:FF Name`): address and name,
/// deduplicated by address, bounded. Names are display data, not evidence:
/// control characters are stripped and length is capped.
pub fn parse_scan(output: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        if found.len() >= MAX_SCAN {
            break;
        }
        let trimmed = line.trim_start();
        // A `[NEW]` tag or no tag (the `devices` listing) announces a device;
        // `[CHG]`/`[DEL]` lines carry property updates, not discoveries.
        let rest = if let Some(rest) = trimmed.strip_prefix("[NEW]") {
            rest.trim_start()
        } else if trimmed.starts_with('[') {
            continue;
        } else {
            trimmed
        };
        let Some(rest) = rest.strip_prefix("Device ") else {
            continue;
        };
        let Some(address) = rest.split_whitespace().next().and_then(parse_address) else {
            continue;
        };
        let name: String = rest[address.len()..]
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .chars()
            .take(64)
            .collect();
        if !found.iter().any(|(a, _)| *a == address) {
            found.push((address, name));
        }
    }
    found
}

/// The byte value of a `gatt.read` reply: 2-digit hex tokens (optionally
/// `0x`-prefixed) on the `Value:` line and up to three continuation lines.
/// Anything else - a missing marker, prose, a longer dump - is unknown.
pub fn parse_gatt_value(output: &str) -> Option<Vec<u8>> {
    let pos = output.find("Value:")?;
    let mut bytes = Vec::new();
    for line in output[pos + "Value:".len()..].lines().take(4) {
        let mut on_this_line = 0usize;
        for token in line.split_whitespace() {
            let hex = token.strip_prefix("0x").unwrap_or(token);
            if hex.len() == 2 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                on_this_line += 1;
            }
        }
        if !bytes.is_empty() && on_this_line == 0 {
            break; // the dump is over; do not absorb unrelated lines
        }
    }
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

pub struct Bluetooth {
    edge: String,
    adapter: String,
    /// Absolute path of bluetoothctl; existence is enforced by the runner.
    bluetoothctl: String,
    targets: BTreeMap<String, BleTarget>,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for Bluetooth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bluetooth")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

impl Bluetooth {
    /// `bluetoothctl` must be an absolute path;
    /// [`super::process::ProcessRunner`] additionally requires the file to
    /// exist before any process is spawned.
    pub fn new(
        edge: &str,
        adapter: &str,
        bluetoothctl: &str,
        targets: BTreeMap<String, BleTarget>,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        if !Path::new(bluetoothctl).is_absolute() {
            return Err(AdapterError::Protocol);
        }
        if targets.is_empty() || targets.len() > 64 {
            return Err(AdapterError::Protocol);
        }
        let mut normalized = BTreeMap::new();
        for (resource, target) in targets {
            validate_id(&resource).map_err(|_| AdapterError::Protocol)?;
            let address = parse_address(&target.address).ok_or(AdapterError::Protocol)?;
            let switch = match target.switch {
                Some(switch) => {
                    if !valid_uuid(&switch.read_uuid) || !valid_uuid(&switch.write_uuid) {
                        return Err(AdapterError::Protocol);
                    }
                    // Payloads must parse and must not alias each other, or a
                    // read-back could never distinguish the states.
                    let on = parse_hex_payload(&switch.on).ok_or(AdapterError::Protocol)?;
                    let off = parse_hex_payload(&switch.off).ok_or(AdapterError::Protocol)?;
                    if on == off {
                        return Err(AdapterError::Protocol);
                    }
                    Some(BleSwitch {
                        on: on.iter().map(|b| format!("{b:02x}")).collect(),
                        off: off.iter().map(|b| format!("{b:02x}")).collect(),
                        ..switch
                    })
                }
                None => None,
            };
            normalized.insert(resource, BleTarget { address, switch });
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            bluetoothctl: bluetoothctl.into(),
            targets: normalized,
            runner,
        })
    }
    fn target_of(&self, resource: &ResourceDescriptor) -> Result<&BleTarget, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.targets
            .get(&resource.resource_id)
            .ok_or(AdapterError::Unsupported)
    }
    /// One bluetoothctl invocation. A non-zero exit status means the tool
    /// itself failed: `Unavailable`, never a parse of error text as data.
    fn run(&self, args: &[String], timeout: Duration) -> Result<String, AdapterError> {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(self.bluetoothctl.clone());
        argv.extend(args.iter().cloned());
        let output = self.runner.run(&argv, timeout)?;
        if output.status != 0 {
            return Err(AdapterError::Unavailable);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn info(&self, address: &str) -> Result<LinkInfo, AdapterError> {
        let report = self.run(&["info".to_string(), address.to_string()], BT_SHORT)?;
        Ok(parse_info(&report))
    }
    /// A configured address is the only address the connection helpers serve.
    fn configured(&self, address: &str) -> Result<String, AdapterError> {
        let address = parse_address(address).ok_or(AdapterError::Protocol)?;
        if self.targets.values().any(|t| t.address == address) {
            Ok(address)
        } else {
            Err(AdapterError::Unsupported)
        }
    }
    /// Connect when not already connected; afterwards `info` must confirm.
    /// Pairing is never implicit: an unpaired device is `Unavailable`.
    fn ensure_connected(&self, address: &str) -> Result<(), AdapterError> {
        let info = self.info(address)?;
        if info.paired == Some(false) {
            return Err(AdapterError::Unavailable);
        }
        if info.connected == Some(true) {
            return Ok(());
        }
        self.run(&["connect".to_string(), address.to_string()], BT_CONNECT)?;
        match self.info(address)?.connected {
            Some(true) => Ok(()),
            _ => Err(AdapterError::Unavailable),
        }
    }
    /// Bounded discovery: addresses and display names seen during the window.
    pub fn scan(&self, seconds: u8) -> Result<Vec<(String, String)>, AdapterError> {
        if !(1..=30).contains(&seconds) {
            return Err(AdapterError::Protocol);
        }
        let report = self.run(
            &[
                "--timeout".to_string(),
                seconds.to_string(),
                "scan".to_string(),
                "on".to_string(),
            ],
            Duration::from_secs(u64::from(seconds)) + BT_SHORT,
        )?;
        Ok(parse_scan(&report))
    }
    /// Pair a configured device; accepted only when `info` reports paired.
    pub fn pair(&self, address: &str) -> Result<(), AdapterError> {
        let address = self.configured(address)?;
        self.run(&["pair".to_string(), address.clone()], BT_CONNECT)?;
        match self.info(&address)?.paired {
            Some(true) => Ok(()),
            _ => Err(AdapterError::Unavailable),
        }
    }
    /// Connect a configured device; accepted only when `info` confirms.
    pub fn connect(&self, address: &str) -> Result<(), AdapterError> {
        let address = self.configured(address)?;
        self.ensure_connected(&address)
    }
    /// Disconnect a configured device; accepted only when `info` confirms the
    /// link is down.
    pub fn disconnect(&self, address: &str) -> Result<(), AdapterError> {
        let address = self.configured(address)?;
        if self.info(&address)?.connected == Some(false) {
            return Ok(());
        }
        self.run(&["disconnect".to_string(), address.clone()], BT_SHORT)?;
        match self.info(&address)?.connected {
            Some(false) => Ok(()),
            _ => Err(AdapterError::Unavailable),
        }
    }
    /// Connect, select the configured read characteristic and read it.
    fn read_switch(&self, target: &BleTarget, uuid: &str) -> Result<Vec<u8>, AdapterError> {
        self.ensure_connected(&target.address)?;
        self.run(
            &["gatt.select-attribute".to_string(), uuid.to_string()],
            BT_GATT,
        )?;
        let report = self.run(&["gatt.read".to_string()], BT_GATT)?;
        parse_gatt_value(&report).ok_or(AdapterError::Protocol)
    }
}

#[async_trait]
impl EdgeAdapter for Bluetooth {
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
        let target = self.target_of(resource)?;
        let switch = target.switch.as_ref().ok_or(AdapterError::Unsupported)?;
        let value = self.read_switch(target, &switch.read_uuid)?;
        let on = parse_hex_payload(&switch.on).ok_or(AdapterError::Protocol)?;
        let off = parse_hex_payload(&switch.off).ok_or(AdapterError::Protocol)?;
        // A value that is neither configured state is honestly unknown.
        let state = if value == on {
            ResourceState::Switch { on: true }
        } else if value == off {
            ResourceState::Switch { on: false }
        } else {
            return Err(AdapterError::Protocol);
        };
        let report = serde_json::to_vec(&(&resource.resource_id, &state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            // Identical measurements keep one evidence identity; no freshness
            // is invented for a state that did not change.
            evidence_id: format!("ble-{:x}", Sha256::digest(report)),
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
        let target = self.target_of(resource)?;
        match command {
            Command::SwitchSet { on } => {
                let switch = target.switch.as_ref().ok_or(AdapterError::Unsupported)?;
                self.ensure_connected(&target.address)?;
                self.run(
                    &[
                        "gatt.select-attribute".to_string(),
                        switch.write_uuid.clone(),
                    ],
                    BT_GATT,
                )?;
                let mut args = vec!["gatt.write".to_string()];
                // Bytes arrive as separate argv elements; bluetoothctl joins
                // its argv into one command line, so `write 01 02` is data.
                args.extend(
                    parse_hex_payload(if *on { &switch.on } else { &switch.off })
                        .ok_or(AdapterError::Protocol)?
                        .iter()
                        .map(|b| format!("{b:02x}")),
                );
                self.run(&args, BT_GATT)?;
                Ok(AdapterAck { accepted: true })
            }
            // Everything else has no readable postcondition here: honest
            // Unsupported, never an unverifiable write.
            _ => Err(AdapterError::Unsupported),
        }
    }
}
