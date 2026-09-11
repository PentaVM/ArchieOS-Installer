//! Applying a hardware event to the capability set, without letting it say more than it
//! knows.
//!
//! # Why a re-probe cannot simply replace the capability set
//!
//! [`crate::runtime::Runtime::refresh_local_descriptor`] re-runs the whole hardware
//! probe and replaces the descriptor wholesale. That is correct for an operator-triggered
//! refresh, which is rare and deliberate.
//!
//! It is *not* correct as the response to a uevent. A DRM `bind` says one thing — a GPU
//! now has a driver. It says nothing about the network. But a wholesale replace lets any
//! momentary weakness in an unrelated probe rewrite the announcement: if `sysinfo`
//! returns an empty interface list for one call — a race with a NIC resetting, a
//! container namespace in flux, a permission blip — then `network.lan` disappears from
//! the manifest, and it disappears *because a GPU was plugged in*.
//!
//! Today that costs little, because refreshes are rare. Wiring hotplug makes the refresh
//! happen on every device event, so the rare failure becomes a routine one.
//!
//! # The rule
//!
//! A uevent for class C may only add or remove capabilities in class C. Everything else
//! is carried forward from the previous set, untouched. An event is allowed to teach the
//! manifest exactly as much as it observed and no more.
//!
//! [`apply_scoped`] is that rule. [`scoped_delta`] reports what it would change, so the
//! caller can log a capability appearing or vanishing rather than have it happen
//! silently.

use archie_observability::kernel::{CapabilityClass, HotplugDebouncer, Uevent};
use archie_types::capability::{Capability, CapabilitySet};
use std::time::{Duration, Instant};

/// What a scoped re-probe changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityDelta {
    /// Capabilities this node did not advertise before and does now.
    pub gained: Vec<Capability>,
    /// Capabilities this node advertised before and no longer does.
    pub lost: Vec<Capability>,
}

impl CapabilityDelta {
    pub fn is_empty(&self) -> bool {
        self.gained.is_empty() && self.lost.is_empty()
    }

    /// A one-line summary for the log, or `None` when nothing changed.
    ///
    /// Returning `None` for the empty case matters: the overwhelming majority of hotplug
    /// re-probes change nothing, and a line per event would bury the ones that did.
    pub fn summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let render = |list: &[Capability]| {
            list.iter()
                .map(|capability| capability.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some(match (self.gained.is_empty(), self.lost.is_empty()) {
            (false, true) => format!("gained {}", render(&self.gained)),
            (true, false) => format!("lost {}", render(&self.lost)),
            _ => format!(
                "gained {}, lost {}",
                render(&self.gained),
                render(&self.lost)
            ),
        })
    }
}

/// Whether a capability belongs to the class a uevent was about.
fn in_class(capability: &Capability, class: CapabilityClass) -> bool {
    capability.as_str().starts_with(class.prefix())
}

/// What a re-probe would change within one capability class.
pub fn scoped_delta(
    previous: &CapabilitySet,
    fresh: &CapabilitySet,
    class: CapabilityClass,
) -> CapabilityDelta {
    let gained = fresh
        .iter()
        .filter(|capability| in_class(capability, class) && !previous.contains(capability))
        .cloned()
        .collect();
    let lost = previous
        .iter()
        .filter(|capability| in_class(capability, class) && !fresh.contains(capability))
        .cloned()
        .collect();
    CapabilityDelta { gained, lost }
}

/// Fold a fresh probe into the previous set, limited to one capability class.
///
/// Capabilities outside `class` are carried forward from `previous` exactly as they were,
/// including any the fresh probe happens to have discovered. A GPU arriving is not
/// evidence about the network in either direction.
pub fn apply_scoped(
    previous: &CapabilitySet,
    fresh: &CapabilitySet,
    class: CapabilityClass,
) -> CapabilitySet {
    let mut merged = CapabilitySet::new();
    for capability in previous.iter() {
        if !in_class(capability, class) {
            merged.insert(capability.clone());
        }
    }
    for capability in fresh.iter() {
        if in_class(capability, class) {
            merged.insert(capability.clone());
        }
    }
    merged
}

/// Where uevents come from.
///
/// Behind a trait because the real source is an `AF_NETLINK` socket bound to the udev
/// monitor group — Linux-only, unopenable on a development host, and unopenable inside
/// `archied`'s sandbox until `RestrictAddressFamilies` admits `AF_NETLINK`. The
/// sequencing around it is the part that must be right everywhere.
pub trait UeventSource {
    /// Read the next event, waiting at most `timeout`. `None` means nothing arrived.
    ///
    /// Must not block past the timeout: the caller is also responsible for firing the
    /// debouncer's due work, and a source that blocks forever on a quiet machine would
    /// hold a burst's re-detection until the next unrelated device event.
    fn next_event(&mut self, timeout: Duration) -> Option<Uevent>;
}

/// Drives hotplug re-detection: events in, capability classes out.
///
/// Owns the debouncer so a burst of a dozen events from one physical device produces one
/// re-probe per capability class rather than a dozen.
pub struct HotplugDriver {
    debouncer: HotplugDebouncer,
}

impl Default for HotplugDriver {
    fn default() -> Self {
        Self::new(HotplugDebouncer::default())
    }
}

impl HotplugDriver {
    pub fn new(debouncer: HotplugDebouncer) -> Self {
        Self { debouncer }
    }

    /// How long to wait for the next event before checking what is due.
    ///
    /// Bounded by the debouncer's next deadline, so a burst that arrived and then went
    /// quiet still fires on time. Without this the re-probe would wait for the next
    /// unrelated device event, which on a quiet server could be days.
    pub fn poll_timeout(&self, now: Instant, idle: Duration) -> Duration {
        self.debouncer.next_deadline(now).unwrap_or(idle).min(idle)
    }

    /// Take one turn: drain what the source offers, then return the classes now due.
    ///
    /// Returning the classes rather than calling the runtime keeps this testable and
    /// keeps the decision about what a capability change *means* in the runtime, where
    /// the descriptor and the fabric live.
    pub fn tick<S: UeventSource>(
        &mut self,
        source: &mut S,
        now: Instant,
        timeout: Duration,
    ) -> Vec<CapabilityClass> {
        // One read per turn, and the caller loops. Draining unboundedly here would let a
        // flapping device starve the due-check that is the whole point of the debouncer:
        // a bad riser emitting events faster than they are consumed would keep this
        // function inside its own loop and the re-probe would never fire.
        if let Some(event) = source.next_event(timeout) {
            self.debouncer.observe(&event, now);
        }
        self.debouncer.due(now)
    }

    pub fn is_idle(&self) -> bool {
        self.debouncer.is_idle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> CapabilitySet {
        let mut capabilities = CapabilitySet::new();
        for name in names {
            capabilities.insert(Capability::parse(*name).unwrap());
        }
        capabilities
    }

    fn names(set: &CapabilitySet) -> Vec<String> {
        set.iter().map(|c| c.as_str().to_string()).collect()
    }

    // ---- the driver ---------------------------------------------------------

    /// Hands out a scripted list of events, then nothing.
    struct Scripted {
        events: Vec<Uevent>,
        reads: usize,
    }

    impl Scripted {
        fn new(events: Vec<Uevent>) -> Self {
            Self { events, reads: 0 }
        }
    }

    impl UeventSource for Scripted {
        fn next_event(&mut self, _timeout: Duration) -> Option<Uevent> {
            self.reads += 1;
            if self.events.is_empty() {
                None
            } else {
                Some(self.events.remove(0))
            }
        }
    }

    fn uevent(subsystem: &str, action: &str) -> Uevent {
        let mut payload = format!("{action}@/devices/{subsystem}").into_bytes();
        for property in [format!("ACTION={action}"), format!("SUBSYSTEM={subsystem}")] {
            payload.push(0);
            payload.extend_from_slice(property.as_bytes());
        }
        payload.push(0);
        Uevent::parse(&payload).unwrap()
    }

    #[test]
    fn a_burst_from_one_device_produces_one_reprobe() {
        let mut driver = HotplugDriver::new(HotplugDebouncer::new(Duration::from_millis(500)));
        let start = Instant::now();
        let mut source = Scripted::new(vec![
            uevent("pci", "add"),
            uevent("drm", "add"),
            uevent("drm", "bind"),
        ]);

        // Inside the window nothing is due yet.
        for offset in [0, 5, 10] {
            let due = driver.tick(
                &mut source,
                start + Duration::from_millis(offset),
                Duration::ZERO,
            );
            assert!(due.is_empty(), "must not fire before the window closes");
        }

        let due = driver.tick(
            &mut source,
            start + Duration::from_millis(600),
            Duration::ZERO,
        );
        assert_eq!(due, vec![CapabilityClass::Compute]);
        assert!(driver.is_idle());
    }

    #[test]
    fn the_poll_timeout_is_bounded_by_the_next_deadline() {
        // Without this the re-probe waits for the next unrelated device event, which on
        // a quiet server could be days.
        let mut driver = HotplugDriver::new(HotplugDebouncer::new(Duration::from_millis(500)));
        let start = Instant::now();
        let idle = Duration::from_secs(60);

        assert_eq!(
            driver.poll_timeout(start, idle),
            idle,
            "nothing pending, so wait the idle interval"
        );

        let mut source = Scripted::new(vec![uevent("drm", "bind")]);
        driver.tick(&mut source, start, Duration::ZERO);

        let remaining = driver.poll_timeout(start + Duration::from_millis(200), idle);
        assert_eq!(remaining, Duration::from_millis(300));
        assert!(remaining < idle);
    }

    #[test]
    fn events_that_change_nothing_never_wake_a_reprobe() {
        let mut driver = HotplugDriver::default();
        let start = Instant::now();
        let mut source = Scripted::new(vec![
            uevent("power_supply", "change"),
            uevent("input", "add"),
            uevent("backlight", "change"),
        ]);

        for offset in [0, 1, 2, 10_000] {
            let due = driver.tick(
                &mut source,
                start + Duration::from_millis(offset),
                Duration::ZERO,
            );
            assert!(due.is_empty());
        }
        assert!(driver.is_idle());
    }

    #[test]
    fn a_quiet_source_does_not_spin() {
        // The driver reads at most once per turn, so a source with nothing to say costs
        // one read and not an unbounded loop.
        let mut driver = HotplugDriver::default();
        let mut source = Scripted::new(vec![]);
        driver.tick(&mut source, Instant::now(), Duration::ZERO);
        assert_eq!(source.reads, 1);
    }

    #[test]
    fn a_gpu_arriving_adds_only_the_gpu() {
        let previous = set(&["compute.cpu", "network.lan", "storage.object"]);
        let fresh = set(&[
            "compute.cpu",
            "compute.gpu",
            "compute.inference",
            "network.lan",
            "storage.object",
        ]);

        let delta = scoped_delta(&previous, &fresh, CapabilityClass::Compute);
        assert_eq!(
            names(&apply_scoped(&previous, &fresh, CapabilityClass::Compute)),
            vec![
                "compute.cpu",
                "compute.gpu",
                "compute.inference",
                "network.lan",
                "storage.object"
            ]
        );
        assert_eq!(delta.gained.len(), 2);
        assert!(delta.lost.is_empty());
    }

    #[test]
    fn a_gpu_event_cannot_withdraw_a_network_capability() {
        // This is the whole reason the module exists. `sysinfo` returning an empty
        // interface list for one call — a NIC resetting, a namespace in flux, a
        // permission blip — must not delete `network.lan` from the manifest *because a
        // GPU was plugged in*.
        let previous = set(&["compute.cpu", "network.lan", "storage.object"]);
        let fresh_but_network_blind = set(&["compute.cpu", "compute.gpu", "storage.object"]);

        let merged = apply_scoped(
            &previous,
            &fresh_but_network_blind,
            CapabilityClass::Compute,
        );
        assert!(
            merged.contains(&Capability::parse("network.lan").unwrap()),
            "an unrelated probe failure must not withdraw a capability"
        );
        assert!(merged.contains(&Capability::parse("compute.gpu").unwrap()));

        let delta = scoped_delta(
            &previous,
            &fresh_but_network_blind,
            CapabilityClass::Compute,
        );
        assert!(
            delta.lost.is_empty(),
            "nothing in the compute class was actually lost"
        );
    }

    #[test]
    fn a_gpu_leaving_does_withdraw_the_gpu() {
        // Scoping must not become a ratchet that only ever adds. Within its own class,
        // a re-probe is authoritative.
        let previous = set(&[
            "compute.cpu",
            "compute.gpu",
            "compute.inference",
            "network.lan",
        ]);
        let fresh = set(&["compute.cpu", "network.lan"]);

        let delta = scoped_delta(&previous, &fresh, CapabilityClass::Compute);
        assert_eq!(delta.gained, vec![]);
        assert_eq!(
            delta.lost.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            vec!["compute.gpu", "compute.inference"]
        );

        let merged = apply_scoped(&previous, &fresh, CapabilityClass::Compute);
        assert!(!merged.contains(&Capability::parse("compute.gpu").unwrap()));
        assert!(merged.contains(&Capability::parse("network.lan").unwrap()));
    }

    #[test]
    fn a_network_event_does_not_touch_compute() {
        let previous = set(&["compute.cpu", "compute.gpu", "network.lan"]);
        // A probe that momentarily cannot see the GPU, during a NIC event.
        let fresh = set(&["compute.cpu", "network.lan", "network.mesh"]);

        let merged = apply_scoped(&previous, &fresh, CapabilityClass::Network);
        assert!(
            merged.contains(&Capability::parse("compute.gpu").unwrap()),
            "a NIC event says nothing about the GPU"
        );
        assert!(merged.contains(&Capability::parse("network.mesh").unwrap()));
    }

    #[test]
    fn an_unchanged_probe_produces_no_delta_and_no_log_line() {
        // The overwhelming majority of hotplug re-probes change nothing. A log line per
        // event would bury the ones that did.
        let previous = set(&["compute.cpu", "network.lan"]);
        let delta = scoped_delta(&previous, &previous, CapabilityClass::Compute);
        assert!(delta.is_empty());
        assert_eq!(delta.summary(), None);
        assert_eq!(
            names(&apply_scoped(
                &previous,
                &previous,
                CapabilityClass::Compute
            )),
            names(&previous)
        );
    }

    #[test]
    fn a_delta_reads_as_one_line() {
        let previous = set(&["compute.cpu", "compute.gpu"]);
        let fresh = set(&["compute.cpu", "compute.inference"]);
        let delta = scoped_delta(&previous, &fresh, CapabilityClass::Compute);
        assert_eq!(
            delta.summary().unwrap(),
            "gained compute.inference, lost compute.gpu"
        );

        let only_gained = scoped_delta(&set(&["compute.cpu"]), &fresh, CapabilityClass::Compute);
        assert_eq!(only_gained.summary().unwrap(), "gained compute.inference");

        let only_lost = scoped_delta(&previous, &set(&["compute.cpu"]), CapabilityClass::Compute);
        assert_eq!(only_lost.summary().unwrap(), "lost compute.gpu");
    }

    #[test]
    fn a_capability_outside_every_class_is_never_disturbed() {
        // Operator-configured capabilities — `simulation.cfd`, `cad.solid` — have no
        // hardware class and must survive every hotplug event untouched.
        let previous = set(&["compute.cpu", "simulation.cfd", "cad.solid"]);
        let fresh = set(&["compute.cpu", "compute.gpu"]);

        for class in [
            CapabilityClass::Compute,
            CapabilityClass::Network,
            CapabilityClass::Storage,
            CapabilityClass::Sensor,
            CapabilityClass::Bus,
            CapabilityClass::Radio,
        ] {
            let merged = apply_scoped(&previous, &fresh, class);
            assert!(
                merged.contains(&Capability::parse("simulation.cfd").unwrap()),
                "{class:?} disturbed an operator capability"
            );
            assert!(merged.contains(&Capability::parse("cad.solid").unwrap()));
        }
    }

    #[test]
    fn the_class_prefix_matches_the_family_not_a_substring() {
        // `compute.` must not match `precompute.x`, and the boundary dot is what makes
        // that true.
        let previous = CapabilitySet::new();
        let fresh = set(&["compute.gpu"]);
        let merged = apply_scoped(&previous, &fresh, CapabilityClass::Compute);
        assert_eq!(names(&merged), vec!["compute.gpu"]);

        assert!(in_class(
            &Capability::parse("compute.gpu").unwrap(),
            CapabilityClass::Compute
        ));
        assert!(!in_class(
            &Capability::parse("storage.object").unwrap(),
            CapabilityClass::Compute
        ));
    }
}
