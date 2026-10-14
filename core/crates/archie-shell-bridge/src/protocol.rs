//! The stream the shell reads: one `Frame` per poll, as one JSON line.
//!
//! Every frame states how it knows what it says (`link`). A shell that draws "connected"
//! must be able to prove it from a frame, and a shell that cannot reach the node must be
//! told so rather than left with the last picture.

use crate::client::{Client, ClientError};
use crate::jobs::{self, Jobs};
use crate::lan::{self, Lan};
use archie_displayd::{
    CatalogSnapshot, DisplaySnapshot, InspectorSnapshot, KernelSnapshot, ProjectionSource,
    SnapshotSource, WorkflowSnapshot,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Version of this wire format. Bumped on any incompatible change.
pub const SCHEMA: u32 = 1;

/// How the bridge knows what the frame says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Link {
    /// `archie-displayd` answered and `archied` is reachable through it, source `live`.
    Live,
    /// `archie-displayd` is serving a recording. Never to be drawn as live.
    Fixture,
    /// `archie-displayd` answered, `archied` did not. Whatever `system` holds is the last
    /// state seen, `stale_ms` old.
    DaemonUnreachable { stale_ms: i64, reason: String },
    /// `archie-displayd` itself could not be reached.
    DisplaydOffline { reason: String },
    /// Something answered but not in a shape this schema understands.
    Invalid { reason: String },
}

/// The few facts of `archied`'s status the shell shows, copied from the real `StatusView`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct System {
    pub hostname: String,
    pub archieos_version: String,
    pub health: String,
    pub modules_loaded: usize,
    pub modules_installed: usize,
    pub models_registered: usize,
    pub active_tasks: u64,
    pub peers: usize,
    pub reachable_peers: usize,
    /// `live` or `fixture`, as `archie-displayd` states it.
    pub source: &'static str,
}

/// One poll.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Frame {
    pub schema: u32,
    pub at_ms: i64,
    pub link: Link,
    /// Present when `archied`'s status is known (possibly stale; see `link`).
    pub system: Option<System>,
    /// Machine telemetry, read by `archie-displayd` locally. Independent of `archied`: it
    /// is still true when the daemon is down. Each source carries its own availability, so
    /// "not measured" can be told from "idle".
    pub kernel: Option<KernelSnapshot>,
    /// The LAN inventory file `archie-lan` keeps, read passively (no scan is started).
    pub lan: Lan,
    /// Long bridge jobs, read from the given workspaces' `.archie/jobs`. Read-only.
    pub jobs: Jobs,
    pub catalog: Option<CatalogSnapshot>,
    pub inspector: InspectorSnapshot,
    pub workflows: WorkflowSnapshot,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn link_of(snapshot: &DisplaySnapshot) -> Link {
    // A recording is a recording even when it claims the daemon was reachable.
    if snapshot.source == SnapshotSource::Fixture {
        return Link::Fixture;
    }
    if !snapshot.daemon_reachable {
        return Link::DaemonUnreachable {
            stale_ms: snapshot.staleness_ms,
            reason: snapshot
                .error
                .clone()
                .unwrap_or_else(|| "archied did not answer".to_string()),
        };
    }
    Link::Live
}

fn system_of(snapshot: &DisplaySnapshot) -> Option<System> {
    let status = snapshot.status.as_ref()?;
    Some(System {
        hostname: status.hostname.clone(),
        archieos_version: status.archieos_version.clone(),
        health: status.health.clone(),
        modules_loaded: status.modules_loaded,
        modules_installed: status.modules_installed,
        models_registered: status.models_registered,
        active_tasks: status.active_tasks,
        peers: status.peers,
        reachable_peers: status.reachable_peers,
        source: snapshot.source.as_str(),
    })
}

/// Poll `archie-displayd` once.
///
/// Never panics and never returns an error: every failure is a `Frame` whose `link` says
/// what failed, because the shell must be able to draw that.
pub fn poll(client: &Client) -> Frame {
    poll_with(client, &Sources::default())
}

/// Where the bridge reads files from, besides `archie-displayd`. Both are optional.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub lan_file: Option<PathBuf>,
    pub workspaces: Vec<PathBuf>,
    /// A folder whose subfolders are workspaces (e.g. `/archie/workspaces`), listed each poll.
    pub workspaces_root: Option<PathBuf>,
}

/// Like [`poll`], also reading the optional file sources.
pub fn poll_with(client: &Client, sources: &Sources) -> Frame {
    let at_ms = now_ms();
    let lan = lan::read(
        sources.lan_file.as_deref().map(Path::new),
        u64::try_from(at_ms / 1000).unwrap_or(0),
    );
    let jobs = jobs::read_sources(
        &sources.workspaces,
        sources.workspaces_root.as_deref(),
        u64::try_from(at_ms).unwrap_or(0),
    );
    let offline = |reason: String| Frame {
        schema: SCHEMA,
        at_ms,
        link: Link::DisplaydOffline { reason },
        system: None,
        kernel: None,
        lan: lan.clone(),
        jobs: jobs.clone(),
        catalog: None,
        inspector: InspectorSnapshot::unavailable("displayd offline"),
        workflows: WorkflowSnapshot::unavailable("displayd offline"),
    };

    let body = match client.get("/api/snapshot") {
        Ok(body) => body,
        Err(e @ (ClientError::Connect(_) | ClientError::Io(_))) => return offline(e.to_string()),
        Err(e) => {
            return Frame {
                schema: SCHEMA,
                at_ms,
                link: Link::Invalid {
                    reason: e.to_string(),
                },
                system: None,
                kernel: None,
                lan: lan.clone(),
                jobs: jobs.clone(),
                catalog: None,
                inspector: InspectorSnapshot::unavailable("displayd response invalid"),
                workflows: WorkflowSnapshot::unavailable("displayd response invalid"),
            }
        }
    };
    let snapshot: DisplaySnapshot = match serde_json::from_slice(&body) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            return Frame {
                schema: SCHEMA,
                at_ms,
                link: Link::Invalid {
                    reason: format!("snapshot did not match the schema: {e}"),
                },
                system: None,
                kernel: None,
                lan: lan.clone(),
                jobs: jobs.clone(),
                catalog: None,
                inspector: InspectorSnapshot::unavailable("displayd snapshot invalid"),
                workflows: WorkflowSnapshot::unavailable("displayd snapshot invalid"),
            }
        }
    };

    // Telemetry is best-effort: its failure must not hide a good snapshot, and it is left
    // absent rather than invented.
    let kernel = client
        .get("/api/kernel")
        .ok()
        .and_then(|body| serde_json::from_slice::<KernelSnapshot>(&body).ok());

    // A lightweight read of the existing Orb lanes, independently of archied.
    // Older displayd versions have no route: absence is explicit, never fabricated.
    let mut workflows = client
        .get("/api/workflows")
        .ok()
        .and_then(|body| serde_json::from_slice::<WorkflowSnapshot>(&body).ok())
        .filter(|view| view.workflows.len() <= 4)
        .unwrap_or_else(|| WorkflowSnapshot::unavailable("workflow projection unavailable"));
    if snapshot.source == SnapshotSource::Fixture {
        workflows.source = ProjectionSource::Fixture;
    }
    let inspector = snapshot.inspector_view();
    let inspector = if inspector.is_bounded() {
        inspector
    } else {
        InspectorSnapshot::unavailable("inspector projection exceeds its bounds")
    };

    Frame {
        schema: SCHEMA,
        at_ms,
        link: link_of(&snapshot),
        system: system_of(&snapshot),
        kernel,
        lan,
        jobs,
        catalog: Some(snapshot.catalog()),
        inspector,
        workflows,
    }
}
