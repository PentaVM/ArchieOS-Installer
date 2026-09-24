use crate::signal::{
    EntityKind, OrbEntity, Signal, SignalKind, StageStatus, WorkflowComponent, WorkflowPhase,
    TOKEN_DIMENSIONS,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ENTITIES: usize = 12_000;
const MAX_NEIGHBOURS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Semantic,
    Hierarchy,
    Runtime,
    Focus,
    Action,
    Accessibility,
    Dependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrbState {
    pub idle: f32,
    pub listen: f32,
    pub think: f32,
    pub speak: f32,
    pub audio_envelope: f32,
}

impl Default for OrbState {
    fn default() -> Self {
        Self {
            idle: 1.0,
            listen: 0.0,
            think: 0.0,
            speak: 0.0,
            audio_envelope: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: EntityKind,
    pub label: String,
    pub direction: [f32; 3],
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub mass: f32,
    pub render_radius: f32,
    pub focus_score: f32,
    pub voice_overlap: f32,
    pub uncertainty: f32,
    pub activity: f32,
    pub shell_factor: f32,
    pub selected: bool,
    pub action_status: Option<StageStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbEdge {
    pub source: String,
    pub target: String,
    pub similarity: f32,
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbFrame {
    pub generated_at_ms: i64,
    pub state: OrbState,
    pub nodes: Vec<OrbNode>,
    pub edges: Vec<OrbEdge>,
    pub field: OrbField,
    pub profile: ResourceProfile,
    pub entity_count: usize,
    pub morphotype: Morphotype,
    pub source_count: usize,
    pub stale_signals: usize,
    pub workflows: Vec<WorkflowView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowView {
    pub component: WorkflowComponent,
    pub phase: WorkflowPhase,
    pub active: bool,
    pub observed: bool,
    pub count: u16,
    pub age_ms: Option<i64>,
}

/// Spherical field sampled for the current hardware profile. Coefficients use real
/// spherical harmonics in degree-major, signed-order layout (81 values at L=8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbField {
    pub degree: u8,
    pub coefficients: Vec<f32>,
    pub vertices: Vec<[f32; 3]>,
    pub triangle_indices: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProfile {
    Low,
    Balanced,
    High,
}

impl ResourceProfile {
    pub fn detect() -> Self {
        if let Ok(value) = std::env::var("ARCHIE_ORB_PROFILE") {
            match value.to_ascii_lowercase().as_str() {
                "low" => return Self::Low,
                "high" => return Self::High,
                "balanced" => return Self::Balanced,
                _ => {}
            }
        }
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let memory_kib = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    line.strip_prefix("MemTotal:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
            })
            .unwrap_or(4 * 1024 * 1024);
        if cores <= 4 || memory_kib <= 8 * 1024 * 1024 {
            Self::Low
        } else if cores >= 16 && memory_kib >= 32 * 1024 * 1024 {
            Self::High
        } else {
            Self::Balanced
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Morphotype {
    pub is_os: bool,
    pub core_strength: f32,
    pub shell_count: u8,
    pub roughness: f32,
    pub lobe_strength: f32,
    pub orbit_strength: f32,
}

impl Default for Morphotype {
    fn default() -> Self {
        Self {
            is_os: false,
            core_strength: 0.0,
            shell_count: 1,
            roughness: 0.025,
            lobe_strength: 0.08,
            orbit_strength: 0.06,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OrbConfig {
    pub neighbours: usize,
    pub max_entities: usize,
    pub visible_nodes: usize,
    pub mesh_latitude_segments: usize,
    pub mesh_longitude_segments: usize,
    pub profile: ResourceProfile,
    pub think_gain: f32,
    pub voice_gain: f32,
    pub state_half_life_ms: f32,
    pub signal_ttl_ms: i64,
}

impl Default for OrbConfig {
    fn default() -> Self {
        Self::for_profile(ResourceProfile::detect())
    }
}

impl OrbConfig {
    pub fn for_profile(profile: ResourceProfile) -> Self {
        let (visible_nodes, lat, lon, neighbours) = match profile {
            ResourceProfile::Low => (256, 18, 36, 12),
            ResourceProfile::Balanced => (512, 30, 60, 18),
            ResourceProfile::High => (2048, 48, 96, MAX_NEIGHBOURS),
        };
        Self {
            neighbours,
            max_entities: MAX_ENTITIES,
            visible_nodes,
            mesh_latitude_segments: lat,
            mesh_longitude_segments: lon,
            profile,
            think_gain: 0.3,
            voice_gain: 0.12,
            state_half_life_ms: 180.0,
            signal_ttl_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone)]
struct EntityState {
    entity: OrbEntity,
    direction: [f32; 3],
    position: [f32; 3],
    velocity: [f32; 3],
    focus: f32,
    voice_overlap: f32,
    last_seen_ms: i64,
}

/// The deterministic, renderer-independent TFOG projection.
///
/// Producers only supply facts and short-lived state. This model assigns geometry from
/// multi-channel relationships, retains directions for persistent identities, and
/// coalesces focus and speech by entity id. It has no execution interface.
pub struct OrbModel {
    config: OrbConfig,
    entities: BTreeMap<String, EntityState>,
    focus: BTreeMap<String, (f32, i64)>,
    working_set: BTreeMap<String, (f32, i64)>,
    speaking_features: Vec<f32>,
    stage: BTreeMap<(String, u32), (String, Option<u32>, StageStatus, i64)>,
    selected: Option<(String, u32, String, i64)>,
    workflows: BTreeMap<WorkflowComponent, (WorkflowPhase, bool, u16, i64)>,
    targets: OrbState,
    state: OrbState,
    stale_signals: usize,
    source_count: usize,
    last_ms: i64,
    graph_dirty: bool,
    cached_edges: Vec<OrbEdge>,
    cached_directions: BTreeMap<String, [f32; 3]>,
}

impl Default for OrbModel {
    fn default() -> Self {
        Self::new(OrbConfig::default())
    }
}

impl OrbModel {
    pub fn new(config: OrbConfig) -> Self {
        Self {
            config,
            entities: BTreeMap::new(),
            focus: BTreeMap::new(),
            working_set: BTreeMap::new(),
            speaking_features: Vec::new(),
            stage: BTreeMap::new(),
            selected: None,
            workflows: BTreeMap::new(),
            targets: OrbState::default(),
            state: OrbState::default(),
            stale_signals: 0,
            source_count: 0,
            last_ms: 0,
            graph_dirty: true,
            cached_edges: Vec::new(),
            cached_directions: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, signal: Signal) -> Result<(), crate::SignalError> {
        signal.validate()?;
        if signal.at_ms < self.last_ms {
            return Ok(());
        }
        self.last_ms = signal.at_ms;
        self.source_count = self.source_count.saturating_add(1);
        match signal.kind {
            SignalKind::EntityUpsert { entity } => {
                if !self.entities.contains_key(&entity.id)
                    && self.entities.len() >= self.config.max_entities.min(MAX_ENTITIES).max(1)
                {
                    return Err(crate::SignalError::Invalid);
                }
                let id = entity.id.clone();
                let old = self.entities.get(&id);
                let graph_changed = old.is_none_or(|previous| {
                    let before = &previous.entity;
                    before.kind != entity.kind
                        || before.parent_id != entity.parent_id
                        || before.related_ids != entity.related_ids
                        || before.token != entity.token
                        || before.class_probs != entity.class_probs
                        || before.size_bytes != entity.size_bytes
                        || before.runtime != entity.runtime
                });
                self.entities.insert(
                    id,
                    EntityState {
                        direction: old
                            .map(|n| n.direction)
                            .unwrap_or_else(|| token_direction_for(&entity.id, &entity.token)),
                        position: old.map(|n| n.position).unwrap_or([0.0; 3]),
                        velocity: old.map(|n| n.velocity).unwrap_or([0.0; 3]),
                        focus: old.map(|n| n.focus).unwrap_or(0.0),
                        voice_overlap: old.map(|n| n.voice_overlap).unwrap_or(0.0),
                        entity,
                        last_seen_ms: signal.at_ms,
                    },
                );
                self.graph_dirty |= graph_changed;
            }
            SignalKind::EntityRemove { id } => {
                self.entities.remove(&id);
                self.focus.remove(&id);
                self.graph_dirty = true;
            }
            SignalKind::Listen {
                active,
                audio_envelope,
            } => {
                self.targets.listen = f32::from(active);
                self.targets.audio_envelope = audio_envelope;
            }
            SignalKind::Think {
                active,
                working_set,
                ..
            } => {
                self.targets.think = f32::from(active);
                if active {
                    self.working_set = working_set
                        .into_iter()
                        .map(|(id, score)| (id, (score, signal.at_ms)))
                        .collect();
                } else {
                    self.working_set.clear();
                }
            }
            SignalKind::Speak {
                active,
                semantic_features,
                audio_envelope,
                ..
            } => {
                self.targets.speak = f32::from(active);
                self.speaking_features = if active {
                    semantic_features
                } else {
                    Vec::new()
                };
                self.targets.audio_envelope = if active { audio_envelope } else { 0.0 };
            }
            SignalKind::Focus { scores, .. } => {
                self.focus = scores
                    .into_iter()
                    .map(|(id, score)| (id, (score, signal.at_ms)))
                    .collect();
            }
            SignalKind::Stage {
                operation_id,
                app_id,
                surface,
                pid,
                status,
            } => {
                self.ensure_surface(&app_id, surface, pid, signal.at_ms);
                let key = (app_id.clone(), surface);
                if self
                    .stage
                    .get(&key)
                    .is_some_and(|(_, _, _, at)| *at > signal.at_ms)
                {
                    return Ok(());
                }
                self.stage
                    .insert(key, (operation_id, pid, status, signal.at_ms));
            }
            SignalKind::AccessibilitySelection {
                app_id,
                surface,
                pid,
                control_id,
            } => {
                self.ensure_surface(&app_id, surface, pid, signal.at_ms);
                self.ensure_control(&app_id, surface, &control_id, signal.at_ms);
                self.selected = Some((
                    app_id,
                    surface,
                    format!("control:{control_id}"),
                    signal.at_ms,
                ));
            }
            SignalKind::Workflow {
                component,
                phase,
                active,
                count,
            } => {
                self.workflows
                    .insert(component, (phase, active, count, signal.at_ms));
            }
        }
        Ok(())
    }

    /// Build a frame at `now_ms`; all time-dependent behavior is explicit and replayable.
    pub fn frame(&mut self, now_ms: i64, dt_seconds: f32) -> OrbFrame {
        self.expire(now_ms);
        let dt = dt_seconds.clamp(0.0, 0.1);
        let blend = if self.config.state_half_life_ms <= 0.0 {
            1.0
        } else {
            1.0 - (-std::f32::consts::LN_2 * dt * 1000.0 / self.config.state_half_life_ms).exp()
        };
        self.state.idle += (1.0
            - self
                .targets
                .listen
                .max(self.targets.think)
                .max(self.targets.speak)
            - self.state.idle)
            * blend;
        self.state.listen += (self.targets.listen - self.state.listen) * blend;
        self.state.think += (self.targets.think - self.state.think) * blend;
        self.state.speak += (self.targets.speak - self.state.speak) * blend;
        self.state.audio_envelope +=
            (self.targets.audio_envelope - self.state.audio_envelope) * blend;

        if self.graph_dirty {
            self.cached_edges = self.knn_edges();
            self.cached_directions =
                spectral_directions(&self.entities, &self.cached_edges, &self.cached_directions);
            self.graph_dirty = false;
        }
        let edges = self.cached_edges.clone();
        let directions = self.cached_directions.clone();
        let morphotype = infer_morphotype(self.entities.values().map(|n| &n.entity));
        for (id, entity) in &mut self.entities {
            // Recover cleanly from a non-finite state (for example, a renderer
            // reconnecting after a suspended system clock) before integrating it.
            if !finite_vec(entity.position)
                || !finite_vec(entity.velocity)
                || norm(entity.position) > 4.0
                || norm(entity.velocity) > 20.0
            {
                entity.position = [0.0; 3];
                entity.velocity = [0.0; 3];
            }
            if let Some(new_direction) = directions.get(id) {
                // Persistent nodes retain their seat. This local temporal anchor prevents
                // one arrival from rotating the whole field between snapshots.
                entity.direction = normalize(add(
                    scale(entity.direction, 0.82),
                    scale(*new_direction, 0.18),
                ));
            }
            let persisted_focus = self.focus.get(id).map_or(0.0, |(score, _)| *score);
            let active_focus = self.working_set.get(id).map_or(0.0, |(score, _)| *score);
            entity.focus = persisted_focus.max(active_focus);
            entity.voice_overlap = if self.speaking_features.len() == TOKEN_DIMENSIONS {
                cosine(&entity.entity.token, &self.speaking_features)
            } else {
                0.0
            };
        }
        let mut ranked =
            self.entities
                .iter()
                .map(|(id, entity)| {
                    let action = self.stage.keys().any(|(app, surface)| {
                        app == id || format!("window:{app}:{surface}") == **id
                    });
                    let selected = self
                        .selected
                        .as_ref()
                        .is_some_and(|(_, _, control, _)| control == id);
                    let score = entity.entity.importance * 3.0
                        + entity.entity.activity * 2.0
                        + entity.focus
                        + entity.voice_overlap
                        + f32::from(action || selected) * 4.0;
                    (id, score)
                })
                .collect::<Vec<_>>();
        ranked.sort_by(|(left_id, left), (right_id, right)| {
            right.total_cmp(left).then_with(|| left_id.cmp(right_id))
        });
        let visible_ids = ranked
            .into_iter()
            .take(self.config.visible_nodes.max(1))
            .map(|(id, _)| id.clone())
            .collect::<BTreeSet<_>>();
        let mut nodes = self
            .entities
            .iter()
            .filter(|(id, _)| visible_ids.contains(*id))
            .map(|(id, entity)| {
                let mass = visual_mass(
                    &entity.entity,
                    edges
                        .iter()
                        .filter(|e| e.source == *id || e.target == *id)
                        .count(),
                );
                let shell = shell_factor(&entity.entity, morphotype);
                let direction = entity.direction;
                let density = density_at(id, direction, mass, &edges, &self.entities);
                let surface = 1.0 + morphotype.roughness * density.tanh();
                let focus_radius = self.config.think_gain * entity.focus.sqrt() * self.state.think;
                let voice_radius = self.config.voice_gain
                    * entity.voice_overlap
                    * self.state.speak
                    * self.state.audio_envelope;
                let uncertainty_depth = entity.entity.uncertainty * 0.06;
                let activity_radius = entity.entity.activity * 0.07;
                let r = (shell * surface + focus_radius + voice_radius + activity_radius
                    - uncertainty_depth)
                    .max(0.08);
                let target = scale(direction, r);
                let omega = 9.0;
                // Semi-implicit Euler at the full display interval becomes unstable
                // with this critically damped spring once dt approaches 100 ms.
                // Keep each integration step at 20 ms or less on slower systems.
                let steps = ((dt / 0.02).ceil() as usize).max(1);
                let step = dt / steps as f32;
                let mut velocity = entity.velocity;
                let mut position = entity.position;
                for _ in 0..steps {
                    let accel = add(
                        scale(sub(target, position), omega * omega),
                        scale(velocity, -2.0 * omega),
                    );
                    velocity = add(velocity, scale(accel, step));
                    position = add(position, scale(velocity, step));
                }
                OrbNode {
                    id: id.clone(),
                    parent_id: entity.entity.parent_id.clone(),
                    kind: entity.entity.kind,
                    label: entity.entity.label.clone(),
                    direction,
                    position,
                    velocity,
                    mass,
                    render_radius: (0.006 + mass.sqrt() * 0.012).clamp(0.006, 0.08),
                    focus_score: entity.focus,
                    voice_overlap: entity.voice_overlap,
                    uncertainty: entity.entity.uncertainty,
                    activity: entity.entity.activity,
                    shell_factor: shell,
                    selected: self
                        .selected
                        .as_ref()
                        .is_some_and(|(_, _, control, _)| control == id),
                    action_status: self
                        .stage
                        .iter()
                        .filter(|((app, surface), _)| {
                            app == id || format!("window:{app}:{surface}") == *id
                        })
                        .max_by_key(|(_, (_, _, _, at))| *at)
                        .map(|(_, (_, _, status, _))| *status),
                }
            })
            .collect::<Vec<_>>();
        project_collisions(&mut nodes);
        for node in &nodes {
            if let Some(entity) = self.entities.get_mut(&node.id) {
                entity.position = node.position;
                entity.velocity = node.velocity;
            }
        }
        let edges = self
            .attach_relations(edges)
            .into_iter()
            .filter(|edge| visible_ids.contains(&edge.source) && visible_ids.contains(&edge.target))
            .collect::<Vec<_>>();
        let field = build_field(&nodes, &edges, morphotype, self.config);
        let workflows = [
            WorkflowComponent::Wanda,
            WorkflowComponent::Cortex,
            WorkflowComponent::Vision,
            WorkflowComponent::Iris,
        ]
        .into_iter()
        .map(|component| match self.workflows.get(&component) {
            Some((phase, active, count, at_ms)) => {
                let age_ms = now_ms.saturating_sub(*at_ms).max(0);
                let observed = age_ms <= 60_000;
                WorkflowView {
                    component,
                    phase: if observed {
                        *phase
                    } else {
                        WorkflowPhase::Waiting
                    },
                    active: *active && age_ms <= self.config.signal_ttl_ms,
                    observed,
                    count: if observed { *count } else { 0 },
                    age_ms: Some(age_ms),
                }
            }
            None => WorkflowView {
                component,
                phase: WorkflowPhase::Waiting,
                active: false,
                observed: false,
                count: 0,
                age_ms: None,
            },
        })
        .collect();
        OrbFrame {
            generated_at_ms: now_ms,
            state: self.state,
            nodes,
            edges,
            field,
            profile: self.config.profile,
            entity_count: self.entities.len(),
            morphotype,
            source_count: self.source_count,
            stale_signals: self.stale_signals,
            workflows,
        }
    }

    fn expire(&mut self, now_ms: i64) {
        let ttl = self.config.signal_ttl_ms.max(0);
        let before = self.focus.len()
            + self.working_set.len()
            + self.stage.len()
            + usize::from(self.selected.is_some());
        // Focus can arrive slightly before the next snapshot upserts its node. Keep
        // the bounded score briefly so that race does not discard the observation.
        self.focus
            .retain(|_, (_, at)| now_ms.saturating_sub(*at) <= ttl.saturating_mul(60));
        self.working_set.retain(|id, (_, at)| {
            self.entities.contains_key(id) && now_ms.saturating_sub(*at) <= ttl
        });
        self.stage
            .retain(|_, (_, _, _, at)| now_ms.saturating_sub(*at) <= ttl);
        if self
            .selected
            .as_ref()
            .is_some_and(|(_, _, _, at)| now_ms.saturating_sub(*at) > ttl)
        {
            self.selected = None;
        }
        let after = self.focus.len()
            + self.working_set.len()
            + self.stage.len()
            + usize::from(self.selected.is_some());
        self.stale_signals = self
            .stale_signals
            .saturating_add(before.saturating_sub(after));
        let old_count = self.entities.len();
        self.entities.retain(|id, entity| {
            !(id.starts_with("window:") || id.starts_with("process:") || id.starts_with("control:"))
                || now_ms.saturating_sub(entity.last_seen_ms) <= ttl
        });
        if self.entities.len() != old_count {
            self.graph_dirty = true;
        }
        if self.last_ms > 0 && now_ms.saturating_sub(self.last_ms) > ttl {
            self.targets.listen = 0.0;
            self.targets.think = 0.0;
            self.targets.speak = 0.0;
            self.targets.audio_envelope = 0.0;
        }
    }

    fn ensure_surface(&mut self, app_id: &str, surface: u32, pid: Option<u32>, at_ms: i64) {
        if !self.entities.contains_key(app_id) {
            self.insert_placeholder(app_id, None, EntityKind::App, "app", at_ms, 0.55);
        } else if let Some(app) = self.entities.get_mut(app_id) {
            app.last_seen_ms = at_ms;
        }
        let parent = if let Some(pid) = pid {
            let process_id = format!("process:{pid}");
            if !self.entities.contains_key(&process_id) {
                self.insert_placeholder(
                    &process_id,
                    Some(app_id),
                    EntityKind::Process,
                    "runtime",
                    at_ms,
                    0.6,
                );
            } else if let Some(process) = self.entities.get_mut(&process_id) {
                process.last_seen_ms = at_ms;
            }
            process_id
        } else {
            app_id.to_string()
        };
        let id = format!("window:{app_id}:{surface}");
        if !self.entities.contains_key(&id) {
            self.insert_placeholder(
                &id,
                Some(&parent),
                EntityKind::Window,
                "window",
                at_ms,
                0.65,
            );
        } else if let Some(window) = self.entities.get_mut(&id) {
            window.last_seen_ms = at_ms;
        }
    }

    fn ensure_control(&mut self, app_id: &str, surface: u32, control_id: &str, at_ms: i64) {
        let id = format!("control:{control_id}");
        if !self.entities.contains_key(&id)
            && self.entities.len() < self.config.max_entities.min(MAX_ENTITIES)
        {
            let parent = format!("window:{app_id}:{surface}");
            self.insert_placeholder(
                &id,
                Some(&parent),
                EntityKind::Control,
                "control",
                at_ms,
                0.82,
            );
        }
    }

    fn insert_placeholder(
        &mut self,
        id: &str,
        parent: Option<&str>,
        kind: EntityKind,
        class: &str,
        at_ms: i64,
        uncertainty: f32,
    ) {
        if self.entities.len() >= self.config.max_entities.min(MAX_ENTITIES) {
            return;
        }
        let mut class_probs = BTreeMap::new();
        class_probs.insert(class.to_string(), 1.0);
        let entity = OrbEntity {
            id: id.to_string(),
            parent_id: parent.map(str::to_string),
            related_ids: Vec::new(),
            kind,
            token: vec![0.0; TOKEN_DIMENSIONS],
            class_probs,
            size_bytes: 0,
            activity: 0.0,
            importance: 0.0,
            uncertainty,
            runtime: Vec::new(),
            label: class.to_string(),
        };
        let direction = parent
            .and_then(|p| self.entities.get(p).map(|n| n.direction))
            .unwrap_or_else(|| token_direction_for(id, &entity.token));
        self.entities.insert(
            id.to_string(),
            EntityState {
                entity,
                direction,
                position: [0.0; 3],
                velocity: [0.0; 3],
                focus: 0.0,
                voice_overlap: 0.0,
                last_seen_ms: at_ms,
            },
        );
        self.graph_dirty = true;
    }

    fn knn_edges(&self) -> Vec<OrbEdge> {
        let ids = self.entities.keys().cloned().collect::<Vec<_>>();
        if ids.len() < 2 {
            return Vec::new();
        }
        let signatures = ids
            .iter()
            .map(|id| {
                let token = &self.entities[id].entity.token;
                simhash(token)
            })
            .collect::<Vec<_>>();
        let mut candidate_map: BTreeMap<(usize, usize), f32> = BTreeMap::new();
        let mut children = BTreeMap::<&str, Vec<usize>>::new();
        for (index, id) in ids.iter().enumerate() {
            if let Some(parent) = self.entities[id].entity.parent_id.as_deref() {
                children.entry(parent).or_default().push(index);
            }
        }
        let mut by_bucket = (0..4)
            .map(|_| BTreeMap::<u8, Vec<usize>>::new())
            .collect::<Vec<_>>();
        for (i, sig) in signatures.iter().enumerate() {
            for band in 0..4 {
                by_bucket[band]
                    .entry((sig >> (band * 8)) as u8)
                    .or_default()
                    .push(i);
            }
        }
        for i in 0..ids.len() {
            let sig = signatures[i];
            let mut candidates = BTreeSet::new();
            for band in 0..4 {
                if let Some(bucket) = by_bucket[band].get(&((sig >> (band * 8)) as u8)) {
                    // A degenerate cohort (for example zero-feature placeholders) must
                    // not turn the sparse lookup into a quadratic scan.
                    let stride = bucket.len().div_ceil(96).max(1);
                    let offset = i % stride;
                    candidates.extend(
                        bucket
                            .iter()
                            .enumerate()
                            .skip(offset)
                            .step_by(stride)
                            .take(96)
                            .map(|(_, candidate)| *candidate),
                    );
                }
                if candidates.len() >= self.config.neighbours.saturating_mul(8) {
                    break;
                }
            }
            if candidates.len() < self.config.neighbours {
                let stride = (ids.len() / self.config.neighbours.max(1)).max(1);
                candidates.extend(
                    (0..ids.len())
                        .step_by(stride)
                        .take(self.config.neighbours * 2),
                );
            }
            if let Some(parent) = self.entities[&ids[i]].entity.parent_id.as_deref() {
                if let Ok(index) = ids.binary_search_by(|id| id.as_str().cmp(parent)) {
                    candidates.insert(index);
                }
            }
            if let Some(descendants) = children.get(ids[i].as_str()) {
                candidates.extend(descendants.iter().copied());
            }
            candidates.remove(&i);
            let mut scored = candidates
                .into_iter()
                .map(|j| {
                    let a = &self.entities[&ids[i]].entity;
                    let b = &self.entities[&ids[j]].entity;
                    (j, entity_similarity(a, b))
                })
                .collect::<Vec<_>>();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| ids[a.0].cmp(&ids[b.0])));
            for (j, score) in scored
                .into_iter()
                .take(self.config.neighbours.min(MAX_NEIGHBOURS))
            {
                if score <= 0.0 {
                    continue;
                }
                let (a, b) = if i < j { (i, j) } else { (j, i) };
                candidate_map
                    .entry((a, b))
                    .and_modify(|s| *s = s.max(score))
                    .or_insert(score);
            }
        }
        candidate_map
            .into_iter()
            .map(|((a, b), similarity)| OrbEdge {
                source: ids[a].clone(),
                target: ids[b].clone(),
                similarity,
                relations: vec![Relation::Semantic],
            })
            .collect()
    }

    fn attach_relations(&self, mut edges: Vec<OrbEdge>) -> Vec<OrbEdge> {
        let mut set = edges
            .iter()
            .map(|e| (e.source.clone(), e.target.clone()))
            .collect::<BTreeSet<_>>();
        let mut add = |source: String, target: String, relation: Relation| {
            let pair = if source <= target {
                (source.clone(), target.clone())
            } else {
                (target.clone(), source.clone())
            };
            if set.insert(pair) {
                edges.push(OrbEdge {
                    source,
                    target,
                    similarity: 1.0,
                    relations: vec![relation],
                });
            } else if let Some(edge) = edges.iter_mut().find(|e| {
                (e.source == source && e.target == target)
                    || (e.source == target && e.target == source)
            }) {
                if !edge.relations.contains(&relation) {
                    edge.relations.push(relation);
                }
            }
        };
        for (id, entity) in &self.entities {
            if let Some(parent) = &entity.entity.parent_id {
                if self.entities.contains_key(parent) {
                    add(id.clone(), parent.clone(), Relation::Hierarchy);
                }
            }
            for related_id in &entity.entity.related_ids {
                if self.entities.contains_key(related_id) {
                    add(id.clone(), related_id.clone(), Relation::Dependency);
                }
            }
        }
        let focused = self
            .entities
            .iter()
            .filter(|(id, _)| {
                self.focus.get(*id).is_some_and(|(score, _)| *score > 0.0)
                    || self
                        .working_set
                        .get(*id)
                        .is_some_and(|(score, _)| *score > 0.0)
            })
            .map(|(id, _)| id.clone())
            .take(16)
            .collect::<Vec<_>>();
        for (index, source) in focused.iter().enumerate() {
            for target in focused.iter().skip(index + 1).take(15) {
                add(source.clone(), target.clone(), Relation::Focus);
            }
        }
        for ((app, surface), _) in &self.stage {
            let window = format!("window:{app}:{surface}");
            if self.entities.contains_key(app) && self.entities.contains_key(&window) {
                add(window, app.clone(), Relation::Action);
            }
        }
        if let Some((app, _, control, _)) = &self.selected {
            let parent = self
                .entities
                .get(control)
                .and_then(|entity| entity.entity.parent_id.as_deref())
                .map(str::to_string);
            if let Some(parent) = parent.filter(|parent| self.entities.contains_key(parent)) {
                add(control.clone(), parent, Relation::Accessibility);
            } else if self.entities.contains_key(app) && self.entities.contains_key(control) {
                add(control.clone(), app.clone(), Relation::Accessibility);
            }
        }
        edges
    }
}

fn entity_similarity(a: &OrbEntity, b: &OrbEntity) -> f32 {
    let sem = cosine(&a.token, &b.token).max(0.0);
    let cls = class_similarity(&a.class_probs, &b.class_probs);
    let tree = if a.parent_id.as_deref() == Some(&b.id) || b.parent_id.as_deref() == Some(&a.id) {
        1.0
    } else if a.parent_id.is_some() && a.parent_id == b.parent_id {
        0.8
    } else {
        0.0
    };
    let run = if a.runtime.is_empty() || b.runtime.is_empty() {
        0.0
    } else {
        let n = a.runtime.len().min(b.runtime.len());
        if n == 0 {
            0.0
        } else {
            (-a.runtime
                .iter()
                .zip(&b.runtime)
                .take(n)
                .map(|(x, y)| (x - y) * (x - y))
                .sum::<f32>()
                .sqrt())
            .exp()
        }
    };
    let byte = if a.size_bytes == 0 || b.size_bytes == 0 {
        0.0
    } else {
        let delta = ((a.size_bytes as f32).ln_1p() - (b.size_bytes as f32).ln_1p()).abs();
        (-delta / 4.0).exp()
    };
    let weights = [
        (
            sem,
            a.token.iter().any(|value| *value != 0.0) && b.token.iter().any(|value| *value != 0.0),
            0.35,
        ),
        (
            cls,
            !a.class_probs.is_empty() && !b.class_probs.is_empty(),
            0.25,
        ),
        (byte, a.size_bytes > 0 && b.size_bytes > 0, 0.15),
        (tree, a.parent_id.is_some() || b.parent_id.is_some(), 0.15),
        (run, !a.runtime.is_empty() && !b.runtime.is_empty(), 0.10),
    ];
    let numerator = weights
        .iter()
        .map(|(value, present, weight)| if *present { *value * weight } else { 0.0 })
        .sum::<f32>();
    let denominator = weights
        .iter()
        .map(|(_, present, weight)| if *present { *weight } else { 0.0 })
        .sum::<f32>();
    if denominator <= 0.0 {
        0.0
    } else {
        (numerator / denominator).clamp(0.0, 1.0)
    }
}

fn class_similarity(a: &BTreeMap<String, f32>, b: &BTreeMap<String, f32>) -> f32 {
    let keys = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
    if keys.is_empty() {
        return 0.0;
    }
    keys.iter()
        .map(|k| {
            a.get(*k).copied().unwrap_or(0.0).sqrt() * b.get(*k).copied().unwrap_or(0.0).sqrt()
        })
        .sum::<f32>()
        .clamp(0.0, 1.0)
}

fn infer_morphotype<'a>(entities: impl Iterator<Item = &'a OrbEntity>) -> Morphotype {
    let mut count = 0.0_f32;
    let mut kernel = 0.0;
    let mut executable = 0.0;
    let mut live = 0.0;
    let mut has_system = false;
    let mut classes = BTreeMap::<String, f32>::new();
    for e in entities {
        count += 1.0;
        has_system |= e.kind == EntityKind::System;
        kernel += e.class_probs.get("kernel").copied().unwrap_or(0.0)
            + e.class_probs.get("driver").copied().unwrap_or(0.0) * 0.5;
        executable += e.class_probs.get("executable").copied().unwrap_or(0.0)
            + e.class_probs.get("library").copied().unwrap_or(0.0);
        live += e.activity;
        for (c, p) in &e.class_probs {
            *classes.entry(c.clone()).or_default() += *p;
        }
    }
    if count == 0.0 {
        return Morphotype::default();
    }
    let kernel = (kernel / count).clamp(0.0, 1.0);
    let executable = (executable / count).clamp(0.0, 1.0);
    let live = (live / count).clamp(0.0, 1.0);
    let os = has_system || (kernel > 0.03 && executable > 0.12);
    let entropy = if classes.is_empty() {
        0.0
    } else {
        let sum = classes.values().sum::<f32>().max(f32::EPSILON);
        let h = classes
            .values()
            .map(|v| {
                let p = *v / sum;
                if p > 0.0 {
                    -p * p.ln()
                } else {
                    0.0
                }
            })
            .sum::<f32>();
        (h / (classes.len() as f32).max(2.0).ln()).clamp(0.0, 1.0)
    };
    Morphotype {
        is_os: os,
        core_strength: if os {
            (0.55 * kernel + 0.45 * executable).clamp(0.0, 1.0).max(0.2)
        } else {
            0.0
        },
        shell_count: if os {
            6
        } else if entropy > 0.55 {
            3
        } else {
            1
        },
        roughness: (0.08 + 0.2 * entropy + 0.12 * live).clamp(0.04, 0.28),
        lobe_strength: (0.08 + 0.25 * entropy).clamp(0.04, 0.36),
        orbit_strength: (0.06 + 0.2 * live).clamp(0.02, 0.3),
    }
}

fn shell_factor(e: &OrbEntity, m: Morphotype) -> f32 {
    if !m.is_os {
        return 0.87 + 0.12 * e.uncertainty;
    }
    let weighted = [
        ("kernel", 0.34),
        ("boot", 0.36),
        ("driver", 0.51),
        ("hal", 0.55),
        ("service", 0.66),
        ("library", 0.75),
        ("config", 0.81),
        ("application", 0.92),
        ("process", 1.08),
    ];
    let mut total = 0.0;
    let mut sum = 0.0;
    for (name, r) in weighted {
        let p = e.class_probs.get(name).copied().unwrap_or(0.0);
        total += p * r;
        sum += p;
    }
    if sum > 0.0 {
        (total / sum).clamp(0.28, 1.25)
    } else {
        0.86
    }
}

fn visual_mass(e: &OrbEntity, degree: usize) -> f32 {
    (e.size_bytes as f32).ln_1p().min(30.0) / 30.0 * 0.35
        + e.activity * 0.25
        + e.importance * 0.25
        + (degree as f32).ln_1p().min(5.0) / 5.0 * 0.15
}

fn density_at(
    id: &str,
    dir: [f32; 3],
    mass: f32,
    edges: &[OrbEdge],
    entities: &BTreeMap<String, EntityState>,
) -> f32 {
    let neighbor_mass = edges
        .iter()
        .filter(|e| e.source == id || e.target == id)
        .filter_map(|e| {
            let other = if e.source == id { &e.target } else { &e.source };
            entities
                .get(other)
                .map(|n| dot(dir, n.direction) * e.similarity)
        })
        .sum::<f32>();
    mass + neighbor_mass.max(0.0)
}

fn build_field(
    nodes: &[OrbNode],
    edges: &[OrbEdge],
    morphotype: Morphotype,
    config: OrbConfig,
) -> OrbField {
    const DEGREE: usize = 8;
    let mut coefficients = vec![0.0_f32; (DEGREE + 1) * (DEGREE + 1)];
    let total_weight = nodes
        .iter()
        .map(|node| node.mass + node.focus_score + node.activity + node.voice_overlap)
        .sum::<f32>()
        .max(1e-5);
    for node in nodes {
        let weight = node.mass + node.focus_score + node.activity + node.voice_overlap;
        for (index, basis) in spherical_harmonics(node.direction, DEGREE)
            .iter()
            .enumerate()
        {
            coefficients[index] += 4.0 * std::f32::consts::PI * weight / total_weight * basis;
        }
    }
    let directions = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.direction))
        .collect::<BTreeMap<_, _>>();
    let edge_centers = edges
        .iter()
        .take(512)
        .filter_map(|edge| {
            let source = directions.get(edge.source.as_str())?;
            let target = directions.get(edge.target.as_str())?;
            Some((normalize(add(*source, *target)), edge.similarity))
        })
        .collect::<Vec<_>>();
    let lat = config.mesh_latitude_segments.clamp(8, 48);
    let lon = config.mesh_longitude_segments.clamp(16, 96);
    let mut directions = Vec::with_capacity((lat + 1) * (lon + 1));
    let mut densities = Vec::with_capacity((lat + 1) * (lon + 1));
    let concentration = 20.0_f32;
    for row in 0..=lat {
        let theta = std::f32::consts::PI * row as f32 / lat as f32;
        for column in 0..=lon {
            let phi = 2.0 * std::f32::consts::PI * column as f32 / lon as f32;
            let direction = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            let density = nodes
                .iter()
                .map(|node| {
                    let weight = node.mass + node.focus_score + node.activity + node.voice_overlap;
                    weight
                        * (concentration * (dot(direction, node.direction).clamp(-1.0, 1.0) - 1.0))
                            .exp()
                })
                .sum::<f32>();
            directions.push(direction);
            densities.push(density);
        }
    }
    let mean = densities.iter().sum::<f32>() / densities.len().max(1) as f32;
    let deviation = (densities
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / densities.len().max(1) as f32)
        .sqrt()
        .max(1e-5);
    let mut vertices = Vec::with_capacity(directions.len());
    for (direction, density) in directions.into_iter().zip(densities) {
        let edge_energy = edge_centers
            .iter()
            .filter(|(center, _)| dot(*center, direction) > 0.92)
            .map(|(_, similarity)| *similarity)
            .sum::<f32>()
            .min(1.0);
        let normalized = (density - mean) / deviation;
        let radial = 0.82 * (1.0 + morphotype.roughness * normalized.tanh()) + 0.04 * edge_energy;
        vertices.push(scale(direction, radial.clamp(0.55, 1.2)));
    }
    let mut triangle_indices = Vec::with_capacity(lat * lon * 6);
    for row in 0..lat {
        for column in 0..lon {
            let a = (row * (lon + 1) + column) as u32;
            let b = a + (lon + 1) as u32;
            triangle_indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    OrbField {
        degree: DEGREE as u8,
        coefficients,
        vertices,
        triangle_indices,
    }
}

fn spherical_harmonics(direction: [f32; 3], maximum_degree: usize) -> Vec<f32> {
    let [x, y, z] = normalize(direction);
    let cos_theta = y.clamp(-1.0, 1.0);
    let phi = z.atan2(x);
    let mut values = Vec::with_capacity((maximum_degree + 1) * (maximum_degree + 1));
    for degree in 0..=maximum_degree {
        for order in -(degree as isize)..=(degree as isize) {
            let absolute_order = order.unsigned_abs();
            let p = associated_legendre(degree, absolute_order, cos_theta);
            let normalization = (((2 * degree + 1) as f32 / (4.0 * std::f32::consts::PI))
                * (factorial(degree - absolute_order) / factorial(degree + absolute_order)) as f32)
                .sqrt();
            let value = if order < 0 {
                2.0_f32.sqrt() * normalization * p * (absolute_order as f32 * phi).sin()
            } else if order > 0 {
                2.0_f32.sqrt() * normalization * p * (absolute_order as f32 * phi).cos()
            } else {
                normalization * p
            };
            values.push(value);
        }
    }
    values
}

fn associated_legendre(degree: usize, order: usize, x: f32) -> f32 {
    let mut pmm = 1.0_f32;
    if order > 0 {
        let root = (1.0 - x * x).max(0.0).sqrt();
        let mut factor = 1.0_f32;
        for _ in 1..=order {
            pmm *= -factor * root;
            factor += 2.0;
        }
    }
    if degree == order {
        return pmm;
    }
    let mut previous = pmm;
    let mut current = x * (2 * order + 1) as f32 * pmm;
    if degree == order + 1 {
        return current;
    }
    for next_degree in order + 2..=degree {
        let next = ((2 * next_degree - 1) as f32 * x * current
            - (next_degree + order - 1) as f32 * previous)
            / (next_degree - order) as f32;
        previous = current;
        current = next;
    }
    current
}

fn factorial(value: usize) -> f64 {
    (1..=value)
        .map(|part| part as f64)
        .product::<f64>()
        .max(1.0)
}

fn spectral_directions(
    entities: &BTreeMap<String, EntityState>,
    edges: &[OrbEdge],
    previous: &BTreeMap<String, [f32; 3]>,
) -> BTreeMap<String, [f32; 3]> {
    let ids = entities.keys().cloned().collect::<Vec<_>>();
    let n = ids.len();
    if n == 0 {
        return BTreeMap::new();
    }
    let index = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let mut adj = vec![Vec::<(usize, f32)>::new(); n];
    let mut degree = vec![1.0_f32; n];
    for e in edges {
        if let (Some(&a), Some(&b)) = (index.get(&e.source), index.get(&e.target)) {
            if a != b {
                let w = e.similarity.max(0.001);
                adj[a].push((b, w));
                adj[b].push((a, w));
                degree[a] += w;
                degree[b] += w;
            }
        }
    }
    let stationary = (0..n).map(|i| degree[i].sqrt()).collect::<Vec<_>>();
    let mut basis = (0..3)
        .map(|axis| {
            (0..n)
                .map(|i| {
                    let t = &entities[&ids[i]].entity.token;
                    t.iter()
                        .enumerate()
                        .map(|(j, v)| {
                            let p = ((j * 17 + axis * 43 + 7) % TOKEN_DIMENSIONS) as f32;
                            *v * (p * 0.6180339).sin()
                        })
                        .sum::<f32>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for v in &mut basis {
        orthogonalize(v, &stationary);
        normalize_vec(v);
    }
    // Block power iteration on the lazy normalized adjacency. Its leading trivial mode
    // is projected out, leaving the next three graph modes without a dense N-by-N L.
    for _ in 0..48 {
        let mut next = vec![vec![0.0; n]; 3];
        for axis in 0..3 {
            for i in 0..n {
                next[axis][i] = 0.5 * basis[axis][i];
                for &(j, w) in &adj[i] {
                    next[axis][i] += 0.5 * w * basis[axis][j] / (degree[i] * degree[j]).sqrt();
                }
            }
        }
        for axis in 0..3 {
            orthogonalize(&mut next[axis], &stationary);
            for prev in 0..axis {
                let p = dot_slice(&next[axis], &next[prev]);
                for i in 0..n {
                    next[axis][i] -= p * next[prev][i];
                }
            }
            normalize_vec(&mut next[axis]);
        }
        basis = next;
    }
    let raw = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            (
                id.clone(),
                normalize([basis[0][i], basis[1][i], basis[2][i]]),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let covariance = ids
        .iter()
        .filter_map(|id| Some((raw.get(id)?, previous.get(id)?)))
        .fold([[0.0_f32; 3]; 3], |mut matrix, (new, old)| {
            for row in 0..3 {
                for column in 0..3 {
                    matrix[row][column] += new[row] * old[column];
                }
            }
            matrix
        });
    if previous.is_empty() {
        return raw;
    }
    let rotation = procrustes_rotation(covariance);
    raw.into_iter()
        .map(|(id, direction)| (id, normalize(rotate_quaternion(rotation, direction))))
        .collect()
}

/// Davenport's 4x4 quaternion solution to the determinant-preserving orthogonal
/// Procrustes problem. It aligns current spectral coordinates to the previous frame.
fn procrustes_rotation(covariance: [[f32; 3]; 3]) -> [f32; 4] {
    let [[a, b, c], [d, e, f], [g, h, i]] = covariance;
    let trace = a + e + i;
    let mut matrix = [
        [trace, h - f, c - g, d - b],
        [h - f, a - e - i, b + d, c + g],
        [c - g, b + d, -a + e - i, f + h],
        [d - b, c + g, f + h, -a - e + i],
    ];
    let shift = covariance
        .iter()
        .flatten()
        .map(|v| v.abs())
        .sum::<f32>()
        .max(1.0);
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] += shift;
    }
    let mut quaternion = [1.0_f32, 0.0, 0.0, 0.0];
    for _ in 0..32 {
        let mut next = [0.0_f32; 4];
        for row in 0..4 {
            for column in 0..4 {
                next[row] += matrix[row][column] * quaternion[column];
            }
        }
        let norm = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm <= 1e-8 {
            return [1.0, 0.0, 0.0, 0.0];
        }
        for value in &mut next {
            *value /= norm;
        }
        quaternion = next;
    }
    quaternion
}

fn rotate_quaternion([w, x, y, z]: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let qv = [x, y, z];
    let twice = scale(cross(qv, v), 2.0);
    add(v, add(scale(twice, w), cross(qv, twice)))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn project_collisions(nodes: &mut [OrbNode]) {
    // Bounded spatial hash: only nearby cells are compared, never all node pairs.
    let cell = 0.035_f32;
    let mut buckets = BTreeMap::<(i32, i32, i32), Vec<usize>>::new();
    for i in 0..nodes.len() {
        let p = nodes[i].position;
        let key = (
            (p[0] / cell).floor() as i32,
            (p[1] / cell).floor() as i32,
            (p[2] / cell).floor() as i32,
        );
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if let Some(others) = buckets.get(&(key.0 + x, key.1 + y, key.2 + z)) {
                        for &j in others {
                            let d = sub(nodes[i].position, nodes[j].position);
                            let dist = norm(d);
                            let need = nodes[i].render_radius + nodes[j].render_radius;
                            if dist < need {
                                let axis = if dist > 1e-5 {
                                    scale(d, 1.0 / dist)
                                } else {
                                    stable_id_direction(&format!("{}:{}", nodes[i].id, nodes[j].id))
                                };
                                let push = scale(axis, (need - dist) * 0.5);
                                nodes[i].position = add(nodes[i].position, push);
                                nodes[j].position = sub(nodes[j].position, push);
                            }
                        }
                    }
                }
            }
        }
        buckets.entry(key).or_default().push(i);
    }
}

fn simhash(v: &[f32]) -> u32 {
    let mut out = 0_u32;
    for bit in 0..32 {
        let s = v
            .iter()
            .enumerate()
            .map(|(i, x)| {
                if ((i * 1103515245 + bit * 12345) >> ((i + bit) % 13)) & 1 == 0 {
                    *x
                } else {
                    -*x
                }
            })
            .sum::<f32>();
        if s >= 0.0 {
            out |= 1 << bit;
        }
    }
    out
}
fn token_direction_for(id: &str, t: &[f32]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (i, v) in t.iter().enumerate() {
        out[i % 3] += *v * ((i as f32 + 1.0) * 0.017).sin();
    }
    if norm(out) > 1e-7 {
        normalize(out)
    } else {
        stable_id_direction(id)
    }
}
fn stable_id_direction(id: &str) -> [f32; 3] {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in id.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    let z = ((hash as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0;
    let phase = ((hash >> 32) as u32 as f32 / u32::MAX as f32) * std::f32::consts::TAU;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    [ring * phase.cos(), z, ring * phase.sin()]
}
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let d = (0..n).map(|i| a[i] * b[i]).sum::<f32>();
    let aa = (0..n).map(|i| a[i] * a[i]).sum::<f32>().sqrt();
    let bb = (0..n).map(|i| b[i] * b[i]).sum::<f32>().sqrt();
    if aa * bb <= f32::EPSILON {
        0.0
    } else {
        (d / (aa * bb)).clamp(-1.0, 1.0)
    }
}
fn orthogonalize(v: &mut [f32], against: &[f32]) {
    let d = dot_slice(v, against);
    let den = dot_slice(against, against).max(f32::EPSILON);
    for i in 0..v.len() {
        v[i] -= d / den * against[i];
    }
}
fn normalize_vec(v: &mut [f32]) {
    let n = dot_slice(v, v).sqrt();
    if n > 1e-9 {
        for x in v {
            *x /= n;
        }
    }
}
fn dot_slice(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let n = norm(a);
    if n > 1e-7 {
        scale(a, 1.0 / n)
    } else {
        [0.0, 0.0, 1.0]
    }
}
fn finite_vec(a: [f32; 3]) -> bool {
    a.into_iter().all(f32::is_finite)
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
