//! The display snapshot.
//!
//! One typed aggregate, fetched atomically, so the renderer never draws a frame that
//! mixes a fresh module list with a stale topology.

use archie_core::boot::BootReport;
use archie_core::control::{
    CortexTrajectoryView, ModelView, ModuleView, PeerView, ProviderView, StatusView, TotalsView,
};
use archie_observability::MetricSnapshot;
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PROVIDERS: usize = 128;
pub const MAX_TRAJECTORIES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSource {
    Live,
    Fixture,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionState {
    Ready,
    Unavailable,
    Stale,
}

/// Manifest metadata only. Diagnostics and credentials never enter this DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSummary {
    pub id: String,
    pub title: String,
    pub capability: String,
    pub status: String,
    pub records: Vec<String>,
    pub known_gaps: Vec<String>,
}

impl From<ProviderView> for ProviderSummary {
    fn from(view: ProviderView) -> Self {
        Self {
            id: view.id,
            title: view.title,
            capability: view.capability,
            status: view.status,
            records: view.records,
            known_gaps: view.known_gaps,
        }
    }
}

/// Cached, bounded replay metadata; no context, references, endpoints or leases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorSnapshot {
    pub source: ProjectionSource,
    pub state: ProjectionState,
    pub reason: Option<String>,
    pub generated_at_ms: i64,
    pub providers: Vec<ProviderSummary>,
    pub cortex_trajectories: Vec<CortexTrajectoryView>,
}

impl InspectorSnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            source: ProjectionSource::Unavailable,
            state: ProjectionState::Unavailable,
            reason: Some(reason.into()),
            generated_at_ms: 0,
            providers: Vec::new(),
            cortex_trajectories: Vec::new(),
        }
    }

    pub fn is_bounded(&self) -> bool {
        self.providers.len() <= MAX_PROVIDERS && self.cortex_trajectories.len() <= MAX_TRAJECTORIES
    }
}

/// The observation lanes of the existing Orb, without its geometry or private content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSnapshot {
    pub source: ProjectionSource,
    pub state: ProjectionState,
    pub reason: Option<String>,
    pub generated_at_ms: i64,
    pub workflows: Vec<archie_orb::WorkflowView>,
}

impl WorkflowSnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            source: ProjectionSource::Unavailable,
            state: ProjectionState::Unavailable,
            reason: Some(reason.into()),
            generated_at_ms: 0,
            workflows: Vec::new(),
        }
    }
}

/// The catalogue already owned by Core, with its own reachability and freshness.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogSnapshot {
    pub source: SnapshotSource,
    pub generated_at: Timestamp,
    pub daemon_reachable: bool,
    pub staleness_ms: i64,
    pub error: Option<String>,
    pub modules: Vec<ModuleView>,
    pub models: Vec<ModelView>,
    pub nodes: Vec<PeerView>,
    pub capabilities: BTreeMap<String, Vec<String>>,
}

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
    /// Added to schema 1. A recording or older server can omit this projection.
    #[serde(default)]
    pub inspector: Option<InspectorSnapshot>,
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
            inspector: None,
        }
    }

    /// Mark an older snapshot as stale rather than presenting it as current.
    ///
    /// When `archied` goes away the bridge keeps serving what it last saw — an operator
    /// staring at a blank screen learns less than one staring at the last known good
    /// state — but it must never claim that data is current.
    pub fn as_stale(&self, error: impl Into<String>) -> Self {
        let now = Timestamp::now();
        let mut stale = Self {
            daemon_reachable: false,
            staleness_ms: self
                .staleness_ms
                .saturating_add(self.generated_at.elapsed_to(now).max(0)),
            error: Some(error.into()),
            generated_at: now,
            ..self.clone()
        };
        if stale.inspector.is_some() {
            stale.inspector = Some(stale.inspector_view());
        }
        stale
    }

    pub fn catalog(&self) -> CatalogSnapshot {
        CatalogSnapshot {
            source: self.source,
            generated_at: self.generated_at,
            daemon_reachable: self.daemon_reachable,
            staleness_ms: self.staleness_ms,
            error: self.error.clone(),
            modules: self.modules.clone(),
            models: self.models.clone(),
            nodes: self.nodes.clone(),
            capabilities: self.capabilities.clone(),
        }
    }

    pub fn inspector_view(&self) -> InspectorSnapshot {
        let Some(mut view) = self.inspector.clone() else {
            let mut missing = InspectorSnapshot::unavailable(if self.daemon_reachable {
                "inspector projection not provided by this server"
            } else {
                "archied unreachable; no inspector projection available"
            });
            if self.source == SnapshotSource::Fixture {
                missing.source = ProjectionSource::Fixture;
            }
            return missing;
        };
        view.source = match self.source {
            SnapshotSource::Live => ProjectionSource::Live,
            SnapshotSource::Fixture => ProjectionSource::Fixture,
        };
        if !self.daemon_reachable {
            view.state = ProjectionState::Stale;
            view.reason = Some("archied unreachable; showing the last inspector projection".into());
        }
        view
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
            inspector: None,
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
    fn older_snapshots_omit_inspector_without_claiming_it_is_empty_and_ready() {
        let mut old = serde_json::to_value(snapshot()).unwrap();
        old.as_object_mut().unwrap().remove("inspector");
        let decoded: DisplaySnapshot = serde_json::from_value(old).unwrap();
        assert!(decoded.inspector.is_none());
        assert_eq!(decoded.inspector_view().state, ProjectionState::Unavailable);
    }

    #[test]
    fn repeated_failures_keep_the_original_age() {
        let mut original = snapshot();
        original.generated_at = Timestamp::now().plus_millis(-5_000);
        let mut first = original.as_stale("down");
        first.generated_at = Timestamp::now().plus_millis(-3_000);
        assert!(first.as_stale("still down").staleness_ms >= 8_000);
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
