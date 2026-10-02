//! Reservation is a durable dispatch fence. A pending record must never be re-sent.
use crate::{contracts::Intent, engine::ExecutionResult};
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub intent_id: String,
    pub operation_id: u64,
    pub digest: String,
    pub reserved_at: Timestamp,
    pub result: Option<ExecutionResult>,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    #[error("intent or operation identity collision")]
    Collision,
    #[error("journal capacity exhausted")]
    Full,
    #[error("journal persistence unavailable")]
    Unavailable,
    #[error("journal record invariant failed")]
    Corrupt,
}
pub trait Journal: Send {
    fn load(&mut self, intent_id: &str) -> Result<Option<JournalRecord>, JournalError>;
    /// Atomically reserves BOTH IDs; Ok(None) means a new reservation.
    fn reserve(
        &mut self,
        intent: &Intent,
        digest: &str,
        at: Timestamp,
    ) -> Result<Option<JournalRecord>, JournalError>;
    fn finish(&mut self, result: &ExecutionResult) -> Result<(), JournalError>;
}

/// Non-durable fixture backend. Never select this for a live daemon.
#[derive(Default)]
pub struct MemoryJournal {
    records: BTreeMap<String, JournalRecord>,
}
impl Journal for MemoryJournal {
    fn load(&mut self, id: &str) -> Result<Option<JournalRecord>, JournalError> {
        Ok(self.records.get(id).cloned())
    }
    fn reserve(
        &mut self,
        intent: &Intent,
        digest: &str,
        at: Timestamp,
    ) -> Result<Option<JournalRecord>, JournalError> {
        if intent.digest().map_err(|_| JournalError::Corrupt)? != digest {
            return Err(JournalError::Corrupt);
        }
        if let Some(record) = self.records.get(&intent.intent_id) {
            if record.digest != digest {
                return Err(JournalError::Collision);
            }
            return Ok(Some(record.clone()));
        }
        if self
            .records
            .values()
            .any(|r| r.operation_id == intent.operation_id)
        {
            return Err(JournalError::Collision);
        }
        if self.records.len() >= 4096 {
            return Err(JournalError::Full);
        }
        self.records.insert(
            intent.intent_id.clone(),
            JournalRecord {
                intent_id: intent.intent_id.clone(),
                operation_id: intent.operation_id,
                digest: digest.into(),
                reserved_at: at,
                result: None,
            },
        );
        Ok(None)
    }
    fn finish(&mut self, result: &ExecutionResult) -> Result<(), JournalError> {
        let record = self
            .records
            .get_mut(&result.intent_id)
            .ok_or(JournalError::Corrupt)?;
        if record.digest != result.intent_digest || record.operation_id != result.operation_id {
            return Err(JournalError::Collision);
        }
        if record.result.is_some() {
            return Err(JournalError::Corrupt);
        }
        record.result = Some(result.clone());
        Ok(())
    }
}
