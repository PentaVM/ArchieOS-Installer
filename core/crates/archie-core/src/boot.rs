//! The canonical boot sequence.
//!
//! ```text
//! UEFI → kernel → systemd → archied
//!   1. load node identity
//!   2. probe local hardware
//!   3. build the capability manifest
//!   4. initialise local state
//!   5. start Archie Fabric
//!   6. discover peers
//!   7. authenticate them and exchange manifests
//!   8. construct the resource topology
//!   9. discover installed modules
//!  10. resolve module requirements
//!  11. construct the cognitive topology
//!  12. register model providers
//!  13. start eligible workers
//!  → ARCHIE READY
//! ```
//!
//! # Degraded is not failed
//!
//! Each stage reports [`StageStatus`]. A node with no peers, no models and no modules
//! still reaches READY — it is a valid, if quiet, Archie. Only a stage that makes the
//! node unable to function at all (no identity, no state store) prevents readiness.
//! Conflating "nothing to do" with "broken" would make an ordinary single-node install
//! look like a failure every boot.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Outcome of one boot stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StageStatus {
    Ready,
    /// Working, but with something worth telling the operator.
    Degraded {
        detail: String,
    },
    /// Did not run, and did not need to.
    Skipped {
        detail: String,
    },
    /// Could not complete. Fatal only if the stage is required.
    Failed {
        detail: String,
    },
}

impl StageStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Degraded { .. } => "DEGRADED",
            Self::Skipped { .. } => "SKIPPED",
            Self::Failed { .. } => "FAILED",
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::Degraded { detail } | Self::Skipped { detail } | Self::Failed { detail } => {
                Some(detail)
            }
        }
    }
}

/// One step of the boot sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootStage {
    pub name: String,
    pub status: StageStatus,
    /// Whether failing this stage prevents the node from becoming ready.
    pub required: bool,
}

impl BootStage {
    pub fn ready(name: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            status: StageStatus::Ready,
            required,
        }
    }

    pub fn degraded(name: impl Into<String>, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: StageStatus::Degraded {
                detail: detail.into(),
            },
            required,
        }
    }

    pub fn skipped(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: StageStatus::Skipped {
                detail: detail.into(),
            },
            required: false,
        }
    }

    pub fn failed(name: impl Into<String>, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: StageStatus::Failed {
                detail: detail.into(),
            },
            required,
        }
    }

    /// Whether this stage stops the node from reaching READY.
    pub fn is_fatal(&self) -> bool {
        self.required && self.status.is_failure()
    }
}

/// The result of a boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BootReport {
    pub stages: Vec<BootStage>,
    /// Non-fatal observations: unknown hardware fields, missing optional components.
    pub warnings: Vec<String>,
}

impl BootReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, stage: BootStage) {
        self.stages.push(stage);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// Whether the node reached ARCHIE READY.
    pub fn is_ready(&self) -> bool {
        !self.stages.iter().any(BootStage::is_fatal)
    }

    /// Stages that failed but were not required.
    pub fn non_fatal_failures(&self) -> Vec<&BootStage> {
        self.stages
            .iter()
            .filter(|s| s.status.is_failure() && !s.required)
            .collect()
    }

    pub fn fatal_failures(&self) -> Vec<&BootStage> {
        self.stages.iter().filter(|s| s.is_fatal()).collect()
    }

    /// Render the operator-facing boot summary.
    ///
    /// Deliberately plain: aligned dots and one word per line. The specification asked
    /// not to waste effort on terminal art, and a boot report is read when something is
    /// wrong, where legibility beats decoration.
    pub fn render(&self) -> String {
        let mut out = String::from("ARCHIE OS\n");
        let width = self
            .stages
            .iter()
            .map(|s| s.name.len())
            .max()
            .unwrap_or(20)
            .max(20);

        for stage in &self.stages {
            let dots = ".".repeat(width.saturating_sub(stage.name.len()) + 4);
            out.push_str(&format!(
                "{} {} {}\n",
                stage.name,
                dots,
                stage.status.label()
            ));
            if let Some(detail) = stage.status.detail() {
                out.push_str(&format!("{:width$}      {}\n", "", detail, width = width));
            }
        }

        if !self.warnings.is_empty() {
            out.push('\n');
            for warning in &self.warnings {
                out.push_str(&format!("warning: {warning}\n"));
            }
        }

        out.push('\n');
        if self.is_ready() {
            out.push_str("ARCHIE READY\n");
        } else {
            out.push_str("ARCHIE NOT READY\n");
            for stage in self.fatal_failures() {
                out.push_str(&format!(
                    "  {} failed: {}\n",
                    stage.name,
                    stage.status.detail().unwrap_or("no detail")
                ));
            }
        }
        out
    }
}

impl fmt::Display for BootReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// The canonical stage names, so the daemon, the CLI and the docs agree.
pub mod stages {
    pub const IDENTITY: &str = "Node identity";
    pub const HARDWARE: &str = "Hardware probe";
    pub const CAPABILITIES: &str = "Capability manifest";
    pub const STATE: &str = "State store";
    pub const FABRIC: &str = "Fabric";
    pub const MEMBERSHIP: &str = "Fabric membership";
    pub const PEERS: &str = "Peers";
    pub const TOPOLOGY: &str = "Resource topology";
    pub const MODULES: &str = "Modules";
    pub const PROVIDERS: &str = "Data providers";
    pub const COGNITIVE: &str = "Cognitive topology";
    pub const MODELS: &str = "Models";
    pub const WORKERS: &str = "Workers";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_all_ready_boot_is_ready() {
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::IDENTITY, true));
        report.push(BootStage::ready(stages::STATE, true));
        assert!(report.is_ready());
        assert!(report.render().contains("ARCHIE READY"));
    }

    #[test]
    fn a_quiet_single_node_still_reaches_ready() {
        // No peers, no models, no modules. This is the ordinary state of a first
        // install and must not look like a failure.
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::IDENTITY, true));
        report.push(BootStage::ready(stages::HARDWARE, true));
        report.push(BootStage::ready(stages::STATE, true));
        report.push(BootStage::degraded(
            stages::FABRIC,
            false,
            "loopback transport; this node discovers nobody",
        ));
        report.push(BootStage::skipped(stages::PEERS, "no peers discovered"));
        report.push(BootStage::skipped(stages::MODELS, "no models registered"));
        report.push(BootStage::skipped(stages::MODULES, "no modules installed"));

        assert!(report.is_ready());
        let rendered = report.render();
        assert!(rendered.contains("ARCHIE READY"));
        assert!(rendered.contains("SKIPPED"));
    }

    #[test]
    fn a_required_stage_failing_blocks_readiness() {
        let mut report = BootReport::new();
        report.push(BootStage::failed(
            stages::IDENTITY,
            true,
            "identity key is unreadable",
        ));
        assert!(!report.is_ready());

        let rendered = report.render();
        assert!(rendered.contains("ARCHIE NOT READY"));
        assert!(rendered.contains("identity key is unreadable"));
    }

    #[test]
    fn an_optional_stage_failing_does_not_block_readiness() {
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::IDENTITY, true));
        report.push(BootStage::failed(
            stages::FABRIC,
            false,
            "could not bind UDP socket",
        ));
        assert!(
            report.is_ready(),
            "a node with no network is isolated, not broken"
        );
        assert_eq!(report.non_fatal_failures().len(), 1);
    }

    #[test]
    fn the_report_names_every_fatal_stage() {
        let mut report = BootReport::new();
        report.push(BootStage::failed(stages::IDENTITY, true, "no key"));
        report.push(BootStage::failed(stages::STATE, true, "disk full"));
        assert_eq!(report.fatal_failures().len(), 2);
        let rendered = report.render();
        assert!(rendered.contains("no key"));
        assert!(rendered.contains("disk full"));
    }

    #[test]
    fn warnings_are_shown_without_affecting_readiness() {
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::HARDWARE, true));
        report.warn("physical core count unavailable; assuming logical");
        assert!(report.is_ready());
        assert!(report.render().contains("warning:"));
    }

    #[test]
    fn stage_details_appear_in_the_render() {
        let mut report = BootReport::new();
        report.push(BootStage::degraded(
            stages::PEERS,
            false,
            "3 peers seen, 1 unverified",
        ));
        assert!(report.render().contains("3 peers seen, 1 unverified"));
    }

    #[test]
    fn the_render_is_stable_and_aligned() {
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::IDENTITY, true));
        report.push(BootStage::ready(stages::COGNITIVE, true));
        let rendered = report.render();

        // Every status label must line up in the same column.
        let columns: Vec<usize> = rendered
            .lines()
            .filter(|l| l.contains("READY") && l.contains('.'))
            .map(|l| l.find("READY").unwrap())
            .collect();
        assert!(columns.windows(2).all(|w| w[0] == w[1]), "{rendered}");
    }

    #[test]
    fn report_round_trips_through_json() {
        let mut report = BootReport::new();
        report.push(BootStage::ready(stages::IDENTITY, true));
        report.push(BootStage::degraded(stages::FABRIC, false, "loopback"));
        report.warn("something to note");
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(serde_json::from_str::<BootReport>(&json).unwrap(), report);
    }
}
