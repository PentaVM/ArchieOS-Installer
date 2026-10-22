use super::*;
use archie_memory::{
    execution::ExecutionCheckpoint,
    inference::{InferenceBinding, InferenceReceipt, OpaqueSessionId},
    storage::InferenceStore,
};
use std::collections::BTreeSet;

const RECEIPT_COLUMNS: &str = "receipt_id,task_id,checkpoint_id,observed_at,scope_sha256,session_sha256,payload_json,payload_sha256";

fn scope_digest(binding: &InferenceBinding) -> Result<String> {
    Ok(digest(&encode(
        &(
            binding.domain.path(),
            &binding.focus_id,
            &binding.objective_ref,
            &binding.runner_id,
            &binding.provider_id,
            &binding.account_id,
        ),
        4096,
    )?))
}
fn session_digest(receipt: &InferenceReceipt) -> Result<Option<String>> {
    receipt
        .session_id
        .as_ref()
        .map(|session| {
            Ok(digest(&encode(
                &(&receipt.binding.provider_id, session.as_str()),
                4096,
            )?))
        })
        .transpose()
}

fn registered(client: &mut impl GenericClient, binding: &InferenceBinding) -> Result<()> {
    binding
        .validate()
        .map_err(|_| StoreError::Invariant("invalid inference binding"))?;
    let row = client.query_opt("SELECT task_id,focus_id,domain,payload_json,payload_sha256 FROM inference_bindings WHERE task_id=$1", &[&binding.task_id])?
        .ok_or(StoreError::Invariant("inference task binding missing"))?;
    let stored: InferenceBinding = decode(row.try_get(3)?, row.try_get(4)?, 4096)?;
    stored
        .validate()
        .map_err(|_| StoreError::Invariant("invalid stored inference binding"))?;
    if stored != *binding
        || stored.task_id != row.try_get::<_, String>(0)?
        || stored.focus_id != row.try_get::<_, String>(1)?
        || stored.domain.path() != row.try_get::<_, String>(2)?
    {
        return Err(StoreError::Invariant("inference binding identity mismatch"));
    }
    let domain: String = client
        .query_opt(
            "SELECT domain FROM inference_focus_domains WHERE focus_id=$1",
            &[&binding.focus_id],
        )?
        .ok_or(StoreError::Invariant("inference Focus domain missing"))?
        .try_get(0)?;
    if domain != binding.domain.path() {
        return Err(StoreError::Invariant("inference Focus domain mismatch"));
    }
    Ok(())
}
fn active_focus(client: &mut impl GenericClient, binding: &InferenceBinding) -> Result<()> {
    let focus = focus_at(client, &binding.focus_id, i64::MAX)?
        .ok_or(StoreError::Invariant("inference Focus missing"))?;
    if focus.suspended || focus.objective_ref != binding.objective_ref {
        return Err(StoreError::Invariant(
            "inference Focus suspended or objective changed",
        ));
    }
    Ok(())
}
fn decode_receipt(client: &mut impl GenericClient, row: Row) -> Result<InferenceReceipt> {
    let receipt: InferenceReceipt = decode(row.try_get(6)?, row.try_get(7)?, 65536)?;
    receipt
        .validate()
        .map_err(|_| StoreError::Invariant("invalid stored inference receipt"))?;
    if receipt.receipt_id != row.try_get::<_, String>(0)?
        || receipt.binding.task_id != row.try_get::<_, String>(1)?
        || receipt.checkpoint_id != row.try_get::<_, String>(2)?
        || receipt.observed_at_ms != row.try_get::<_, i64>(3)?
        || scope_digest(&receipt.binding)? != row.try_get::<_, String>(4)?
        || session_digest(&receipt)? != row.try_get::<_, Option<String>>(5)?
    {
        return Err(StoreError::Invariant("inference receipt index mismatch"));
    }
    registered(client, &receipt.binding)?;
    let checkpoint = execution::get(client, &receipt.checkpoint_id)?.ok_or(
        StoreError::Invariant("inference receipt checkpoint missing"),
    )?;
    receipt
        .validate_checkpoint(&checkpoint)
        .map_err(|_| StoreError::Invariant("inference receipt checkpoint mismatch"))?;
    Ok(receipt)
}
fn get(client: &mut impl GenericClient, id: &str) -> Result<Option<InferenceReceipt>> {
    client
        .query_opt(
            &format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE receipt_id=$1"),
            &[&id],
        )?
        .map(|row| decode_receipt(client, row))
        .transpose()
}
fn active_checkpoint_ids(client: &mut impl GenericClient, focus: &str) -> Result<BTreeSet<String>> {
    let head = execution::head(client, focus, i64::MAX)?;
    Ok(execution::path(client, head)?
        .into_iter()
        .map(|checkpoint| checkpoint.checkpoint_id)
        .collect())
}

impl PostgresStore {
    pub fn commit_inference_before(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected: Option<&str>,
        deadline_ms: i64,
    ) -> Result<()> {
        self.commit_inference_guarded(receipt, checkpoint, expected, Some(deadline_ms))
    }
    fn commit_inference_guarded(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected: Option<&str>,
        deadline: Option<i64>,
    ) -> Result<()> {
        receipt
            .validate_checkpoint(checkpoint)
            .map_err(|_| StoreError::Invariant("invalid inference receipt/checkpoint"))?;
        let json = encode(receipt, 65536)?;
        let commit = |tx: &mut Transaction<'_>| {
            registered(tx, &receipt.binding)?;
            if let Some(old) = get(tx, &receipt.receipt_id)? {
                return if old == *receipt
                    && execution::get(tx, &receipt.checkpoint_id)?.as_ref() == Some(checkpoint)
                {
                    Ok(())
                } else {
                    Err(StoreError::Invariant("inference receipt ID collision"))
                };
            }
            active_focus(tx, &receipt.binding)?;
            let session = session_digest(receipt)?;
            if let Some(session) = &session {
                if let Some(row) = tx.query_opt(&format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE session_sha256=$1 ORDER BY sequence LIMIT 1"), &[session])? {
                    let prior = decode_receipt(tx,row)?;
                    if !prior.binding.same_session_scope(&receipt.binding) {
                        return Err(StoreError::Invariant("opaque session cannot cross inference scopes"));
                    }
                }
            }
            if execution::get(tx, &checkpoint.checkpoint_id)?.is_some() {
                return Err(StoreError::Invariant(
                    "inference checkpoint already exists without receipt",
                ));
            }
            execution::append_checkpoint_in(tx, checkpoint, expected)?;
            tx.execute("INSERT INTO inference_receipts(receipt_id,task_id,checkpoint_id,observed_at,scope_sha256,session_sha256,payload_json,payload_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)", &[&receipt.receipt_id,&receipt.binding.task_id,&receipt.checkpoint_id,&receipt.observed_at_ms,&scope_digest(&receipt.binding)?,&session,&json,&digest(&json)])?;
            enqueue(
                tx,
                "inference",
                &receipt.receipt_id,
                &json,
                receipt.observed_at_ms,
            )
        };
        if let Some(deadline) = deadline {
            self.write_before(deadline, commit)
        } else {
            self.write(commit)
        }
    }
}

impl InferenceStore for PostgresStore {
    type Error = StoreError;
    fn bind_inference(&self, binding: &InferenceBinding) -> Result<()> {
        binding
            .validate()
            .map_err(|_| StoreError::Invariant("invalid inference binding"))?;
        let json = encode(binding, 4096)?;
        self.write(|tx| {
            active_focus(tx,binding)?;
            if tx.query_opt("SELECT task_id FROM inference_bindings WHERE task_id=$1", &[&binding.task_id])?.is_some() {
                return registered(tx,binding);
            }
            tx.execute("INSERT INTO inference_focus_domains(focus_id,domain) VALUES ($1,$2) ON CONFLICT (focus_id) DO NOTHING", &[&binding.focus_id,&binding.domain.path()])?;
            let domain: String = tx.query_one("SELECT domain FROM inference_focus_domains WHERE focus_id=$1", &[&binding.focus_id])?.try_get(0)?;
            if domain != binding.domain.path() { return Err(StoreError::Invariant("inference Focus cannot cross domains")); }
            tx.execute("INSERT INTO inference_bindings(task_id,focus_id,domain,payload_json,payload_sha256) VALUES ($1,$2,$3,$4,$5)", &[&binding.task_id,&binding.focus_id,&binding.domain.path(),&json,&digest(&json)])?;
            Ok(())
        })
    }
    fn commit_inference(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected: Option<&str>,
    ) -> Result<()> {
        self.commit_inference_guarded(receipt, checkpoint, expected, None)
    }
    fn inference_receipt(
        &self,
        binding: &InferenceBinding,
        id: &str,
    ) -> Result<Option<InferenceReceipt>> {
        query_key(id, 1)?;
        self.read(|client| {
            registered(client, binding)?;
            // Scope in SQL before decoding any evidence references. The decoded
            // binding is checked again to reject a tampered secondary index.
            let row = client.query_opt(
                &format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE receipt_id=$1 AND task_id=$2"),
                &[&id, &binding.task_id],
            )?;
            row.map(|row| {
                let receipt = decode_receipt(client, row)?;
                if receipt.binding != *binding {
                    return Err(StoreError::Invariant("inference receipt binding mismatch"));
                }
                Ok(receipt)
            }).transpose()
        })
    }
    fn recent_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        bounded_limit(limit)?;
        self.read(|client| {
            let mut tx = client.build_transaction().isolation_level(postgres::IsolationLevel::RepeatableRead).read_only(true).start()?;
            registered(&mut tx,binding)?;
            let active = active_checkpoint_ids(&mut tx,&binding.focus_id)?;
            let active_ids: Vec<_> = active.iter().cloned().collect();
            // Apply LIMIT after the exact active lineage, so abandoned history
            // cannot hide an older valid checkpoint behind the read budget.
            let rows = tx.query(&format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE task_id=$1 AND checkpoint_id=ANY($3::text[]) ORDER BY observed_at DESC,sequence DESC LIMIT $2"), &[&binding.task_id,&i64::from(limit), &active_ids])?;
            let mut receipts = Vec::new();
            for row in rows {
                let receipt = decode_receipt(&mut tx,row)?;
                if receipt.binding != *binding { return Err(StoreError::Invariant("inference receipt binding mismatch")); }
                if active.contains(&receipt.checkpoint_id) { receipts.push(receipt); }
            }
            tx.commit()?;
            Ok(receipts)
        })
    }
    fn recent_session_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        if !(1..=128).contains(&limit) {
            return Err(StoreError::Invariant(
                "invalid inference context read limit",
            ));
        }
        self.read(|client| {
            let mut tx = client.build_transaction()
                .isolation_level(postgres::IsolationLevel::RepeatableRead).read_only(true).start()?;
            registered(&mut tx, binding)?;
            active_focus(&mut tx, binding)?;
            let active = active_checkpoint_ids(&mut tx, &binding.focus_id)?;
            let active_ids: Vec<_> = active.iter().cloned().collect();
            let rows = tx.query(&format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE scope_sha256=$1 AND checkpoint_id=ANY($3::text[]) ORDER BY observed_at DESC,sequence DESC LIMIT $2"), &[&scope_digest(binding)?, &i64::from(limit), &active_ids])?;
            let mut receipts = Vec::new();
            for row in rows {
                let receipt = decode_receipt(&mut tx, row)?;
                if !receipt.binding.same_session_scope(binding) {
                    return Err(StoreError::Invariant("inference context scope mismatch"));
                }
                if active.contains(&receipt.checkpoint_id) { receipts.push(receipt); }
            }
            tx.commit()?;
            Ok(receipts)
        })
    }
    fn inference_session(&self, binding: &InferenceBinding) -> Result<Option<OpaqueSessionId>> {
        self.read(|client| {
            let mut tx = client.build_transaction().isolation_level(postgres::IsolationLevel::RepeatableRead).read_only(true).start()?;
            registered(&mut tx,binding)?;
            active_focus(&mut tx,binding)?;
            let active = active_checkpoint_ids(&mut tx,&binding.focus_id)?;
            let active_ids: Vec<_> = active.iter().cloned().collect();
            let rows = tx.query(&format!("SELECT {RECEIPT_COLUMNS} FROM inference_receipts WHERE scope_sha256=$1 AND session_sha256 IS NOT NULL AND checkpoint_id=ANY($2::text[]) ORDER BY observed_at DESC,sequence DESC LIMIT 1"), &[&scope_digest(binding)?, &active_ids])?;
            let mut session = None;
            for row in rows {
                let receipt = decode_receipt(&mut tx,row)?;
                if !receipt.binding.same_session_scope(binding) { return Err(StoreError::Invariant("inference session scope mismatch")); }
                if active.contains(&receipt.checkpoint_id) { session = receipt.session_id; break; }
            }
            tx.commit()?;
            Ok(session)
        })
    }
}
