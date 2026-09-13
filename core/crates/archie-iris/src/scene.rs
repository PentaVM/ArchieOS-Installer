use crate::geometry::{Affine2D, GeometryError, Rect};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

const MAX_ID_BYTES: usize = 96;
const MAX_LABEL_BYTES: usize = 512;
const MAX_EVENTS_PER_REVISION: usize = 4_096;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SceneError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > MAX_ID_BYTES
                    || !value.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '-' | '_' | ':' | '.' | '/')
                    })
                {
                    return Err(SceneError::InvalidIdentifier);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(SourceId);
identifier!(NodeId);
identifier!(TrackId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Source,
    Display,
    Window,
    Application,
    Document,
    BrowserPage,
    Button,
    Input,
    Link,
    Checkbox,
    Radio,
    Menu,
    MenuItem,
    Tab,
    Dialog,
    Toolbar,
    TreeItem,
    Viewport,
    Icon,
    Image,
    Canvas,
    Text,
    Person,
    Animal,
    Horse,
    Cow,
    Dog,
    Cat,
    Vehicle,
    Drone,
    Food,
    Tool,
    Door,
    Screen,
    Container,
    Object3d,
    UnknownUi,
    UnknownObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Inside,
    Near,
    Holding,
    LookingAt,
    ParentOf,
    ChildOf,
    Selected,
    Focused,
    Contains,
    ConnectedTo,
    Speaking,
    InteractingWith,
    On,
    LabelFor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Relation {
    pub from: NodeId,
    pub kind: RelationKind,
    pub to: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneNode {
    pub id: NodeId,
    pub source: SourceId,
    pub kind: NodeKind,
    pub bbox: Option<Rect>,
    pub parent: Option<NodeId>,
    pub track_id: Option<TrackId>,
    pub confidence_permille: u16,
    pub uncertainty_permille: u16,
    pub last_seen_ms: i64,
    /// Local semantic state. It is never copied into content-free traces.
    pub label: Option<String>,
    pub revision: u64,
}

impl SceneNode {
    pub fn validate(&self) -> Result<(), SceneError> {
        if self.confidence_permille > 1_000
            || self.uncertainty_permille > 1_000
            || self.last_seen_ms < 0
            || self.label.as_ref().is_some_and(|label| {
                label.is_empty() || label.len() > MAX_LABEL_BYTES || label.contains('\0')
            })
            || self.parent.as_ref() == Some(&self.id)
        {
            return Err(SceneError::InvalidNode);
        }
        if let Some(bbox) = self.bbox {
            bbox.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneEvent {
    Upsert(SceneNode),
    Delete(NodeId),
    Relate(Relation),
    Unrelate(Relation),
    TransformSubtree { root: NodeId, transform: Affine2D },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SceneError {
    #[error("scene identifier is invalid")]
    InvalidIdentifier,
    #[error("scene node is invalid")]
    InvalidNode,
    #[error("scene event batch is empty or too large")]
    InvalidBatch,
    #[error("scene revision is stale")]
    StaleRevision,
    #[error("scene revision exhausted")]
    RevisionExhausted,
    #[error("scene node is missing")]
    MissingNode,
    #[error("scene parent would form a cycle")]
    ParentCycle,
    #[error("scene relationship is invalid")]
    InvalidRelation,
    #[error(transparent)]
    Geometry(#[from] GeometryError),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneGraph {
    revision: u64,
    nodes: BTreeMap<NodeId, SceneNode>,
    relations: BTreeSet<Relation>,
}

impl SceneGraph {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn node(&self, id: &NodeId) -> Option<&SceneNode> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.values()
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.iter()
    }

    /// Applies one source delta atomically. A malformed event leaves the graph unchanged.
    pub fn apply(
        &mut self,
        expected_revision: u64,
        events: &[SceneEvent],
    ) -> Result<u64, SceneError> {
        if expected_revision != self.revision {
            return Err(SceneError::StaleRevision);
        }
        if events.is_empty() || events.len() > MAX_EVENTS_PER_REVISION {
            return Err(SceneError::InvalidBatch);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(SceneError::RevisionExhausted)?;
        let mut candidate = self.clone();
        for event in events {
            candidate.apply_event(event, next_revision)?;
        }
        candidate.validate()?;
        candidate.revision = next_revision;
        *self = candidate;
        Ok(next_revision)
    }

    pub fn children(&self, parent: &NodeId) -> Vec<&SceneNode> {
        self.nodes
            .values()
            .filter(|node| node.parent.as_ref() == Some(parent))
            .collect()
    }

    pub fn best_track_match(
        &self,
        source: &SourceId,
        kind: NodeKind,
        bbox: Rect,
        minimum_iou_permille: u16,
    ) -> Option<&SceneNode> {
        self.nodes
            .values()
            .filter(|node| node.source == *source && node.kind == kind && node.track_id.is_some())
            .filter_map(|node| node.bbox.map(|old| (old.iou_permille(&bbox), node)))
            .filter(|(iou, _)| *iou >= minimum_iou_permille)
            .max_by(|(left_iou, left), (right_iou, right)| {
                left_iou.cmp(right_iou).then_with(|| right.id.cmp(&left.id))
            })
            .map(|(_, node)| node)
    }

    fn apply_event(&mut self, event: &SceneEvent, revision: u64) -> Result<(), SceneError> {
        match event {
            SceneEvent::Upsert(node) => {
                let mut node = node.clone();
                node.validate()?;
                node.revision = revision;
                self.nodes.insert(node.id.clone(), node);
            }
            SceneEvent::Delete(id) => {
                if self.nodes.remove(id).is_none() {
                    return Err(SceneError::MissingNode);
                }
                self.relations
                    .retain(|relation| &relation.from != id && &relation.to != id);
                for child in self.nodes.values_mut() {
                    if child.parent.as_ref() == Some(id) {
                        child.parent = None;
                        child.revision = revision;
                    }
                }
            }
            SceneEvent::Relate(relation) => {
                if relation.from == relation.to
                    || !self.nodes.contains_key(&relation.from)
                    || !self.nodes.contains_key(&relation.to)
                {
                    return Err(SceneError::InvalidRelation);
                }
                self.relations.insert(relation.clone());
            }
            SceneEvent::Unrelate(relation) => {
                self.relations.remove(relation);
            }
            SceneEvent::TransformSubtree { root, transform } => {
                let ids = self.subtree(root)?;
                for id in ids {
                    let node = self.nodes.get_mut(&id).ok_or(SceneError::MissingNode)?;
                    if let Some(bbox) = node.bbox {
                        node.bbox = Some(transform.apply_rect(bbox)?);
                        node.revision = revision;
                    }
                }
            }
        }
        Ok(())
    }

    fn subtree(&self, root: &NodeId) -> Result<Vec<NodeId>, SceneError> {
        if !self.nodes.contains_key(root) {
            return Err(SceneError::MissingNode);
        }
        let mut output = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(next) = queue.pop_front() {
            output.push(next.clone());
            queue.extend(
                self.nodes
                    .values()
                    .filter(|node| node.parent.as_ref() == Some(&next))
                    .map(|node| node.id.clone()),
            );
        }
        Ok(output)
    }

    fn validate(&self) -> Result<(), SceneError> {
        for node in self.nodes.values() {
            node.validate()?;
            if let Some(parent) = &node.parent {
                if !self.nodes.contains_key(parent) {
                    return Err(SceneError::MissingNode);
                }
                let mut cursor = Some(parent);
                let mut seen = BTreeSet::from([node.id.clone()]);
                while let Some(id) = cursor {
                    if !seen.insert(id.clone()) {
                        return Err(SceneError::ParentCycle);
                    }
                    cursor = self.nodes.get(id).and_then(|entry| entry.parent.as_ref());
                }
            }
        }
        for relation in &self.relations {
            if relation.from == relation.to
                || !self.nodes.contains_key(&relation.from)
                || !self.nodes.contains_key(&relation.to)
            {
                return Err(SceneError::InvalidRelation);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> NodeId {
        NodeId::new(value).unwrap()
    }

    fn source() -> SourceId {
        SourceId::new("display:0").unwrap()
    }

    fn node(value: &str, kind: NodeKind, bbox: Rect, parent: Option<&str>) -> SceneNode {
        SceneNode {
            id: id(value),
            source: source(),
            kind,
            bbox: Some(bbox),
            parent: parent.map(id),
            track_id: Some(TrackId::new(format!("track:{value}")).unwrap()),
            confidence_permille: 900,
            uncertainty_permille: 100,
            last_seen_ms: 1,
            label: None,
            revision: 0,
        }
    }

    #[test]
    fn revisions_are_atomic_and_stale_writers_are_rejected() {
        let mut graph = SceneGraph::default();
        graph
            .apply(
                0,
                &[SceneEvent::Upsert(node(
                    "window:1",
                    NodeKind::Window,
                    Rect::new(10, 10, 100, 100).unwrap(),
                    None,
                ))],
            )
            .unwrap();
        assert_eq!(graph.revision(), 1);
        assert_eq!(
            graph.apply(0, &[SceneEvent::Delete(id("window:1"))]),
            Err(SceneError::StaleRevision)
        );
        assert!(graph.node(&id("window:1")).is_some());
    }

    #[test]
    fn moving_a_window_transforms_children_without_rediscovery() {
        let mut graph = SceneGraph::default();
        graph
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node(
                        "window:1",
                        NodeKind::Window,
                        Rect::new(100, 100, 400, 300).unwrap(),
                        None,
                    )),
                    SceneEvent::Upsert(node(
                        "button:1",
                        NodeKind::Button,
                        Rect::new(120, 140, 80, 30).unwrap(),
                        Some("window:1"),
                    )),
                ],
            )
            .unwrap();
        graph
            .apply(
                1,
                &[SceneEvent::TransformSubtree {
                    root: id("window:1"),
                    transform: Affine2D::translation(182, 34),
                }],
            )
            .unwrap();
        assert_eq!(
            graph.node(&id("button:1")).unwrap().bbox,
            Some(Rect::new(302, 174, 80, 30).unwrap())
        );
    }

    #[test]
    fn parent_cycles_and_dangling_relationships_leave_graph_unchanged() {
        let mut graph = SceneGraph::default();
        let before = graph.clone();
        let child = node(
            "child",
            NodeKind::Button,
            Rect::new(0, 0, 10, 10).unwrap(),
            Some("missing"),
        );
        assert_eq!(
            graph.apply(0, &[SceneEvent::Upsert(child)]),
            Err(SceneError::MissingNode)
        );
        assert_eq!(graph, before);
    }

    #[test]
    fn track_matching_is_stable_and_geometry_based() {
        let mut graph = SceneGraph::default();
        graph
            .apply(
                0,
                &[SceneEvent::Upsert(node(
                    "horse:12",
                    NodeKind::Horse,
                    Rect::new(100, 100, 100, 100).unwrap(),
                    None,
                ))],
            )
            .unwrap();
        let matched = graph
            .best_track_match(
                &source(),
                NodeKind::Horse,
                Rect::new(105, 100, 100, 100).unwrap(),
                700,
            )
            .unwrap();
        assert_eq!(matched.id.as_str(), "horse:12");
    }
}
