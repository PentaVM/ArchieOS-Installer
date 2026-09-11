//! The read-only bridge to `archied`.
//!
//! # The one rule this module enforces
//!
//! Everything reachable from the renderer passes through [`Bridge::snapshot`], and every
//! request it issues is checked against [`ControlRequest::is_mutating`] *before* it is
//! sent. Today that check can never fire, because no v0.1 request mutates. It exists so
//! that the day someone adds one, it cannot become reachable from a web page by
//! accident — it will trip an assertion in the bridge instead.

use crate::snapshot::{DisplaySnapshot, SnapshotSource};
use archie_core::control::{request_over, ControlRequest, ControlResponse};
use archie_core::endpoint;
use archie_state::StatePaths;
use archie_types::Timestamp;
use std::collections::BTreeMap;
use tracing::{debug, warn};

/// The read-only requests the observatory needs, in the order they are issued.
const SNAPSHOT_REQUESTS: &[fn() -> ControlRequest] = &[
    || ControlRequest::Status,
    || ControlRequest::Boot,
    || ControlRequest::Topology,
    || ControlRequest::Modules,
    || ControlRequest::Models,
    || ControlRequest::Metrics,
];

/// Talks to `archied` on behalf of the renderer.
#[derive(Debug, Clone)]
pub struct Bridge {
    paths: StatePaths,
}

impl Bridge {
    pub fn new(paths: StatePaths) -> Self {
        Self { paths }
    }

    /// Assemble a complete snapshot.
    ///
    /// Every request rides one connection, so the parts cannot come from two different
    /// daemon lifetimes. A failure at any point yields an `unreachable` snapshot rather
    /// than a partially-populated one that would render as a half-dead system.
    pub async fn snapshot(&self) -> DisplaySnapshot {
        for build in SNAPSHOT_REQUESTS {
            let request = build();
            // See the module docs: this guards a future change, not a present bug.
            assert!(
                !request.is_mutating(),
                "archie-displayd must never issue a mutating request; \
                 {} claims to mutate state",
                request.name()
            );
        }

        let mut stream = match endpoint::connect(&self.paths).await {
            Ok(stream) => stream,
            Err(e) => {
                debug!(error = %e, "archied not reachable");
                return DisplaySnapshot::unreachable(e.to_string());
            }
        };

        let mut snapshot = DisplaySnapshot {
            source: SnapshotSource::Live,
            generated_at: Timestamp::now(),
            daemon_reachable: true,
            staleness_ms: 0,
            error: None,
            status: None,
            boot: None,
            capabilities: BTreeMap::new(),
            nodes: Vec::new(),
            totals: Default::default(),
            modules: Vec::new(),
            load_order: Vec::new(),
            models: Vec::new(),
            metrics: None,
        };

        for build in SNAPSHOT_REQUESTS {
            let request = build();
            let name = request.name();
            match request_over(&mut stream, &request).await {
                Ok(response) => self.absorb(&mut snapshot, response, name),
                Err(e) => {
                    warn!(request = name, error = %e, "control request failed");
                    return DisplaySnapshot::unreachable(format!("{name}: {e}"));
                }
            }
        }

        snapshot
    }

    fn absorb(&self, snapshot: &mut DisplaySnapshot, response: ControlResponse, name: &str) {
        match response {
            ControlResponse::Status(status) => snapshot.status = Some(*status),
            ControlResponse::Boot(report) => snapshot.boot = Some(*report),
            ControlResponse::Topology {
                capabilities,
                nodes,
                totals,
            } => {
                snapshot.capabilities = capabilities;
                snapshot.nodes = nodes;
                snapshot.totals = totals;
            }
            ControlResponse::Modules {
                modules,
                load_order,
            } => {
                snapshot.modules = modules;
                snapshot.load_order = load_order;
            }
            ControlResponse::Models { models } => snapshot.models = models,
            ControlResponse::Metrics(metrics) => snapshot.metrics = Some(*metrics),
            ControlResponse::Error { message } => {
                warn!(request = name, error = %message, "daemon returned an error");
                snapshot.error = Some(format!("{name}: {message}"));
            }
            // A response of a kind we did not ask for means the stream desynchronised.
            // Recording it is more useful than silently dropping it.
            other => {
                warn!(request = name, "unexpected response kind");
                snapshot.error = Some(format!("{name}: unexpected response {other:?}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_snapshot_request_mutates_state() {
        // The property the whole isolation argument rests on.
        for build in SNAPSHOT_REQUESTS {
            let request = build();
            assert!(
                !request.is_mutating(),
                "{} would let the renderer change state",
                request.name()
            );
        }
    }

    #[test]
    fn the_snapshot_covers_every_view_the_observatory_renders() {
        let names: Vec<&str> = SNAPSHOT_REQUESTS.iter().map(|b| b().name()).collect();
        for expected in ["status", "boot", "topology", "modules", "models", "metrics"] {
            assert!(names.contains(&expected), "snapshot is missing {expected}");
        }
    }

    #[tokio::test]
    async fn an_absent_daemon_yields_an_unreachable_snapshot_not_a_hang() {
        let root = std::env::temp_dir().join(format!("archie-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = StatePaths::rooted(&root);
        paths.ensure_directories().unwrap();

        let snapshot = Bridge::new(paths).snapshot().await;
        assert!(!snapshot.daemon_reachable);
        assert!(snapshot.error.is_some());
        assert_eq!(snapshot.headline(), "ARCHIED UNREACHABLE");
        // Nothing fabricated.
        assert!(snapshot.status.is_none());
        assert!(snapshot.modules.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
