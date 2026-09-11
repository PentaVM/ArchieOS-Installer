//! Kernel telemetry read directly from `/proc` and `/sys`.
//!
//! ArchieOS is an operating system, not an application that runs on one. These modules
//! are where it stops inferring the machine's state from userspace approximations and
//! reads what the kernel already knows.
//!
//! * [`psi`] — Pressure Stall Information. The correct answer to "is this machine
//!   busy", replacing a CPU-percentage heuristic that is wrong in both directions.
//! * [`thermal`] — thermal zones classified against the trip points the firmware
//!   itself declared, so a node stops accepting work before it is throttled.
//! * [`cgroup`] — cgroup v2, where a resource budget stops being a request the process
//!   may ignore and becomes a limit the kernel enforces.
//! * [`udev`] — hardware that arrives after boot, so a node never has to be restarted
//!   to notice its own GPU.
//!
//! # A rule for everything in here
//!
//! The parsing and the judgement are pure functions over text; the only I/O is reading a
//! file. That split is deliberate. Every format below is stable, documented kernel ABI,
//! so the logic can be exercised anywhere against captured fixtures, and what remains
//! untestable off Linux is a `read_to_string` call rather than any decision.
//!
//! # Absence is not failure
//!
//! A kernel without `CONFIG_PSI`, a machine with no thermal driver, or a non-Linux
//! development host must degrade to a cruder signal and *say so*. None of these
//! functions fabricates a reading, because a fabricated reading is one the scheduler
//! will act on.

pub mod cgroup;
pub mod psi;
pub mod thermal;
pub mod udev;

pub use cgroup::{
    CgroupController, CgroupError, CgroupPath, CgroupState, CpuLimit, FreezeState, MemoryLimit,
};
pub use psi::{
    read_system_pressure, PressureError, PressureThresholds, PressureVerdict, ResourcePressure,
    SystemPressure,
};
pub use thermal::{read_thermal, ThermalReport, ThermalState, ThermalZone, TripKind, TripPoint};
pub use udev::{CapabilityClass, HotplugDebouncer, Uevent, UeventAction, UeventError};
