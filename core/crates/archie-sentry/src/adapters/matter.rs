//! Matter adapter: controls Matter/Thread devices (plugs, lights, thermostats)
//! through the system `chip-tool` binary, always via [`CommandRunner`] with an
//! absolute configured path. There is never a shell: every argument is a
//! separate argv element, so a value can never become syntax. Node id and
//! endpoint come from operator configuration and are validated numerically
//! before they ever reach an argv.
//!
//! Supported commands, each verified by the engine through `observe`:
//! - `SwitchSet`: `chip-tool onoff on|off <node> <endpoint>`. Postcondition:
//!   `chip-tool onoff read on-off <node> <endpoint>` parsed as the OnOff
//!   boolean.
//! - `LightSetBrightness`: percent 0-100 is mapped onto the LevelControl
//!   `CurrentLevel` range 0-254 and applied with
//!   `chip-tool levelcontrol move-to-level-with-on-off <level> 0 0 0 <node>
//!   <endpoint>` (level 0 also turns the device off, so `percent == 0` reads
//!   back as `on: false, brightness: 0`). Postcondition: `onoff read on-off`
//!   plus `levelcontrol read current-level`, mapped back to a percent. A
//!   device that reports a level beyond 254, or no level at all, makes the
//!   observation fail honestly instead of guessing.
//! - `ClimateSetTemperature`: milli-Celsius is rendered as the thermostat
//!   cluster's hundredths-of-a-degree `OccupiedHeatingSetpoint` and applied
//!   with `chip-tool thermostat write occupied-heating-setpoint <centi>
//!   <node> <endpoint>`. Postcondition: `thermostat read
//!   occupied-heating-setpoint`. Values not divisible by 10 milli-Celsius
//!   lose precision in the render; the read-back then honestly diverges from
//!   the request instead of pretending to match.
//!
//! Provisioning (not contract commands, so not policy-gated actuations):
//! `pair_code` runs `chip-tool pairing code <node> <code>` and
//! `pair_onnetwork` runs `chip-tool pairing onnetwork <node> <pin>` for an
//! already-commissioned-network device. Both require chip-tool to print a
//! commissioning-success marker; a zero exit status without the marker is
//! `Unavailable`, never assumed success.
//!
//! Honesty notes, deliberately part of the module contract:
//! - Door locks, color control, scenes, and sensors have no contract command
//!   (or no readable postcondition) and are `Unsupported`. This adapter never
//!   sends an invocation it cannot read back.
//! - chip-tool's human-readable attribute report (`OnOff: TRUE`,
//!   `CurrentLevel: 127`, `OccupiedHeatingSetpoint: 2250`) varies across
//!   connectedhomeip releases. The parsers accept the documented markers and
//!   fail with `Protocol` when a release prints something else; a read-back
//!   that does not parse verifies nothing.
//! - A commissioned fabric, a reachable Thread border router, and the
//!   chip-tool binary itself are operator prerequisites. This adapter never
//!   installs, commissions, or discovers anything on its own beyond the two
//!   explicit pairing calls above.

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

/// On/off, level and thermostat invocations plus their attribute read-backs.
const CHIP_CALL: Duration = Duration::from_secs(15);
/// Commissioning waits for the full PASE/CASE exchange; still bounded.
const CHIP_PAIR: Duration = Duration::from_secs(120);
/// LevelControl's CurrentLevel maximum (Matter spec: 0x00..=0xFE).
const LEVEL_MAX: u32 = 254;

/// Which cluster family a configured resource is actuated and read through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterKind {
    /// OnOff cluster: plugs and simple switches.
    Switch,
    /// OnOff + LevelControl clusters: dimmable lights.
    Light,
    /// Thermostat cluster: occupied heating setpoint.
    Climate,
}

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatterTarget {
    /// Commissioned node id on the fabric; never 0.
    pub node_id: u64,
    /// Endpoint carrying the clusters; endpoint 0 is the root node and is
    /// never an actuable device here.
    pub endpoint: u16,
    pub kind: MatterKind,
}

#[derive(Clone)]
pub struct Matter {
    edge: String,
    adapter: String,
    /// Absolute path of the chip-tool binary; existence is enforced by the runner.
    chip_tool: String,
    targets: BTreeMap<String, MatterTarget>,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for Matter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Matter")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

/// Percent 0-100 → CurrentLevel 0-254, rounded to nearest.
pub fn level_for_percent(percent: u8) -> u8 {
    ((u32::from(percent) * LEVEL_MAX + 50) / 100) as u8
}
/// CurrentLevel → percent 0-100, rounded to nearest. Levels above 254 are not
/// Matter evidence.
pub fn percent_for_level(level: u8) -> Option<u8> {
    if u32::from(level) > LEVEL_MAX {
        return None;
    }
    u8::try_from((u32::from(level) * 100 + LEVEL_MAX / 2) / LEVEL_MAX).ok()
}

/// Find `name` as a whole token (not a suffix of a longer identifier) and
/// return the text right after its colon.
fn attribute_tail<'a>(output: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}:");
    let mut rest = output;
    while let Some(pos) = rest.find(&needle) {
        let boundary = pos == 0
            || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[pos - 1] != b'_';
        if boundary {
            return Some(rest[pos + needle.len()..].trim_start());
        }
        rest = &rest[pos + needle.len()..];
    }
    None
}

/// A boolean attribute from chip-tool's report, e.g. `OnOff: TRUE`. Anything
/// else is unknown, never guessed.
pub fn parse_bool_attribute(output: &str, name: &str) -> Option<bool> {
    let tail = attribute_tail(output, name)?;
    if tail.starts_with("TRUE") {
        Some(true)
    } else if tail.starts_with("FALSE") {
        Some(false)
    } else {
        None
    }
}

/// A signed integer attribute from chip-tool's report, e.g.
/// `OccupiedHeatingSetpoint: 2250`. Anything else is unknown, never guessed.
pub fn parse_int_attribute(output: &str, name: &str) -> Option<i64> {
    let tail = attribute_tail(output, name)?;
    let (sign, digits) = match tail.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, tail),
    };
    let text: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if text.is_empty() {
        return None;
    }
    text.parse::<i64>().ok().map(|value| sign * value)
}

/// chip-tool confirms commissioning with one of these markers. A zero exit
/// status without a marker is not success.
pub fn pairing_succeeded(output: &str) -> bool {
    [
        "Commissioning complete",
        "Secure Pairing Success",
        "Pairing Success",
    ]
    .iter()
    .any(|marker| output.contains(marker))
}

/// A pairing code is either the QR payload (base38 alphabet plus `MT:` prefix
/// characters) or the all-digits manual code; bounded and free of anything
/// that could matter to a shell (there is no shell, but argv stays clean data).
fn valid_pairing_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b':' | b'-' | b'.' | b'$' | b'%' | b'*' | b'+' | b'/' | b'=' | b'?'
                )
        })
}

impl Matter {
    /// `chip_tool` must be an absolute path; [`super::process::ProcessRunner`]
    /// additionally requires the file to exist before any process is spawned.
    pub fn new(
        edge: &str,
        adapter: &str,
        chip_tool: &str,
        targets: BTreeMap<String, MatterTarget>,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        if !Path::new(chip_tool).is_absolute() {
            return Err(AdapterError::Protocol);
        }
        if targets.is_empty() || targets.len() > 64 {
            return Err(AdapterError::Protocol);
        }
        for (resource, target) in &targets {
            validate_id(resource).map_err(|_| AdapterError::Protocol)?;
            if target.node_id == 0 || target.endpoint == 0 {
                return Err(AdapterError::Protocol);
            }
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            chip_tool: chip_tool.into(),
            targets,
            runner,
        })
    }
    fn target_of(&self, resource: &ResourceDescriptor) -> Result<MatterTarget, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.targets
            .get(&resource.resource_id)
            .copied()
            .ok_or(AdapterError::Unsupported)
    }
    /// One chip-tool invocation. A non-zero exit status means chip-tool itself
    /// failed: `Unavailable`, never a parse of error text as if it were data.
    fn run(&self, args: &[String], timeout: Duration) -> Result<String, AdapterError> {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(self.chip_tool.clone());
        argv.extend(args.iter().cloned());
        let output = self.runner.run(&argv, timeout)?;
        if output.status != 0 {
            return Err(AdapterError::Unavailable);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn cluster_call(
        &self,
        target: &MatterTarget,
        cluster: &str,
        verb: &str,
        extra: &[String],
    ) -> Result<String, AdapterError> {
        let mut args = vec![cluster.to_string(), verb.to_string()];
        args.extend(extra.iter().cloned());
        args.push(target.node_id.to_string());
        args.push(target.endpoint.to_string());
        self.run(&args, CHIP_CALL)
    }
    fn read_attribute(
        &self,
        target: &MatterTarget,
        cluster: &str,
        attribute: &str,
    ) -> Result<String, AdapterError> {
        self.cluster_call(target, cluster, "read", &[attribute.to_string()])
    }
    /// Commission a device with its setup code (QR payload or manual code).
    /// Provisioning, not an actuation: no contract postcondition exists, so
    /// success is the tool's own commissioning marker, honestly required.
    pub fn pair_code(&self, node_id: u64, code: &str) -> Result<(), AdapterError> {
        if node_id == 0 || !valid_pairing_code(code) {
            return Err(AdapterError::Protocol);
        }
        let reply = self.run(
            &[
                "pairing".to_string(),
                "code".to_string(),
                node_id.to_string(),
                code.to_string(),
            ],
            CHIP_PAIR,
        )?;
        if !pairing_succeeded(&reply) {
            return Err(AdapterError::Unavailable);
        }
        Ok(())
    }
    /// Commission a device already on the IP network with its setup PIN.
    pub fn pair_onnetwork(&self, node_id: u64, pin: u32) -> Result<(), AdapterError> {
        if node_id == 0 || pin > 99_999_999 {
            return Err(AdapterError::Protocol);
        }
        let reply = self.run(
            &[
                "pairing".to_string(),
                "onnetwork".to_string(),
                node_id.to_string(),
                pin.to_string(),
            ],
            CHIP_PAIR,
        )?;
        if !pairing_succeeded(&reply) {
            return Err(AdapterError::Unavailable);
        }
        Ok(())
    }
    /// The measured state of one resource, per its configured kind. A read
    /// that does not run makes the observation `Unavailable`; a marker that
    /// does not parse makes it `Protocol`. Neither is ever guessed.
    fn measure(&self, target: &MatterTarget) -> Result<ResourceState, AdapterError> {
        match target.kind {
            MatterKind::Switch => {
                let report = self.read_attribute(target, "onoff", "on-off")?;
                let on = parse_bool_attribute(&report, "OnOff").ok_or(AdapterError::Protocol)?;
                Ok(ResourceState::Switch { on })
            }
            MatterKind::Light => {
                let onoff = self.read_attribute(target, "onoff", "on-off")?;
                let on = parse_bool_attribute(&onoff, "OnOff").ok_or(AdapterError::Protocol)?;
                let level = self.read_attribute(target, "levelcontrol", "current-level")?;
                let brightness_percent = parse_int_attribute(&level, "CurrentLevel")
                    .and_then(|raw| u8::try_from(raw).ok())
                    .and_then(percent_for_level)
                    .ok_or(AdapterError::Protocol)?;
                Ok(ResourceState::Light {
                    on,
                    brightness_percent,
                })
            }
            MatterKind::Climate => {
                let report =
                    self.read_attribute(target, "thermostat", "occupied-heating-setpoint")?;
                let centi = parse_int_attribute(&report, "OccupiedHeatingSetpoint")
                    .ok_or(AdapterError::Protocol)?;
                let target_milli_celsius =
                    i32::try_from(centi.saturating_mul(10)).map_err(|_| AdapterError::Protocol)?;
                Ok(ResourceState::Climate {
                    target_milli_celsius,
                })
            }
        }
    }
}

#[async_trait]
impl EdgeAdapter for Matter {
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
        let this = self.clone();
        let resource = resource.clone();
        super::process::off_runtime(move || this.observe_now(&resource)).await
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        let this = self.clone();
        let resource = resource.clone();
        let command = command.clone();
        super::process::off_runtime(move || this.apply_now(&resource, &command)).await
    }
}

/// The blocking halves of the trait methods, run off the async runtime.
impl Matter {
    fn observe_now(&self, resource: &ResourceDescriptor) -> Result<Observation, AdapterError> {
        let target = self.target_of(resource)?;
        let state = self.measure(&target)?;
        let report = serde_json::to_vec(&(&resource.resource_id, &state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            // Identical measurements keep one evidence identity; no freshness
            // is invented for a state that did not change.
            evidence_id: format!("matter-{:x}", Sha256::digest(report)),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    fn apply_now(
        &self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        if !resource.capabilities.contains(&command.capability()) {
            return Err(AdapterError::Unsupported);
        }
        let target = self.target_of(resource)?;
        match (target.kind, command) {
            (MatterKind::Switch, Command::SwitchSet { on }) => {
                self.cluster_call(&target, "onoff", if *on { "on" } else { "off" }, &[])?;
            }
            // move-to-level-with-on-off handles both directions: a level above
            // zero turns the device on, level zero turns it off. Transition,
            // options mask and override are all 0 - immediate, device-default.
            (MatterKind::Light, Command::LightSetBrightness { percent }) => {
                self.cluster_call(
                    &target,
                    "levelcontrol",
                    "move-to-level-with-on-off",
                    &[
                        level_for_percent(*percent).to_string(),
                        "0".to_string(),
                        "0".to_string(),
                        "0".to_string(),
                    ],
                )?;
            }
            // The thermostat cluster speaks hundredths of a degree Celsius.
            (MatterKind::Climate, Command::ClimateSetTemperature { milli_celsius }) => {
                self.cluster_call(
                    &target,
                    "thermostat",
                    "write",
                    &[
                        "occupied-heating-setpoint".to_string(),
                        (milli_celsius / 10).to_string(),
                    ],
                )?;
            }
            // Locks, color, scenes, sensors and every cross-kind mismatch: no
            // readable postcondition on this resource - honest Unsupported.
            _ => return Err(AdapterError::Unsupported),
        }
        Ok(AdapterAck { accepted: true })
    }
}
