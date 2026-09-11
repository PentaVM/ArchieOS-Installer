//! Thermal state, read from the kernel's own trip points.
//!
//! # Why this belongs in the control plane
//!
//! A node that is about to be thermally throttled should stop accepting new inference
//! work *before* the kernel throttles it, not after. Once throttling starts, every
//! running job slows together and the scheduler's placement decisions were made against
//! capacity that no longer exists.
//!
//! # Why not invent thresholds
//!
//! The kernel already publishes what the hardware considers too hot, as trip points on
//! each thermal zone. Hardcoding "80 °C is hot" would be wrong on a laptop CPU designed
//! to run at 95 °C and dangerously permissive on an NVMe drive that should never see 70.
//! This module reads the trip points the firmware declared and classifies against those.
//!
//! Where a zone publishes no usable trip point, the honest answer is [`ThermalState::Unknown`]
//! — not a guess. A fabricated threshold is worse than no reading, because the scheduler
//! would act on it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Kernel temperatures are millidegrees Celsius throughout.
pub type MilliCelsius = i64;

/// How a zone is doing relative to what its own firmware declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermalState {
    /// No usable trip point. The temperature is reported; no judgement is made.
    Unknown,
    Nominal,
    /// Within 10 °C of the first throttling trip point. Stop admitting new work.
    Warm,
    /// At or past a throttling trip point. The kernel is, or is about to be, reducing
    /// clocks.
    Throttling,
    /// At or past a critical trip point, where the firmware may power the machine off.
    Critical,
}

impl ThermalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Nominal => "nominal",
            Self::Warm => "warm",
            Self::Throttling => "throttling",
            Self::Critical => "critical",
        }
    }

    /// Whether new work may be placed on this node.
    ///
    /// `Unknown` permits work: a node with unreadable sensors is not thereby a hot node,
    /// and refusing to schedule on every machine with an unusual thermal driver would
    /// make the whole fabric unusable.
    pub fn permits_new_work(&self) -> bool {
        matches!(self, Self::Unknown | Self::Nominal)
    }

    /// Whether running work should be shed.
    pub fn should_shed_load(&self) -> bool {
        matches!(self, Self::Throttling | Self::Critical)
    }

    /// The node health this state implies.
    pub fn node_health(&self) -> &'static str {
        match self {
            Self::Unknown | Self::Nominal => "ok",
            Self::Warm | Self::Throttling => "degraded",
            Self::Critical => "unhealthy",
        }
    }
}

/// A trip point as the firmware declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TripKind {
    /// Firmware may cut power at this temperature.
    Critical,
    /// Firmware may begin an emergency shutdown sequence.
    Hot,
    /// Active cooling (a fan) engages.
    Active,
    /// Passive cooling: clocks are reduced. This is the one the scheduler cares about.
    Passive,
    Other,
}

impl TripKind {
    fn parse(value: &str) -> Self {
        match value.trim() {
            "critical" => Self::Critical,
            "hot" => Self::Hot,
            "active" => Self::Active,
            "passive" => Self::Passive,
            _ => Self::Other,
        }
    }

    /// Whether crossing this trip point means performance is being reduced.
    fn is_throttling(&self) -> bool {
        matches!(self, Self::Passive | Self::Hot | Self::Critical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TripPoint {
    pub kind: TripKind,
    pub temperature: MilliCelsius,
}

/// One thermal zone, as the kernel presents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThermalZone {
    /// The zone type string, e.g. `x86_pkg_temp`, `acpitz`, `nvme`.
    pub zone_type: String,
    pub temperature: MilliCelsius,
    pub trips: Vec<TripPoint>,
}

impl ThermalZone {
    /// Degrees Celsius, for display.
    pub fn celsius(&self) -> f32 {
        self.temperature as f32 / 1000.0
    }

    /// The lowest trip point that means throttling.
    ///
    /// Lowest rather than highest: the first one crossed is the one that starts costing
    /// performance, and that is what the scheduler must anticipate.
    pub fn throttle_trip(&self) -> Option<TripPoint> {
        self.trips
            .iter()
            .filter(|trip| trip.kind.is_throttling() && trip.temperature > 0)
            .min_by_key(|trip| trip.temperature)
            .copied()
    }

    pub fn critical_trip(&self) -> Option<TripPoint> {
        self.trips
            .iter()
            .filter(|trip| trip.kind == TripKind::Critical && trip.temperature > 0)
            .min_by_key(|trip| trip.temperature)
            .copied()
    }

    /// Classify against this zone's own declared limits.
    pub fn state(&self) -> ThermalState {
        if let Some(critical) = self.critical_trip() {
            if self.temperature >= critical.temperature {
                return ThermalState::Critical;
            }
        }
        let Some(throttle) = self.throttle_trip() else {
            // No usable trip point. Report the temperature, judge nothing.
            return ThermalState::Unknown;
        };
        if self.temperature >= throttle.temperature {
            return ThermalState::Throttling;
        }
        // Ten degrees of warning before the first throttle, so the scheduler can stop
        // admitting work while there is still headroom to finish what is running.
        if self.temperature >= throttle.temperature - 10_000 {
            return ThermalState::Warm;
        }
        ThermalState::Nominal
    }

    /// Headroom to the first throttling trip, in millidegrees. Negative once past it.
    pub fn headroom(&self) -> Option<MilliCelsius> {
        self.throttle_trip()
            .map(|trip| trip.temperature - self.temperature)
    }
}

/// Every zone the machine exposes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThermalReport {
    pub zones: Vec<ThermalZone>,
}

impl ThermalReport {
    /// The worst state across all zones.
    ///
    /// Worst rather than average, for the same reason pressure takes the worst resource:
    /// a machine with a cool CPU and a critical NVMe is not "fine on average".
    pub fn state(&self) -> ThermalState {
        self.zones
            .iter()
            .map(ThermalZone::state)
            .max()
            .unwrap_or(ThermalState::Unknown)
    }

    /// The zone driving the verdict.
    pub fn hottest_zone(&self) -> Option<&ThermalZone> {
        self.zones.iter().max_by_key(|zone| {
            // Rank by state first, then by how far past its own limit it is — comparing
            // raw temperatures across zones is meaningless when each has a different
            // design limit.
            (zone.state(), -zone.headroom().unwrap_or(MilliCelsius::MAX))
        })
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// A line for the boot report and the observatory.
    pub fn explain(&self) -> String {
        if self.zones.is_empty() {
            return "no thermal zones exposed by this kernel".to_string();
        }
        let state = self.state();
        match self.hottest_zone() {
            Some(zone) => match zone.throttle_trip() {
                Some(trip) => format!(
                    "{}: {} at {:.1} °C, throttles at {:.1} °C",
                    state.as_str(),
                    zone.zone_type,
                    zone.celsius(),
                    trip.temperature as f32 / 1000.0
                ),
                None => format!(
                    "{}: {} at {:.1} °C, no trip point published",
                    state.as_str(),
                    zone.zone_type,
                    zone.celsius()
                ),
            },
            None => "no thermal zones exposed by this kernel".to_string(),
        }
    }
}

/// Read every `thermal_zone*` under a sysfs-shaped directory.
///
/// Takes the directory so tests can point it at fixtures.
pub fn read_from_dir(dir: impl AsRef<Path>) -> ThermalReport {
    let dir = dir.as_ref();
    let Ok(entries) = std::fs::read_dir(dir) else {
        // No thermal class at all: not Linux, or a kernel without the subsystem. An
        // empty report is the truthful answer, and `state()` will say `Unknown`.
        return ThermalReport::default();
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("thermal_zone"))
        })
        .collect();
    // Stable order so two reads of an unchanged machine compare equal.
    paths.sort();

    let zones = paths.iter().filter_map(|path| read_zone(path)).collect();
    ThermalReport { zones }
}

fn read_zone(path: &Path) -> Option<ThermalZone> {
    // A zone with no readable temperature is not a zone worth reporting. Its trip points
    // alone tell the scheduler nothing.
    let temperature = read_milli(&path.join("temp"))?;
    let zone_type = read_trimmed(&path.join("type")).unwrap_or_else(|| "unknown".to_string());

    // Trip points are numbered and sparse; stop after a run of misses rather than
    // scanning a fixed range, and cap it so a malformed sysfs cannot spin.
    let mut trips = Vec::new();
    for index in 0..32 {
        let kind_path = path.join(format!("trip_point_{index}_type"));
        let temp_path = path.join(format!("trip_point_{index}_temp"));
        let (Some(kind), Some(temperature)) = (read_trimmed(&kind_path), read_milli(&temp_path))
        else {
            continue;
        };
        trips.push(TripPoint {
            kind: TripKind::parse(&kind),
            temperature,
        });
    }

    Some(ThermalZone {
        zone_type,
        temperature,
        trips,
    })
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn read_milli(path: &Path) -> Option<MilliCelsius> {
    read_trimmed(path)?.parse().ok()
}

/// Read the machine's thermal state from the kernel.
pub fn read_thermal() -> ThermalReport {
    read_from_dir("/sys/class/thermal")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(temp: MilliCelsius, trips: Vec<TripPoint>) -> ThermalZone {
        ThermalZone {
            zone_type: "x86_pkg_temp".into(),
            temperature: temp,
            trips,
        }
    }

    fn passive(temp: MilliCelsius) -> TripPoint {
        TripPoint {
            kind: TripKind::Passive,
            temperature: temp,
        }
    }

    fn critical(temp: MilliCelsius) -> TripPoint {
        TripPoint {
            kind: TripKind::Critical,
            temperature: temp,
        }
    }

    // ───────────────────────────────────────────────────────────── classification

    #[test]
    fn a_cool_zone_is_nominal_and_accepts_work() {
        let z = zone(45_000, vec![passive(95_000), critical(105_000)]);
        assert_eq!(z.state(), ThermalState::Nominal);
        assert!(z.state().permits_new_work());
        assert!(!z.state().should_shed_load());
        assert_eq!(z.celsius(), 45.0);
    }

    #[test]
    fn a_zone_is_judged_against_its_own_trip_points_not_a_fixed_number() {
        // THE reason this module reads trip points. 90 °C is nominal on a CPU designed
        // for 100, and throttling on an NVMe designed for 70. A hardcoded threshold gets
        // one of these badly wrong.
        let cpu = ThermalZone {
            zone_type: "x86_pkg_temp".into(),
            temperature: 90_000,
            trips: vec![passive(100_000), critical(105_000)],
        };
        let nvme = ThermalZone {
            zone_type: "nvme".into(),
            temperature: 90_000,
            trips: vec![passive(70_000), critical(85_000)],
        };
        assert_eq!(cpu.state(), ThermalState::Warm);
        assert_eq!(
            nvme.state(),
            ThermalState::Critical,
            "the same temperature is fine on one device and critical on another"
        );
    }

    #[test]
    fn warm_starts_ten_degrees_before_the_throttle_point() {
        // Enough headroom to stop admitting work while what is running can still finish.
        let z = zone(86_000, vec![passive(95_000)]);
        assert_eq!(z.state(), ThermalState::Warm);
        assert!(
            !z.state().permits_new_work(),
            "a warm node should stop taking new work before it is throttled"
        );
        assert!(
            !z.state().should_shed_load(),
            "but it need not abandon what it is already doing"
        );
        assert_eq!(z.state().node_health(), "degraded");
    }

    #[test]
    fn at_the_throttle_point_load_is_shed() {
        let z = zone(95_000, vec![passive(95_000)]);
        assert_eq!(z.state(), ThermalState::Throttling);
        assert!(z.state().should_shed_load());
        assert_eq!(z.headroom(), Some(0));
    }

    #[test]
    fn past_critical_the_node_is_unhealthy() {
        let z = zone(106_000, vec![passive(95_000), critical(105_000)]);
        assert_eq!(z.state(), ThermalState::Critical);
        assert_eq!(z.state().node_health(), "unhealthy");
    }

    #[test]
    fn a_zone_with_no_trip_points_is_unknown_not_guessed() {
        // A fabricated threshold is worse than no reading, because the scheduler acts on
        // it. Unknown still permits work: an unusual thermal driver must not make a
        // whole machine unschedulable.
        let z = zone(70_000, vec![]);
        assert_eq!(z.state(), ThermalState::Unknown);
        assert!(z.state().permits_new_work());
        assert_eq!(z.state().node_health(), "ok");
        assert_eq!(z.headroom(), None);
    }

    #[test]
    fn disabled_trip_points_are_ignored() {
        // The kernel publishes 0 or a negative value for a disabled trip point; taking
        // it literally would put every zone permanently past critical.
        let z = zone(50_000, vec![passive(0), critical(0)]);
        assert_eq!(z.state(), ThermalState::Unknown);
    }

    #[test]
    fn the_lowest_throttling_trip_is_the_one_that_matters() {
        // The first one crossed is the one that starts costing performance.
        let z = zone(50_000, vec![passive(90_000), critical(105_000)]);
        assert_eq!(z.throttle_trip().unwrap().temperature, 90_000);
    }

    #[test]
    fn active_cooling_is_not_throttling() {
        // A fan spinning up is not a performance reduction.
        let z = zone(
            60_000,
            vec![
                TripPoint {
                    kind: TripKind::Active,
                    temperature: 55_000,
                },
                passive(95_000),
            ],
        );
        assert_eq!(z.state(), ThermalState::Nominal);
        assert_eq!(z.throttle_trip().unwrap().temperature, 95_000);
    }

    // ─────────────────────────────────────────────────────────────────── report

    #[test]
    fn the_worst_zone_decides_the_verdict() {
        // A cool CPU and a critical NVMe is not "fine on average".
        let report = ThermalReport {
            zones: vec![
                zone(40_000, vec![passive(95_000)]),
                ThermalZone {
                    zone_type: "nvme".into(),
                    temperature: 86_000,
                    trips: vec![passive(70_000), critical(85_000)],
                },
            ],
        };
        assert_eq!(report.state(), ThermalState::Critical);
        assert_eq!(report.hottest_zone().unwrap().zone_type, "nvme");
        assert!(report.explain().contains("nvme"));
    }

    #[test]
    fn an_empty_report_is_unknown_and_says_so() {
        let report = ThermalReport::default();
        assert_eq!(report.state(), ThermalState::Unknown);
        assert!(report.is_empty());
        assert!(report.explain().contains("no thermal zones"));
        assert!(
            report.state().permits_new_work(),
            "a machine with no sensors is not thereby a hot machine"
        );
    }

    #[test]
    fn the_explanation_names_the_zone_and_its_limit() {
        let report = ThermalReport {
            zones: vec![zone(96_000, vec![passive(95_000)])],
        };
        let explanation = report.explain();
        assert!(explanation.contains("throttling"), "{explanation}");
        assert!(explanation.contains("x86_pkg_temp"), "{explanation}");
        assert!(explanation.contains("96.0"), "{explanation}");
        assert!(explanation.contains("95.0"), "{explanation}");
    }

    // ────────────────────────────────────────────────────────────────── reading

    #[test]
    fn reads_a_sysfs_shaped_directory() {
        let dir = std::env::temp_dir().join(format!("archie-thermal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let z0 = dir.join("thermal_zone0");
        std::fs::create_dir_all(&z0).unwrap();
        std::fs::write(z0.join("type"), "x86_pkg_temp\n").unwrap();
        std::fs::write(z0.join("temp"), "72000\n").unwrap();
        std::fs::write(z0.join("trip_point_0_type"), "passive\n").unwrap();
        std::fs::write(z0.join("trip_point_0_temp"), "95000\n").unwrap();
        std::fs::write(z0.join("trip_point_1_type"), "critical\n").unwrap();
        std::fs::write(z0.join("trip_point_1_temp"), "105000\n").unwrap();
        // A non-zone directory that must be skipped.
        std::fs::create_dir_all(dir.join("cooling_device0")).unwrap();

        let report = read_from_dir(&dir);
        assert_eq!(report.zones.len(), 1, "cooling devices are not zones");
        let zone = &report.zones[0];
        assert_eq!(zone.zone_type, "x86_pkg_temp");
        assert_eq!(zone.temperature, 72_000);
        assert_eq!(zone.trips.len(), 2);
        assert_eq!(zone.state(), ThermalState::Nominal);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zone_with_no_readable_temperature_is_skipped() {
        // Trip points alone tell the scheduler nothing.
        let dir = std::env::temp_dir().join(format!("archie-thermal-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let z0 = dir.join("thermal_zone0");
        std::fs::create_dir_all(&z0).unwrap();
        std::fs::write(z0.join("type"), "broken\n").unwrap();
        std::fs::write(z0.join("trip_point_0_type"), "passive\n").unwrap();
        std::fs::write(z0.join("trip_point_0_temp"), "95000\n").unwrap();

        assert!(read_from_dir(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sparse_trip_point_numbering_is_handled() {
        // Trip points are not guaranteed contiguous.
        let dir =
            std::env::temp_dir().join(format!("archie-thermal-sparse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let z0 = dir.join("thermal_zone0");
        std::fs::create_dir_all(&z0).unwrap();
        std::fs::write(z0.join("type"), "acpitz\n").unwrap();
        std::fs::write(z0.join("temp"), "50000\n").unwrap();
        std::fs::write(z0.join("trip_point_0_type"), "critical\n").unwrap();
        std::fs::write(z0.join("trip_point_0_temp"), "105000\n").unwrap();
        // gap at 1 and 2
        std::fs::write(z0.join("trip_point_3_type"), "passive\n").unwrap();
        std::fs::write(z0.join("trip_point_3_temp"), "90000\n").unwrap();

        let report = read_from_dir(&dir);
        assert_eq!(report.zones[0].trips.len(), 2);
        assert_eq!(report.zones[0].throttle_trip().unwrap().temperature, 90_000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_thermal_class_yields_an_empty_report() {
        let missing = std::env::temp_dir().join("archie-no-thermal-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(read_from_dir(&missing).is_empty());
    }

    #[test]
    fn reading_the_real_kernel_never_panics() {
        let report = read_thermal();
        for zone in &report.zones {
            // Sanity: the kernel reports millidegrees, so a plausible machine is well
            // under 200 °C. This catches a units mistake on a real host.
            assert!(
                zone.temperature < 200_000,
                "zone {} reported {} millidegrees; units are probably wrong",
                zone.zone_type,
                zone.temperature
            );
        }
        let _ = report.state();
    }

    #[test]
    fn a_report_round_trips_through_json() {
        let report = ThermalReport {
            zones: vec![zone(72_000, vec![passive(95_000), critical(105_000)])],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<ThermalReport>(&json).unwrap(),
            report
        );
    }
}
