//! Core's inference attribution adapter. The PostgreSQL transaction owns receipt,
//! checkpoint and outbox consistency; Vault/Policy admission remains with callers.
use crate::{Result, StateError, StateStore};
use archie_memory::{
    execution::ExecutionCheckpoint,
    inference::{InferenceBinding, InferenceReceipt, OpaqueSessionId},
    storage::InferenceStore,
};

impl StateStore {
    pub fn bind_inference(&self, binding: &InferenceBinding) -> Result<()> {
        Ok(self.cognitive.bind_inference(binding)?)
    }
    pub fn commit_inference(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected_head: Option<&str>,
    ) -> Result<()> {
        Ok(self
            .cognitive
            .commit_inference(receipt, checkpoint, expected_head)?)
    }
    pub fn inference_receipt(
        &self,
        binding: &InferenceBinding,
        id: &str,
    ) -> Result<Option<InferenceReceipt>> {
        Ok(self.cognitive.inference_receipt(binding, id)?)
    }
    pub fn commit_inference_before(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected_head: Option<&str>,
        deadline_ms: i64,
    ) -> Result<()> {
        Ok(self.cognitive.commit_inference_before(
            receipt,
            checkpoint,
            expected_head,
            deadline_ms,
        )?)
    }
    pub fn recent_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        Ok(self.cognitive.recent_inferences(binding, limit)?)
    }
    pub fn inference_session(&self, binding: &InferenceBinding) -> Result<Option<OpaqueSessionId>> {
        Ok(self.cognitive.inference_session(binding)?)
    }
    pub fn recent_session_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        Ok(self.cognitive.recent_session_inferences(binding, limit)?)
    }
}
impl InferenceStore for StateStore {
    type Error = StateError;
    fn bind_inference(&self, binding: &InferenceBinding) -> Result<()> {
        StateStore::bind_inference(self, binding)
    }
    fn commit_inference(
        &self,
        receipt: &InferenceReceipt,
        checkpoint: &ExecutionCheckpoint,
        expected_head: Option<&str>,
    ) -> Result<()> {
        StateStore::commit_inference(self, receipt, checkpoint, expected_head)
    }
    fn inference_receipt(
        &self,
        binding: &InferenceBinding,
        id: &str,
    ) -> Result<Option<InferenceReceipt>> {
        StateStore::inference_receipt(self, binding, id)
    }
    fn recent_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        StateStore::recent_inferences(self, binding, limit)
    }
    fn inference_session(&self, binding: &InferenceBinding) -> Result<Option<OpaqueSessionId>> {
        StateStore::inference_session(self, binding)
    }
    fn recent_session_inferences(
        &self,
        binding: &InferenceBinding,
        limit: u32,
    ) -> Result<Vec<InferenceReceipt>> {
        StateStore::recent_session_inferences(self, binding, limit)
    }
}
