use crate::{CortexCorrectionTrajectoryWrite, StateError, StateStore};
use archie_memory::{
    cognitive::{CognitiveEvent, FocusRecord},
    execution::ExecutionCheckpoint,
    graph::GraphSnapshot,
    storage::*,
    CorrectionTrajectory,
};
impl SharedThoughtStore for StateStore {
    type Error = StateError;
    fn thought_at(
        &self,
        id: &str,
        time: i64,
    ) -> Result<Option<archie_memory::shared_thought::SharedThoughtState>, Self::Error> {
        self.shared_thought_at(id, time)
    }
    fn publish_thought(
        &self,
        state: &archie_memory::shared_thought::SharedThoughtState,
        expected: Option<u64>,
    ) -> Result<(), Self::Error> {
        self.record_shared_thought(state, expected)
    }
}
impl EventStore for StateStore {
    type Error = StateError;
    fn append_event(&self, e: &CognitiveEvent) -> Result<i64, Self::Error> {
        self.record_cognitive_event(e)
    }
    fn event_watermark(&self) -> Result<i64, Self::Error> {
        self.cognitive_high_watermark()
    }
    fn focus_at(&self, id: &str, time: i64) -> Result<Option<FocusRecord>, Self::Error> {
        self.cognitive_focus_at(id, time)
    }
}
impl ReplayStore for StateStore {
    fn replay_events(
        &self,
        after: i64,
        through: i64,
        time: i64,
        limit: u32,
    ) -> Result<Vec<RecordedCognitiveEvent>, Self::Error> {
        self.cognitive_events(after, through, time, limit)
    }
}
impl GraphStore for StateStore {
    type Error = StateError;
    fn graph_at(&self, time: i64) -> Result<Option<GraphSnapshot>, Self::Error> {
        self.cortex_graph_at(time)
    }
    fn publish_graph(&self, g: &GraphSnapshot, parent: Option<&str>) -> Result<(), Self::Error> {
        self.publish_cortex_graph(g, parent)
    }
}
impl TrajectoryStore for StateStore {
    type Error = StateError;
    type Receipt = CortexCorrectionTrajectoryWrite;
    fn apply_correction(
        &self,
        t: &CorrectionTrajectory,
        g: &GraphSnapshot,
    ) -> Result<Self::Receipt, Self::Error> {
        self.apply_cortex_correction(t, g)
    }
}
impl CheckpointStore for StateStore {
    type Error = StateError;
    fn append_checkpoint(
        &self,
        c: &ExecutionCheckpoint,
        head: Option<&str>,
    ) -> Result<(), Self::Error> {
        self.record_execution_checkpoint(c, head)
    }
    fn checkpoint(&self, id: &str) -> Result<Option<ExecutionCheckpoint>, Self::Error> {
        self.execution_checkpoint(id)
    }
    fn execution_head_at(
        &self,
        id: &str,
        time: i64,
    ) -> Result<Option<ExecutionCheckpoint>, Self::Error> {
        StateStore::execution_head_at(self, id, time)
    }
    fn execution_path_at(
        &self,
        id: &str,
        time: i64,
    ) -> Result<Vec<ExecutionCheckpoint>, Self::Error> {
        StateStore::execution_path_at(self, id, time)
    }
}
