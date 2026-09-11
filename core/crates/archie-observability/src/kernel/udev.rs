//! udev — noticing hardware that arrives after boot.
//!
//! # The gap this closes
//!
//! Capability detection runs once, at daemon start. A GPU plugged into a Thunderbolt
//! enclosure at 14:00, a capture card, a USB SDR, a second NIC — none of them exist as
//! far as the router is concerned until someone restarts `archied`. A node that has to
//! be restarted to notice its own hardware is not a node that can be left running.
//!
//! The kernel already broadcasts every one of these events over a netlink socket. This
//! module parses them.
//!
//! # `add` is not the event that matters — `bind` is
//!
//! The kernel emits `add` when a device *exists*, which happens before any driver has
//! claimed it. A GPU that has appeared but whose driver has not bound cannot run a
//! single kernel; advertising `compute.gpu` at `add` would publish a capability that
//! fails on first use, and a capability that fails on first use is worse than one that
//! was never advertised — the router will have already routed work to it.
//!
//! So [`UeventAction::implies_usable`] is true for `Bind` and `Online`, not for `Add`.
//! `Add` is worth *recording*; only `Bind` is worth *advertising*.
//!
//! # Events arrive in bursts, and re-detection is not free
//!
//! One physical GPU produces a dozen uevents in a few milliseconds — the PCI device, its
//! function, the DRM card, the render node, the hwmon under it. Re-probing on each is a
//! dozen redundant scans, and re-probing the whole capability set on each is worse.
//!
//! [`HotplugDebouncer`] collapses a burst into one re-detection per *capability class*,
//! so a DRM event never triggers a rescan of the network stack.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// What happened to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UeventAction {
    /// The device now exists. No driver has necessarily claimed it.
    Add,
    /// The device is gone.
    Remove,
    /// Some attribute changed. Says nothing about presence.
    Change,
    /// The device moved in the sysfs tree — a rename, not an arrival.
    Move,
    /// A previously offline device was brought online.
    Online,
    /// Administratively taken offline. Still present, not usable.
    Offline,
    /// A driver claimed the device. This is the moment it can do work.
    Bind,
    /// The driver let go. The device is still present but can no longer do work.
    Unbind,
}

impl UeventAction {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "add" => Self::Add,
            "remove" => Self::Remove,
            "change" => Self::Change,
            "move" => Self::Move,
            "online" => Self::Online,
            "offline" => Self::Offline,
            "bind" => Self::Bind,
            "unbind" => Self::Unbind,
            _ => return None,
        })
    }

    /// Whether this action means the device can now actually do work.
    ///
    /// Deliberately excludes `Add`: a device that exists but has no driver bound cannot
    /// run anything, and advertising a capability that fails on first use is worse than
    /// never advertising it, because by then work has been routed to it.
    pub fn implies_usable(&self) -> bool {
        matches!(self, Self::Bind | Self::Online)
    }

    /// Whether this action means a previously usable device no longer is.
    ///
    /// A capability must be withdrawn on `Unbind` as well as `Remove`. A driver crash
    /// unbinds without removing, and the device is just as unable to do work.
    pub fn implies_unusable(&self) -> bool {
        matches!(self, Self::Remove | Self::Unbind | Self::Offline)
    }

    /// Whether this event could change the capability set at all.
    ///
    /// `Change` and `Move` are noise for our purposes: an attribute edit or a sysfs
    /// rename does not add or remove the ability to do work.
    pub fn is_capability_relevant(&self) -> bool {
        self.implies_usable() || self.implies_unusable() || matches!(self, Self::Add)
    }
}

/// The class of capability a device could affect.
///
/// Re-detection is scoped to one of these so a DRM event never triggers a rescan of the
/// network stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    /// GPUs and accelerators — `compute.gpu`, `compute.cuda`.
    Compute,
    /// Block devices — `storage.*`.
    Storage,
    /// Network interfaces — `network.*`.
    Network,
    /// Cameras, microphones, capture cards — `sensor.*`, `media.*`.
    Sensor,
    /// Serial buses, CAN, GPIO — `bus.*`, `robot.*`.
    Bus,
    /// SDRs and radios — `radio.*`.
    Radio,
}

impl CapabilityClass {
    /// The capability prefix this class re-probes.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Compute => "compute.",
            Self::Storage => "storage.",
            Self::Network => "network.",
            Self::Sensor => "sensor.",
            Self::Bus => "bus.",
            Self::Radio => "radio.",
        }
    }
}

/// One kernel uevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uevent {
    pub action: UeventAction,
    /// Path under `/sys`, e.g. `/devices/pci0000:00/0000:00:01.0/drm/card0`.
    pub devpath: String,
    /// The subsystem, e.g. `drm`, `net`, `block`, `pci`.
    pub subsystem: Option<String>,
    /// Further discrimination within a subsystem, e.g. `disk` vs `partition`.
    pub devtype: Option<String>,
    pub properties: BTreeMap<String, String>,
}

impl Uevent {
    /// Which capability class this event could affect, if any.
    ///
    /// Returns `None` for the overwhelming majority of events. A running desktop emits
    /// uevents constantly — power supply readings, input autorepeat, backlight steps —
    /// and none of them change what work this node can do. Answering `None` loudly is
    /// the point: it is what keeps a hotplug listener from becoming a rescan loop.
    pub fn capability_class(&self) -> Option<CapabilityClass> {
        let subsystem = self.subsystem.as_deref()?;
        Some(match subsystem {
            // A render node or card is a GPU regardless of vendor.
            "drm" | "accel" => CapabilityClass::Compute,
            "net" => CapabilityClass::Network,
            "block" | "nvme" => CapabilityClass::Storage,
            "video4linux" | "media" | "sound" => CapabilityClass::Sensor,
            "tty" | "can" | "gpio" | "i2c-dev" | "spidev" => CapabilityClass::Bus,
            // A `pci` event on its own is ambiguous: it could be anything on the bus.
            // Only the device class resolves it, and when it does not, the DRM event
            // that follows will.
            "pci" | "usb" => return self.class_from_pci_or_usb(),
            _ => return None,
        })
    }

    /// Resolve an otherwise ambiguous bus event from the device class it declares.
    ///
    /// `PCI_CLASS` is a hex string; the high byte is the base class. `0x03` is a display
    /// controller and `0x12` a processing accelerator — both are compute. Anything we
    /// cannot resolve returns `None` rather than a guess: the subsystem-specific event
    /// (`drm`, `net`) follows within milliseconds and carries the answer without one.
    fn class_from_pci_or_usb(&self) -> Option<CapabilityClass> {
        let class = self.properties.get("PCI_CLASS")?;
        let value = u32::from_str_radix(class.trim_start_matches("0x"), 16).ok()?;
        // PCI_CLASS is base<<16 | sub<<8 | prog-if.
        Some(match value >> 16 {
            0x03 | 0x12 => CapabilityClass::Compute,
            0x01 => CapabilityClass::Storage,
            0x02 => CapabilityClass::Network,
            0x04 => CapabilityClass::Sensor,
            _ => return None,
        })
    }

    /// Whether this event should trigger a re-probe.
    pub fn warrants_redetection(&self) -> bool {
        self.action.is_capability_relevant() && self.capability_class().is_some()
    }

    /// A stable identity for the device, for correlating add with the later bind.
    pub fn device_key(&self) -> &str {
        &self.devpath
    }

    /// Parse the kernel's netlink uevent payload.
    ///
    /// The wire format is a header line — `<action>@<devpath>` — followed by
    /// NUL-separated `KEY=VALUE` pairs. `ACTION=` and `DEVPATH=` appear again among the
    /// properties; the header is authoritative and the duplicates are kept in
    /// `properties` unchanged, so nothing is silently dropped.
    pub fn parse(payload: &[u8]) -> Result<Self, UeventError> {
        let mut fields = payload.split(|byte| *byte == 0).filter(|f| !f.is_empty());

        let header = fields.next().ok_or(UeventError::Empty)?;
        let header = std::str::from_utf8(header).map_err(|_| UeventError::NotUtf8)?;
        let (action, devpath) = header
            .split_once('@')
            .ok_or_else(|| UeventError::Malformed(header.to_string()))?;
        let action = UeventAction::parse(action)
            .ok_or_else(|| UeventError::UnknownAction(action.to_string()))?;

        let mut properties = BTreeMap::new();
        for field in fields {
            // A property that is not UTF-8 is skipped rather than failing the whole
            // event. Firmware puts arbitrary bytes in device strings, and losing one
            // property is better than losing the arrival of a GPU.
            let Ok(text) = std::str::from_utf8(field) else {
                continue;
            };
            if let Some((key, value)) = text.split_once('=') {
                properties.insert(key.to_string(), value.to_string());
            }
        }

        Ok(Self {
            action,
            devpath: devpath.to_string(),
            subsystem: properties.get("SUBSYSTEM").cloned(),
            devtype: properties.get("DEVTYPE").cloned(),
            properties,
        })
    }

    /// Parse a message from the udev monitor group, which wraps the same payload in a
    /// `libudev` header.
    ///
    /// Messages on that group carry an eight-byte `"libudev\0"` prefix and a magic word,
    /// then offsets to the properties. A message whose magic does not match is not
    /// ours — very likely a kernel-group message that arrived on the wrong socket — and
    /// is refused rather than reinterpreted.
    pub fn parse_monitor(payload: &[u8]) -> Result<Self, UeventError> {
        const PREFIX: &[u8] = b"libudev\0";
        const MAGIC: u32 = 0xfeed_cafe;

        if payload.len() < 16 || !payload.starts_with(PREFIX) {
            // Not a monitor message; it is the plain kernel format.
            return Self::parse(payload);
        }
        let magic = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
        if magic != MAGIC {
            return Err(UeventError::BadMagic(magic));
        }
        let properties_off =
            u32::from_ne_bytes([payload[16], payload[17], payload[18], payload[19]]) as usize;
        let body = payload
            .get(properties_off..)
            .ok_or(UeventError::Truncated)?;

        // The monitor format has no `action@devpath` header line — the action is a
        // property. Reconstruct the event from properties alone.
        let mut properties = BTreeMap::new();
        for field in body.split(|byte| *byte == 0).filter(|f| !f.is_empty()) {
            let Ok(text) = std::str::from_utf8(field) else {
                continue;
            };
            if let Some((key, value)) = text.split_once('=') {
                properties.insert(key.to_string(), value.to_string());
            }
        }

        let action_text = properties
            .get("ACTION")
            .ok_or_else(|| UeventError::Malformed("monitor message without ACTION".into()))?;
        let action = UeventAction::parse(action_text)
            .ok_or_else(|| UeventError::UnknownAction(action_text.clone()))?;
        let devpath = properties
            .get("DEVPATH")
            .cloned()
            .ok_or_else(|| UeventError::Malformed("monitor message without DEVPATH".into()))?;

        Ok(Self {
            action,
            devpath,
            subsystem: properties.get("SUBSYSTEM").cloned(),
            devtype: properties.get("DEVTYPE").cloned(),
            properties,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UeventError {
    Empty,
    NotUtf8,
    Truncated,
    BadMagic(u32),
    UnknownAction(String),
    Malformed(String),
}

impl std::fmt::Display for UeventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("empty uevent payload"),
            Self::NotUtf8 => f.write_str("uevent header was not utf-8"),
            Self::Truncated => f.write_str("uevent message truncated before its properties"),
            Self::BadMagic(magic) => write!(f, "not a udev monitor message (magic {magic:#x})"),
            Self::UnknownAction(action) => write!(f, "unknown uevent action {action:?}"),
            Self::Malformed(detail) => write!(f, "malformed uevent: {detail}"),
        }
    }
}

/// Collapses a burst of uevents into at most one re-detection per capability class.
///
/// One physical GPU produces a dozen events in milliseconds — the PCI device, its
/// function, the DRM card, the render node, the hwmon beneath it. Each would otherwise
/// trigger a probe, and each probe costs more than the event that caused it.
#[derive(Debug)]
pub struct HotplugDebouncer {
    window: Duration,
    pending: BTreeMap<CapabilityClass, Instant>,
    last_fired: BTreeMap<CapabilityClass, Instant>,
}

impl HotplugDebouncer {
    /// A window long enough to cover a device's whole arrival burst, short enough that a
    /// human plugging something in does not notice the wait.
    pub const DEFAULT_WINDOW: Duration = Duration::from_millis(750);

    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: BTreeMap::new(),
            last_fired: BTreeMap::new(),
        }
    }

    /// Record an event. Returns the class if this is the first of a burst.
    ///
    /// The first event of a burst starts the window; everything inside it is folded in.
    /// The caller is expected to call [`Self::due`] after the window to actually probe.
    pub fn observe(&mut self, event: &Uevent, now: Instant) -> Option<CapabilityClass> {
        if !event.warrants_redetection() {
            return None;
        }
        let class = event.capability_class()?;
        self.pending.entry(class).or_insert(now);
        Some(class)
    }

    /// The classes whose window has elapsed, clearing them.
    ///
    /// A class fires once per burst. If events keep arriving the window is *not* extended
    /// indefinitely — a device flapping (a bad cable, a failing riser) would otherwise
    /// starve re-detection forever, which is exactly when an accurate capability set
    /// matters most.
    pub fn due(&mut self, now: Instant) -> Vec<CapabilityClass> {
        let window = self.window;
        let ready: Vec<CapabilityClass> = self
            .pending
            .iter()
            .filter(|(_, started)| now.duration_since(**started) >= window)
            .map(|(class, _)| *class)
            .collect();
        for class in &ready {
            self.pending.remove(class);
            self.last_fired.insert(*class, now);
        }
        ready
    }

    /// Whether anything is waiting.
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }

    /// How long until the next class is due, for a caller that wants to sleep exactly
    /// that long rather than poll.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.pending
            .values()
            .map(|started| self.window.saturating_sub(now.duration_since(*started)))
            .min()
    }
}

impl Default for HotplugDebouncer {
    fn default() -> Self {
        Self::new(Self::DEFAULT_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel_payload(header: &str, properties: &[&str]) -> Vec<u8> {
        let mut bytes = header.as_bytes().to_vec();
        for property in properties {
            bytes.push(0);
            bytes.extend_from_slice(property.as_bytes());
        }
        bytes.push(0);
        bytes
    }

    // ──────────────────────────────────────────────────────────────── parsing

    #[test]
    fn a_gpu_bind_parses_into_a_compute_event() {
        let payload = kernel_payload(
            "bind@/devices/pci0000:00/0000:01:00.0/drm/card0",
            &[
                "ACTION=bind",
                "DEVPATH=/devices/pci0000:00/0000:01:00.0/drm/card0",
                "SUBSYSTEM=drm",
                "DEVTYPE=drm_minor",
                "SEQNUM=4821",
            ],
        );
        let event = Uevent::parse(&payload).unwrap();
        assert_eq!(event.action, UeventAction::Bind);
        assert_eq!(event.subsystem.as_deref(), Some("drm"));
        assert_eq!(event.capability_class(), Some(CapabilityClass::Compute));
        assert!(event.warrants_redetection());
        assert_eq!(event.properties.get("SEQNUM").unwrap(), "4821");
    }

    #[test]
    fn add_does_not_mean_usable_but_bind_does() {
        // A device that exists without a bound driver cannot run anything. Advertising
        // a capability at `add` publishes one that fails on first use — by which time
        // work has already been routed to it.
        assert!(!UeventAction::Add.implies_usable());
        assert!(UeventAction::Bind.implies_usable());
        assert!(UeventAction::Online.implies_usable());
        // But `add` is still worth re-probing on, because probing is what discovers
        // whether a driver bound already.
        assert!(UeventAction::Add.is_capability_relevant());
    }

    #[test]
    fn a_driver_that_lets_go_withdraws_the_capability_without_the_device_leaving() {
        // A driver crash unbinds without removing. The device is just as unable to work.
        assert!(UeventAction::Unbind.implies_unusable());
        assert!(UeventAction::Remove.implies_unusable());
        assert!(UeventAction::Offline.implies_unusable());
        assert!(!UeventAction::Unbind.implies_usable());
    }

    #[test]
    fn attribute_changes_and_renames_are_not_capability_events() {
        // A running desktop emits these constantly. Treating them as relevant turns a
        // hotplug listener into a rescan loop.
        assert!(!UeventAction::Change.is_capability_relevant());
        assert!(!UeventAction::Move.is_capability_relevant());
    }

    #[test]
    fn the_overwhelming_majority_of_events_affect_nothing() {
        for subsystem in ["power_supply", "input", "backlight", "leds", "rtc", "mem"] {
            let payload = kernel_payload(
                "change@/devices/x",
                &["ACTION=change", &format!("SUBSYSTEM={subsystem}")],
            );
            let event = Uevent::parse(&payload).unwrap();
            assert_eq!(
                event.capability_class(),
                None,
                "{subsystem} should affect no capability class"
            );
            assert!(!event.warrants_redetection());
        }
    }

    #[test]
    fn each_subsystem_maps_to_the_class_it_belongs_to() {
        for (subsystem, expected) in [
            ("drm", CapabilityClass::Compute),
            ("accel", CapabilityClass::Compute),
            ("net", CapabilityClass::Network),
            ("block", CapabilityClass::Storage),
            ("nvme", CapabilityClass::Storage),
            ("video4linux", CapabilityClass::Sensor),
            ("sound", CapabilityClass::Sensor),
            ("tty", CapabilityClass::Bus),
            ("can", CapabilityClass::Bus),
        ] {
            let payload = kernel_payload(
                "add@/devices/x",
                &["ACTION=add", &format!("SUBSYSTEM={subsystem}")],
            );
            let event = Uevent::parse(&payload).unwrap();
            assert_eq!(event.capability_class(), Some(expected), "{subsystem}");
        }
    }

    #[test]
    fn a_bare_pci_event_is_resolved_by_device_class_not_guessed() {
        // 0x030000 is a display controller.
        let gpu = Uevent::parse(&kernel_payload(
            "add@/devices/pci0000:00/0000:01:00.0",
            &["ACTION=add", "SUBSYSTEM=pci", "PCI_CLASS=30000"],
        ))
        .unwrap();
        assert_eq!(gpu.capability_class(), Some(CapabilityClass::Compute));

        // 0x020000 is an ethernet controller.
        let nic = Uevent::parse(&kernel_payload(
            "add@/devices/pci0000:00/0000:02:00.0",
            &["ACTION=add", "SUBSYSTEM=pci", "PCI_CLASS=20000"],
        ))
        .unwrap();
        assert_eq!(nic.capability_class(), Some(CapabilityClass::Network));
    }

    #[test]
    fn an_unresolvable_pci_event_returns_none_rather_than_a_guess() {
        // The subsystem-specific event follows within milliseconds and carries the
        // answer; guessing here would only be wrong sooner.
        let bridge = Uevent::parse(&kernel_payload(
            "add@/devices/pci0000:00/0000:00:1c.0",
            &["ACTION=add", "SUBSYSTEM=pci", "PCI_CLASS=60400"],
        ))
        .unwrap();
        assert_eq!(bridge.capability_class(), None);

        let no_class = Uevent::parse(&kernel_payload(
            "add@/devices/x",
            &["ACTION=add", "SUBSYSTEM=pci"],
        ))
        .unwrap();
        assert_eq!(no_class.capability_class(), None);
    }

    #[test]
    fn an_accelerator_class_device_counts_as_compute() {
        // 0x12 is "processing accelerator" — a TPU or NPU, not a display device.
        let npu = Uevent::parse(&kernel_payload(
            "bind@/devices/pci0000:00/0000:03:00.0",
            &["ACTION=bind", "SUBSYSTEM=pci", "PCI_CLASS=120000"],
        ))
        .unwrap();
        assert_eq!(npu.capability_class(), Some(CapabilityClass::Compute));
    }

    #[test]
    fn a_property_that_is_not_utf8_costs_one_property_not_the_event() {
        // Firmware puts arbitrary bytes in device strings. Losing the arrival of a GPU
        // over one unreadable model name would be a bad trade.
        let mut payload = b"bind@/devices/x".to_vec();
        payload.push(0);
        payload.extend_from_slice(b"SUBSYSTEM=drm");
        payload.push(0);
        payload.extend_from_slice(b"MODEL=\xff\xfe bad");
        payload.push(0);
        payload.extend_from_slice(b"SEQNUM=9");
        payload.push(0);

        let event = Uevent::parse(&payload).unwrap();
        assert_eq!(event.action, UeventAction::Bind);
        assert_eq!(event.capability_class(), Some(CapabilityClass::Compute));
        assert!(!event.properties.contains_key("MODEL"));
        assert_eq!(event.properties.get("SEQNUM").unwrap(), "9");
    }

    #[test]
    fn malformed_payloads_are_errors_not_silent_defaults() {
        assert_eq!(Uevent::parse(b""), Err(UeventError::Empty));
        assert!(matches!(
            Uevent::parse(b"no-at-sign-here\0"),
            Err(UeventError::Malformed(_))
        ));
        assert_eq!(
            Uevent::parse(b"levitate@/devices/x\0"),
            Err(UeventError::UnknownAction("levitate".into()))
        );
    }

    #[test]
    fn an_unknown_action_is_refused_rather_than_treated_as_add() {
        // A future kernel action defaulting to `add` would advertise capabilities on an
        // event whose meaning we do not know.
        assert!(UeventAction::parse("recalibrate").is_none());
    }

    #[test]
    fn the_header_is_authoritative_and_duplicate_properties_are_preserved() {
        let event = Uevent::parse(&kernel_payload(
            "bind@/devices/real",
            &["ACTION=bind", "DEVPATH=/devices/real", "SUBSYSTEM=drm"],
        ))
        .unwrap();
        assert_eq!(event.devpath, "/devices/real");
        assert_eq!(event.device_key(), "/devices/real");
        // Nothing is silently dropped.
        assert_eq!(event.properties.get("DEVPATH").unwrap(), "/devices/real");
    }

    // ───────────────────────────────────────────────────────── monitor format

    fn monitor_payload(properties: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();
        for property in properties {
            body.extend_from_slice(property.as_bytes());
            body.push(0);
        }
        let properties_off: u32 = 40;
        let mut bytes = b"libudev\0".to_vec();
        bytes.extend_from_slice(&0xfeed_cafeu32.to_be_bytes());
        bytes.extend_from_slice(&40u32.to_ne_bytes()); // header size
        bytes.extend_from_slice(&properties_off.to_ne_bytes());
        bytes.extend_from_slice(&(body.len() as u32).to_ne_bytes());
        bytes.resize(properties_off as usize, 0);
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn a_monitor_message_parses_from_its_properties() {
        // The monitor format has no `action@devpath` header; the action is a property.
        let payload = monitor_payload(&[
            "ACTION=bind",
            "DEVPATH=/devices/pci0000:00/0000:01:00.0/drm/card0",
            "SUBSYSTEM=drm",
            "ID_PATH=pci-0000:01:00.0",
        ]);
        let event = Uevent::parse_monitor(&payload).unwrap();
        assert_eq!(event.action, UeventAction::Bind);
        assert!(event.devpath.ends_with("drm/card0"));
        assert_eq!(event.capability_class(), Some(CapabilityClass::Compute));
    }

    #[test]
    fn a_message_with_the_wrong_magic_is_refused_rather_than_reinterpreted() {
        let mut payload = monitor_payload(&["ACTION=add", "DEVPATH=/x", "SUBSYSTEM=drm"]);
        payload[8..12].copy_from_slice(&0xdead_beefu32.to_be_bytes());
        assert_eq!(
            Uevent::parse_monitor(&payload),
            Err(UeventError::BadMagic(0xdead_beef))
        );
    }

    #[test]
    fn the_monitor_parser_still_accepts_the_plain_kernel_format() {
        // Both groups can be read from one socket; a kernel-format message must not be
        // rejected just because it lacks the libudev prefix.
        let payload = kernel_payload("add@/devices/x", &["ACTION=add", "SUBSYSTEM=net"]);
        let event = Uevent::parse_monitor(&payload).unwrap();
        assert_eq!(event.capability_class(), Some(CapabilityClass::Network));
    }

    #[test]
    fn a_truncated_monitor_message_is_an_error() {
        let mut payload = monitor_payload(&["ACTION=add", "DEVPATH=/x"]);
        payload[16..20].copy_from_slice(&9999u32.to_ne_bytes());
        assert_eq!(Uevent::parse_monitor(&payload), Err(UeventError::Truncated));
    }

    #[test]
    fn a_monitor_message_missing_its_action_is_refused() {
        let payload = monitor_payload(&["DEVPATH=/x", "SUBSYSTEM=drm"]);
        assert!(matches!(
            Uevent::parse_monitor(&payload),
            Err(UeventError::Malformed(_))
        ));
    }

    // ────────────────────────────────────────────────────────────── debouncing

    fn event(subsystem: &str, action: &str) -> Uevent {
        Uevent::parse(&kernel_payload(
            &format!("{action}@/devices/{subsystem}"),
            &[
                &format!("ACTION={action}"),
                &format!("SUBSYSTEM={subsystem}"),
            ],
        ))
        .unwrap()
    }

    #[test]
    fn one_gpu_arrival_burst_produces_one_redetection() {
        // A single physical GPU emits the PCI device, its function, the DRM card, the
        // render node and the hwmon beneath it, all within milliseconds.
        let mut debouncer = HotplugDebouncer::new(Duration::from_millis(750));
        let start = Instant::now();

        for offset in [0, 3, 7, 11, 15] {
            let moment = start + Duration::from_millis(offset);
            debouncer.observe(&event("drm", "add"), moment);
        }
        assert!(!debouncer.is_idle());
        assert!(
            debouncer.due(start + Duration::from_millis(100)).is_empty(),
            "must not fire before the window closes"
        );

        let fired = debouncer.due(start + Duration::from_millis(800));
        assert_eq!(fired, vec![CapabilityClass::Compute]);
        assert!(debouncer.is_idle());
    }

    #[test]
    fn a_drm_event_never_triggers_a_network_rescan() {
        let mut debouncer = HotplugDebouncer::default();
        let start = Instant::now();
        debouncer.observe(&event("drm", "bind"), start);

        let fired = debouncer.due(start + Duration::from_secs(1));
        assert_eq!(fired, vec![CapabilityClass::Compute]);
        assert!(!fired.contains(&CapabilityClass::Network));
    }

    #[test]
    fn independent_classes_fire_independently() {
        let mut debouncer = HotplugDebouncer::new(Duration::from_millis(500));
        let start = Instant::now();
        debouncer.observe(&event("drm", "bind"), start);
        debouncer.observe(&event("net", "add"), start + Duration::from_millis(400));

        // Compute's window has closed; network's has not.
        let fired = debouncer.due(start + Duration::from_millis(600));
        assert_eq!(fired, vec![CapabilityClass::Compute]);

        let fired = debouncer.due(start + Duration::from_millis(950));
        assert_eq!(fired, vec![CapabilityClass::Network]);
    }

    #[test]
    fn a_flapping_device_does_not_starve_redetection() {
        // A bad cable or failing riser can emit events forever. Extending the window on
        // every event would postpone re-detection indefinitely — exactly when an
        // accurate capability set matters most.
        let mut debouncer = HotplugDebouncer::new(Duration::from_millis(500));
        let start = Instant::now();
        for tick in 0..100 {
            debouncer.observe(
                &event("net", "add"),
                start + Duration::from_millis(tick * 20),
            );
        }
        let fired = debouncer.due(start + Duration::from_millis(600));
        assert_eq!(
            fired,
            vec![CapabilityClass::Network],
            "the window is anchored to the first event, not the last"
        );
    }

    #[test]
    fn irrelevant_events_never_enter_the_debouncer() {
        let mut debouncer = HotplugDebouncer::default();
        let start = Instant::now();
        assert_eq!(debouncer.observe(&event("drm", "change"), start), None);
        assert_eq!(
            debouncer.observe(&event("power_supply", "add"), start),
            None
        );
        assert!(debouncer.is_idle());
        assert!(debouncer.due(start + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn a_caller_can_sleep_exactly_as_long_as_needed() {
        let mut debouncer = HotplugDebouncer::new(Duration::from_millis(500));
        let start = Instant::now();
        assert_eq!(debouncer.next_deadline(start), None);

        debouncer.observe(&event("drm", "bind"), start);
        let remaining = debouncer
            .next_deadline(start + Duration::from_millis(200))
            .unwrap();
        assert_eq!(remaining, Duration::from_millis(300));

        // Past the deadline it saturates at zero rather than underflowing.
        assert_eq!(
            debouncer.next_deadline(start + Duration::from_secs(5)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_capability_class_names_the_prefix_it_reprobes() {
        assert_eq!(CapabilityClass::Compute.prefix(), "compute.");
        assert_eq!(CapabilityClass::Network.prefix(), "network.");
        assert_eq!(CapabilityClass::Radio.prefix(), "radio.");
    }
}
