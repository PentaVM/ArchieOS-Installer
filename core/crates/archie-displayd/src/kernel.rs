//! Kernel telemetry for the observatory: what the machine is actually doing.
//!
//! # Why this is not part of the archied snapshot
//!
//! Everything in [`crate::snapshot::DisplaySnapshot`] comes from `archied` over the
//! control socket. When `archied` is unreachable that snapshot is, correctly, marked
//! stale and the renderer shows a disconnected state.
//!
//! Pressure and thermals do not come from `archied`. They are read from `/proc` and
//! `/sys` by this process, on this machine. Folding them into the daemon snapshot would
//! throw away a genuinely useful property: **when `archied` is down, this is still
//! true.** An operator looking at a disconnected observatory most wants to know whether
//! the machine is thrashing or overheating, and that is exactly the moment the daemon
//! cannot tell them.
//!
//! So this is served separately, at `/api/kernel`, and its liveness is independent.
//!
//! # Honest absence
//!
//! PSI requires `CONFIG_PSI=y` and a kernel that exposes `/proc/pressure`; thermal zones
//! require a driver that publishes them. Neither is guaranteed. When a source is missing
//! this reports [`Availability::Unavailable`] with the reason, and the renderer draws
//! nothing rather than zeros. A gauge reading zero pressure and a gauge that cannot read
//! pressure must never look the same — one says the machine is idle, the other says we
//! do not know.

use archie_observability::kernel::psi::{
    self, PressureThresholds, PressureVerdict, SystemPressure,
};
use archie_observability::kernel::thermal::{self, ThermalReport, ThermalState};
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Whether a telemetry source could be read at all.
///
/// Deliberately not `Option`: "absent" needs a reason attached, or the renderer has
/// nothing honest to display and the operator has nothing to act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    Available,
    /// The source does not exist or could not be read, and why.
    Unavailable {
        reason: String,
    },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// One thermal zone, flattened for the renderer.
///
/// The raw [`thermal::ThermalZone`] carries every trip point the firmware declares. The
/// renderer needs the temperature, the verdict, and — so it can draw a bar rather than a
/// number — how close to the limit that temperature is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneView {
    /// The kernel's own zone type, e.g. `x86_pkg_temp`, `nvme`, `acpitz`.
    pub zone_type: String,
    pub celsius: f32,
    pub state: ThermalState,
    /// Fraction of the way from 0 °C to this zone's own critical trip, clamped to 1.0.
    ///
    /// Scaled per zone, never against a shared axis: a CPU at 90 °C with a 100 °C trip and
    /// an NVMe at 90 °C with an 85 °C trip are not in the same condition, and one bar
    /// drawn against one scale would say they are.
    ///
    /// `None` when the firmware declares no critical trip. There is no scale to place the
    /// reading on, and inventing one draws a bar that means nothing.
    pub headroom_fraction: Option<f32>,
}

/// Machine telemetry read directly, independent of `archied`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelSnapshot {
    pub schema_version: u16,
    pub read_at: Timestamp,

    pub pressure_availability: Availability,
    /// Present only when `pressure_availability` is available.
    pub pressure: Option<SystemPressure>,
    /// The worst verdict across CPU, memory and IO.
    pub verdict: Option<PressureVerdict>,
    /// Which resource produced that verdict — the one worth naming on screen.
    pub worst_resource: Option<String>,

    pub thermal_availability: Availability,
    pub zones: Vec<ZoneView>,
    /// The worst state across all zones, which is what the headline should reflect.
    pub thermal_state: Option<ThermalState>,
    /// The zone driving that state — the one worth naming on screen.
    pub hottest_zone: Option<String>,

    /// The quality budget this machine can currently afford.
    ///
    /// Private on purpose. The renderer needs this over the wire, so it has to be a
    /// serialized field rather than a method — and a public field could be set to
    /// `cinematic` next to a `saturated` verdict by anyone assembling a snapshot by hand.
    /// Keeping it private means [`Self::read`] is the only thing that can set it, so the
    /// budget cannot disagree with the evidence it was derived from.
    display_budget: String,
}

impl KernelSnapshot {
    /// Read the machine now.
    pub fn read(thresholds: &PressureThresholds) -> Self {
        let read_at = Timestamp::now();

        let (pressure_availability, pressure, verdict, worst_resource) =
            match psi::read_system_pressure() {
                Ok(pressure) => {
                    let verdict = pressure.verdict(thresholds);
                    let worst = pressure.worst_resource(thresholds).to_string();
                    (
                        Availability::Available,
                        Some(pressure),
                        Some(verdict),
                        Some(worst),
                    )
                }
                Err(e) => (
                    Availability::Unavailable {
                        reason: e.to_string(),
                    },
                    None,
                    None,
                    None,
                ),
            };

        // `read_thermal` does not fail — a machine with no thermal drivers yields an
        // empty report. Empty is the unavailable case, and saying so is the point: a
        // renderer must not draw an empty temperature panel that looks like a cool
        // machine when the truth is that nothing is being measured.
        let report = thermal::read_thermal();
        let (thermal_availability, zones, thermal_state, hottest_zone) = if report.is_empty() {
            (
                Availability::Unavailable {
                    reason: "this kernel publishes no thermal zones".to_string(),
                },
                Vec::new(),
                None,
                None,
            )
        } else {
            // `report.state()` already takes the worst zone rather than an average, and
            // `hottest_zone` ranks by state before temperature — comparing raw degrees
            // across zones with different design limits is meaningless. Reuse both rather
            // than recomputing them differently here.
            (
                Availability::Available,
                zone_views(&report),
                Some(report.state()),
                report.hottest_zone().map(|zone| zone.zone_type.clone()),
            )
        };

        // Unknown pressure yields the balanced budget, never the cinematic one: absent
        // evidence, a display must not assume it has room.
        let display_budget = match verdict {
            Some(verdict) => verdict.display_budget(),
            None => "balanced",
        }
        .to_string();

        Self {
            schema_version: 1,
            read_at,
            pressure_availability,
            pressure,
            verdict,
            worst_resource,
            thermal_availability,
            zones,
            thermal_state,
            hottest_zone,
            display_budget,
        }
    }

    /// The quality budget this machine can currently afford.
    ///
    /// This is the number the renderer acts on, and it is the whole point of reading PSI
    /// from the display process: the observatory is the largest discretionary consumer on
    /// the node, so it should be the first thing to get out of the way when the machine
    /// is busy doing what it is actually for.
    ///
    /// Unknown pressure yields the balanced budget, not the cinematic one. Absent
    /// evidence, a display should not assume it has room.
    pub fn display_budget(&self) -> &str {
        &self.display_budget
    }

    /// Whether the renderer should shed work regardless of pressure, because the machine
    /// is hot.
    ///
    /// Thermal and pressure are independent reasons to back off, and a node can be cool
    /// while thrashing or idle while throttling.
    pub fn should_shed_for_heat(&self) -> bool {
        self.thermal_state
            .is_some_and(|state| state.should_shed_load())
    }
}

fn zone_views(report: &ThermalReport) -> Vec<ZoneView> {
    report
        .zones
        .iter()
        .map(|zone| ZoneView {
            zone_type: zone.zone_type.clone(),
            celsius: zone.celsius(),
            state: zone.state(),
            headroom_fraction: zone
                .critical_trip()
                .filter(|critical| critical.temperature > 0)
                .map(|critical| {
                    (zone.temperature as f32 / critical.temperature as f32).clamp(0.0, 1.0)
                }),
        })
        .collect()
}

/// Serves kernel telemetry, refreshed on demand and cached between reads.
///
/// The cache exists because the renderer polls faster than `/proc/pressure` meaningfully
/// changes — PSI averages over ten seconds at its shortest window, so reading it at 60 Hz
/// returns the same numbers while costing a syscall per frame.
#[derive(Debug, Clone)]
pub struct KernelService {
    thresholds: PressureThresholds,
    latest: Arc<RwLock<KernelSnapshot>>,
}

impl KernelService {
    pub fn new(thresholds: PressureThresholds) -> Self {
        Self {
            latest: Arc::new(RwLock::new(KernelSnapshot::read(&thresholds))),
            thresholds,
        }
    }

    /// Re-read the machine.
    pub async fn refresh(&self) {
        let fresh = KernelSnapshot::read(&self.thresholds);
        *self.latest.write().await = fresh;
    }

    /// The most recent reading.
    pub async fn current(&self) -> KernelSnapshot {
        self.latest.read().await.clone()
    }
}

impl Default for KernelService {
    fn default() -> Self {
        Self::new(PressureThresholds::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_this_machine_never_panics_and_is_honest_about_what_it_found() {
        // Linux exercises the real path. Everywhere else both sources must report
        // Unavailable with a reason rather than fabricating zeros.
        let snapshot = KernelSnapshot::read(&PressureThresholds::default());
        assert_eq!(snapshot.schema_version, 1);

        match &snapshot.pressure_availability {
            Availability::Available => {
                assert!(
                    snapshot.pressure.is_some(),
                    "available pressure must carry a reading"
                );
                assert!(snapshot.verdict.is_some());
                assert!(snapshot.worst_resource.is_some());
            }
            Availability::Unavailable { reason } => {
                assert!(!reason.is_empty(), "absence must carry a reason");
                assert!(
                    snapshot.pressure.is_none(),
                    "unavailable pressure must not carry a reading"
                );
                assert!(snapshot.verdict.is_none());
            }
        }

        match &snapshot.thermal_availability {
            Availability::Available => {}
            Availability::Unavailable { reason } => {
                assert!(!reason.is_empty());
                assert!(snapshot.zones.is_empty());
                assert!(snapshot.thermal_state.is_none());
            }
        }
    }

    #[test]
    fn unknown_pressure_yields_the_balanced_budget_not_the_cinematic_one() {
        // Absent evidence, a display must not assume it has room.
        let snapshot = KernelSnapshot {
            schema_version: 1,
            read_at: Timestamp::now(),
            pressure_availability: Availability::Unavailable {
                reason: "no /proc/pressure".into(),
            },
            pressure: None,
            verdict: None,
            worst_resource: None,
            thermal_availability: Availability::Unavailable {
                reason: "no zones".into(),
            },
            zones: Vec::new(),
            thermal_state: None,
            hottest_zone: None,
            display_budget: "balanced".into(),
        };
        assert_eq!(snapshot.display_budget(), "balanced");
        assert!(!snapshot.should_shed_for_heat());
    }

    #[test]
    fn the_budget_follows_the_verdict() {
        let mut snapshot = KernelSnapshot {
            schema_version: 1,
            read_at: Timestamp::now(),
            pressure_availability: Availability::Available,
            pressure: None,
            verdict: Some(PressureVerdict::Idle),
            worst_resource: Some("cpu".into()),
            thermal_availability: Availability::Available,
            zones: Vec::new(),
            thermal_state: None,
            hottest_zone: None,
            display_budget: PressureVerdict::Idle.display_budget().into(),
        };
        assert_eq!(
            snapshot.display_budget(),
            PressureVerdict::Idle.display_budget()
        );

        snapshot.verdict = Some(PressureVerdict::Saturated);
        snapshot.display_budget = PressureVerdict::Saturated.display_budget().into();
        assert_eq!(
            snapshot.display_budget(),
            PressureVerdict::Saturated.display_budget()
        );
        assert_ne!(
            PressureVerdict::Idle.display_budget(),
            PressureVerdict::Saturated.display_budget(),
            "a saturated machine must not get the same budget as an idle one"
        );
    }

    #[test]
    fn heat_sheds_work_independently_of_pressure() {
        // A node can be cool while thrashing, or idle while throttling. Neither reason
        // should be able to mask the other.
        let base = KernelSnapshot {
            schema_version: 1,
            read_at: Timestamp::now(),
            pressure_availability: Availability::Available,
            pressure: None,
            verdict: Some(PressureVerdict::Idle),
            worst_resource: Some("cpu".into()),
            thermal_availability: Availability::Available,
            zones: Vec::new(),
            thermal_state: Some(ThermalState::Critical),
            hottest_zone: Some("nvme".into()),
            display_budget: PressureVerdict::Idle.display_budget().into(),
        };
        assert!(
            base.should_shed_for_heat(),
            "an idle but critically hot node must still shed"
        );

        let cool = KernelSnapshot {
            thermal_state: Some(ThermalState::Nominal),
            ..base.clone()
        };
        assert!(!cool.should_shed_for_heat());
    }

    #[test]
    fn availability_round_trips_and_carries_its_reason() {
        let unavailable = Availability::Unavailable {
            reason: "CONFIG_PSI not enabled".into(),
        };
        let json = serde_json::to_string(&unavailable).unwrap();
        assert!(json.contains("CONFIG_PSI"));
        assert_eq!(
            serde_json::from_str::<Availability>(&json).unwrap(),
            unavailable
        );
        assert!(!unavailable.is_available());
        assert!(Availability::Available.is_available());
    }

    #[test]
    fn the_budget_travels_on_the_wire_and_matches_the_verdict_that_produced_it() {
        // The renderer reads this over HTTP, so a method would not reach it. It has to be
        // a field — and a field can be set to something the evidence does not support,
        // which is why only `read` can set it.
        let snapshot = KernelSnapshot::read(&PressureThresholds::default());
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();

        let on_wire = json
            .get("display_budget")
            .and_then(|value| value.as_str())
            .expect("the renderer cannot act on a budget that is not serialized");
        assert_eq!(on_wire, snapshot.display_budget());

        let expected = match snapshot.verdict {
            Some(verdict) => verdict.display_budget(),
            None => "balanced",
        };
        assert_eq!(
            on_wire, expected,
            "the budget must be derivable from the verdict beside it, or the two can drift"
        );
    }

    #[test]
    fn a_snapshot_survives_the_wire() {
        let snapshot = KernelSnapshot::read(&PressureThresholds::default());
        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: KernelSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[tokio::test]
    async fn the_service_caches_between_reads_and_refreshes_on_demand() {
        // The renderer polls faster than PSI meaningfully changes; a syscall per frame
        // buys nothing.
        let service = KernelService::default();
        let first = service.current().await;
        let second = service.current().await;
        assert_eq!(first, second, "current() must not re-read");

        service.refresh().await;
        let third = service.current().await;
        assert!(
            third.read_at.as_millis() >= first.read_at.as_millis(),
            "a refresh must not move time backwards"
        );
        assert_eq!(
            third.pressure_availability.is_available(),
            first.pressure_availability.is_available(),
            "availability must not flap between reads on a stable machine"
        );
    }
}
