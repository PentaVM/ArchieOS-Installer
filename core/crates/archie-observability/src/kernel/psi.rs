//! Pressure Stall Information — the correct signal for "is this machine busy".
//!
//! # Why not CPU percentage
//!
//! The scheduler needs to know when it may run deep-idle research and when it must get
//! out of the way. CPU utilisation answers a different question and is wrong in both
//! directions:
//!
//! * A batch job pinning eight cores, stalling nothing, reads as 100% busy. The machine
//!   is in fact perfectly responsive and there is capacity to schedule against.
//! * A machine at 20% CPU that is thrashing memory reads as idle. It is the least
//!   responsive it will ever be, and starting more work makes it worse.
//!
//! PSI measures the thing that actually matters: how much time runnable work spent
//! *waiting* on a resource it could not get. That is the definition of contention, and
//! it is what the kernel already computes.
//!
//! # The two lines
//!
//! ```text
//! some avg10=0.00 avg60=0.00 avg300=0.00 total=0
//! full avg10=0.00 avg60=0.00 avg300=0.00 total=0
//! ```
//!
//! `some` is "at least one task was stalled" — the onset of contention. `full` is
//! "every runnable task was stalled" — the machine achieving nothing at all. They are
//! not redundant: `some` rising is a warning, `full` rising is damage.
//!
//! `/proc/pressure/cpu` has no `full` line on many kernels, because a CPU cannot stall
//! every task and still be running one. Treating that absence as a parse failure would
//! make PSI unusable on exactly the resource the scheduler cares most about.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The three averaging windows the kernel publishes, as percentages of wall time.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PressureWindow {
    pub avg10: f32,
    pub avg60: f32,
    pub avg300: f32,
    /// Cumulative stall time in microseconds since boot. Monotonic, so two samples give
    /// a precise rate over any interval — more useful than the averages for anything
    /// that needs to be exact.
    pub total_us: u64,
}

/// One resource's pressure.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ResourcePressure {
    /// At least one task stalled. The onset of contention.
    pub some: PressureWindow,
    /// Every runnable task stalled. `None` for CPU on kernels that omit the line, which
    /// is most of them.
    pub full: Option<PressureWindow>,
}

/// The whole machine's pressure.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SystemPressure {
    pub cpu: ResourcePressure,
    pub memory: ResourcePressure,
    pub io: ResourcePressure,
}

/// What the scheduler should do with the machine right now.
///
/// Ordered so comparisons read naturally: `Idle < Light < Contended < Saturated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureVerdict {
    /// Nothing is waiting. Deep-idle research and cinematic rendering are welcome.
    Idle,
    /// Some contention. Background work continues; do not start new deep-idle work.
    Light,
    /// Real contention. Yield idle work.
    Contended,
    /// The machine is achieving little. Only active user work should be running.
    Saturated,
}

impl PressureVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Light => "light",
            Self::Contended => "contended",
            Self::Saturated => "saturated",
        }
    }

    /// Whether new deep-idle work may be admitted.
    ///
    /// Deliberately stricter than [`Self::permits_background`]: deep-idle work is the
    /// most preemptable thing on the machine, so it should also be the most reluctant to
    /// start. Starting it during light contention only guarantees an eviction shortly
    /// after, and an eviction costs a checkpoint.
    pub fn permits_deep_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Whether background work may be admitted.
    pub fn permits_background(&self) -> bool {
        matches!(self, Self::Idle | Self::Light)
    }

    /// Whether running idle work should be asked to yield now.
    pub fn should_yield_idle_work(&self) -> bool {
        matches!(self, Self::Contended | Self::Saturated)
    }

    /// Display quality budget appropriate to this pressure, as named in RFC-006.
    ///
    /// The observatory must never starve inference. This is the mapping that makes the
    /// resource governor a decision rather than a hope.
    pub fn display_budget(&self) -> &'static str {
        match self {
            Self::Idle => "cinematic",
            Self::Light => "high",
            Self::Contended => "balanced",
            Self::Saturated => "low",
        }
    }
}

/// Thresholds for turning pressure into a verdict.
///
/// Held as data rather than constants so a node with different hardware can be tuned
/// without a rebuild, and so the numbers are visible to an operator reading config.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureThresholds {
    /// `some avg10` above this on any resource means contention has begun.
    pub light_some: f32,
    /// `some avg10` above this means real contention.
    pub contended_some: f32,
    /// `full avg10` above this means the machine is achieving little.
    pub saturated_full: f32,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            // 1% of wall time with something waiting is the practical floor for "not
            // perfectly idle". Below this, ordinary daemon housekeeping dominates.
            light_some: 1.0,
            // 20% is where interactive work starts to feel it.
            contended_some: 20.0,
            // Any sustained `full` pressure is serious: it means nothing is progressing.
            saturated_full: 10.0,
        }
    }
}

impl SystemPressure {
    /// Classify the machine.
    ///
    /// Takes the worst resource rather than an average. A machine with idle CPU and
    /// saturated I/O is not "half busy"; it is unable to do I/O work, and averaging
    /// would hide exactly the resource that is the problem.
    pub fn verdict(&self, thresholds: &PressureThresholds) -> PressureVerdict {
        [self.cpu, self.memory, self.io]
            .iter()
            .map(|resource| resource.verdict(thresholds))
            .max()
            .unwrap_or(PressureVerdict::Idle)
    }

    /// The resource in the worst state, for reporting.
    pub fn worst_resource(&self, thresholds: &PressureThresholds) -> &'static str {
        let mut worst = ("cpu", self.cpu.verdict(thresholds));
        for (name, resource) in [("memory", self.memory), ("io", self.io)] {
            let verdict = resource.verdict(thresholds);
            if verdict > worst.1 {
                worst = (name, verdict);
            }
        }
        worst.0
    }

    /// A line for the boot report and the observatory.
    pub fn explain(&self, thresholds: &PressureThresholds) -> String {
        let verdict = self.verdict(thresholds);
        if verdict == PressureVerdict::Idle {
            return "idle: nothing is waiting on cpu, memory or io".to_string();
        }
        let worst = self.worst_resource(thresholds);
        let resource = match worst {
            "memory" => self.memory,
            "io" => self.io,
            _ => self.cpu,
        };
        format!(
            "{}: {worst} stalled {:.1}% of the last 10s (some), {:.1}% (full)",
            verdict.as_str(),
            resource.some.avg10,
            resource.full.map(|f| f.avg10).unwrap_or(0.0)
        )
    }
}

impl ResourcePressure {
    fn verdict(&self, thresholds: &PressureThresholds) -> PressureVerdict {
        // `full` is checked first and independently: a resource can show modest `some`
        // pressure while every task on it is stalled, and that is the worse condition.
        if let Some(full) = self.full {
            if full.avg10 >= thresholds.saturated_full {
                return PressureVerdict::Saturated;
            }
        }
        if self.some.avg10 >= thresholds.contended_some {
            PressureVerdict::Contended
        } else if self.some.avg10 >= thresholds.light_some {
            PressureVerdict::Light
        } else {
            PressureVerdict::Idle
        }
    }
}

/// Why pressure could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PressureError {
    /// The kernel was built without `CONFIG_PSI`, or this is not Linux.
    ///
    /// Not an error in the ordinary sense: the caller should fall back to a cruder
    /// signal and say that it did, rather than reporting a fault.
    Unsupported,
    /// The file existed but did not parse. Worth surfacing: it means the format changed.
    Malformed(String),
}

impl std::fmt::Display for PressureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("kernel does not expose PSI (CONFIG_PSI)"),
            Self::Malformed(detail) => write!(f, "malformed pressure data: {detail}"),
        }
    }
}

/// Parse one `/proc/pressure/*` file.
///
/// Tolerates a missing `full` line, which is normal for `cpu`, and tolerates unknown
/// trailing fields, so a future kernel adding a column does not break the parser.
pub fn parse_resource(text: &str) -> Result<ResourcePressure, PressureError> {
    let mut some = None;
    let mut full = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let kind = fields.next().unwrap_or_default();

        let mut window = PressureWindow::default();
        let mut saw_avg10 = false;
        for field in fields {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "avg10" => {
                    window.avg10 = parse_f32(value, line)?;
                    saw_avg10 = true;
                }
                "avg60" => window.avg60 = parse_f32(value, line)?,
                "avg300" => window.avg300 = parse_f32(value, line)?,
                "total" => {
                    window.total_us = value
                        .parse()
                        .map_err(|_| PressureError::Malformed(line.to_string()))?
                }
                // An unrecognised key is ignored on purpose: a kernel adding a column
                // should not take the scheduler's idle signal away.
                _ => {}
            }
        }
        if !saw_avg10 {
            return Err(PressureError::Malformed(line.to_string()));
        }

        match kind {
            "some" => some = Some(window),
            "full" => full = Some(window),
            _ => return Err(PressureError::Malformed(line.to_string())),
        }
    }

    match some {
        Some(some) => Ok(ResourcePressure { some, full }),
        // Every PSI file has a `some` line. Its absence means this is not a PSI file.
        None => Err(PressureError::Malformed("no `some` line".into())),
    }
}

/// Read pressure for the whole machine from a `/proc/pressure` directory.
///
/// Takes the directory so tests can point it at fixtures. Production callers use
/// [`read_system_pressure`].
pub fn read_from_dir(dir: impl AsRef<Path>) -> Result<SystemPressure, PressureError> {
    let dir = dir.as_ref();
    if !dir.is_dir() {
        return Err(PressureError::Unsupported);
    }

    let read_one = |name: &str| -> Result<ResourcePressure, PressureError> {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(text) => parse_resource(&text),
            // A missing individual file on a directory that exists means a partial PSI
            // build. Zero pressure is the truthful reading: nothing is known to be
            // stalled on a resource the kernel does not account for.
            Err(_) => Ok(ResourcePressure::default()),
        }
    };

    Ok(SystemPressure {
        cpu: read_one("cpu")?,
        memory: read_one("memory")?,
        io: read_one("io")?,
    })
}

/// Read the machine's pressure from the kernel.
///
/// Returns [`PressureError::Unsupported`] anywhere PSI is absent — a non-Linux host, or
/// a kernel without `CONFIG_PSI`. The caller is expected to degrade to a cruder signal
/// and report that it did.
pub fn read_system_pressure() -> Result<SystemPressure, PressureError> {
    read_from_dir("/proc/pressure")
}

fn parse_f32(value: &str, line: &str) -> Result<f32, PressureError> {
    value
        .parse()
        .map_err(|_| PressureError::Malformed(line.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from a real Linux 6.x host.
    const IDLE_CPU: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
    const BUSY_CPU: &str = "some avg10=32.15 avg60=18.44 avg300=6.02 total=184729103\n";
    const THRASHING_MEMORY: &str = "some avg10=45.20 avg60=30.11 avg300=12.03 total=99120384\n\
                                    full avg10=28.40 avg60=15.77 avg300=5.91 total=52001923\n";

    #[test]
    fn parses_a_cpu_file_with_no_full_line() {
        // The normal case for /proc/pressure/cpu. Treating this as a parse failure would
        // make PSI unusable on the resource the scheduler cares most about.
        let parsed = parse_resource(IDLE_CPU).unwrap();
        assert_eq!(parsed.some.avg10, 0.0);
        assert_eq!(parsed.some.total_us, 0);
        assert!(parsed.full.is_none());
    }

    #[test]
    fn parses_both_lines_when_present() {
        let parsed = parse_resource(THRASHING_MEMORY).unwrap();
        assert_eq!(parsed.some.avg10, 45.20);
        assert_eq!(parsed.some.avg60, 30.11);
        assert_eq!(parsed.some.avg300, 12.03);
        assert_eq!(parsed.some.total_us, 99_120_384);
        let full = parsed.full.expect("memory pressure has a full line");
        assert_eq!(full.avg10, 28.40);
        assert_eq!(full.total_us, 52_001_923);
    }

    #[test]
    fn an_unknown_field_is_ignored_rather_than_fatal() {
        // A future kernel adding a column must not take the idle signal away.
        let text = "some avg10=1.00 avg60=2.00 avg300=3.00 total=4 avg900=5.00\n";
        let parsed = parse_resource(text).unwrap();
        assert_eq!(parsed.some.avg10, 1.0);
        assert_eq!(parsed.some.total_us, 4);
    }

    #[test]
    fn malformed_input_is_reported_not_silently_zeroed() {
        // Silently reading zero pressure from a corrupt file would tell the scheduler
        // the machine is idle, which is the most dangerous possible wrong answer.
        assert!(matches!(
            parse_resource("some avg10=not-a-number avg60=0 avg300=0 total=0\n"),
            Err(PressureError::Malformed(_))
        ));
        assert!(matches!(
            parse_resource("garbage line here\n"),
            Err(PressureError::Malformed(_))
        ));
        assert!(matches!(
            parse_resource(""),
            Err(PressureError::Malformed(_))
        ));
    }

    #[test]
    fn a_line_without_avg10_is_malformed() {
        assert!(matches!(
            parse_resource("some avg60=1.0 avg300=1.0 total=1\n"),
            Err(PressureError::Malformed(_))
        ));
    }

    // ───────────────────────────────────────────────────────────── classification

    fn pressure(cpu_some: f32, mem_some: f32, mem_full: f32, io_some: f32) -> SystemPressure {
        SystemPressure {
            cpu: ResourcePressure {
                some: PressureWindow {
                    avg10: cpu_some,
                    ..Default::default()
                },
                full: None,
            },
            memory: ResourcePressure {
                some: PressureWindow {
                    avg10: mem_some,
                    ..Default::default()
                },
                full: Some(PressureWindow {
                    avg10: mem_full,
                    ..Default::default()
                }),
            },
            io: ResourcePressure {
                some: PressureWindow {
                    avg10: io_some,
                    ..Default::default()
                },
                full: None,
            },
        }
    }

    #[test]
    fn a_quiet_machine_is_idle_and_welcomes_deep_idle_work() {
        let verdict = pressure(0.0, 0.0, 0.0, 0.0).verdict(&PressureThresholds::default());
        assert_eq!(verdict, PressureVerdict::Idle);
        assert!(verdict.permits_deep_idle());
        assert!(verdict.permits_background());
        assert!(!verdict.should_yield_idle_work());
        assert_eq!(verdict.display_budget(), "cinematic");
    }

    #[test]
    fn a_busy_cpu_yields_idle_work() {
        let verdict = pressure(32.0, 0.0, 0.0, 0.0).verdict(&PressureThresholds::default());
        assert_eq!(verdict, PressureVerdict::Contended);
        assert!(verdict.should_yield_idle_work());
        assert!(!verdict.permits_background());
        assert_eq!(verdict.display_budget(), "balanced");
    }

    #[test]
    fn a_cpu_bound_job_that_stalls_nothing_does_not_look_busy() {
        // THE case CPU percentage gets wrong. Eight cores pinned by a batch job with
        // nothing waiting on them is a responsive machine with capacity to schedule.
        let verdict = pressure(0.2, 0.0, 0.0, 0.0).verdict(&PressureThresholds::default());
        assert_eq!(verdict, PressureVerdict::Idle);
        assert!(
            verdict.permits_deep_idle(),
            "a machine where nothing is waiting has capacity, whatever the CPU meter says"
        );
    }

    #[test]
    fn a_thrashing_machine_is_saturated_even_at_low_cpu() {
        // The other case CPU percentage gets wrong: 20% CPU while memory thrashes is the
        // least responsive the machine will ever be.
        let verdict = pressure(2.0, 45.0, 28.0, 0.0).verdict(&PressureThresholds::default());
        assert_eq!(verdict, PressureVerdict::Saturated);
        assert!(verdict.should_yield_idle_work());
        assert_eq!(verdict.display_budget(), "low");
    }

    #[test]
    fn the_worst_resource_decides_rather_than_the_average() {
        // Averaging would hide exactly the resource that is the problem.
        let p = pressure(0.0, 0.0, 0.0, 60.0);
        let thresholds = PressureThresholds::default();
        assert_eq!(p.verdict(&thresholds), PressureVerdict::Contended);
        assert_eq!(p.worst_resource(&thresholds), "io");
    }

    #[test]
    fn deep_idle_is_stricter_about_starting_than_background() {
        // Deep-idle work is the most preemptable thing on the machine, so it should be
        // the most reluctant to start: an eviction costs a checkpoint.
        let light = pressure(5.0, 0.0, 0.0, 0.0).verdict(&PressureThresholds::default());
        assert_eq!(light, PressureVerdict::Light);
        assert!(!light.permits_deep_idle());
        assert!(light.permits_background());
        assert!(!light.should_yield_idle_work());
    }

    #[test]
    fn full_pressure_outranks_modest_some_pressure() {
        // A resource can show low `some` while every task on it is stalled, and that is
        // the worse condition.
        let p = pressure(0.0, 3.0, 15.0, 0.0);
        assert_eq!(
            p.verdict(&PressureThresholds::default()),
            PressureVerdict::Saturated
        );
    }

    #[test]
    fn thresholds_are_tunable() {
        let strict = PressureThresholds {
            light_some: 0.1,
            contended_some: 1.0,
            saturated_full: 1.0,
        };
        let p = pressure(2.0, 0.0, 0.0, 0.0);
        assert_eq!(p.verdict(&strict), PressureVerdict::Contended);
        assert_eq!(
            p.verdict(&PressureThresholds::default()),
            PressureVerdict::Light
        );
    }

    #[test]
    fn the_explanation_names_the_resource_and_the_numbers() {
        let p = pressure(0.0, 45.0, 28.0, 0.0);
        let explanation = p.explain(&PressureThresholds::default());
        assert!(explanation.contains("memory"), "{explanation}");
        assert!(explanation.contains("45.0"), "{explanation}");
        assert!(explanation.contains("saturated"), "{explanation}");
    }

    #[test]
    fn an_idle_machine_explains_itself_plainly() {
        let explanation = pressure(0.0, 0.0, 0.0, 0.0).explain(&PressureThresholds::default());
        assert!(explanation.starts_with("idle"));
    }

    // ─────────────────────────────────────────────────────────────────── reading

    #[test]
    fn a_missing_pressure_directory_is_unsupported_not_a_fault() {
        // Absent PSI means "fall back and say so", not "something broke".
        let missing = std::env::temp_dir().join("archie-no-psi-here-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert_eq!(read_from_dir(&missing), Err(PressureError::Unsupported));
    }

    #[test]
    fn reads_a_full_pressure_directory() {
        let dir = std::env::temp_dir().join(format!("archie-psi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cpu"), BUSY_CPU).unwrap();
        std::fs::write(dir.join("memory"), THRASHING_MEMORY).unwrap();
        std::fs::write(dir.join("io"), IDLE_CPU).unwrap();

        let pressure = read_from_dir(&dir).unwrap();
        assert_eq!(pressure.cpu.some.avg10, 32.15);
        assert!(pressure.cpu.full.is_none());
        assert_eq!(pressure.memory.full.unwrap().avg10, 28.40);
        assert_eq!(
            pressure.verdict(&PressureThresholds::default()),
            PressureVerdict::Saturated
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_partial_psi_build_reads_zero_for_the_missing_resource() {
        // A kernel accounting only some resources should still give a usable signal for
        // the ones it does account.
        let dir = std::env::temp_dir().join(format!("archie-psi-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cpu"), BUSY_CPU).unwrap();

        let pressure = read_from_dir(&dir).unwrap();
        assert_eq!(pressure.cpu.some.avg10, 32.15);
        assert_eq!(pressure.memory, ResourcePressure::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reading_the_real_kernel_never_panics() {
        // On Linux this exercises the real path; elsewhere it must report Unsupported.
        match read_system_pressure() {
            Ok(pressure) => {
                let _ = pressure.verdict(&PressureThresholds::default());
            }
            Err(PressureError::Unsupported) => {}
            Err(PressureError::Malformed(detail)) => {
                panic!("the kernel's own PSI format did not parse: {detail}")
            }
        }
    }

    #[test]
    fn pressure_round_trips_through_json() {
        let p = pressure(1.0, 2.0, 3.0, 4.0);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<SystemPressure>(&json).unwrap(), p);
    }
}
