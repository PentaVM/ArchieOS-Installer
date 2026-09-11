use super::*;
use archie_memory::{storage::TrajectoryStore, CorrectionTrajectory};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionReceipt {
    pub already_recorded: bool,
    pub trajectory: CorrectionTrajectory,
    pub payload_sha256: String,
}
const COLUMNS: &str = "trajectory_id,correction_id,model_id,captured_at,source_snapshot_id,resulting_snapshot_id,payload_json,payload_sha256";
fn decode_trajectory(row: Row) -> Result<CorrectionTrajectory> {
    let trajectory: CorrectionTrajectory = decode(row.try_get(6)?, row.try_get(7)?, 2097152)?;
    trajectory
        .validate()
        .map_err(|_| StoreError::Invariant("invalid correction trajectory"))?;
    if trajectory.trajectory_id != row.try_get::<_, String>(0)?
        || trajectory.correction.correction_id != row.try_get::<_, String>(1)?
        || trajectory.model_id != row.try_get::<_, String>(2)?
        || trajectory.captured_at_ms != row.try_get::<_, i64>(3)?
        || trajectory.graph_update.source_graph_snapshot_id != row.try_get::<_, String>(4)?
        || trajectory.graph_update.resulting_graph_snapshot_id != row.try_get::<_, String>(5)?
    {
        return Err(StoreError::Invariant(
            "correction trajectory index mismatch",
        ));
    }
    Ok(trajectory)
}
impl PostgresStore {
    pub fn trajectory(&self, id: &str) -> Result<Option<CorrectionTrajectory>> {
        query_key(id, 1)?;
        self.read(|client| client.query_opt(&format!("SELECT {COLUMNS} FROM cortex_correction_trajectories WHERE trajectory_id=$1"), &[&id])?.map(decode_trajectory).transpose())
    }
}
impl TrajectoryStore for PostgresStore {
    type Error = StoreError;
    type Receipt = CorrectionReceipt;
    fn apply_correction(
        &self,
        trajectory: &CorrectionTrajectory,
        successor: &GraphSnapshot,
    ) -> Result<CorrectionReceipt> {
        trajectory
            .validate()
            .map_err(|_| StoreError::Invariant("invalid correction trajectory"))?;
        successor
            .validate()
            .map_err(|_| StoreError::Invariant("invalid successor graph"))?;
        let receipt = &trajectory.graph_update;
        if receipt.resulting_graph_snapshot_id != successor.snapshot_id
            || receipt.applied_at_ms != successor.captured_at_ms
        {
            return Err(StoreError::Invariant(
                "receipt does not describe supplied successor",
            ));
        }
        let json = encode(trajectory, 2097152)?;
        self.write(|tx| {
            let existing = tx.query_opt(&format!("SELECT {GRAPH_COLUMNS} FROM cortex_graph_snapshots WHERE snapshot_id=$1"), &[&successor.snapshot_id])?;
            if let Some(existing) = existing {
                if decode_graph(existing)? != *successor { return Err(StoreError::Invariant("successor identity conflict")); }
                let bound = tx.query_opt(&format!("SELECT {COLUMNS} FROM cortex_correction_trajectories WHERE correction_id=$1"), &[&trajectory.correction.correction_id])?.map(decode_trajectory).transpose()?;
                if bound.as_ref() != Some(trajectory) { return Err(StoreError::Invariant("successor was not published by this correction")); }
                return Ok(CorrectionReceipt { already_recorded: true,trajectory: trajectory.clone(),payload_sha256: digest(&json) });
            }
            let source = graph_at(tx,i64::MAX)?.ok_or(StoreError::Invariant("no graph to correct"))?;
            if source.snapshot_id != receipt.source_graph_snapshot_id || source.captured_at_ms > trajectory.correction.recorded_at_ms {
                return Err(StoreError::Invariant("correction does not name current historical source"));
            }
            if source.nodes.len() != successor.nodes.len() || source.edges.len() != successor.edges.len() { return Err(StoreError::Invariant("correction must preserve evidence identities")); }
            let nodes: BTreeMap<_,_> = successor.nodes.iter().map(|n| (&n.id,n)).collect();
            let edges: BTreeMap<_,_> = successor.edges.iter().map(|e| (&e.id,e)).collect();
            let mut changed = BTreeSet::new();
            for old in &source.nodes {
                let new = nodes.get(&old.id).ok_or(StoreError::Invariant("correction removed node"))?;
                if old != *new {
                    if new.version <= old.version { return Err(StoreError::Invariant("corrected node version did not advance")); }
                    changed.insert(old.id.clone());
                }
            }
            for old in &source.edges {
                let new = edges.get(&old.id).ok_or(StoreError::Invariant("correction removed edge"))?;
                if old != *new {
                    if new.version <= old.version { return Err(StoreError::Invariant("corrected edge version did not advance")); }
                    changed.insert(old.id.clone());
                }
            }
            let targets: BTreeSet<_> = trajectory.correction.target_ids.iter().cloned().collect();
            if changed.is_empty() || changed != targets { return Err(StoreError::Invariant("correction targets do not match changed records")); }
            publish_graph_tx(tx,successor,Some(&source.snapshot_id))?;
            tx.execute("INSERT INTO cortex_correction_trajectories(trajectory_id,correction_id,model_id,captured_at,source_snapshot_id,resulting_snapshot_id,payload_json,payload_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)", &[&trajectory.trajectory_id,&trajectory.correction.correction_id,&trajectory.model_id,&trajectory.captured_at_ms,&receipt.source_graph_snapshot_id,&receipt.resulting_graph_snapshot_id,&json,&digest(&json)])?;
            Ok(CorrectionReceipt { already_recorded: false,trajectory: trajectory.clone(),payload_sha256: digest(&json) })
        })
    }
}
