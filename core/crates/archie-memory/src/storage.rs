//! Backend-independent cognitive persistence contracts. Implementations preserve
//! append-only history, bounded reads and compare-and-swap; no network consensus
//! or privileged mutation authority is implied by implementing these traits.
use crate::{
    cognitive::{CognitiveEvent, FocusRecord},
    execution::ExecutionCheckpoint,
    graph::GraphSnapshot,
    CorrectionTrajectory,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCognitiveEvent {
    pub sequence: i64,
    pub event: CognitiveEvent,
}

pub trait EventStore {
    type Error;
    fn append_event(&self, event: &CognitiveEvent) -> Result<i64, Self::Error>;
    fn event_watermark(&self) -> Result<i64, Self::Error>;
    fn focus_at(&self, focus: &str, as_of_ms: i64) -> Result<Option<FocusRecord>, Self::Error>;
}
pub trait ReplayStore: EventStore {
    fn replay_events(
        &self,
        after: i64,
        through: i64,
        as_of_ms: i64,
        limit: u32,
    ) -> Result<Vec<RecordedCognitiveEvent>, Self::Error>;
}
pub trait GraphStore {
    type Error;
    fn graph_at(&self, as_of_ms: i64) -> Result<Option<GraphSnapshot>, Self::Error>;
    fn publish_graph(
        &self,
        graph: &GraphSnapshot,
        expected_parent: Option<&str>,
    ) -> Result<(), Self::Error>;
}
pub trait TrajectoryStore {
    type Error;
    type Receipt;
    /// The successor graph and correction trajectory must commit together.
    fn apply_correction(
        &self,
        trajectory: &CorrectionTrajectory,
        successor: &GraphSnapshot,
    ) -> Result<Self::Receipt, Self::Error>;
}
pub trait CheckpointStore {
    type Error;
    fn append_checkpoint(
        &self,
        checkpoint: &ExecutionCheckpoint,
        expected_head: Option<&str>,
    ) -> Result<(), Self::Error>;
    fn checkpoint(&self, id: &str) -> Result<Option<ExecutionCheckpoint>, Self::Error>;
    fn execution_head_at(
        &self,
        focus: &str,
        as_of_ms: i64,
    ) -> Result<Option<ExecutionCheckpoint>, Self::Error>;
    /// Root-to-head lineage only; abandoned siblings never appear in this view.
    fn execution_path_at(
        &self,
        focus: &str,
        as_of_ms: i64,
    ) -> Result<Vec<ExecutionCheckpoint>, Self::Error>;
}
pub trait ExperienceStore: ReplayStore + GraphStore + TrajectoryStore + CheckpointStore {}
impl<T: ReplayStore + GraphStore + TrajectoryStore + CheckpointStore> ExperienceStore for T {}

/// Separate extension keeps existing event/graph backends source-compatible.
pub trait SharedThoughtStore {
    type Error;
    fn thought_at(
        &self,
        id: &str,
        as_of_ms: i64,
    ) -> Result<Option<crate::shared_thought::SharedThoughtState>, Self::Error>;
    fn publish_thought(
        &self,
        state: &crate::shared_thought::SharedThoughtState,
        expected_revision: Option<u64>,
    ) -> Result<(), Self::Error>;
}
