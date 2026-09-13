use crate::{Rect, SourceId, VisualReflexModel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use thiserror::Error;

const MAX_SOURCES: usize = 64;
const MAX_PENDING_PER_SOURCE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PerceptionSessionId(String);

impl PerceptionSessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, MultiSourceError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
        {
            return Err(MultiSourceError::InvalidSession);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedRoi {
    pub source: SourceId,
    pub captured_at_ms: i64,
    pub roi: Rect,
    pub urgency_permille: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRoi {
    pub session: PerceptionSessionId,
    pub item: QueuedRoi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicroBatchPlan {
    pub items: Vec<PlannedRoi>,
    pub scheduler_demand: InferenceDemand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceDemand {
    pub slots: u16,
    pub urgency_permille: u16,
    pub checkpointable: bool,
    pub shared_weight_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MultiSourceError {
    #[error("perception session identifier is invalid")]
    InvalidSession,
    #[error("perception session already exists")]
    DuplicateSession,
    #[error("perception session is missing")]
    MissingSession,
    #[error("multi-source runtime is at capacity")]
    Capacity,
    #[error("queued ROI is invalid")]
    InvalidRoi,
}

struct Session<M> {
    source: SourceId,
    model: Arc<M>,
    pending: VecDeque<QueuedRoi>,
}

pub struct MultiSourceRuntime<M> {
    model: Arc<M>,
    sessions: BTreeMap<PerceptionSessionId, Session<M>>,
    max_batch: usize,
    shared_weight_bytes: u64,
}

impl<M: VisualReflexModel> MultiSourceRuntime<M> {
    pub fn new(
        model: M,
        max_batch: usize,
        shared_weight_bytes: u64,
    ) -> Result<Self, MultiSourceError> {
        if max_batch == 0 || max_batch > 64 || shared_weight_bytes == 0 {
            return Err(MultiSourceError::Capacity);
        }
        Ok(Self {
            model: Arc::new(model),
            sessions: BTreeMap::new(),
            max_batch,
            shared_weight_bytes,
        })
    }

    pub fn open_session(
        &mut self,
        id: PerceptionSessionId,
        source: SourceId,
    ) -> Result<(), MultiSourceError> {
        if self.sessions.len() >= MAX_SOURCES {
            return Err(MultiSourceError::Capacity);
        }
        if self.sessions.contains_key(&id) {
            return Err(MultiSourceError::DuplicateSession);
        }
        self.sessions.insert(
            id,
            Session {
                source,
                model: Arc::clone(&self.model),
                pending: VecDeque::new(),
            },
        );
        Ok(())
    }

    pub fn enqueue(
        &mut self,
        session: &PerceptionSessionId,
        item: QueuedRoi,
    ) -> Result<(), MultiSourceError> {
        item.roi
            .validate()
            .map_err(|_| MultiSourceError::InvalidRoi)?;
        if item.captured_at_ms < 0 || item.urgency_permille > 1_000 {
            return Err(MultiSourceError::InvalidRoi);
        }
        let state = self
            .sessions
            .get_mut(session)
            .ok_or(MultiSourceError::MissingSession)?;
        if state.source != item.source || state.pending.len() >= MAX_PENDING_PER_SOURCE {
            return Err(MultiSourceError::Capacity);
        }
        state.pending.push_back(item);
        Ok(())
    }

    /// Round-robin one ROI per source before taking a second from any source. This prevents a
    /// 60 FPS camera from starving a changed browser or CAD viewport.
    pub fn plan_batch(&mut self) -> Option<MicroBatchPlan> {
        let mut items = Vec::new();
        while items.len() < self.max_batch {
            let mut progress = false;
            for (id, session) in &mut self.sessions {
                if items.len() == self.max_batch {
                    break;
                }
                if let Some(item) = session.pending.pop_front() {
                    items.push(PlannedRoi {
                        session: id.clone(),
                        item,
                    });
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        if items.is_empty() {
            return None;
        }
        let urgency_permille = items
            .iter()
            .map(|item| item.item.urgency_permille)
            .max()
            .unwrap_or(0);
        Some(MicroBatchPlan {
            items,
            scheduler_demand: InferenceDemand {
                slots: 1,
                urgency_permille,
                checkpointable: true,
                shared_weight_bytes: self.shared_weight_bytes,
            },
        })
    }

    pub fn shares_model_allocation(&self) -> bool {
        self.sessions
            .values()
            .all(|session| Arc::ptr_eq(&session.model, &self.model))
    }

    pub fn model(&self) -> &Arc<M> {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeterministicReflexBaseline;

    fn queued(source: &str, urgency: u16) -> QueuedRoi {
        QueuedRoi {
            source: SourceId::new(source).unwrap(),
            captured_at_ms: 1,
            roi: Rect::new(0, 0, 10, 10).unwrap(),
            urgency_permille: urgency,
        }
    }

    #[test]
    fn sessions_share_one_weight_allocation() {
        let mut runtime =
            MultiSourceRuntime::new(DeterministicReflexBaseline::default(), 4, 1024).unwrap();
        runtime
            .open_session(
                PerceptionSessionId::new("browser").unwrap(),
                SourceId::new("browser:1").unwrap(),
            )
            .unwrap();
        runtime
            .open_session(
                PerceptionSessionId::new("camera").unwrap(),
                SourceId::new("camera:1").unwrap(),
            )
            .unwrap();
        assert!(runtime.shares_model_allocation());
        assert_eq!(Arc::strong_count(runtime.model()), 3);
    }

    #[test]
    fn micro_batch_is_fair_and_exposes_scheduler_demand() {
        let mut runtime =
            MultiSourceRuntime::new(DeterministicReflexBaseline::default(), 3, 2048).unwrap();
        let browser = PerceptionSessionId::new("browser").unwrap();
        let camera = PerceptionSessionId::new("camera").unwrap();
        runtime
            .open_session(browser.clone(), SourceId::new("browser:1").unwrap())
            .unwrap();
        runtime
            .open_session(camera.clone(), SourceId::new("camera:1").unwrap())
            .unwrap();
        runtime.enqueue(&camera, queued("camera:1", 100)).unwrap();
        runtime.enqueue(&camera, queued("camera:1", 200)).unwrap();
        runtime.enqueue(&browser, queued("browser:1", 900)).unwrap();
        let plan = runtime.plan_batch().unwrap();
        assert_eq!(plan.items.len(), 3);
        assert!(plan
            .items
            .iter()
            .take(2)
            .any(|item| item.session == browser));
        assert!(plan.items.iter().take(2).any(|item| item.session == camera));
        assert_eq!(plan.scheduler_demand.slots, 1);
        assert_eq!(plan.scheduler_demand.urgency_permille, 900);
        assert!(plan.scheduler_demand.checkpointable);
    }

    #[test]
    fn session_cannot_enqueue_another_sources_roi() {
        let mut runtime =
            MultiSourceRuntime::new(DeterministicReflexBaseline::default(), 2, 1024).unwrap();
        let session = PerceptionSessionId::new("browser").unwrap();
        runtime
            .open_session(session.clone(), SourceId::new("browser:1").unwrap())
            .unwrap();
        assert_eq!(
            runtime.enqueue(&session, queued("camera:1", 1)),
            Err(MultiSourceError::Capacity)
        );
    }
}
