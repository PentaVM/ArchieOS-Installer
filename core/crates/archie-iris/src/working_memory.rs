use crate::{NodeId, NodeKind, Relation, RelationKind, SceneGraph, SceneNode, SourceId, TrackId};
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
    /// How long a relation may go unseen before it counts as ended. A hand hides a phone
    /// for a frame or two; that is not letting go of it.
    pub relation_grace_ms: i64,
    pub max_relation_spans: usize,
}

impl Default for MemoryBounds {
    fn default() -> Self {
        Self {
            reflex_ttl_ms: 5_000,
            episodic_ttl_ms: 300_000,
            max_reflex_nodes: 4_096,
            max_episodes: 1_024,
            max_deltas: 2_048,
            relation_grace_ms: 1_500,
            max_relation_spans: 1_024,
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
            || self.relation_grace_ms < 0
            || self.max_relation_spans == 0
            || self.max_relation_spans > 16_384
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

/// What a node was while a relation held: enough to answer after the node itself expired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Local semantic state, as on [`SceneNode::label`]. It never leaves this memory in a trace.
    pub label: Option<String>,
    pub track_id: Option<TrackId>,
}

impl NodeSnapshot {
    fn of(node: &SceneNode) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind,
            label: node.label.clone(),
            track_id: node.track_id.clone(),
        }
    }
}

/// One stretch of time during which a relation held, such as the owner holding a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationSpan {
    pub kind: RelationKind,
    pub source: SourceId,
    pub subject: NodeSnapshot,
    pub object: NodeSnapshot,
    pub started_at_ms: i64,
    pub last_seen_ms: i64,
    /// `None` while it still holds. Once ended, this is when it was last seen.
    pub ended_at_ms: Option<i64>,
}

impl RelationSpan {
    pub fn is_open(&self) -> bool {
        self.ended_at_ms.is_none()
    }

    fn matches(&self, relation: &Relation) -> bool {
        self.kind == relation.kind
            && self.subject.id == relation.from
            && self.object.id == relation.to
    }
}

/// "What did I have in my hand a moment ago?" is `kind: Holding, subject: owner, since: now - 60 s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationQuery {
    pub kind: RelationKind,
    pub subject: Option<NodeId>,
    /// Spans that ended before this are left out; open spans always count.
    pub since_ms: i64,
    pub limit: usize,
}

/// Relations about the physical scene that are worth remembering. Layout relations of a
/// window, and `Near`, change every frame and would push the rest out of the ring.
fn remembered(kind: RelationKind) -> bool {
    matches!(
        kind,
        RelationKind::Holding
            | RelationKind::LookingAt
            | RelationKind::Speaking
            | RelationKind::InteractingWith
            | RelationKind::On
            | RelationKind::Inside
    )
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
    spans: VecDeque<RelationSpan>,
}

impl WorkingSceneMemory {
    pub fn new(bounds: MemoryBounds) -> Result<Self, WorkingMemoryError> {
        Ok(Self {
            bounds: bounds.validate()?,
            reflex: BTreeMap::new(),
            episodes: VecDeque::new(),
            deltas: VecDeque::new(),
            spans: VecDeque::new(),
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

    /// Opens a span for each remembered relation the scene now holds, keeps the open ones it
    /// still holds, and ends those unseen for longer than the grace period. The snapshot keeps
    /// the newest label, so an object named after it was picked up is recalled by that name.
    pub fn observe_relations(
        &mut self,
        scene: &SceneGraph,
        now_ms: i64,
    ) -> Result<(), WorkingMemoryError> {
        if now_ms < 0 {
            return Err(WorkingMemoryError::InvalidTime);
        }
        for relation in scene
            .relations()
            .filter(|relation| remembered(relation.kind))
        {
            let (Some(subject), Some(object)) =
                (scene.node(&relation.from), scene.node(&relation.to))
            else {
                continue;
            };
            match self
                .spans
                .iter_mut()
                .find(|span| span.is_open() && span.matches(relation))
            {
                Some(span) => {
                    span.last_seen_ms = span.last_seen_ms.max(now_ms);
                    span.subject = NodeSnapshot::of(subject);
                    if object.label.is_some() || span.object.label.is_none() {
                        span.object = NodeSnapshot::of(object);
                    }
                }
                None => self.spans.push_back(RelationSpan {
                    kind: relation.kind,
                    source: object.source.clone(),
                    subject: NodeSnapshot::of(subject),
                    object: NodeSnapshot::of(object),
                    started_at_ms: now_ms,
                    last_seen_ms: now_ms,
                    ended_at_ms: None,
                }),
            }
        }
        let grace = self.bounds.relation_grace_ms;
        for span in self.spans.iter_mut().filter(|span| span.is_open()) {
            if now_ms.saturating_sub(span.last_seen_ms) > grace {
                span.ended_at_ms = Some(span.last_seen_ms);
            }
        }
        while self.spans.len() > self.bounds.max_relation_spans {
            // The oldest ended span goes first; an open one only when nothing has ended.
            let index = self
                .spans
                .iter()
                .position(|span| !span.is_open())
                .unwrap_or(0);
            self.spans.remove(index);
        }
        self.expire(now_ms)
    }

    /// Matching spans, the ones still holding first, then the most recently ended.
    pub fn recall(&self, query: &RelationQuery) -> Vec<&RelationSpan> {
        let mut found: Vec<&RelationSpan> = self
            .spans
            .iter()
            .filter(|span| span.kind == query.kind)
            .filter(|span| {
                query
                    .subject
                    .as_ref()
                    .is_none_or(|subject| span.subject.id == *subject)
            })
            .filter(|span| span.ended_at_ms.is_none_or(|ended| ended >= query.since_ms))
            .collect();
        found.sort_by_key(|span| {
            (
                span.is_open(),
                span.ended_at_ms.unwrap_or(span.last_seen_ms),
                span.started_at_ms,
            )
        });
        found.reverse();
        found.truncate(query.limit);
        found
    }

    pub fn relation_spans(&self) -> impl Iterator<Item = &RelationSpan> {
        self.spans.iter()
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
        let span_cutoff = now_ms.saturating_sub(self.bounds.episodic_ttl_ms);
        self.spans
            .retain(|span| span.ended_at_ms.is_none_or(|ended| ended >= span_cutoff));
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
    use crate::{Rect, SceneEvent};

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

    fn node(id: &str, kind: NodeKind, label: Option<&str>, last_seen_ms: i64) -> SceneNode {
        SceneNode {
            id: NodeId::new(id).unwrap(),
            source: SourceId::new("camera:0").unwrap(),
            kind,
            bbox: Some(Rect::new(0, 0, 10, 10).unwrap()),
            parent: None,
            track_id: Some(TrackId::new(format!("track:{id}")).unwrap()),
            confidence_permille: 900,
            uncertainty_permille: 100,
            last_seen_ms,
            label: label.map(str::to_owned),
            revision: 0,
        }
    }

    fn holding(object: &str) -> Relation {
        Relation {
            from: NodeId::new("person:owner").unwrap(),
            kind: RelationKind::Holding,
            to: NodeId::new(object).unwrap(),
        }
    }

    fn held_by_owner(since_ms: i64) -> RelationQuery {
        RelationQuery {
            kind: RelationKind::Holding,
            subject: Some(NodeId::new("person:owner").unwrap()),
            since_ms,
            limit: 4,
        }
    }

    #[test]
    fn a_phone_put_down_is_recalled_by_name_after_its_node_is_gone() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds::default()).unwrap();
        let mut scene = SceneGraph::default();
        scene
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node("person:owner", NodeKind::Person, None, 1_000)),
                    SceneEvent::Upsert(node(
                        "object:7",
                        NodeKind::UnknownObject,
                        Some("phone"),
                        1_000,
                    )),
                    SceneEvent::Relate(holding("object:7")),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 1_000).unwrap();
        memory.observe_relations(&scene, 9_000).unwrap();
        scene
            .apply(
                1,
                &[
                    SceneEvent::Unrelate(holding("object:7")),
                    SceneEvent::Delete(NodeId::new("object:7").unwrap()),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 9_500).unwrap();
        assert!(
            memory.recall(&held_by_owner(0))[0].is_open(),
            "within the grace period it still holds"
        );
        memory.observe_relations(&scene, 20_000).unwrap();

        let recalled = memory.recall(&held_by_owner(0));
        assert_eq!(recalled.len(), 1);
        let span = recalled[0];
        assert_eq!(span.object.label.as_deref(), Some("phone"));
        assert_eq!((span.started_at_ms, span.ended_at_ms), (1_000, Some(9_000)));
        assert!(
            memory.recall(&held_by_owner(10_000)).is_empty(),
            "it ended before the question's window"
        );
    }

    #[test]
    fn a_flicker_shorter_than_the_grace_is_one_span_and_a_name_learned_later_is_kept() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds::default()).unwrap();
        let mut scene = SceneGraph::default();
        scene
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node("person:owner", NodeKind::Person, None, 0)),
                    SceneEvent::Upsert(node("object:3", NodeKind::UnknownObject, None, 0)),
                    SceneEvent::Relate(holding("object:3")),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 0).unwrap();
        scene
            .apply(1, &[SceneEvent::Unrelate(holding("object:3"))])
            .unwrap();
        memory.observe_relations(&scene, 800).unwrap();
        scene
            .apply(
                2,
                &[
                    SceneEvent::Upsert(node(
                        "object:3",
                        NodeKind::UnknownObject,
                        Some("plush toy"),
                        1_200,
                    )),
                    SceneEvent::Relate(holding("object:3")),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 1_200).unwrap();
        assert_eq!(memory.relation_spans().count(), 1);
        assert_eq!(
            memory
                .relation_spans()
                .next()
                .unwrap()
                .object
                .label
                .as_deref(),
            Some("plush toy")
        );
    }

    #[test]
    fn open_spans_come_first_then_the_most_recently_ended_and_old_ones_expire() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds {
            episodic_ttl_ms: 60_000,
            ..MemoryBounds::default()
        })
        .unwrap();
        let mut scene = SceneGraph::default();
        let mut revision = scene
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node("person:owner", NodeKind::Person, None, 0)),
                    SceneEvent::Upsert(node("object:1", NodeKind::Tool, Some("screwdriver"), 0)),
                    SceneEvent::Upsert(node("object:2", NodeKind::UnknownObject, Some("phone"), 0)),
                    SceneEvent::Relate(holding("object:1")),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 0).unwrap();
        revision = scene
            .apply(
                revision,
                &[
                    SceneEvent::Unrelate(holding("object:1")),
                    SceneEvent::Relate(holding("object:2")),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 5_000).unwrap();
        memory.observe_relations(&scene, 30_000).unwrap();
        let labels: Vec<_> = memory
            .recall(&held_by_owner(0))
            .iter()
            .map(|span| span.object.label.clone().unwrap())
            .collect();
        assert_eq!(labels, ["phone", "screwdriver"]);
        scene
            .apply(revision, &[SceneEvent::Unrelate(holding("object:2"))])
            .unwrap();
        memory.observe_relations(&scene, 70_000).unwrap();
        let labels: Vec<_> = memory
            .recall(&held_by_owner(0))
            .iter()
            .map(|span| span.object.label.clone().unwrap())
            .collect();
        assert_eq!(labels, ["phone"], "the screwdriver ended over a minute ago");
    }

    #[test]
    fn layout_relations_are_not_remembered() {
        let mut memory = WorkingSceneMemory::new(MemoryBounds::default()).unwrap();
        let mut scene = SceneGraph::default();
        scene
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node("window:1", NodeKind::Window, None, 0)),
                    SceneEvent::Upsert(node("button:1", NodeKind::Button, None, 0)),
                    SceneEvent::Relate(Relation {
                        from: NodeId::new("window:1").unwrap(),
                        kind: RelationKind::Contains,
                        to: NodeId::new("button:1").unwrap(),
                    }),
                ],
            )
            .unwrap();
        memory.observe_relations(&scene, 0).unwrap();
        assert_eq!(memory.relation_spans().count(), 0);
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
