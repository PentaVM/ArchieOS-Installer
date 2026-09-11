//! The display snapshot.
//!
//! One typed aggregate, fetched atomically, so the renderer never draws a frame that
//! mixes a fresh module list with a stale topology.

use archie_core::boot::BootReport;
use archie_core::control::{ModelView, ModuleView, PeerView, StatusView, TotalsView};
use archie_observability::MetricSnapshot;
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where the data in a snapshot came from.
///
/// This field is the reason the renderer can be trusted. A recorded fixture is a
/// legitimate development tool; a fixture that *looks* live is a lie. The renderer is
/// required to display this distinction, and the type has no default so it cannot be
/// omitted by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    /// Read from a running `archied` just now.
    Live,
    /// Replayed from a recording. The UI must say so.
    Fixture,
}

impl SnapshotSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Fixture => "fixture",
        }
    }

    /// Whether this data reflects the current state of a real system.
    pub fn is_authoritative(&self) -> bool {
        matches!(self, Self::Live)
    }
}

/// The complete state the observatory renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplaySnapshot {
    pub source: SnapshotSource,
    /// When this snapshot was assembled.
    pub generated_at: Timestamp,
    /// Whether `archied` answered. False means everything below is stale.
    pub daemon_reachable: bool,
    /// How old the underlying data is, in milliseconds. Zero for a fresh live read.
    pub staleness_ms: i64,
    /// Present when the last attempt to reach the daemon failed.
    pub error: Option<String>,

    pub status: Option<StatusView>,
    pub boot: Option<BootReport>,
    pub capabilities: BTreeMap<String, Vec<String>>,
    pub nodes: Vec<PeerView>,
    pub totals: TotalsView,
    pub modules: Vec<ModuleView>,
    pub load_order: Vec<String>,
    pub models: Vec<ModelView>,
    pub metrics: Option<MetricSnapshot>,
}

impl DisplaySnapshot {
    /// An empty snapshot describing a daemon that could not be reached.
    ///
    /// Deliberately not `Default`: constructing one of these is always a statement that
    /// something is wrong, and it should read that way at the call site.
    pub fn unreachable(error: impl Into<String>) -> Self {
        Self {
            source: SnapshotSource::Live,
            generated_at: Timestamp::now(),
            daemon_reachable: false,
            staleness_ms: 0,
            error: Some(error.into()),
            status: None,
            boot: None,
            capabilities: BTreeMap::new(),
            nodes: Vec::new(),
            totals: TotalsView::default(),
            modules: Vec::new(),
            load_order: Vec::new(),
            models: Vec::new(),
            metrics: None,
        }
    }

    /// Mark an older snapshot as stale rather than presenting it as current.
    ///
    /// When `archied` goes away the bridge keeps serving what it last saw — an operator
    /// staring at a blank screen learns less than one staring at the last known good
    /// state — but it must never claim that data is current.
    pub fn as_stale(&self, error: impl Into<String>) -> Self {
        let now = Timestamp::now();
        Self {
            daemon_reachable: false,
            staleness_ms: self.generated_at.elapsed_to(now).max(0),
            error: Some(error.into()),
            generated_at: now,
            ..self.clone()
        }
    }

    /// Modules that resolved and will run.
    pub fn loaded_modules(&self) -> Vec<&ModuleView> {
        self.modules
            .iter()
            .filter(|m| m.state == "loaded")
            .collect()
    }

    /// Modules that are installed but did not load, each with its reason.
    ///
    /// The renderer draws these dimmed rather than hiding them: a screen showing only
    /// healthy components looks serene while the system is broken.
    pub fn excluded_modules(&self) -> Vec<&ModuleView> {
        self.modules
            .iter()
            .filter(|m| m.state != "loaded")
            .collect()
    }

    /// Whether the node reached ARCHIE READY.
    pub fn is_ready(&self) -> bool {
        self.boot.as_ref().is_some_and(|b| b.is_ready())
    }

    /// A one-line health verdict for the renderer's headline.
    pub fn headline(&self) -> String {
        if !self.daemon_reachable {
            return "ARCHIED UNREACHABLE".to_string();
        }
        if self.is_ready() {
            "ARCHIE READY".to_string()
        } else {
            "ARCHIE NOT READY".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> DisplaySnapshot {
        DisplaySnapshot {
            source: SnapshotSource::Live,
            generated_at: Timestamp::now(),
            daemon_reachable: true,
            staleness_ms: 0,
            error: None,
            status: None,
            boot: None,
            capabilities: BTreeMap::new(),
            nodes: Vec::new(),
            totals: TotalsView::default(),
            modules: vec![
                ModuleView {
                    id: "core.archie".into(),
                    version: "0.1.0".into(),
                    state: "loaded".into(),
                    explanation: "loaded".into(),
                    dependencies: vec![],
                },
                ModuleView {
                    id: "engineering.cad".into(),
                    version: "0.1.0".into(),
                    state: "unsatisfied".into(),
                    explanation: "node does not provide: cad.parametric".into(),
                    dependencies: vec!["core.nova".into()],
                },
            ],
            load_order: vec!["core.archie".into()],
            models: Vec::new(),
            metrics: None,
        }
    }

    #[test]
    fn the_source_marker_distinguishes_live_from_fixture() {
        assert!(SnapshotSource::Live.is_authoritative());
        assert!(!SnapshotSource::Fixture.is_authoritative());
        assert_eq!(SnapshotSource::Fixture.as_str(), "fixture");
    }

    #[test]
    fn the_source_field_has_no_default_and_survives_serialization() {
        // A missing `source` must be a decode error, not a silent default to "live".
        let json = r#"{
            "generated_at": 0, "daemon_reachable": true, "staleness_ms": 0,
            "error": null, "status": null, "boot": null, "capabilities": {},
            "nodes": [], "totals": {"nodes":0,"schedulable_nodes":0,"logical_cores":0,
            "memory_bytes":0,"accelerator_bytes":0,"accelerators":0},
            "modules": [], "load_order": [], "models": [], "metrics": null
        }"#;
        assert!(
            serde_json::from_str::<DisplaySnapshot>(json).is_err(),
            "a snapshot without a source marker must be rejected"
        );
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = snapshot();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<DisplaySnapshot>(&json).unwrap(), s);
    }

    #[test]
    fn excluded_modules_are_kept_not_hidden() {
        let s = snapshot();
        assert_eq!(s.loaded_modules().len(), 1);
        assert_eq!(s.excluded_modules().len(), 1);
        assert!(s.excluded_modules()[0]
            .explanation
            .contains("cad.parametric"));
    }

    #[test]
    fn an_unreachable_daemon_is_stated_plainly() {
        let s = DisplaySnapshot::unreachable("connection refused");
        assert!(!s.daemon_reachable);
        assert_eq!(s.headline(), "ARCHIED UNREACHABLE");
        assert!(s.error.as_deref().unwrap().contains("connection refused"));
    }

    #[test]
    fn stale_data_is_marked_stale_rather_than_presented_as_current() {
        let mut original = snapshot();
        original.generated_at = Timestamp::now().plus_millis(-5_000);

        let stale = original.as_stale("daemon restarted");
        assert!(!stale.daemon_reachable);
        assert!(
            stale.staleness_ms >= 5_000,
            "staleness must reflect how old the data really is, got {}",
            stale.staleness_ms
        );
        // The content is preserved so the operator still sees the last known state.
        assert_eq!(stale.modules.len(), original.modules.len());
        assert_eq!(stale.headline(), "ARCHIED UNREACHABLE");
    }

    #[test]
    fn readiness_comes_from_the_boot_report_not_from_optimism() {
        let mut s = snapshot();
        assert!(!s.is_ready(), "no boot report means not ready");

        let mut report = BootReport::new();
        report.push(archie_core::boot::BootStage::ready("Node identity", true));
        s.boot = Some(report);
        assert!(s.is_ready());
        assert_eq!(s.headline(), "ARCHIE READY");

        let mut failed = BootReport::new();
        failed.push(archie_core::boot::BootStage::failed(
            "State store",
            true,
            "disk full",
        ));
        s.boot = Some(failed);
        assert!(!s.is_ready());
        assert_eq!(s.headline(), "ARCHIE NOT READY");
    }
}
