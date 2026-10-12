//! # archie-displayd
//!
//! The unprivileged read-only bridge between `archied` and the Archie Display.
//!
//! See `docs/RFC-006-DISPLAY-ISOLATION.md` for the reasoning. In short: the renderer
//! must be beautiful and must be incapable of harming the control plane, and those two
//! requirements are met by putting this process between them.
//!
//! ```text
//! archied ──unix socket──▶ archie-displayd ──loopback http──▶ renderer
//!  (trusted)                (read-only)                        (untrusted)
//! ```

#![forbid(unsafe_code)]

pub mod bridge;
pub mod geofabric;
pub mod http;
pub mod kernel;
pub mod orb;
pub mod runtime;
pub mod snapshot;
pub mod world;

pub use bridge::Bridge;
pub use geofabric::GeoFabricSnapshot;
pub use http::{Request, Response};
pub use kernel::{Availability, KernelService, KernelSnapshot, ZoneView};
pub use orb::OrbService;
pub use snapshot::{DisplaySnapshot, SnapshotSource};
pub use world::{LanDeviceEntity, LanSource, WorldDataSource, WorldService, WorldSnapshot};

use archie_state::StatePaths;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// What the display serves, and where it gets it.
pub struct Observatory {
    bridge: Bridge,
    /// Last successful read, kept so an operator sees the last known good state rather
    /// than a blank screen when the daemon restarts.
    cache: Arc<RwLock<Option<DisplaySnapshot>>>,
    /// When set, every response is this recording, marked as a fixture.
    fixture: Option<DisplaySnapshot>,
    world: WorldService,
    /// Machine telemetry, read locally. Deliberately not part of the daemon snapshot:
    /// when `archied` is unreachable this is still true, and that is exactly when an
    /// operator most wants to know whether the machine is thrashing or overheating.
    kernel: KernelService,
    orb: OrbService,
}

impl Observatory {
    pub fn live(paths: StatePaths) -> Self {
        Self {
            bridge: Bridge::new(paths),
            cache: Arc::new(RwLock::new(None)),
            fixture: None,
            world: WorldService::live(),
            kernel: KernelService::default(),
            orb: OrbService::default(),
        }
    }

    /// Run the local observatory without connecting to the isolated world adapter.
    pub fn live_without_world(paths: StatePaths) -> Self {
        Self {
            bridge: Bridge::new(paths),
            cache: Arc::new(RwLock::new(None)),
            fixture: None,
            world: WorldService::disabled(WorldDataSource::Disabled),
            kernel: KernelService::default(),
            orb: OrbService::default(),
        }
    }

    /// Serve a recording instead of a live daemon.
    ///
    /// The snapshot's source is forced to [`SnapshotSource::Fixture`] regardless of what
    /// the recording claims, so a captured-live file replayed here can never present
    /// itself as current.
    pub fn from_fixture(mut snapshot: DisplaySnapshot) -> Self {
        snapshot.source = SnapshotSource::Fixture;
        Self {
            bridge: Bridge::new(StatePaths::system()),
            cache: Arc::new(RwLock::new(None)),
            fixture: Some(snapshot),
            world: WorldService::disabled(WorldDataSource::Fixture),
            kernel: KernelService::default(),
            orb: OrbService::default(),
        }
    }

    /// Load a fixture from disk.
    pub fn from_fixture_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let snapshot: DisplaySnapshot = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self::from_fixture(snapshot))
    }

    pub fn is_fixture(&self) -> bool {
        self.fixture.is_some()
    }

    /// Poll `archied` and update the cache.
    ///
    /// On failure the previous snapshot is kept but re-marked stale. It is never
    /// re-served as if it were current — see [`DisplaySnapshot::as_stale`].
    pub async fn refresh(&self) {
        if self.fixture.is_some() {
            return;
        }
        let fresh = self.bridge.snapshot().await;
        let mut cache = self.cache.write().await;

        if fresh.daemon_reachable {
            self.orb.observe_snapshot(&fresh).await;
            *cache = Some(fresh);
            return;
        }

        let error = fresh
            .error
            .clone()
            .unwrap_or_else(|| "archied unreachable".to_string());
        match cache.as_ref() {
            Some(previous) if previous.daemon_reachable => {
                debug!("archied went away; marking the last snapshot stale");
                *cache = Some(previous.as_stale(error));
            }
            Some(previous) => {
                // Already stale; keep ageing the original rather than resetting the clock.
                *cache = Some(previous.as_stale(error));
            }
            None => *cache = Some(fresh),
        }
    }

    /// The snapshot to serve right now.
    pub async fn current(&self) -> DisplaySnapshot {
        if let Some(fixture) = &self.fixture {
            return fixture.clone();
        }
        match self.cache.read().await.as_ref() {
            Some(snapshot) => snapshot.clone(),
            None => DisplaySnapshot::unreachable("no snapshot taken yet"),
        }
    }

    /// Poll the fixed loopback world adapter independently of `archied`.
    pub async fn refresh_world(&self) {
        self.world.refresh().await;
    }

    /// The normalized physical-world view to serve right now.
    pub async fn current_world(&self) -> WorldSnapshot {
        self.world.current().await
    }

    /// Re-read `/proc` and `/sys`. Independent of whether `archied` answered.
    pub async fn refresh_kernel(&self) {
        self.kernel.refresh().await;
    }

    /// Read bounded local runtime facts independently of the control daemon.
    pub async fn refresh_runtime(&self) {
        self.orb.observe_runtime().await;
    }

    /// The machine's own telemetry to serve right now.
    pub async fn current_kernel(&self) -> KernelSnapshot {
        self.kernel.current().await
    }

    /// The semantic orbital frame, made only from source signals and existing machine
    /// projections. The model cannot issue commands or change policy.
    pub async fn current_orb(&self) -> archie_orb::OrbFrame {
        self.orb.current().await
    }

    /// Accepts local, one-way TFOG signals. The socket is private to the Archie user
    /// group and carries no Stage command or response path.
    #[cfg(unix)]
    pub async fn receive_orb_signals(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<()> {
        self.orb.receive(path).await
    }

    /// Dispatch one request.
    pub async fn route(&self, request: Request) -> Response {
        match request.path.as_str() {
            "/api/snapshot" => {
                let snapshot = self.current().await;
                match serde_json::to_vec(&snapshot) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode snapshot: {e}"}}"#).into_bytes(),
                    },
                }
            }
            "/api/world" => {
                let snapshot = self.current_world().await;
                match serde_json::to_vec(&snapshot) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode world snapshot: {e}"}}"#)
                            .into_bytes(),
                    },
                }
            }
            "/api/kernel" => {
                let snapshot = self.current_kernel().await;
                match serde_json::to_vec(&snapshot) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode kernel snapshot: {e}"}}"#)
                            .into_bytes(),
                    },
                }
            }
            "/api/orb/lumen" => {
                // The same frame, composed as the Lumenfield view asks. Read-only: the
                // query picks what is folded and where it sits, never what is observed.
                let frame = self.current_orb().await;
                let view = archie_orb::lumen::LumenView::from_query(&request.query);
                let scene = archie_orb::lumen::compose(&frame, &view);
                match serde_json::to_vec(&scene) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode Lumenfield scene: {e}"}}"#)
                            .into_bytes(),
                    },
                }
            }
            "/api/orb" => {
                let frame = self.current_orb().await;
                match serde_json::to_vec(&frame) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode TFOG frame: {e}"}}"#)
                            .into_bytes(),
                    },
                }
            }
            // Pure and synchronous: this is the same nine-entry table and the same
            // comparisons `archie-geofabric`'s own tests exercise, computed fresh
            // because it is cheaper than caching a value that never changes.
            "/api/geofabric" => {
                let snapshot = geofabric::snapshot();
                match serde_json::to_vec(&snapshot) {
                    Ok(body) => Response::json(body),
                    Err(e) => Response {
                        status: 500,
                        content_type: "application/json; charset=utf-8",
                        body: format!(r#"{{"error":"cannot encode geofabric snapshot: {e}"}}"#)
                            .into_bytes(),
                    },
                }
            }
            "/healthz" => {
                let snapshot = self.current().await;
                let body = format!(
                    r#"{{"ok":true,"source":"{}","daemon_reachable":{},"staleness_ms":{}}}"#,
                    snapshot.source.as_str(),
                    snapshot.daemon_reachable,
                    snapshot.staleness_ms
                );
                Response::json(body.into_bytes())
            }
            _ => Response::not_found(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::Timestamp;
    use std::collections::BTreeMap;

    fn fixture() -> DisplaySnapshot {
        DisplaySnapshot {
            // Claim to be live; `from_fixture` must override this.
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
        }
    }

    #[tokio::test]
    async fn a_replayed_recording_can_never_claim_to_be_live() {
        // The recording says `live`. Replaying it must override that.
        let observatory = Observatory::from_fixture(fixture());
        let served = observatory.current().await;
        assert_eq!(served.source, SnapshotSource::Fixture);
        assert!(!served.source.is_authoritative());
        assert!(observatory.is_fixture());
    }

    #[tokio::test]
    async fn the_bridge_has_no_web_ui_route() {
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 404);
    }

    #[tokio::test]
    async fn the_snapshot_route_serves_decodable_json() {
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/api/snapshot".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 200);
        let decoded: DisplaySnapshot = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(decoded.source, SnapshotSource::Fixture);
    }

    #[tokio::test]
    async fn the_world_route_is_typed_and_fixture_marked() {
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/api/world".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 200);
        let decoded: WorldSnapshot = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(decoded.source, WorldDataSource::Fixture);
        assert!(decoded.observations.is_empty());
        assert_eq!(
            decoded.providers[0].condition,
            world::ProviderCondition::Disabled
        );
    }

    #[tokio::test]
    async fn the_geofabric_route_is_typed_and_bills_nothing() {
        use archie_geofabric::GeoProviderId;

        // Deliberately served even from a fixture-mode observatory: this reads
        // GeoFabric's catalogue and the process environment, not `archied` or the
        // world adapter, so there is nothing here for fixture mode to override.
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/api/geofabric".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 200);

        // `GeoFabricSnapshot` travels one way: its provider entries borrow `&'static
        // str` from the catalogue, so — the same choice `archie-geofabric`'s own
        // catalogue makes, and for the same reason — it serialises out and is never
        // read back as itself. Only the id round-trips.
        let decoded: serde_json::Value = serde_json::from_slice(&response.body).unwrap();

        assert_eq!(decoded["free_only"], serde_json::json!(true));
        let enabled = decoded["enabled"].as_array().expect("an array");
        assert!(!enabled.is_empty());

        let enabled_ids: Vec<GeoProviderId> = enabled
            .iter()
            .map(|entry| serde_json::from_value(entry["id"].clone()).expect("a known id"))
            .collect();
        // The one provider this whole subsystem exists to keep off by default.
        assert!(!enabled_ids.contains(&GeoProviderId::GooglePhotorealistic));

        // Absent is not the same as unexplained. Every enabled entry has nothing to
        // explain; Google is in `disabled`, and says why.
        for entry in enabled {
            assert!(
                entry["reason"].is_null(),
                "{entry:?} is enabled but names a reason"
            );
        }
        let disabled = decoded["disabled"].as_array().expect("an array");
        let google = disabled
            .iter()
            .find(|entry| {
                serde_json::from_value::<GeoProviderId>(entry["id"].clone()).ok()
                    == Some(GeoProviderId::GooglePhotorealistic)
            })
            .expect("Google is disabled, not merely absent");
        let reason = google["reason"].as_str().expect("a stated reason");
        assert!(!reason.is_empty());

        // Cesium is a different kind of disabled: not billed, waiting on an account.
        // This test cannot set `ARCHIE_CESIUM_ION_TOKEN` and check it turns on —
        // `cargo test` runs this process's tests concurrently, and mutating real
        // process environment from one test is exactly the race `archie-geofabric`'s
        // own suite already declined for the same reason (see
        // `an_account_that_does_not_exist_does_not_enable_a_provider`). What is safe
        // and real to assert is the state of *this* environment, where the variable is
        // not set: Cesium is disabled, its reason names the variable, and
        // `fully_configured` says so rather than claiming everything is set up.
        let cesium = disabled
            .iter()
            .find(|entry| {
                serde_json::from_value::<GeoProviderId>(entry["id"].clone()).ok()
                    == Some(GeoProviderId::CesiumOsmBuildings)
            })
            .expect("Cesium needs an account nobody configured in this test process");
        assert_eq!(
            cesium["reason"].as_str(),
            Some("ARCHIE_CESIUM_ION_TOKEN"),
            "the reason should name the variable to set, not a vaguer complaint"
        );
        assert_eq!(decoded["fully_configured"], serde_json::json!(false));

        // No id is in both lists — the plan draws one line, not two independent ones
        // that could disagree about where a provider stands.
        let disabled_ids: Vec<GeoProviderId> = disabled
            .iter()
            .map(|entry| serde_json::from_value(entry["id"].clone()).expect("a known id"))
            .collect();
        for id in &enabled_ids {
            assert!(
                !disabled_ids.contains(id),
                "{id:?} is both enabled and disabled"
            );
        }

        // A band never names a provider that is not in the enabled list — the two are
        // computed from the same plan, and a display that let them drift apart would
        // be showing a legend that does not match its own table.
        for band in decoded["selection"].as_array().expect("an array") {
            for key in ["terrain", "buildings", "photoreal"] {
                if band[key].is_null() {
                    continue;
                }
                let chosen: GeoProviderId =
                    serde_json::from_value(band[key].clone()).expect("a known id");
                assert!(
                    enabled_ids.contains(&chosen),
                    "{chosen:?} selected at {:?} but not in the enabled list",
                    band["band"]
                );
            }
        }
    }

    #[tokio::test]
    async fn healthz_reports_the_source_and_reachability() {
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/healthz".into(),
                query: String::new(),
            })
            .await;
        let body = String::from_utf8(response.body).unwrap();
        assert!(body.contains(r#""source":"fixture""#));
    }

    #[tokio::test]
    async fn an_unknown_route_is_404() {
        let observatory = Observatory::from_fixture(fixture());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/etc/passwd".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 404);
    }

    #[tokio::test]
    async fn with_no_daemon_the_first_snapshot_is_honestly_empty() {
        let root = std::env::temp_dir().join(format!("archie-obs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = StatePaths::rooted(&root);
        paths.ensure_directories().unwrap();

        let observatory = Observatory::live(paths);
        observatory.refresh().await;

        let snapshot = observatory.current().await;
        assert!(!snapshot.daemon_reachable);
        assert!(snapshot.modules.is_empty(), "nothing may be fabricated");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn kernel_telemetry_is_served_and_survives_an_unreachable_daemon() {
        // The whole reason this is a separate endpoint: an operator staring at a
        // disconnected observatory most wants to know whether the machine is thrashing or
        // overheating, and that is precisely when archied cannot say.
        let observatory = Observatory::live_without_world(StatePaths::system());
        let response = observatory
            .route(Request {
                method: "GET".into(),
                path: "/api/kernel".into(),
                query: String::new(),
            })
            .await;
        assert_eq!(response.status, 200);

        let snapshot: KernelSnapshot = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(snapshot.schema_version, 1);

        // The daemon snapshot from the same observatory is unreachable here; the kernel
        // one must still answer with a real verdict about its own availability.
        let daemon = observatory.current().await;
        assert!(
            !daemon.daemon_reachable,
            "no archied is running in this test"
        );
        assert!(
            snapshot.pressure_availability.is_available()
                || matches!(
                    snapshot.pressure_availability,
                    Availability::Unavailable { .. }
                ),
            "availability must be a decision, never absent"
        );
        // Absent evidence, the budget is balanced rather than cinematic.
        assert!(!snapshot.display_budget().is_empty());
    }

    #[tokio::test]
    async fn a_lost_daemon_leaves_the_last_state_visible_but_marked_stale() {
        let root = std::env::temp_dir().join(format!("archie-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = StatePaths::rooted(&root);
        paths.ensure_directories().unwrap();
        let observatory = Observatory::live(paths);

        // Seed the cache with a good snapshot, as a successful refresh would.
        {
            let mut good = fixture();
            good.generated_at = Timestamp::now().plus_millis(-3_000);
            good.load_order = vec!["core.archie".into()];
            *observatory.cache.write().await = Some(good);
        }

        // The daemon is not running, so this refresh fails.
        observatory.refresh().await;

        let snapshot = observatory.current().await;
        assert!(!snapshot.daemon_reachable, "must not claim reachability");
        assert!(
            snapshot.staleness_ms >= 3_000,
            "staleness must reflect real age, got {}",
            snapshot.staleness_ms
        );
        assert_eq!(
            snapshot.load_order,
            vec!["core.archie".to_string()],
            "the last known state stays visible"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
