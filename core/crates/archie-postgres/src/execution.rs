use super::*;
use archie_memory::{
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus},
    storage::CheckpointStore,
};
const COLUMNS: &str =
    "checkpoint_id,focus_id,branch_id,parent_id,recorded_at,payload_json,payload_sha256";
fn decode_checkpoint(row: Row) -> Result<ExecutionCheckpoint> {
    let checkpoint: ExecutionCheckpoint = decode(row.try_get(5)?, row.try_get(6)?, 131072)?;
    checkpoint
        .validate()
        .map_err(|_| StoreError::Invariant("invalid execution checkpoint"))?;
    if checkpoint.checkpoint_id != row.try_get::<_, String>(0)?
        || checkpoint.focus_id != row.try_get::<_, String>(1)?
        || checkpoint.branch_id != row.try_get::<_, String>(2)?
        || checkpoint.parent_checkpoint_id != row.try_get::<_, Option<String>>(3)?
        || checkpoint.recorded_at_ms != row.try_get::<_, i64>(4)?
    {
        return Err(StoreError::Invariant("execution index mismatch"));
    }
    Ok(checkpoint)
}
fn get(client: &mut impl GenericClient, id: &str) -> Result<Option<ExecutionCheckpoint>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM execution_checkpoints WHERE checkpoint_id=$1"),
            &[&id],
        )?
        .map(decode_checkpoint)
        .transpose()
}
fn head(
    client: &mut impl GenericClient,
    focus: &str,
    time: i64,
) -> Result<Option<ExecutionCheckpoint>> {
    client.query_opt(&format!("SELECT {COLUMNS} FROM execution_checkpoints WHERE focus_id=$1 AND recorded_at<=$2 ORDER BY recorded_at DESC,sequence DESC LIMIT 1"), &[&focus,&time])?.map(decode_checkpoint).transpose()
}
fn path(
    client: &mut impl GenericClient,
    mut current: Option<ExecutionCheckpoint>,
) -> Result<Vec<ExecutionCheckpoint>> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let started = std::time::Instant::now();
    while let Some(checkpoint) = current {
        if result.len() >= 1024
            || !seen.insert(checkpoint.checkpoint_id.clone())
            || started.elapsed() > Duration::from_secs(5)
        {
            return Err(StoreError::Invariant(
                "execution ancestry exceeds bound or cycles",
            ));
        }
        current = if let Some(parent) = &checkpoint.parent_checkpoint_id {
            let parent =
                get(client, parent)?.ok_or(StoreError::Invariant("missing execution ancestor"))?;
            if parent.focus_id != checkpoint.focus_id
                || parent.recorded_at_ms >= checkpoint.recorded_at_ms
                || parent.status != ExecutionStatus::Current
            {
                return Err(StoreError::Invariant("invalid execution ancestry"));
            }
            Some(parent)
        } else {
            None
        };
        result.push(checkpoint);
    }
    result.reverse();
    Ok(result)
}
impl CheckpointStore for PostgresStore {
    type Error = StoreError;
    fn append_checkpoint(
        &self,
        checkpoint: &ExecutionCheckpoint,
        expected: Option<&str>,
    ) -> Result<()> {
        checkpoint
            .validate()
            .map_err(|_| StoreError::Invariant("invalid execution checkpoint"))?;
        let json = encode(checkpoint, 131072)?;
        self.write(|tx| {
            if let Some(old) = get(tx,&checkpoint.checkpoint_id)? {
                return if old == *checkpoint { Ok(()) } else { Err(StoreError::Invariant("checkpoint ID collision")) };
            }
            let focus = focus_at(tx,&checkpoint.focus_id,i64::MAX)?.ok_or(StoreError::Invariant("execution Focus missing"))?;
            if focus.suspended || focus.objective_ref != checkpoint.objective_ref || focus.updated_at_ms > checkpoint.recorded_at_ms {
                return Err(StoreError::Invariant("suspended Focus or changed objective"));
            }
            let current = head(tx,&checkpoint.focus_id,i64::MAX)?;
            if current.as_ref().map(|h| h.checkpoint_id.as_str()) != expected || current.as_ref().is_some_and(|h| h.recorded_at_ms >= checkpoint.recorded_at_ms) {
                return Err(StoreError::Invariant("stale execution head or time"));
            }
            let ancestry = path(tx,current.clone())?;
            let parent = checkpoint.parent_checkpoint_id.as_ref().map(|id| ancestry.iter().find(|a| &a.checkpoint_id == id).ok_or(StoreError::Invariant("parent is not on active execution path"))).transpose()?;
            if parent.is_some_and(|p| p.status != ExecutionStatus::Current) { return Err(StoreError::Invariant("terminal state cannot be continued")); }
            let continuing = current.as_ref().is_some_and(|h| checkpoint.parent_checkpoint_id.as_deref() == Some(h.checkpoint_id.as_str()) && checkpoint.branch_id == h.branch_id);
            if continuing {
                if checkpoint.kind == ExecutionKind::Rollback || ancestry.len() >= 1024 { return Err(StoreError::Invariant("invalid continuation or maximum ancestry")); }
            } else {
                let used: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM execution_checkpoints WHERE focus_id=$1 AND branch_id=$2)", &[&checkpoint.focus_id,&checkpoint.branch_id])?.try_get(0)?;
                if used || (current.is_some() && checkpoint.kind != ExecutionKind::Rollback) || (current.is_none() && (checkpoint.kind == ExecutionKind::Rollback || checkpoint.parent_checkpoint_id.is_some())) {
                    return Err(StoreError::Invariant("rewind requires a fresh branch and explicit rollback"));
                }
                if parent.is_some_and(|p| ancestry.iter().position(|a| a.checkpoint_id == p.checkpoint_id).unwrap_or(1024) >= 1023) {
                    return Err(StoreError::Invariant("maximum execution ancestry"));
                }
            }
            tx.execute("INSERT INTO execution_checkpoints(checkpoint_id,focus_id,branch_id,parent_id,recorded_at,payload_json,payload_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7)", &[&checkpoint.checkpoint_id,&checkpoint.focus_id,&checkpoint.branch_id,&checkpoint.parent_checkpoint_id,&checkpoint.recorded_at_ms,&json,&digest(&json)])?;
            enqueue(tx,"execution",&checkpoint.checkpoint_id,&json,checkpoint.recorded_at_ms)
        })
    }
    fn checkpoint(&self, id: &str) -> Result<Option<ExecutionCheckpoint>> {
        query_key(id, 1)?;
        self.read(|client| get(client, id))
    }
    fn execution_head_at(&self, focus: &str, time: i64) -> Result<Option<ExecutionCheckpoint>> {
        query_key(focus, time)?;
        self.read(|client| head(client, focus, time))
    }
    fn execution_path_at(&self, focus: &str, time: i64) -> Result<Vec<ExecutionCheckpoint>> {
        query_key(focus, time)?;
        self.read(|client| {
            let mut tx = client
                .build_transaction()
                .isolation_level(postgres::IsolationLevel::RepeatableRead)
                .read_only(true)
                .start()?;
            let current = head(&mut tx, focus, time)?;
            let result = path(&mut tx, current)?;
            tx.commit()?;
            Ok(result)
        })
    }
}
