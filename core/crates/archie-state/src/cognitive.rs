//! Cognitive contracts delegated to the authoritative PostgreSQL store.
use crate::{
    CortexCorrectionTrajectoryRecord, CortexCorrectionTrajectoryWrite, Result, StateStore,
};
pub use archie_memory::storage::RecordedCognitiveEvent;
use archie_memory::{
    cognitive::{CognitiveEvent, CognitivePayload, FocusRecord},
    execution::ExecutionCheckpoint,
    graph::GraphSnapshot,
    shared_thought::SharedThoughtState,
    storage::*,
    CorrectionTrajectory,
};
use archie_orb::{publish, EntityKind, OrbEntity, SignalKind};
pub use archie_postgres::OutboxEntry;
use sha2::Digest;
use std::collections::BTreeMap;
impl StateStore {
    pub fn record_cognitive_event(&self, e: &CognitiveEvent) -> Result<i64> {
        let sequence = self.cognitive.append_event(e)?;
        if let CognitivePayload::FocusUpdated(record) = &e.payload {
            let mut scores = BTreeMap::new();
            if !record.suspended {
                for activation in &record.activations {
                    let digest = sha2::Sha256::digest(activation.node_id.as_bytes());
                    let id = format!(
                        "cognitive:{}",
                        digest[..16]
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    );
                    let (kind, label) = match activation.node_id.split_once(':') {
                        Some(("file" | "path", _)) => (EntityKind::File, "focused file"),
                        Some(("tool", _)) => (EntityKind::Tool, "focused tool"),
                        Some(("process", _)) => (EntityKind::Process, "focused process"),
                        Some(("window", _)) => (EntityKind::Window, "focused window"),
                        Some(("app", _)) => (EntityKind::App, "focused app"),
                        _ => (EntityKind::Memory, "focused memory"),
                    };
                    let score = activation.strength.clamp(0.0, 1.0) as f32;
                    scores.insert(id.clone(), score);
                    publish(&archie_orb::signal(SignalKind::EntityUpsert {
                        entity: OrbEntity {
                            id,
                            parent_id: Some("system:archieos".into()),
                            related_ids: Vec::new(),
                            kind,
                            token: archie_orb::semantic_features(&activation.node_id),
                            class_probs: BTreeMap::from([("cognitive".into(), 1.0)]),
                            size_bytes: 0,
                            activity: score,
                            importance: score,
                            uncertainty: 0.2,
                            runtime: Vec::new(),
                            label: label.into(),
                        },
                    }));
                }
            }
            publish(&archie_orb::signal(SignalKind::Focus {
                focus_id: record.focus_id.clone(),
                scores,
            }));
        }
        Ok(sequence)
    }
    pub fn cognitive_high_watermark(&self) -> Result<i64> {
        Ok(self.cognitive.event_watermark()?)
    }
    pub fn cognitive_focus_at(&self, id: &str, time: i64) -> Result<Option<FocusRecord>> {
        Ok(self.cognitive.focus_at(id, time)?)
    }
    pub fn cognitive_events(
        &self,
        after: i64,
        through: i64,
        time: i64,
        limit: u32,
    ) -> Result<Vec<RecordedCognitiveEvent>> {
        Ok(self.cognitive.replay_events(after, through, time, limit)?)
    }
    pub fn cortex_graph_at(&self, time: i64) -> Result<Option<GraphSnapshot>> {
        Ok(self.cognitive.graph_at(time)?)
    }
    pub fn publish_cortex_graph(&self, g: &GraphSnapshot, parent: Option<&str>) -> Result<()> {
        Ok(self.cognitive.publish_graph(g, parent)?)
    }
    pub fn apply_cortex_correction(
        &self,
        t: &CorrectionTrajectory,
        g: &GraphSnapshot,
    ) -> Result<CortexCorrectionTrajectoryWrite> {
        let receipt = self.cognitive.apply_correction(t, g)?;
        let record = CortexCorrectionTrajectoryRecord {
            trajectory: receipt.trajectory,
            payload_sha256: receipt.payload_sha256,
        };
        Ok(if receipt.already_recorded {
            CortexCorrectionTrajectoryWrite::AlreadyRecorded(record)
        } else {
            CortexCorrectionTrajectoryWrite::Recorded(record)
        })
    }
    pub fn record_execution_checkpoint(
        &self,
        c: &ExecutionCheckpoint,
        head: Option<&str>,
    ) -> Result<()> {
        Ok(self.cognitive.append_checkpoint(c, head)?)
    }
    pub fn execution_checkpoint(&self, id: &str) -> Result<Option<ExecutionCheckpoint>> {
        Ok(self.cognitive.checkpoint(id)?)
    }
    pub fn execution_head_at(&self, id: &str, time: i64) -> Result<Option<ExecutionCheckpoint>> {
        Ok(self.cognitive.execution_head_at(id, time)?)
    }
    pub fn execution_path_at(&self, id: &str, time: i64) -> Result<Vec<ExecutionCheckpoint>> {
        Ok(self.cognitive.execution_path_at(id, time)?)
    }
    pub fn shared_thought_at(&self, id: &str, time: i64) -> Result<Option<SharedThoughtState>> {
        Ok(self.cognitive.thought_at(id, time)?)
    }
    pub fn record_shared_thought(
        &self,
        t: &SharedThoughtState,
        revision: Option<u64>,
    ) -> Result<()> {
        Ok(self.cognitive.publish_thought(t, revision)?)
    }
    pub fn cognitive_outbox(&self, limit: u32) -> Result<Vec<OutboxEntry>> {
        Ok(self.cognitive.pending_outbox(limit)?)
    }
    pub fn acknowledge_cognitive_outbox(&self, sequence: i64, hash: &str) -> Result<()> {
        Ok(self.cognitive.acknowledge_outbox(sequence, hash)?)
    }
}
