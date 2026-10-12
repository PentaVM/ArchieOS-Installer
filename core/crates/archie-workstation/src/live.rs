//! The live callers of the Cerebra bridges, exposed for the CLI.
//!
//! `archie-cerebra` owns the machinery (`alert_follower`, `wanda_watcher`,
//! `vault_io`); this crate is the one the CLI already depends on that can name
//! it, so the entry points live here as thin, honest wrappers:
//!
//! - [`follow_alerts_once`] tails the netwatch `alerts.jsonl` and runs one
//!   Cerebra pass per batch of new alerts.
//! - [`watch_exports_once`] / [`ingest_export_file`] feed chat exports to the
//!   Wanda bridge.
//!
//! The passes run against an empty model registry: on a node with no registered
//! local model, every restricted pass is refused with `NoLocalModel` — the
//! designed outcome of `docs/PENDIENTES.md` §3–4, recorded in the pass trace
//! and in the returned report. Claiming a model that was never probed would be
//! a fixture reported as hardware, which this crate does not do.

use std::path::{Path, PathBuf};

pub use archie_cerebra::alert_follower::{FollowReport, FollowerError, FollowerPaths, PassSummary};
pub use archie_cerebra::vault_io;
pub use archie_cerebra::wanda_watcher::{WatchReport, WatcherError};

/// The vault directory the followers share, under the node's state dir.
pub fn cerebra_vault_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("cerebra/vault")
}

/// One follow pass over the netwatch alerts file. `netwatch_state_dir` is the
/// directory holding `alerts.jsonl` and `told.json` (the `[sentry.netwatch]`
/// `state_dir`).
pub fn follow_alerts_once(
    netwatch_state_dir: &Path,
    vault_dir: &Path,
    now_ms: i64,
) -> Result<FollowReport, FollowerError> {
    let paths = FollowerPaths {
        alerts: netwatch_state_dir.join("alerts.jsonl"),
        told: Some(netwatch_state_dir.join("told.json")),
        journal: vault_dir.join("alert-follower-journal.json"),
        vault_dir: vault_dir.to_path_buf(),
    };
    archie_cerebra::alert_follower::follow_alerts_once(
        &archie_cerebra::alert_follower::local_cerebra(),
        &paths,
        &archie_cerebra::alert_follower::no_local_models(),
        "sentry-alerts",
        now_ms,
    )
}

/// One pass over the chat-export watch directory.
pub fn watch_exports_once(
    watch_dir: &Path,
    vault_dir: &Path,
    salt: &str,
) -> Result<WatchReport, WatcherError> {
    archie_cerebra::wanda_watcher::watch_exports_once(
        vault_dir,
        &vault_dir.join("wanda-watcher-journal.json"),
        watch_dir,
        salt,
        "wanda-watch",
    )
}

/// Manually ingest one export file (`archie wanda-ingest`).
pub fn ingest_export_file(
    file: &Path,
    vault_dir: &Path,
    salt: &str,
) -> Result<WatchReport, WatcherError> {
    archie_cerebra::wanda_watcher::ingest_export_file(
        vault_dir,
        &vault_dir.join("wanda-watcher-journal.json"),
        file,
        salt,
        "wanda-manual",
    )
}
