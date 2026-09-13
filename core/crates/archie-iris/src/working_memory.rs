use crate::{NodeId, SceneGraph, SceneNode, SourceId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

const MAX_EVENT_CODE: usize = 64;
const MAX_EVENT_NODES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBounds {
    pub reflex_ttl_ms: i64,
    pub episodic_ttl_ms: i64,
    pub max_reflex_nodes: usize,
    pub max_episodes: usize,
    pub max_deltas: usize,
}

impl Default for MemoryBounds {
    fn default() -> Self {
        Self {
            reflex_ttl_ms: 5_000,
            episodic_ttl_ms: 300_000,
            max_reflex_nodes: 4_096,
            max_episodes: 1_024,
            max_deltas: 2_048,
        }
    }
}

impl MemoryBounds {
    fn validate(self) -> Result<Self, WorkingMemoryError> {
        if self.reflex_ttl_ms <= 0
            || self.episodic_ttl_ms <= 0
            || self.max_reflex_nodes == 0
            || self.max_reflex_nodes > 65_536
            || self.max_episodes == 0
            || self.max_episodes > 16_384
            || self.max_deltas == 0
            || self.max_deltas > 65_536
        {
            return Err(WorkingMemoryError::InvalidBounds);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEpisode {
    pub code: String,
    pub source: SourceId,
    pub nodes: Vec<NodeId>,
    pub observed_at_ms: i64,
    pub expires_at_ms: i64,
}

impl SemanticEpisode {
    fn validate(&self) -> Result<(), WorkingMemoryError> {
        if self.code.is_empty()
            || self.code.len() > MAX_EVENT_CODE
            || !self.code.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
            || self.nodes.len() > MAX_EVENT_NODES
            || self.observed_at_ms < 0
            || self.expires_at_ms <= self.observed_at_ms
        {
            return Err(WorkingMemoryError::InvalidEpisode);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDeltaRecord {
    pub source: SourceId,
    pub scene_revision: u64,
    pub changed_nodes: Vec<NodeId>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurablePromotionCandidate {
    pub episode_code: String,
    pub source: SourceId,
    pub node_ids: Vec<NodeId>,
    pub scene_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkingMemoryError {
    #[error("working scene memory bounds are invalid")]
    InvalidBounds,
    #[error("semantic episode is invalid")]
    InvalidEpisode,
    #[error("scene time is invalid")]
    InvalidTime,
    #[error("scene delta is invalid")]
    InvalidDelta,
}

/// Bounded semantic working memory. It has no frame/audio field and no durable write API.
/// Durable candidates still have to pass `archie-memory` admission outside this crate.
pub struct WorkingSceneMemory {
    bounds: MemoryBounds,
    reflex: BTreeMap<NodeId, SceneNode>,
    episodes: VecDeque<SemanticEpisode>,
    deltas: VecDeque<SceneDeltaRecord>,
}

impl WorkingSceneMemory {
    pub fn new(bounds: MemoryBounds) -> Result<Self, WorkingMemoryError> {
        Ok(Self {
            bounds: bounds.validate()?,
            reflex: BTreeMap::new(),
            episodes: VecDeque::new(),
            deltas: VecDeque::new(),
        })
    }

    pub fn observe_scene(
        &mut self,
        scene: &SceneGraph,
        changed_nodes: Vec<NodeId>,
        now_ms: i64,
    ) -> Result<(), WorkingMemoryError> {
        if now_ms < 0 || changed_nodes.is_empty() || changed_nodes.len() > MAX_EVENT_NODES * 8 {
            return Err(WorkingMemoryError::InvalidDelta);
        }
        let first = changed_nodes
            .first()
            .and_then(|id| scene.node(id))
            .ok_or(WorkingMemoryError::InvalidDelta)?;
        if changed_nodes.iter().any(|id| scene.node(id).is_none()) {
            return Err(WorkingMemoryError::InvalidDelta);
        }
        for id in &changed_nodes {
            let node = scene.node(id).ok_or(WorkingMemoryError::InvalidDelta)?;
            self.reflex.insert(id.clone(), node.clone());
        }
        while self.reflex.len() > self.bounds.max_reflex_nodes {
            let evict = self
                .reflex
                .values()
                .min_by_key(|node| (node.last_seen_ms, node.id.clone()))
                .map(|node| node.id.clone())
                .ok_or(WorkingMemoryError::InvalidDelta)?;
            self.reflex.remove(&evict);
        }
        self.deltas.push_back(SceneDeltaRecord {
            source: first.source.clone(),
            scene_revision: scene.revision(),
            changed_nodes,
            observed_at_ms: now_ms,
        });
        while self.deltas.len() > self.bounds.max_deltas {
            self.deltas.pop_front();
        }
        self.expire(now_ms)
    }

    pub fn remember_episode(
        &mut self,
        code: impl Into<String>,
        source: SourceId,
        nodes: Vec<NodeId>,
        now_ms: i64,
    ) -> Result<(), WorkingMemoryError> {
        let episode = SemanticEpisode {
            code: code.into(),
            source,
            nodes,
            observed_at_ms: now_ms,
            expires_at_ms: now_ms
                .checked_add(self.bounds.episodic_ttl_ms)
                .ok_or(WorkingMemoryError::InvalidTime)?,
        };
        episode.validate()?;
        self.episodes.push_back(episode);
        while self.episodes.len() > self.bounds.max_episodes {
            self.episodes.pop_front();
        }
        self.expire(now_ms)
    }

    pub fn expire(&mut self, now_ms: i64) -> Result<(), WorkingMemoryError> {
        if now_ms < 0 {
            return Err(WorkingMemoryError::InvalidTime);
        }
        let reflex_cutoff = now_ms.saturating_sub(self.bounds.reflex_ttl_ms);
        self.reflex
            .retain(|_, node| node.last_seen_ms >= reflex_cutoff);
        self.episodes
            .retain(|episode| episode.expires_at_ms > now_ms);
        self.deltas.retain(|delta| {
            delta.observed_at_ms >= now_ms.saturating_sub(self.bounds.episodic_ttl_ms)
        });
        Ok(())
    }

    pub fn reflex_nodes(&self) -> impl Iterator<Item = &SceneNode> {
        self.reflex.values()
    }

    pub fn episodes(&self) -> impl Iterator<Item = &SemanticEpisode> {
        self.episodes.iter()
    }

    pub fn deltas(&self) -> impl Iterator<Item = &SceneDeltaRecord> {
        self.deltas.iter()
    }

    pub fn promotion_candidate(
        &self,
        episode_index: usize,
        scene_revision: u64,
    ) -> Option<DurablePromotionCandidate> {
        let episode = self.episodes.get(episode_index)?;
        Some(DurablePromotionCandidate {
            episode_code: episode.code.clone(),
            source: episode.source.clone(),
            node_ids: episode.nodes.clone(),
            scene_revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeKind, Rect, SceneEvent, TrackId};

    fn graph(last_seen_ms: i64) -> SceneGraph {
        let source = SourceId::new("camera:0").unwrap();
        let node = SceneNode {
            id: NodeId::new("person:1").unwrap(),
            source,
            kind: NodeKind::Person,
            bbox: Some(Rect::new(0, 0, 10, 10).unwrap()),
            parent: None,
            track_id: Some(TrackId::new("track:1").unwrap()),
            confidence_permille: 900,
            uncertainty_permille: 100,
            last_seen_ms,
            label: None,
            revision: 0,
        };
        let mut graph = SceneGraph::default();
        graph.apply(0, &[SceneEvent::Upsert(node)]).unwrap();
        graph
    }

    #[test]
    fn reflex_and_episode_state_expires_under_fixed_bounds() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds {
            reflex_ttl_ms: 10,
            episodic_ttl_ms: 20,
            ..MemoryBounds::default()
        })
        .unwrap();
        let graph = graph(5);
        memory
            .observe_scene(&graph, vec![NodeId::new("person:1").unwrap()], 5)
            .unwrap();
        memory
            .remember_episode(
                "person_entered",
                SourceId::new("camera:0").unwrap(),
                vec![NodeId::new("person:1").unwrap()],
                5,
            )
            .unwrap();
        memory.expire(16).unwrap();
        assert_eq!(memory.reflex_nodes().count(), 0);
        assert_eq!(memory.episodes().count(), 1);
        memory.expire(25).unwrap();
        assert_eq!(memory.episodes().count(), 0);
    }

    #[test]
    fn durable_promotion_is_only_a_semantic_candidate() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds::default()).unwrap();
        memory
            .remember_episode(
                "door_opened",
                SourceId::new("camera:0").unwrap(),
                vec![NodeId::new("door:1").unwrap()],
                1,
            )
            .unwrap();
        let candidate = memory.promotion_candidate(0, 7).unwrap();
        assert_eq!(candidate.episode_code, "door_opened");
        assert_eq!(candidate.scene_revision, 7);
    }

    #[test]
    fn ring_capacity_evicts_oldest_semantic_delta() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds {
            max_deltas: 1,
            ..MemoryBounds::default()
        })
        .unwrap();
        let graph = graph(1);
        let id = NodeId::new("person:1").unwrap();
        memory.observe_scene(&graph, vec![id.clone()], 1).unwrap();
        memory.observe_scene(&graph, vec![id], 2).unwrap();
        assert_eq!(memory.deltas().count(), 1);
        assert_eq!(memory.deltas().next().unwrap().observed_at_ms, 2);
    }
}
