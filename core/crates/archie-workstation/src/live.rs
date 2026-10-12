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
//! - [`focus_once`] runs the unified feeder over everything those two sealed,
//!   under one focus profile, and one Cerebra pass over what it admits.
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
pub use archie_notices as notices;

/// The vault directory the followers share, under the node's state dir.
pub fn cerebra_vault_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("cerebra/vault")
}

/// The owner notice board, next to the vault: `<state>/cerebra/notices.json`.
pub fn notice_board_path(vault_dir: &Path) -> PathBuf {
    vault_dir.parent().unwrap_or(vault_dir).join("notices.json")
}

/// One follow pass over the netwatch alerts file. `netwatch_state_dir` is the
/// directory holding `alerts.jsonl` and `told.json` (the `[sentry.netwatch]`
/// `state_dir`).
pub fn follow_alerts_once(
    netwatch_state_dir: &Path,
    vault_dir: &Path,
    notice_policy: archie_notices::Policy,
    now_ms: i64,
) -> Result<FollowReport, FollowerError> {
    let paths = FollowerPaths {
        alerts: netwatch_state_dir.join("alerts.jsonl"),
        told: Some(netwatch_state_dir.join("told.json")),
        journal: vault_dir.join("alert-follower-journal.json"),
        vault_dir: vault_dir.to_path_buf(),
        notices: Some((notice_board_path(vault_dir), notice_policy)),
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

/// What one focused pass admitted, dropped and came to. References and
/// numbers only; no record content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FocusReport {
    pub profile: String,
    pub domain: String,
    pub admitted: Vec<FocusAdmitted>,
    /// Drop reason → count.
    pub dropped: std::collections::BTreeMap<String, usize>,
    /// `nothing`, `reasoning`, or `refused:<code>`.
    pub outcome: String,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FocusAdmitted {
    pub source: &'static str,
    pub reference: String,
    pub priority: u8,
    pub weight: u8,
    pub at_ms: i64,
}

/// One pass of the unified feeder over the followers' vault: collect every
/// alert, device and Wanda record of `domain` sealed at or after `since_ms`,
/// plan it under the named profile, and consider the plan once. Like the
/// followers, it runs with no registered local model, so a restricted pass is
/// refused and reported as such.
pub fn focus_once(
    vault_dir: &Path,
    profile: &str,
    domain: &str,
    since_ms: i64,
    now_ms: i64,
) -> Result<FocusReport, String> {
    use archie_cerebra::feeder::{collect_from_vault, consider_plan, plan, FeedOutcome};
    let profile = archie_cerebra::focus::FocusProfile::preset(profile).map_err(|e| {
        format!(
            "{e}; known: {}",
            archie_cerebra::focus::FocusProfile::PRESETS.join(", ")
        )
    })?;
    let domain =
        archie_vault::ContextDomain::try_from(domain.to_owned()).map_err(|e| e.to_string())?;
    let mut vault = vault_io::open_vault(vault_dir).map_err(|e| e.to_string())?;
    let items = collect_from_vault(&mut vault, &domain, since_ms, now_ms);
    let planned = plan(
        &profile,
        &domain,
        &format!("focus-{}", profile.name),
        &items,
    )
    .map_err(|e| e.to_string())?;
    let admitted = planned
        .admitted()
        .iter()
        .map(|a| FocusAdmitted {
            source: a.source.label(),
            reference: a.reference.path().to_owned(),
            priority: a.priority,
            weight: a.weight,
            at_ms: a.at_ms,
        })
        .collect();
    let mut dropped = std::collections::BTreeMap::new();
    for d in planned.dropped() {
        *dropped.entry(format!("{:?}", d.reason)).or_insert(0) += 1;
    }
    let outcome = consider_plan(
        &archie_cerebra::alert_follower::local_cerebra(),
        &mut vault,
        &archie_cerebra::alert_follower::no_local_models(),
        planned,
        archie_types::TraceId::new(),
        now_ms,
    )
    .map_err(|e| e.to_string())?;
    let (outcome, trace_id) = match &outcome {
        FeedOutcome::Nothing { .. } => ("nothing".to_owned(), None),
        FeedOutcome::Reasoning { pass, .. } => (
            "reasoning".to_owned(),
            Some(pass.trace.trace_id.to_string()),
        ),
        FeedOutcome::Refused { pass, refusal, .. } => (
            format!("refused:{refusal:?}"),
            Some(pass.trace.trace_id.to_string()),
        ),
    };
    Ok(FocusReport {
        profile: profile.name,
        domain: domain.to_string(),
        admitted,
        dropped,
        outcome,
        trace_id,
    })
}
