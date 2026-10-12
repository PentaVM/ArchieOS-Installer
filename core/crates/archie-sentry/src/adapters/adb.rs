//! Android adapter: controls a phone through the system `adb` binary, always
//! via [`CommandRunner`] with an absolute configured path. There is never a
//! shell on the desktop side: every argument is a separate argv element, so a
//! value can never become syntax. The device-side `shell` command receives its
//! arguments as monkey/input/cmd argv, and every value that reaches it
//! (package id, volume index) is contract- or adapter-validated first.
//!
//! Supported commands, each with a measured postcondition the engine verifies:
//! - `AppLaunch`: `adb -s <serial> shell monkey -p <app_id> -c
//!   android.intent.category.LAUNCHER 1`. Verified through the foreground
//!   package parsed from `dumpsys window`.
//! - `SessionLock`: `KEYCODE_POWER` is a *toggle*, so `apply` first measures
//!   the lock state and sends the keyevent only when it differs from the
//!   request; when the state cannot be measured it refuses (`Unavailable`)
//!   rather than toggling blind. **Locked** means the keyguard is showing
//!   (`mDreamingLockscreen`/`mShowingLockscreen` in `dumpsys window`) or the
//!   device is asleep (`dumpsys power` wakefulness not `Awake`): both require
//!   passing the keyguard to use the device. **Unlocked** means awake with no
//!   keyguard. Note that waking a phone with a secured lockscreen still leaves
//!   the keyguard showing; `SessionLock { locked: false }` then honestly
//!   verifies only on swipe/no-lockscreen devices, and the engine records
//!   `Diverged` otherwise.
//! - `VolumeSet`: percent 0-100 is mapped onto the STREAM_MUSIC index range
//!   parsed from `dumpsys audio` (`Max volume index:` / `streamVolumeMax:`),
//!   then applied with `adb -s <serial> shell cmd media_session volume
//!   --stream 3 --set <index>`. If the range cannot be parsed the command is
//!   `Unavailable`; an index is never guessed.
//! - `MediaPause`/`MediaPlay` are `Unsupported`. Their contract postcondition
//!   requires a `ResourceState::Media` observation, while every command this
//!   adapter serves verifies through `ResourceState::Host`; and
//!   `dumpsys media_session` playback state is per-session and stale-prone
//!   (any app may hold a long-paused session), so there is no single honest
//!   playback bit to measure. The adapter does not send unverifiable keyevents.
//!
//! Connection management: a network-attached target (wireless debugging) is
//! `adb connect <ip:port>`-ed before anything else, and every action starts
//! with an `adb devices` listing - if the configured serial is not present and
//! `device`, the command is `Unavailable` without acting.
//!
//! Honesty notes, deliberately part of the module contract:
//! - Tested: every parser and the exact argv construction, against recorded
//!   fixtures (see `tests/adapters_nodes.rs`). Not tested here: everything
//!   that needs a real phone. `dumpsys` formats vary across Android versions;
//!   the parsers accept the documented markers and report "unknown"/
//!   `Unavailable` rather than guessing when a version differs.
//! - `dumpsys` output can be megabytes. The runner caps output (256 KiB) and
//!   the parsers scan line-by-line for their markers and ignore the rest; if a
//!   marker sits beyond the cap the parse fails safely instead of misreading.
//! - Wireless debugging must be enabled on the phone by the owner (Developer
//!   options), and the phone must have authorized this host's adb key. This
//!   adapter never enables, pairs, or authorizes anything.

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
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::Path,
    sync::Arc,
    time::Duration,
};

/// Short adb calls: connect, devices, monkey, keyevent, volume set.
const ADB_SHORT: Duration = Duration::from_secs(4);
/// dumpsys can take a moment on a busy device; still bounded.
const ADB_DUMPSYS: Duration = Duration::from_secs(8);
/// STREAM_MUSIC, the stream volume commands act on and observe reads back.
const MUSIC_STREAM: &str = "3";

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidTarget {
    /// The `adb devices` serial: a USB serial, or `<ip>:<port>` when the phone
    /// is reached through wireless debugging.
    pub serial: String,
    /// Run `adb connect <serial>` before acting (wireless debugging).
    pub network: bool,
}

struct Measured {
    awake: Option<bool>,
    locked: Option<bool>,
    foreground: Option<String>,
    volume: Option<(u32, u32)>,
}

pub struct Adb {
    edge: String,
    adapter: String,
    /// Absolute path of the adb binary; existence is enforced by the runner.
    adb: String,
    targets: BTreeMap<String, AndroidTarget>,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for Adb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adb")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

/// `adb devices` output: is `serial` listed with state `device` (authorized
/// and online)? `offline`/`unauthorized`/absent are all "not usable".
pub fn device_online(devices_output: &str, serial: &str) -> bool {
    let mut listed = false;
    for line in devices_output.lines() {
        if line.starts_with("List of devices") {
            listed = true;
            continue;
        }
        if !listed {
            continue;
        }
        let mut fields = line.split_whitespace();
        if fields.next() == Some(serial) {
            return fields.next() == Some("device");
        }
    }
    false
}

/// `dumpsys power` wakefulness. `Awake` is awake; `Asleep`, `Dozing` and
/// `Dreaming` are not. Anything unrecognized is unknown, never guessed.
pub fn parse_wakefulness(dumpsys_power: &str) -> Option<bool> {
    for key in ["mWakefulness=", "wakefulness="] {
        let Some(pos) = dumpsys_power.find(key) else {
            continue;
        };
        let value: String = dumpsys_power[pos + key.len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        return match value.as_str() {
            "Awake" => Some(true),
            "Asleep" | "Dozing" | "Dreaming" => Some(false),
            _ => None,
        };
    }
    None
}

fn bool_field(text: &str, key: &str) -> Option<bool> {
    let pos = text.find(key)?;
    let rest = &text[pos + key.len()..];
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Is the keyguard showing, per `dumpsys window`? Both the modern
/// (`mDreamingLockscreen`) and older (`mShowingLockscreen`) markers count.
pub fn parse_keyguard(dumpsys_window: &str) -> Option<bool> {
    for key in ["mDreamingLockscreen=", "mShowingLockscreen="] {
        if let Some(value) = bool_field(dumpsys_window, key) {
            return Some(value);
        }
    }
    None
}

/// The module's locked definition: keyguard showing, or the device asleep.
/// Awake with the keyguard dismissed is the only "unlocked". Unknown inputs
/// yield unknown, which verifies nothing.
pub fn locked_state(awake: Option<bool>, keyguard: Option<bool>) -> Option<bool> {
    match (awake, keyguard) {
        (_, Some(true)) | (Some(false), _) => Some(true),
        (Some(true), Some(false)) => Some(false),
        _ => None,
    }
}

/// The foreground package from `dumpsys window`: `mCurrentFocus` first, then
/// `mFocusedApp`. A focus of `null` (sleeping, or between windows) is "none".
/// The package id must satisfy the contract's identifier rules, or it is not
/// evidence.
pub fn parse_foreground(dumpsys_window: &str) -> Option<String> {
    for key in ["mCurrentFocus=", "mFocusedApp="] {
        let Some(pos) = dumpsys_window.find(key) else {
            continue;
        };
        let line = dumpsys_window[pos + key.len()..].lines().next()?;
        if line.contains("null") {
            return None;
        }
        for token in line.split_whitespace() {
            let Some((package, _)) = token.split_once('/') else {
                continue;
            };
            let package =
                package.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
            if validate_id(package).is_ok() {
                return Some(package.to_string());
            }
        }
        return None;
    }
    None
}

fn number_after(line: &str, key: &str) -> Option<u32> {
    let pos = line.find(key)?;
    let digits: String = line[pos + key.len()..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// STREAM_MUSIC `(current, max)` index from `dumpsys audio`. Accepts the
/// modern `Max volume index:` / `Current: <dev> (<name>): <idx>, ...` wording
/// and the older `streamVolumeMax:` / `streamVolume:` keys. Both numbers are
/// required; a partial read is unknown, never guessed.
pub fn parse_volume(dumpsys_audio: &str) -> Option<(u32, u32)> {
    let start = dumpsys_audio.find("STREAM_MUSIC")?;
    let mut current = None;
    let mut max = None;
    for line in dumpsys_audio[start..].lines().take(60) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- STREAM_") && !trimmed.contains("STREAM_MUSIC") {
            break; // next stream's section: stop, the music block is over
        }
        if max.is_none() {
            max = number_after(line, "Max volume index:")
                .or_else(|| number_after(line, "streamVolumeMax:"));
        }
        if current.is_none() {
            current = number_after(line, "streamVolume:").or_else(|| {
                if trimmed.starts_with("Current:") {
                    number_after(line, "):")
                } else {
                    None
                }
            });
        }
        if current.is_some() && max.is_some() {
            break;
        }
    }
    Some((current?, max?))
}

/// Percent 0-100 → stream index, rounded to nearest, bounded by `max`.
pub fn index_for_percent(percent: u8, max: u32) -> u32 {
    (u32::from(percent) * max + 50) / 100
}
/// Stream index → percent, rounded to nearest. `max == 0` is a fixed-silent
/// stream: only index 0 exists, and it reads back as 0%.
pub fn percent_for_index(index: u32, max: u32) -> Option<u8> {
    if max == 0 {
        return (index == 0).then_some(0);
    }
    u8::try_from((index * 100 + max / 2) / max).ok()
}

impl Adb {
    /// `adb` must be an absolute path; [`super::process::ProcessRunner`]
    /// additionally requires the file to exist before any process is spawned.
    /// Serials are `[A-Za-z0-9.:_-]` and at most 64 characters; a `network`
    /// target's serial must be a literal `<ip>:<port>`.
    pub fn new(
        edge: &str,
        adapter: &str,
        adb: &str,
        targets: BTreeMap<String, AndroidTarget>,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        if !Path::new(adb).is_absolute() {
            return Err(AdapterError::Protocol);
        }
        if targets.is_empty() || targets.len() > 64 {
            return Err(AdapterError::Protocol);
        }
        for (resource, target) in &targets {
            validate_id(resource).map_err(|_| AdapterError::Protocol)?;
            let serial = &target.serial;
            if serial.is_empty()
                || serial.len() > 64
                || !serial.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-' | b'_')
                })
            {
                return Err(AdapterError::Protocol);
            }
            if target.network {
                let Some((host, port)) = serial.rsplit_once(':') else {
                    return Err(AdapterError::Protocol);
                };
                host.parse::<IpAddr>().map_err(|_| AdapterError::Protocol)?;
                if port.parse::<u16>().ok().filter(|p| *p != 0).is_none() {
                    return Err(AdapterError::Protocol);
                }
            }
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            adb: adb.into(),
            targets,
            runner,
        })
    }
    fn target_of(&self, resource: &ResourceDescriptor) -> Result<AndroidTarget, AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.targets
            .get(&resource.resource_id)
            .cloned()
            .ok_or(AdapterError::Unsupported)
    }
    /// One adb invocation. A non-zero exit status means adb itself failed:
    /// `Unavailable`, never a parse of error text as if it were data.
    fn run(&self, args: &[&str], timeout: Duration) -> Result<String, AdapterError> {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(self.adb.clone());
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        let output = self.runner.run(&argv, timeout)?;
        if output.status != 0 {
            return Err(AdapterError::Unavailable);
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn shell(&self, serial: &str, command: &[&str], timeout: Duration) -> Result<String, AdapterError> {
        let mut args = vec!["-s", serial, "shell"];
        args.extend_from_slice(command);
        self.run(&args, timeout)
    }
    /// Wireless targets are (re)connected first; every action then requires
    /// the configured serial to be present and online in `adb devices`.
    fn ensure_online(&self, target: &AndroidTarget) -> Result<(), AdapterError> {
        if target.network {
            let reply = self.run(&["connect", &target.serial], ADB_SHORT)?;
            // "connected to ..." and "already connected to ..." both pass;
            // "failed to connect to ..." does not contain "connected to".
            if !reply.contains("connected to") {
                return Err(AdapterError::Unavailable);
            }
        }
        let list = self.run(&["devices"], ADB_SHORT)?;
        if !device_online(&list, &target.serial) {
            return Err(AdapterError::Unavailable);
        }
        Ok(())
    }
    /// Full measurement for `observe`. A dumpsys invocation that fails to run
    /// makes the whole observation `Unavailable`; a marker that fails to parse
    /// leaves that field unknown (`None`), which verifies nothing.
    fn measure(&self, serial: &str) -> Result<Measured, AdapterError> {
        let power = self.shell(serial, &["dumpsys", "power"], ADB_DUMPSYS)?;
        let window = self.shell(serial, &["dumpsys", "window"], ADB_DUMPSYS)?;
        let audio = self.shell(serial, &["dumpsys", "audio"], ADB_DUMPSYS)?;
        let awake = parse_wakefulness(&power);
        Ok(Measured {
            awake,
            locked: locked_state(awake, parse_keyguard(&window)),
            foreground: parse_foreground(&window),
            volume: parse_volume(&audio),
        })
    }
    /// Lock state only (power + window), for the toggle decision in `apply`.
    fn measure_lock(&self, serial: &str) -> Result<Option<bool>, AdapterError> {
        let power = self.shell(serial, &["dumpsys", "power"], ADB_DUMPSYS)?;
        let window = self.shell(serial, &["dumpsys", "window"], ADB_DUMPSYS)?;
        Ok(locked_state(
            parse_wakefulness(&power),
            parse_keyguard(&window),
        ))
    }
}

#[async_trait]
impl EdgeAdapter for Adb {
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
        self.ensure_online(&target)?;
        let measured = self.measure(&target.serial)?;
        let state = ResourceState::Host {
            reachable: true, // the serial answered `adb devices` as `device`
            awake: measured.awake,
            session_locked: measured.locked,
            volume_percent: measured
                .volume
                .and_then(|(index, max)| percent_for_index(index, max)),
            foreground_app: measured.foreground,
        };
        let report = serde_json::to_vec(&(&resource.resource_id, &state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            // Identical measurements keep one evidence identity; no freshness
            // is invented for a state that did not change.
            evidence_id: format!("adb-{:x}", Sha256::digest(report)),
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
            // app_id is contract-validated ([A-Za-z0-9._-] only), so it is a
            // safe argv element for the device-side monkey.
            Command::AppLaunch { app_id } => {
                self.ensure_online(&target)?;
                let reply = self.shell(
                    &target.serial,
                    &[
                        "monkey",
                        "-p",
                        app_id,
                        "-c",
                        "android.intent.category.LAUNCHER",
                        "1",
                    ],
                    ADB_SHORT,
                )?;
                // monkey exits 0 even when the package has no launchable
                // activity; that report is a dispatch failure, not a success.
                if reply.contains("No activities found") {
                    return Err(AdapterError::Unavailable);
                }
                Ok(AdapterAck { accepted: true })
            }
            Command::SessionLock { locked } => {
                self.ensure_online(&target)?;
                // KEYCODE_POWER toggles; send it only when the measured state
                // differs, and refuse to toggle a state we cannot measure.
                match self.measure_lock(&target.serial)? {
                    Some(current) if current == *locked => Ok(AdapterAck { accepted: true }),
                    Some(_) => {
                        self.shell(
                            &target.serial,
                            &["input", "keyevent", "KEYCODE_POWER"],
                            ADB_SHORT,
                        )?;
                        Ok(AdapterAck { accepted: true })
                    }
                    None => Err(AdapterError::Unavailable),
                }
            }
            Command::VolumeSet { percent } => {
                self.ensure_online(&target)?;
                let audio = self.shell(&target.serial, &["dumpsys", "audio"], ADB_DUMPSYS)?;
                let (_, max) = parse_volume(&audio).ok_or(AdapterError::Unavailable)?;
                let index = index_for_percent(*percent, max).to_string();
                self.shell(
                    &target.serial,
                    &[
                        "cmd",
                        "media_session",
                        "volume",
                        "--stream",
                        MUSIC_STREAM,
                        "--set",
                        &index,
                    ],
                    ADB_SHORT,
                )?;
                Ok(AdapterAck { accepted: true })
            }
            // MediaPause/MediaPlay: no honest postcondition - see module docs.
            _ => Err(AdapterError::Unsupported),
        }
    }
}
