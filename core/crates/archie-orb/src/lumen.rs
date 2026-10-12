//! Lumenfield: the TFOG graph read the way the Lumenfield design reads a tree.
//!
//! The orb's entities already form a tree: the system at the root, services, devices and
//! modules under it, and processes nested under the process that started them. Lumenfield
//! draws such a tree as a membrane of regions. Every branch is a module orb, the leaves
//! of one parent are gathered into one group per category until someone opens it, and
//! the relations the model actually observed are drawn between whichever orbs hold their
//! ends.
//!
//! This module decides where everything sits and what it is connected to. It is pure and
//! deterministic: the same frame and view give the same scene, byte for byte, so the
//! renderer can animate towards it without the field jumping between polls. It invents
//! nothing: every node is an observed entity, an aggregate of observed entities (a
//! module's members, a group's leaves), or the root; every relation is an observed edge.
//!
//! Three lenses, as in the design:
//! - `architecture`: modules folded, leaves grouped by category;
//! - `folders`: the first two levels opened;
//! - `entities`: every visible entity as its own point.

use crate::model::{OrbFrame, OrbNode, OrbState, Relation, ResourceProfile, WorkflowView};
use crate::signal::{EntityKind, StageStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

/// Most nodes a composed scene holds, whatever the view asks for.
pub const LUMEN_MODULE_LIMIT: usize = 420;
const MAX_EXPANDED: usize = 256;
const MAX_ID_BYTES: usize = 256;
const MAX_OBSERVED_RELATIONS: usize = 100;
const MAX_SEMANTIC_RELATIONS: usize = 32;
const SEMANTIC_THRESHOLD: f32 = 0.48;
/// Root id used when the frame carries no system entity of its own.
pub const LUMEN_ROOT_ID: &str = "system:archieos";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LumenLens {
    #[default]
    Architecture,
    Folders,
    Entities,
}

impl LumenLens {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "architecture" => Some(Self::Architecture),
            "folders" => Some(Self::Folders),
            "entities" => Some(Self::Entities),
            _ => None,
        }
    }
}

/// `regions` is the Lumenfield layout; `legacy` is the earlier V2 one, kept because the
/// design's "before" comparison draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LumenLayout {
    #[default]
    Regions,
    Legacy,
}

/// The shape controls of the design's composition panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LumenShape {
    pub layout: LumenLayout,
    /// How strongly entities of one category gather ("Separación por tipo").
    pub category_pull: f32,
    /// How strongly children stay with their parent ("Cohesión de carpetas").
    pub hierarchy_pull: f32,
    /// How far depth pushes an entity outwards ("Profundidad radial").
    pub radial_layers: f32,
    /// How much mass pushes an entity outwards ("Influencia de tamaño").
    pub size_influence: f32,
    /// Sphere scale ("Escala de esfera").
    pub scale: f32,
}

impl Default for LumenShape {
    fn default() -> Self {
        Self {
            layout: LumenLayout::Regions,
            category_pull: 0.44,
            hierarchy_pull: 0.59,
            radial_layers: 0.63,
            size_influence: 0.09,
            scale: 1.0,
        }
    }
}

impl LumenShape {
    fn clamped(self) -> Self {
        let unit = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        let d = Self::default();
        Self {
            layout: self.layout,
            category_pull: unit(self.category_pull, 0.0, 1.0, d.category_pull),
            hierarchy_pull: unit(self.hierarchy_pull, 0.0, 1.0, d.hierarchy_pull),
            radial_layers: unit(self.radial_layers, 0.0, 0.85, d.radial_layers),
            size_influence: unit(self.size_influence, 0.0, 0.4, d.size_influence),
            scale: unit(self.scale, 0.4, 2.0, d.scale),
        }
    }
}

/// What the viewer asked to see. Everything here is presentation: it chooses which
/// observed entities are folded together, never what is observed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LumenView {
    pub lens: LumenLens,
    pub expanded: BTreeSet<String>,
    /// Draw only this entity's subtree, with it at the centre.
    pub focus: Option<String>,
    pub shape: LumenShape,
    pub limit: usize,
}

impl Default for LumenView {
    fn default() -> Self {
        Self {
            lens: LumenLens::Architecture,
            expanded: BTreeSet::new(),
            focus: None,
            shape: LumenShape::default(),
            limit: 360,
        }
    }
}

impl LumenView {
    /// Reads a view from a URL query such as
    /// `lens=folders&expand=system%3Aarchieos,service%3Ax&category_pull=0.5`.
    /// Unknown keys are ignored and every value is bounded, so any query gives a view.
    pub fn from_query(query: &str) -> Self {
        let mut view = Self::default();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let number = || decode(value).and_then(|v| v.parse::<f32>().ok());
            match key {
                "lens" => {
                    if let Some(lens) = decode(value).as_deref().and_then(LumenLens::parse) {
                        view.lens = lens;
                    }
                }
                "layout" => {
                    if decode(value).as_deref() == Some("legacy") {
                        view.shape.layout = LumenLayout::Legacy;
                    }
                }
                "focus" => {
                    view.focus = decode(value).filter(|id| valid_id(id));
                }
                "expand" => {
                    for part in value.split(',') {
                        if view.expanded.len() >= MAX_EXPANDED {
                            break;
                        }
                        if let Some(id) = decode(part).filter(|id| valid_id(id)) {
                            view.expanded.insert(id);
                        }
                    }
                }
                "limit" => {
                    if let Some(limit) = number() {
                        if limit.is_finite() {
                            view.limit = (limit as usize).clamp(12, LUMEN_MODULE_LIMIT);
                        }
                    }
                }
                "category_pull" => view.shape.category_pull = number().unwrap_or(f32::NAN),
                "hierarchy_pull" => view.shape.hierarchy_pull = number().unwrap_or(f32::NAN),
                "radial_layers" => view.shape.radial_layers = number().unwrap_or(f32::NAN),
                "size_influence" => view.shape.size_influence = number().unwrap_or(f32::NAN),
                "scale" => view.shape.scale = number().unwrap_or(f32::NAN),
                _ => {}
            }
        }
        view.shape = view.shape.clamped();
        view
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

/// Percent-decoding for one query value; `+` is a space. `None` for broken escapes or
/// bytes that are not UTF-8, so a malformed value is dropped rather than guessed at.
fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let text = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(text, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The design's "types": what kind of thing an entity is, in eight families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LumenCategory {
    Processes,
    Services,
    Devices,
    Interface,
    Cognition,
    Modules,
    Storage,
    System,
}

impl LumenCategory {
    pub const ALL: [Self; 8] = [
        Self::Processes,
        Self::Services,
        Self::Devices,
        Self::Interface,
        Self::Cognition,
        Self::Modules,
        Self::Storage,
        Self::System,
    ];

    pub fn of(kind: EntityKind) -> Self {
        match kind {
            EntityKind::Process => Self::Processes,
            EntityKind::Service => Self::Services,
            EntityKind::Device => Self::Devices,
            EntityKind::App | EntityKind::Window | EntityKind::Control => Self::Interface,
            EntityKind::Model | EntityKind::Memory | EntityKind::Tool => Self::Cognition,
            EntityKind::Module | EntityKind::Library => Self::Modules,
            EntityKind::File | EntityKind::Directory => Self::Storage,
            EntityKind::System | EntityKind::Node | EntityKind::Abstract | EntityKind::Unknown => {
                Self::System
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Processes => "Procesos",
            Self::Services => "Servicios",
            Self::Devices => "Dispositivos",
            Self::Interface => "Interfaz",
            Self::Cognition => "Cognición",
            Self::Modules => "Módulos",
            Self::Storage => "Almacenamiento",
            Self::System => "Sistema",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Processes => "processes",
            Self::Services => "services",
            Self::Devices => "devices",
            Self::Interface => "interface",
            Self::Cognition => "cognition",
            Self::Modules => "modules",
            Self::Storage => "storage",
            Self::System => "system",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(7)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LumenKind {
    /// The centre of the view: the system, or the entity the view is focused on.
    Root,
    /// An entity with children of its own.
    Module,
    /// The leaves of one parent that share a category, folded into one orb.
    Group,
    /// One observed entity.
    Entity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LumenNode {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub kind: LumenKind,
    /// The observed entity's own kind. `None` for a group, which is not one entity.
    pub entity_kind: Option<EntityKind>,
    pub category: LumenCategory,
    /// What the node holds, in words: a category, or the category most of its members are.
    pub role: String,
    /// Observed entities this node stands for, itself included.
    pub member_count: usize,
    pub depth: u32,
    pub position: [f32; 3],
    /// Mass on a 0..1 scale.
    pub size: f32,
    pub activity: f32,
    pub focus: f32,
    pub voice: f32,
    pub uncertainty: f32,
    pub status: Option<StageStatus>,
    pub selected: bool,
    /// Has hidden members a viewer can open.
    pub expandable: bool,
    pub expanded: bool,
    /// Relation kinds observed on this node or its members, most frequent first.
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LumenRelationKind {
    /// Runtime, dependency, action, focus or accessibility edges the model observed.
    Observed,
    /// Entities whose feature vectors are close, and nothing observed between them.
    Semantic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LumenRelation {
    pub source: String,
    pub target: String,
    pub kind: LumenRelationKind,
    /// Observed edges folded into this one.
    pub count: usize,
    pub strength: f32,
    pub relations: Vec<Relation>,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LumenLink {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LumenCount {
    pub category: LumenCategory,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LumenScene {
    pub generated_at_ms: i64,
    pub lens: LumenLens,
    pub root_id: String,
    /// The entity at the centre, which is the root unless the view is focused.
    pub focus_id: String,
    /// From the focus up to the root, for the design's "up" and structure panel.
    pub focus_path: Vec<LumenLink>,
    pub nodes: Vec<LumenNode>,
    /// The tree drawn as links: every node to its parent.
    pub links: Vec<LumenLink>,
    pub relations: Vec<LumenRelation>,
    pub radius: f32,
    pub counts: Vec<LumenCount>,
    /// Entities under the focus in this frame.
    pub source_count: usize,
    /// Of those, how many are folded into a module or group rather than drawn alone.
    pub collapsed_count: usize,
    pub entity_count: usize,
    pub state: OrbState,
    pub profile: ResourceProfile,
    pub workflows: Vec<WorkflowView>,
    pub stale_signals: usize,
}

/// One entity of the frame, placed in the tree.
struct Raw<'a> {
    id: String,
    name: String,
    parent: Option<String>,
    category: LumenCategory,
    depth: u32,
    size: f32,
    direction: Option<[f32; 3]>,
    node: Option<&'a OrbNode>,
}

struct Tree<'a> {
    raw: Vec<Raw<'a>>,
    index: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
    root: String,
}

impl<'a> Tree<'a> {
    fn build(frame: &'a OrbFrame) -> Option<Self> {
        if frame.nodes.is_empty() {
            return None;
        }
        let ids: BTreeSet<&str> = frame.nodes.iter().map(|n| n.id.as_str()).collect();
        let declared_root = frame
            .nodes
            .iter()
            .filter(|n| n.kind == EntityKind::System && n.parent_id.is_none())
            .map(|n| n.id.clone())
            .min();
        let root = declared_root.unwrap_or_else(|| LUMEN_ROOT_ID.to_string());
        let mut raw = Vec::with_capacity(frame.nodes.len() + 1);
        if !ids.contains(root.as_str()) {
            raw.push(Raw {
                id: root.clone(),
                name: "ArchieOS".into(),
                parent: None,
                category: LumenCategory::System,
                depth: 0,
                size: 1.0,
                direction: None,
                node: None,
            });
        }
        let mut nodes: Vec<&OrbNode> = frame.nodes.iter().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        nodes.dedup_by(|a, b| a.id == b.id);
        for node in nodes {
            let parent = if node.id == root {
                None
            } else {
                Some(
                    node.parent_id
                        .as_ref()
                        .filter(|p| **p != node.id && ids.contains(p.as_str()))
                        .cloned()
                        .unwrap_or_else(|| root.clone()),
                )
            };
            let direction = finite3(node.direction).filter(|d| length(*d) > 0.5);
            raw.push(Raw {
                id: node.id.clone(),
                name: if node.label.trim().is_empty() {
                    node.id.clone()
                } else {
                    node.label.clone()
                },
                parent,
                category: LumenCategory::of(node.kind),
                depth: 0,
                size: finite(node.mass).map_or(0.0, |m| (m.max(0.0) / (1.0 + m.max(0.0))).min(1.0)),
                direction,
                node: Some(node),
            });
        }
        let index: HashMap<String, usize> = raw
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        let mut tree = Self {
            raw,
            index,
            children: HashMap::new(),
            root,
        };
        tree.relink();
        Some(tree)
    }

    /// Rebuilds the children lists and depths from the root. A parent chain that never
    /// reaches the root (a cycle in reported parents) is cut where it is first found and
    /// hung from the root, so every entity is placed exactly once.
    fn relink(&mut self) {
        loop {
            self.children.clear();
            for (i, item) in self.raw.iter().enumerate() {
                if let Some(parent) = &item.parent {
                    self.children.entry(parent.clone()).or_default().push(i);
                }
            }
            for list in self.children.values_mut() {
                list.sort_by(|a, b| self.raw[*a].id.cmp(&self.raw[*b].id));
            }
            let mut reached = vec![false; self.raw.len()];
            let mut queue = VecDeque::new();
            if let Some(&root) = self.index.get(&self.root) {
                reached[root] = true;
                self.raw[root].depth = 0;
                queue.push_back(root);
            }
            while let Some(at) = queue.pop_front() {
                let depth = self.raw[at].depth;
                for &child in self.children.get(&self.raw[at].id).into_iter().flatten() {
                    if !reached[child] {
                        reached[child] = true;
                        self.raw[child].depth = depth + 1;
                        queue.push_back(child);
                    }
                }
            }
            match reached.iter().position(|r| !r) {
                None => return,
                Some(orphan) => self.raw[orphan].parent = Some(self.root.clone()),
            }
        }
    }

    fn kids(&self, id: &str) -> &[usize] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    fn is_branch(&self, id: &str) -> bool {
        !self.kids(id).is_empty()
    }

    fn subtree(&self, id: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.index.get(id).copied().into_iter().collect();
        while let Some(at) = stack.pop() {
            out.push(at);
            stack.extend(self.kids(&self.raw[at].id).iter().rev());
        }
        out
    }
}

/// A node on its way into the field.
#[derive(Clone)]
struct Entry {
    id: String,
    name: String,
    parent: Option<String>,
    kind: LumenKind,
    branch: bool,
    category: LumenCategory,
    role: String,
    depth: u32,
    size: f32,
    member_count: usize,
    direction: Option<[f32; 3]>,
    source: Option<usize>,
    members: Vec<usize>,
}

/// Composes the scene for `view` from `frame`.
pub fn compose(frame: &OrbFrame, view: &LumenView) -> LumenScene {
    let shape = view.shape.clamped();
    let limit = view.limit.clamp(12, LUMEN_MODULE_LIMIT);
    let mut scene = LumenScene {
        generated_at_ms: frame.generated_at_ms,
        lens: view.lens,
        root_id: LUMEN_ROOT_ID.into(),
        focus_id: LUMEN_ROOT_ID.into(),
        focus_path: Vec::new(),
        nodes: Vec::new(),
        links: Vec::new(),
        relations: Vec::new(),
        radius: field_radius(1, shape.scale),
        counts: LumenCategory::ALL
            .iter()
            .map(|c| LumenCount {
                category: *c,
                label: c.label().into(),
                count: 0,
            })
            .collect(),
        source_count: 0,
        collapsed_count: 0,
        entity_count: frame.entity_count,
        state: frame.state,
        profile: frame.profile,
        workflows: frame.workflows.clone(),
        stale_signals: frame.stale_signals,
    };
    let Some(tree) = Tree::build(frame) else {
        return scene;
    };
    scene.root_id = tree.root.clone();
    let focus = view
        .focus
        .as_ref()
        .filter(|id| tree.index.contains_key(*id))
        .cloned()
        .unwrap_or_else(|| tree.root.clone());
    scene.focus_id = focus.clone();
    let mut up = focus.clone();
    while let Some(parent) = tree
        .index
        .get(&up)
        .and_then(|&i| tree.raw[i].parent.clone())
    {
        scene.focus_path.push(LumenLink {
            source: parent.clone(),
            target: up,
        });
        up = parent;
    }

    let within = tree.subtree(&focus);
    let base_depth = tree.index.get(&focus).map_or(0, |&i| tree.raw[i].depth);
    scene.source_count = within.len();
    for &at in &within {
        scene.counts[tree.raw[at].category.index()].count += 1;
    }

    let entries = match view.lens {
        LumenLens::Entities => entity_entries(&tree, &within, &focus, base_depth, limit),
        LumenLens::Architecture | LumenLens::Folders => {
            let mut open = view.expanded.clone();
            if view.lens == LumenLens::Folders {
                for &at in &within {
                    let item = &tree.raw[at];
                    if item.depth <= base_depth + 1 && tree.is_branch(&item.id) {
                        open.insert(item.id.clone());
                    }
                }
            }
            architecture_entries(&tree, &focus, base_depth, &open, limit)
        }
    };

    let positions = layout(&entries, &shape, scene.source_count);
    scene.radius = positions.radius;
    let visible: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();

    // Which drawn node stands for each observed entity.
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        if let Some(source) = entry.source {
            owner.insert(tree.raw[source].id.as_str(), i);
        }
        for &member in &entry.members {
            owner.entry(tree.raw[member].id.as_str()).or_insert(i);
        }
    }
    let owner_of = |id: &str| -> Option<usize> {
        let mut at = id;
        loop {
            if let Some(&i) = owner.get(at) {
                return Some(i);
            }
            let parent = tree
                .index
                .get(at)
                .and_then(|&i| tree.raw[i].parent.as_deref())?;
            at = parent;
        }
    };

    // Relations: observed edges folded onto the orbs holding their ends, then semantic
    // neighbours where nothing was observed.
    let mut observed: BTreeMap<(usize, usize), LumenRelation> = BTreeMap::new();
    let mut semantic: BTreeMap<(usize, usize), LumenRelation> = BTreeMap::new();
    let mut signals: Vec<BTreeMap<&'static str, usize>> = vec![BTreeMap::new(); entries.len()];
    for edge in &frame.edges {
        let (Some(a), Some(b)) = (owner_of(&edge.source), owner_of(&edge.target)) else {
            continue;
        };
        for relation in &edge.relations {
            let name = relation_name(*relation);
            *signals[a].entry(name).or_default() += 1;
            if a != b {
                *signals[b].entry(name).or_default() += 1;
            }
        }
        if a == b {
            continue;
        }
        let structural: Vec<Relation> = edge
            .relations
            .iter()
            .copied()
            .filter(|r| {
                matches!(
                    r,
                    Relation::Runtime
                        | Relation::Dependency
                        | Relation::Action
                        | Relation::Focus
                        | Relation::Accessibility
                )
            })
            .collect();
        let strength = finite(edge.similarity).unwrap_or(0.0).clamp(0.0, 1.0);
        if !structural.is_empty() {
            let relation = observed.entry((a, b)).or_insert_with(|| LumenRelation {
                source: entries[a].id.clone(),
                target: entries[b].id.clone(),
                kind: LumenRelationKind::Observed,
                count: 0,
                strength: 0.0,
                relations: Vec::new(),
                evidence: format!("{} → {}", edge.source, edge.target),
            });
            relation.count += 1;
            relation.strength = relation.strength.max(strength);
            for r in structural {
                if !relation.relations.contains(&r) {
                    relation.relations.push(r);
                }
            }
        } else if edge.relations.contains(&Relation::Semantic) && strength >= SEMANTIC_THRESHOLD {
            let key = (a.min(b), a.max(b));
            let relation = semantic.entry(key).or_insert_with(|| LumenRelation {
                source: entries[key.0].id.clone(),
                target: entries[key.1].id.clone(),
                kind: LumenRelationKind::Semantic,
                count: 0,
                strength: 0.0,
                relations: vec![Relation::Semantic],
                evidence: String::new(),
            });
            relation.count += 1;
            relation.strength = relation.strength.max(strength);
        }
    }
    let mut observed: Vec<LumenRelation> = observed.into_values().collect();
    observed.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.target.cmp(&b.target))
    });
    observed.truncate(MAX_OBSERVED_RELATIONS);
    let linked: BTreeSet<(&str, &str)> = observed
        .iter()
        .flat_map(|r| {
            [
                (r.source.as_str(), r.target.as_str()),
                (r.target.as_str(), r.source.as_str()),
            ]
        })
        .collect();
    let mut inferred: Vec<LumenRelation> = semantic
        .into_values()
        .filter(|r| !linked.contains(&(r.source.as_str(), r.target.as_str())))
        .map(|mut r| {
            r.evidence = format!(
                "{} {} con vectores cercanos · similitud {:.2}",
                r.count,
                if r.count == 1 { "par" } else { "pares" },
                r.strength
            );
            r
        })
        .collect();
    inferred.sort_by(|a, b| {
        b.strength
            .total_cmp(&a.strength)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.target.cmp(&b.target))
    });
    inferred.truncate(MAX_SEMANTIC_RELATIONS);
    scene.relations = observed;
    scene.relations.extend(inferred);

    // Drawn nodes, with the live state of what they stand for.
    for (i, entry) in entries.iter().enumerate() {
        let members: Vec<&OrbNode> = entry
            .source
            .iter()
            .chain(entry.members.iter())
            .filter_map(|&m| tree.raw[m].node)
            .collect();
        let most = |f: fn(&OrbNode) -> f32| {
            members
                .iter()
                .filter_map(|n| finite(f(n)))
                .fold(0.0_f32, f32::max)
                .clamp(0.0, 1.0)
        };
        let status = members
            .iter()
            .filter_map(|n| n.action_status)
            .max_by_key(|s| status_rank(*s));
        let mut signal_list: Vec<(&'static str, usize)> =
            signals[i].iter().map(|(k, v)| (*k, *v)).collect();
        signal_list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let expandable = matches!(entry.kind, LumenKind::Module | LumenKind::Group);
        scene.nodes.push(LumenNode {
            id: entry.id.clone(),
            name: entry.name.clone(),
            parent: entry.parent.clone(),
            kind: entry.kind,
            entity_kind: entry
                .source
                .and_then(|s| tree.raw[s].node.map(|n| n.kind))
                .or(if entry.kind == LumenKind::Root {
                    Some(EntityKind::System)
                } else {
                    None
                }),
            category: entry.category,
            role: entry.role.clone(),
            member_count: entry.member_count,
            depth: entry.depth,
            position: positions.at[i],
            size: entry.size,
            activity: most(|n| n.activity),
            focus: most(|n| n.focus_score),
            voice: most(|n| n.voice_overlap),
            uncertainty: most(|n| n.uncertainty),
            status,
            selected: members.iter().any(|n| n.selected),
            expandable: expandable && entry.member_count > 1,
            expanded: view.expanded.contains(&entry.id)
                || (view.lens == LumenLens::Folders
                    && entry.kind == LumenKind::Module
                    && entry.depth <= 1),
            signals: signal_list
                .into_iter()
                .take(8)
                .map(|(k, _)| k.to_string())
                .collect(),
        });
        if let Some(parent) = entry
            .parent
            .as_ref()
            .filter(|p| visible.contains_key(p.as_str()))
        {
            scene.links.push(LumenLink {
                source: parent.clone(),
                target: entry.id.clone(),
            });
        }
    }
    scene.collapsed_count = scene.source_count.saturating_sub(
        entries
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    LumenKind::Entity | LumenKind::Module | LumenKind::Root
                )
            })
            .count(),
    );
    scene
}

fn status_rank(status: StageStatus) -> u8 {
    match status {
        StageStatus::Error => 3,
        StageStatus::Active => 2,
        StageStatus::Predicted => 1,
        StageStatus::Verified => 0,
    }
}

fn relation_name(relation: Relation) -> &'static str {
    match relation {
        Relation::Semantic => "semántica",
        Relation::Hierarchy => "jerarquía",
        Relation::Runtime => "ejecución",
        Relation::Focus => "foco",
        Relation::Action => "acción",
        Relation::Accessibility => "accesibilidad",
        Relation::Dependency => "dependencia",
    }
}

fn entity_entries(
    tree: &Tree<'_>,
    within: &[usize],
    focus: &str,
    base_depth: u32,
    limit: usize,
) -> Vec<Entry> {
    // Every entity, as the original V2 view drew one point per index entry. The frame is
    // already bounded by the model's visible budget; the scene keeps its own limit too,
    // choosing breadth first so the focus and the levels nearest it are never dropped.
    let mut order: Vec<usize> = within.to_vec();
    order.sort_by(|a, b| {
        tree.raw[*a]
            .depth
            .cmp(&tree.raw[*b].depth)
            .then_with(|| tree.raw[*a].id.cmp(&tree.raw[*b].id))
    });
    order.truncate(LUMEN_MODULE_LIMIT.max(limit));
    let kept: BTreeSet<usize> = order.iter().copied().collect();
    order
        .into_iter()
        .map(|at| {
            let item = &tree.raw[at];
            let branch = tree.is_branch(&item.id);
            let is_focus = item.id == focus;
            Entry {
                id: item.id.clone(),
                name: item.name.clone(),
                parent: if is_focus {
                    None
                } else {
                    item.parent
                        .clone()
                        .filter(|p| tree.index.get(p).is_some_and(|i| kept.contains(i)))
                },
                kind: if is_focus {
                    LumenKind::Root
                } else {
                    LumenKind::Entity
                },
                branch,
                category: item.category,
                role: item.category.label().into(),
                depth: item.depth - base_depth,
                size: item.size,
                member_count: 1,
                direction: item.direction,
                source: Some(at),
                members: Vec::new(),
            }
        })
        .collect()
}

fn architecture_entries(
    tree: &Tree<'_>,
    focus: &str,
    base_depth: u32,
    open: &BTreeSet<String>,
    limit: usize,
) -> Vec<Entry> {
    let Some(&root_at) = tree.index.get(focus) else {
        return Vec::new();
    };
    let root_item = &tree.raw[root_at];
    let subtree_members = |id: &str| tree.subtree(id);
    let dominant = |members: &[usize]| -> LumenCategory {
        let mut counts = [0usize; 8];
        for &m in members {
            counts[tree.raw[m].category.index()] += 1;
        }
        let best = (0..8)
            .max_by(|a, b| counts[*a].cmp(&counts[*b]).then_with(|| b.cmp(a)))
            .unwrap_or(7);
        LumenCategory::ALL[best]
    };
    let mean_direction = |members: &[usize]| -> Option<[f32; 3]> {
        let mut sum = [0.0_f32; 3];
        let mut any = false;
        for &m in members {
            if let Some(d) = tree.raw[m].direction {
                sum = add(sum, d);
                any = true;
            }
        }
        any.then(|| normalize(sum)).filter(|d| length(*d) > 0.5)
    };

    let mut entries = Vec::new();
    let everything = subtree_members(focus);
    entries.push(Entry {
        id: root_item.id.clone(),
        name: root_item.name.clone(),
        parent: None,
        kind: LumenKind::Root,
        branch: true,
        category: root_item.category,
        role: "Sistema".into(),
        depth: 0,
        size: 1.0,
        member_count: everything.len(),
        direction: None,
        source: Some(root_at),
        members: Vec::new(),
    });
    let mut queue = VecDeque::from([root_at]);
    'outer: while let Some(parent_at) = queue.pop_front() {
        if entries.len() >= limit {
            break;
        }
        let parent = &tree.raw[parent_at];
        if parent.id != focus && !open.contains(&parent.id) {
            continue;
        }
        let parent_depth = parent.depth - base_depth;
        let siblings = tree.kids(&parent.id);
        for &at in siblings
            .iter()
            .filter(|&&at| tree.is_branch(&tree.raw[at].id))
        {
            let item = &tree.raw[at];
            let members = subtree_members(&item.id);
            let hidden: Vec<usize> = if open.contains(&item.id) {
                Vec::new()
            } else {
                members.iter().copied().filter(|&m| m != at).collect()
            };
            let category = dominant(&members);
            if entries.len() >= limit {
                break 'outer;
            }
            entries.push(Entry {
                id: item.id.clone(),
                name: item.name.clone(),
                parent: Some(parent.id.clone()),
                kind: LumenKind::Module,
                branch: true,
                category: item.category,
                role: category.label().into(),
                depth: parent_depth + 1,
                size: item.size.max(
                    ((members.len() as f32).ln_1p() / (tree.raw.len().max(2) as f32).ln_1p())
                        .min(1.0),
                ),
                member_count: members.len(),
                direction: item.direction.or_else(|| mean_direction(&members)),
                source: Some(at),
                members: hidden,
            });
            queue.push_back(at);
        }
        let mut groups: BTreeMap<LumenCategory, Vec<usize>> = BTreeMap::new();
        for &at in siblings
            .iter()
            .filter(|&&at| !tree.is_branch(&tree.raw[at].id))
        {
            groups.entry(tree.raw[at].category).or_default().push(at);
        }
        for (category, leaves) in groups {
            if entries.len() >= limit {
                break 'outer;
            }
            let id = group_id(&parent.id, category);
            let open_group = open.contains(&id);
            entries.push(Entry {
                id: id.clone(),
                name: format!("{} · {}", category.label(), leaves.len()),
                parent: Some(parent.id.clone()),
                kind: LumenKind::Group,
                branch: true,
                category,
                role: category.label().into(),
                depth: parent_depth + 1,
                size: leaves.iter().map(|&l| tree.raw[l].size).fold(0.0, f32::max),
                member_count: leaves.len(),
                direction: mean_direction(&leaves),
                source: None,
                members: if open_group {
                    Vec::new()
                } else {
                    leaves.clone()
                },
            });
            if open_group {
                for &at in &leaves {
                    if entries.len() >= limit {
                        break 'outer;
                    }
                    let item = &tree.raw[at];
                    entries.push(Entry {
                        id: item.id.clone(),
                        name: item.name.clone(),
                        parent: Some(id.clone()),
                        kind: LumenKind::Entity,
                        branch: false,
                        category: item.category,
                        role: category.label().into(),
                        depth: parent_depth + 1,
                        size: item.size,
                        member_count: 1,
                        direction: item.direction,
                        source: Some(at),
                        members: Vec::new(),
                    });
                }
            }
        }
    }
    entries
}

/// The id of the group holding `parent`'s leaves of one category.
pub fn group_id(parent: &str, category: LumenCategory) -> String {
    format!("@grupo:{}:{}", parent, category.key())
}

// --- The field: where every node sits. A port of the design's layout, with the TFOG
// spectral direction standing in for the design's content signature wherever the model
// has one, so that entities the model found related sit near each other. ---

const DIMS: usize = 8 + 4;
const AXES: [[f32; DIMS]; 3] = [
    [
        1.2, -0.85, 0.52, -0.52, 0.96, -1.06, 0.16, -0.34, 0.16, 0.28, -0.2, 0.12,
    ],
    [
        -0.4, 0.88, -1.2, 0.46, 0.82, 0.2, -0.62, 0.58, -0.17, 0.1, 0.26, 0.08,
    ],
    [
        0.8, 0.42, -0.24, -1.14, -0.15, 1.14, -0.58, -0.44, 0.12, 0.11, -0.12, 0.31,
    ],
];
const GOLDEN_ANGLE: f32 = 2.399_963_2;

struct Placed {
    at: Vec<[f32; 3]>,
    radius: f32,
}

fn field_radius(total: usize, scale: f32) -> f32 {
    (72.0 + (total.max(1) as f32).cbrt() * 17.0).min(430.0) * scale
}

fn fnv(text: &str) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

fn token(entry: &Entry, max_depth: u32) -> [f32; DIMS] {
    let mut value = [0.0; DIMS];
    value[entry.category.index()] = 1.0;
    value[8] = entry.size;
    value[9] = entry.depth as f32 / max_depth.max(1) as f32;
    value[10] = ((fnv(&entry.name.to_lowercase()) & 1023) as f32 / 1023.0 - 0.5) * 0.32;
    value[11] = if entry.branch { 0.5 } else { 0.0 };
    value
}

fn project(value: &[f32; DIMS]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (axis, o) in AXES.iter().zip(out.iter_mut()) {
        *o = axis.iter().zip(value).map(|(c, v)| c * v).sum();
    }
    out
}

fn golden(index: usize, count: usize) -> [f32; 3] {
    let z = 1.0 - 2.0 * (index as f32 + 0.5) / count.max(1) as f32;
    let angle = index as f32 * GOLDEN_ANGLE;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    normalize([angle.cos() * ring, angle.sin() * ring, z])
}

fn layout(entries: &[Entry], shape: &LumenShape, total: usize) -> Placed {
    let radius = field_radius(total, shape.scale);
    let count = entries.len();
    let mut at = vec![[0.0_f32; 3]; count];
    if count == 0 {
        return Placed { at, radius };
    }
    let index: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let parent = entry.parent.as_deref().and_then(|p| index.get(p).copied());
        children.entry(parent).or_default().push(i);
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| entries[*a].id.cmp(&entries[*b].id));
    }
    let max_depth = entries.iter().map(|e| e.depth).max().unwrap_or(1).max(1);

    // Subtree sizes, deepest first.
    let mut subtree = vec![1usize; count];
    let mut by_depth: Vec<usize> = (0..count).collect();
    by_depth.sort_by(|a, b| entries[*b].depth.cmp(&entries[*a].depth));
    for &i in &by_depth {
        if let Some(parent) = entries[i].parent.as_deref().and_then(|p| index.get(p)) {
            if *parent != i {
                subtree[*parent] += subtree[i];
            }
        }
    }

    // Signatures: leaves from their own token, branches from their children's.
    let mut signature: Vec<[f32; DIMS]> = entries.iter().map(|e| token(e, max_depth)).collect();
    for &i in &by_depth {
        if !entries[i].branch {
            continue;
        }
        let kids = children.get(&Some(i)).map_or(&[][..], Vec::as_slice);
        let mut aggregate = [0.0; DIMS];
        let mut weight_sum = 0.0;
        for &k in kids {
            let w = if entries[k].branch { 1.5 } else { 1.0 };
            weight_sum += w;
            for (sum, value) in aggregate.iter_mut().zip(&signature[k]) {
                *sum += value * w;
            }
        }
        if weight_sum > 0.0 {
            for value in &mut aggregate {
                *value /= weight_sum;
            }
            aggregate[11] = 0.5;
            signature[i] = aggregate;
        }
    }
    let leaves: Vec<usize> = (0..count).filter(|&i| !entries[i].branch).collect();
    let mut centroid = [0.0; DIMS];
    for &i in &leaves {
        for (sum, value) in centroid.iter_mut().zip(&signature[i]) {
            *sum += value;
        }
    }
    if !leaves.is_empty() {
        for value in &mut centroid {
            *value /= leaves.len() as f32;
        }
    }

    let roots = children.get(&None).cloned().unwrap_or_default();
    let top: Vec<usize> = {
        let mut top: Vec<usize> = roots
            .iter()
            .flat_map(|r| children.get(&Some(*r)).cloned().unwrap_or_default())
            .collect();
        top.sort_by(|a, b| {
            subtree[*b]
                .cmp(&subtree[*a])
                .then_with(|| entries[*a].id.cmp(&entries[*b].id))
        });
        top
    };
    let region: HashMap<usize, usize> = top.iter().enumerate().map(|(r, &i)| (i, r)).collect();
    let mut direction = vec![[0.0_f32, 0.0, 1.0]; count];
    let mut placed = vec![false; count];
    let mut queue: VecDeque<usize> = roots.iter().copied().collect();
    for &r in &roots {
        placed[r] = true;
    }
    while let Some(parent) = queue.pop_front() {
        let parent_is_root = entries[parent].parent.is_none();
        let parent_direction = direction[parent];
        let kids = children.get(&Some(parent)).cloned().unwrap_or_default();
        for (k, &i) in kids.iter().enumerate() {
            if placed[i] {
                continue;
            }
            placed[i] = true;
            let entry = &entries[i];
            let mut centered = signature[i];
            for d in 0..DIMS {
                centered[d] -= centroid[d] * 0.62;
            }
            let mut semantic = project(&centered);
            if let Some(model) = entry.direction {
                // The model's own direction outweighs the design's fixed projection:
                // it comes from the relations actually observed.
                semantic = add(scale3(normalize(semantic), 0.45), scale3(model, 1.1));
            }
            let seed = fnv(&entry.id);
            let jitter = normalize([
                (seed & 255) as f32 / 127.5 - 1.0,
                ((seed >> 8) & 255) as f32 / 127.5 - 1.0,
                ((seed >> 16) & 255) as f32 / 127.5 - 1.0,
            ]);
            let signal = 0.2 + shape.category_pull * 0.55;
            let spread = 0.42 + (1.0 - shape.category_pull) * 0.9;
            let data = normalize(add(scale3(semantic, signal), scale3(jitter, spread)));
            let chosen = match shape.layout {
                LumenLayout::Legacy => blend(
                    data,
                    parent_direction,
                    if parent_is_root {
                        shape.hierarchy_pull * 0.24
                    } else {
                        shape.hierarchy_pull
                    },
                ),
                LumenLayout::Regions if parent_is_root => blend(
                    golden(region.get(&i).copied().unwrap_or(k), top.len()),
                    data,
                    shape.category_pull * 0.26,
                ),
                LumenLayout::Regions => {
                    let mass = subtree[parent] as f32;
                    let width = (mass.ln_1p() / (count.max(2) as f32).ln_1p()).min(1.0);
                    let cohesion = (shape.hierarchy_pull * (1.0 - width * 0.45)).max(0.12);
                    blend(data, parent_direction, cohesion)
                }
            };
            let depth_fraction = entry.depth as f32 / max_depth as f32;
            let size_term = entry.size * shape.size_influence;
            let wobble = ((seed >> 24) as f32 / 255.0) - 0.5;
            let shell = match shape.layout {
                LumenLayout::Legacy => {
                    (0.22 + depth_fraction * shape.radial_layers + size_term + wobble * 0.05
                        - if entry.branch { 0.06 } else { 0.0 })
                    .clamp(0.16, 0.94)
                }
                LumenLayout::Regions => (0.19
                    + depth_fraction.sqrt() * shape.radial_layers * 0.57
                    + size_term * 0.2
                    + wobble * 0.17
                    - if entry.branch {
                        (subtree[i] as f32).ln_1p() / (count.max(2) as f32).ln_1p() * 0.1
                    } else {
                        0.0
                    })
                .clamp(0.12, 0.9),
            };
            direction[i] = chosen;
            at[i] = scale3(chosen, radius * shell);
            queue.push_back(i);
        }
    }
    for i in 0..count {
        if !placed[i] {
            let seed = fnv(&entries[i].id);
            let d = normalize([
                (seed & 255) as f32 - 128.0,
                ((seed >> 8) & 255) as f32 - 128.0,
                ((seed >> 16) & 255) as f32 - 128.0,
            ]);
            at[i] = scale3(d, radius * 0.58);
        }
    }
    separate(entries, &mut at, radius);
    Placed { at, radius }
}

/// Pushes overlapping orbs apart, deterministically: no random step between refreshes.
fn separate(entries: &[Entry], at: &mut [[f32; 3]], radius: f32) {
    let movable: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].parent.is_some())
        .collect();
    for _ in 0..12 {
        for (n, &i) in movable.iter().enumerate() {
            for &j in &movable[n + 1..] {
                let mut delta = sub(at[j], at[i]);
                let mut distance = length(delta);
                let ideal = (if entries[i].branch { 9.0 } else { 5.0 })
                    + (if entries[j].branch { 9.0 } else { 5.0 });
                if distance >= ideal {
                    continue;
                }
                if distance < 0.001 {
                    delta = [1.0, ((j % 5) as f32 - 2.0) * 0.3, 0.7];
                    distance = length(delta);
                }
                let push = (ideal - distance) * 0.16 / distance;
                at[i] = sub(at[i], scale3(delta, push));
                at[j] = add(at[j], scale3(delta, push));
            }
        }
    }
    for p in at.iter_mut() {
        let distance = length(*p);
        if distance > radius * 0.94 {
            *p = scale3(*p, radius * 0.94 / distance);
        }
    }
}

fn finite(v: f32) -> Option<f32> {
    v.is_finite().then_some(v)
}

fn finite3(v: [f32; 3]) -> Option<[f32; 3]> {
    v.iter().all(|c| c.is_finite()).then_some(v)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > f32::EPSILON {
        scale3(a, 1.0 / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn blend(a: [f32; 3], b: [f32; 3], weight: f32) -> [f32; 3] {
    normalize(add(scale3(a, 1.0 - weight), scale3(b, weight)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Morphotype, OrbEdge, OrbField};

    fn node(id: &str, parent: Option<&str>, kind: EntityKind) -> OrbNode {
        let seed = fnv(id) as f32;
        OrbNode {
            id: id.into(),
            parent_id: parent.map(Into::into),
            kind,
            label: id.split(':').next_back().unwrap_or(id).into(),
            direction: normalize([seed.sin(), (seed * 0.7).cos(), (seed * 1.3).sin()]),
            position: [0.0; 3],
            velocity: [0.0; 3],
            mass: 0.5,
            render_radius: 0.02,
            focus_score: 0.0,
            voice_overlap: 0.0,
            uncertainty: 0.1,
            activity: 0.2,
            shell_factor: 1.0,
            selected: false,
            action_status: None,
        }
    }

    fn edge(a: &str, b: &str, similarity: f32, relations: &[Relation]) -> OrbEdge {
        OrbEdge {
            source: a.into(),
            target: b.into(),
            similarity,
            relations: relations.to_vec(),
        }
    }

    /// The system, two services with processes under them, a device, and one process
    /// that started another.
    fn frame() -> OrbFrame {
        let root = "system:archieos";
        let mut nodes = vec![
            node(root, None, EntityKind::System),
            node("service:a", Some(root), EntityKind::Service),
            node("service:b", Some(root), EntityKind::Service),
            node("device:gpu", Some(root), EntityKind::Device),
            node("device:disk", Some(root), EntityKind::Device),
            node("process:1", Some(root), EntityKind::Process),
            node("process:2", Some("process:1"), EntityKind::Process),
        ];
        for i in 0..6 {
            nodes.push(node(
                &format!("process:a{i}"),
                Some("service:a"),
                EntityKind::Process,
            ));
            nodes.push(node(
                &format!("process:b{i}"),
                Some("service:b"),
                EntityKind::Process,
            ));
        }
        OrbFrame {
            generated_at_ms: 1,
            state: OrbState::default(),
            nodes,
            edges: vec![
                edge("process:a1", "process:b2", 0.3, &[Relation::Runtime]),
                edge("process:a2", "process:b3", 0.4, &[Relation::Dependency]),
                edge("process:a1", "service:a", 0.9, &[Relation::Hierarchy]),
                edge("device:gpu", "device:disk", 0.8, &[Relation::Semantic]),
            ],
            field: OrbField {
                degree: 0,
                coefficients: Vec::new(),
                vertices: Vec::new(),
                triangle_indices: Vec::new(),
            },
            profile: ResourceProfile::Low,
            entity_count: 19,
            morphotype: Morphotype::default(),
            source_count: 1,
            stale_signals: 0,
            workflows: Vec::new(),
        }
    }

    fn ids(scene: &LumenScene) -> BTreeSet<&str> {
        scene.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn architecture_folds_modules_and_groups_leaves_by_category() {
        let scene = compose(&frame(), &LumenView::default());
        let ids = ids(&scene);
        assert!(ids.contains("system:archieos"));
        assert!(ids.contains("service:a") && ids.contains("service:b"));
        assert!(
            ids.contains("process:1"),
            "a process with a child is a module"
        );
        assert!(ids.contains(group_id("system:archieos", LumenCategory::Devices).as_str()));
        assert!(
            !ids.contains("process:a1"),
            "a folded service hides its processes"
        );
        let service = scene.nodes.iter().find(|n| n.id == "service:a").unwrap();
        assert_eq!(service.kind, LumenKind::Module);
        assert_eq!(service.member_count, 7);
        assert!(service.expandable && !service.expanded);
        let devices = scene
            .nodes
            .iter()
            .find(|n| n.kind == LumenKind::Group)
            .unwrap();
        assert_eq!(devices.name, "Dispositivos · 2");
        assert_eq!(scene.source_count, 19);
        assert_eq!(
            scene.collapsed_count,
            19 - 4,
            "root, two services and one parent process are drawn alone"
        );
    }

    #[test]
    fn opening_a_module_and_its_group_shows_the_entities() {
        let mut view = LumenView::default();
        view.expanded.insert("service:a".into());
        let scene = compose(&frame(), &view);
        let group = group_id("service:a", LumenCategory::Processes);
        assert!(ids(&scene).contains(group.as_str()));
        assert!(!ids(&scene).contains("process:a1"));
        view.expanded.insert(group.clone());
        let scene = compose(&frame(), &view);
        let process = scene.nodes.iter().find(|n| n.id == "process:a1").unwrap();
        assert_eq!(process.parent.as_deref(), Some(group.as_str()));
        assert_eq!(process.kind, LumenKind::Entity);
    }

    #[test]
    fn folders_open_the_first_two_levels_and_entities_show_everything() {
        let view = LumenView {
            lens: LumenLens::Folders,
            ..LumenView::default()
        };
        let scene = compose(&frame(), &view);
        assert!(ids(&scene).contains(group_id("service:a", LumenCategory::Processes).as_str()));
        let view = LumenView {
            lens: LumenLens::Entities,
            ..LumenView::default()
        };
        let scene = compose(&frame(), &view);
        assert_eq!(scene.nodes.len(), 19);
        assert_eq!(
            scene.links.len(),
            18,
            "every entity but the root hangs from its parent"
        );
        assert_eq!(scene.collapsed_count, 0);
    }

    #[test]
    fn observed_edges_fold_onto_the_orbs_that_hold_their_ends() {
        let scene = compose(&frame(), &LumenView::default());
        let observed: Vec<_> = scene
            .relations
            .iter()
            .filter(|r| r.kind == LumenRelationKind::Observed)
            .collect();
        assert_eq!(observed.len(), 1, "{observed:?}");
        let relation = observed[0];
        assert_eq!(
            (relation.source.as_str(), relation.target.as_str()),
            ("service:a", "service:b")
        );
        assert_eq!(relation.count, 2);
        assert!(relation.relations.contains(&Relation::Runtime));
        assert!(relation.relations.contains(&Relation::Dependency));
        // The hierarchy edge is the tree itself, and both devices sit in one group.
        assert!(scene
            .relations
            .iter()
            .all(|r| !r.relations.contains(&Relation::Hierarchy)));
        let service = scene.nodes.iter().find(|n| n.id == "service:a").unwrap();
        assert!(service.signals.iter().any(|s| s == "ejecución"));
    }

    #[test]
    fn semantic_neighbours_appear_where_nothing_was_observed() {
        let view = LumenView {
            lens: LumenLens::Entities,
            ..LumenView::default()
        };
        let scene = compose(&frame(), &view);
        let semantic: Vec<_> = scene
            .relations
            .iter()
            .filter(|r| r.kind == LumenRelationKind::Semantic)
            .collect();
        assert_eq!(semantic.len(), 1);
        assert_eq!(
            (semantic[0].source.as_str(), semantic[0].target.as_str()),
            ("device:disk", "device:gpu")
        );
    }

    #[test]
    fn the_same_frame_places_everything_in_the_same_place_inside_the_field() {
        let a = compose(&frame(), &LumenView::default());
        let b = compose(&frame(), &LumenView::default());
        assert_eq!(a, b);
        for node in &a.nodes {
            assert!(node.position.iter().all(|c| c.is_finite()));
            assert!(length(node.position) <= a.radius * 0.94 + 1e-3);
        }
        let root = a.nodes.iter().find(|n| n.kind == LumenKind::Root).unwrap();
        assert_eq!(root.position, [0.0; 3]);
    }

    #[test]
    fn a_focused_view_holds_only_that_subtree_and_knows_the_way_up() {
        let view = LumenView {
            focus: Some("service:a".into()),
            ..LumenView::default()
        };
        let scene = compose(&frame(), &view);
        assert_eq!(scene.focus_id, "service:a");
        assert_eq!(scene.source_count, 7);
        assert!(ids(&scene)
            .iter()
            .all(|id| *id == "service:a" || id.contains("service:a")));
        assert_eq!(scene.focus_path.len(), 1);
        assert_eq!(scene.focus_path[0].source, "system:archieos");
    }

    #[test]
    fn a_cycle_in_reported_parents_is_cut_rather_than_followed() {
        let mut frame = frame();
        frame
            .nodes
            .push(node("process:x", Some("process:y"), EntityKind::Process));
        frame
            .nodes
            .push(node("process:y", Some("process:x"), EntityKind::Process));
        let view = LumenView {
            lens: LumenLens::Entities,
            ..LumenView::default()
        };
        let scene = compose(&frame, &view);
        assert_eq!(scene.nodes.len(), 21);
    }

    #[test]
    fn an_empty_frame_is_an_empty_scene() {
        let mut frame = frame();
        frame.nodes.clear();
        frame.edges.clear();
        let scene = compose(&frame, &LumenView::default());
        assert!(scene.nodes.is_empty() && scene.relations.is_empty());
    }

    #[test]
    fn any_query_gives_a_bounded_view() {
        let view = LumenView::from_query(
            "lens=folders&expand=service%3Aa,%ZZ,,process%3A1&category_pull=9&scale=nan&limit=5&focus=service%3Ab&layout=legacy",
        );
        assert_eq!(view.lens, LumenLens::Folders);
        assert_eq!(
            view.expanded,
            BTreeSet::from(["service:a".to_string(), "process:1".to_string()])
        );
        assert_eq!(view.shape.category_pull, 1.0);
        assert_eq!(view.shape.scale, 1.0);
        assert_eq!(view.limit, 12);
        assert_eq!(view.focus.as_deref(), Some("service:b"));
        assert_eq!(view.shape.layout, LumenLayout::Legacy);
        assert_eq!(
            LumenView::from_query("&&=&lens=nonsense"),
            LumenView::default()
        );
    }
}
