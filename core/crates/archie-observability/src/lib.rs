//! # archie-observability
//!
//! Logging, metrics and the audit trail.
//!
//! The rule this crate exists to enforce: **execution must be observable rather than
//! magical**. If Archie made a decision, there is a record of what it decided and why.
//!
//! Three distinct streams, deliberately not merged:
//!
//! | Stream | Question it answers | Retention |
//! |---|---|---|
//! | Logs | What happened? | Short. Rotated by journald. |
//! | Metrics | How much / how fast? | Aggregated. |
//! | Audit | Who authorised this, and on what grounds? | Long. Append-only. |
//!
//! A fourth source sits alongside them: [`kernel`] reads what the kernel already knows
//! about the machine — pressure and thermal state — instead of inferring it from
//! userspace approximations.
//!
//! Audit is separate from logs on purpose. A log line can be dropped under pressure;
//! an audit record about granting device actuation cannot.

#![forbid(unsafe_code)]

pub mod audit;
pub mod cognition;
pub mod kernel;
pub mod metrics;

pub use audit::{AuditError, AuditEvent, AuditOutcome, AuditSink, InMemoryAuditSink};
pub use cognition::{
    CognitiveTrace, HudProjection, Stage, StageCost, StageOutcome, StageRecord, Subject, TraceError,
};
pub use kernel::{
    read_system_pressure, read_thermal, PressureThresholds, PressureVerdict, SystemPressure,
    ThermalReport, ThermalState,
};
pub use metrics::{MetricSnapshot, Metrics};

use std::sync::atomic::{AtomicBool, Ordering};
use tracing_subscriber::filter::EnvFilter;

static LOGGING_INITIALISED: AtomicBool = AtomicBool::new(false);

/// How log output should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// One JSON object per line. What `archied` uses under systemd.
    Json,
    /// Human-readable. For interactive CLI use.
    Text,
}

/// Initialise process-wide logging.
///
/// Safe to call more than once: subsequent calls are no-ops rather than panics, because
/// a double-init should not be able to take the daemon down.
///
/// Honours `ARCHIE_LOG` (falling back to `RUST_LOG`, then `default_level`).
pub fn init_logging(format: LogFormat, default_level: &str) -> bool {
    if LOGGING_INITIALISED.swap(true, Ordering::SeqCst) {
        return false;
    }

    let filter = EnvFilter::try_from_env("ARCHIE_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    let result = match format {
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_current_span(true)
            .with_span_list(false)
            .try_init(),
        LogFormat::Text => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .try_init(),
    };

    result.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_init_is_not_fatal() {
        // The second call must return false rather than panic.
        let _ = init_logging(LogFormat::Text, "info");
        assert!(!init_logging(LogFormat::Text, "info"));
    }
}
