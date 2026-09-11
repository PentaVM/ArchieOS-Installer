//! Trusted bounded graph read views matching Cortex IPC v1. Raw evidence stays in
//! the vault. Claim verification remains in `claim`; this is a cited projection,
//! not an alternative verification or authority system.
use crate::cognitive::{reference, FocusRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum NodeKind {
    Person = 0,
    Message = 1,
    Conversation = 2,
    Claim = 3,
    Event = 4,
    Place = 5,
    Device = 6,
    Sensor = 7,
    Project = 8,
    Task = 9,
    Process = 10,
    Model = 11,
    Document = 12,
    Paper = 13,
    Equation = 14,
    Component = 15,
    Simulation = 16,
    Company = 17,
    Country = 18,
    NewsArticle = 19,
    ProviderObservation = 20,
    Episode = 21,
    Unknown = 22,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum EdgeKind {
    Said = 0,
    Mentions = 1,
    ReplyTo = 2,
    Knows = 3,
    FriendOf = 4,
    ParentOf = 5,
    WorksWith = 6,
    Owns = 7,
    LocatedAt = 8,
    Uses = 9,
    DependsOn = 10,
    Produced = 11,
    Contradicts = 12,
    Supports = 13,
    DerivedFrom = 14,
    ObservedBy = 15,
    RelatedTo = 16,
    ExecutedOn = 17,
    BlockedBy = 18,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Sensitivity {
    Public = 0,
    LocalRestricted = 1,
    PersonalPrivate = 2,
    Secret = 3,
    Biometric = 4,
    Location = 5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: String,
    /// Stable RFC-009 wire enum: Person=0 ... Episode=21, Unknown=22.
    pub kind: NodeKind,
    pub summary: String,
    pub aliases: Vec<String>,
    pub provenance_ref: String,
    /// Explicit labels: public=0, local=1, private=2, secret=3, biometric=4, location=5.
    pub sensitivity: Sensitivity,
    pub observed_at_ms: i64,
    pub version: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    /// Stable RFC-009 wire enum: Said=0 ... BlockedBy=18.
    pub kind: EdgeKind,
    pub provenance_ref: String,
    pub observed_at_ms: i64,
    pub version: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshot {
    pub snapshot_id: String,
    pub captured_at_ms: i64,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl GraphSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if !reference(&self.snapshot_id)
            || self.captured_at_ms <= 0
            || self.nodes.len() > 4096
            || self.edges.len() > 16384
        {
            return Err("invalid snapshot identity, time or size".into());
        }
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            if !reference(&n.id)
                || !ids.insert(&n.id)
                || n.summary.len() > 8192
                || n.aliases.len() > 256
                || n.aliases.iter().any(|s| s.is_empty() || s.len() > 192)
                || !reference(&n.provenance_ref)
                || n.observed_at_ms <= 0
                || n.observed_at_ms > self.captured_at_ms
            {
                return Err("invalid graph node".into());
            }
        }
        let mut edges = BTreeSet::new();
        for e in &self.edges {
            if !reference(&e.id)
                || !edges.insert(&e.id)
                || ids.contains(&e.id)
                || !ids.contains(&e.source_id)
                || !ids.contains(&e.target_id)
                || !reference(&e.provenance_ref)
                || e.observed_at_ms <= 0
                || e.observed_at_ms > self.captured_at_ms
            {
                return Err("invalid graph edge".into());
            }
        }
        Ok(())
    }

    /// Privacy is applied before serializing, including edges through hidden nodes.
    /// Future observations are excluded even if supplied in a later snapshot.
    pub fn encode_policy_view(
        &self,
        permitted: &[Sensitivity],
        as_of_ms: i64,
    ) -> Result<Vec<u8>, String> {
        self.validate()?;
        if self.captured_at_ms > as_of_ms {
            return Err("historical export requires a historical snapshot, not timestamp filtering of current truth".into());
        }
        if permitted.is_empty() || permitted.len() > 6 || as_of_ms <= 0 {
            return Err("invalid policy view".into());
        }
        let nodes: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| permitted.contains(&n.sensitivity) && n.observed_at_ms <= as_of_ms)
            .collect();
        let ids: BTreeSet<_> = nodes.iter().map(|n| &n.id).collect();
        let edges: Vec<_> = self
            .edges
            .iter()
            .filter(|e| {
                ids.contains(&e.source_id)
                    && ids.contains(&e.target_id)
                    && e.observed_at_ms <= as_of_ms
            })
            .collect();
        let mut w = Wire(vec![1, 1]);
        w.text(&self.snapshot_id);
        w.i64(self.captured_at_ms.min(as_of_ms));
        w.u32(nodes.len() as u32);
        for n in nodes {
            w.text(&n.id);
            w.0.push(n.kind as u8);
            w.text(&n.summary);
            w.u32(n.aliases.len() as u32);
            for a in &n.aliases {
                w.text(a);
            }
            w.text(&n.provenance_ref);
            w.0.push(n.sensitivity as u8);
            w.i64(n.observed_at_ms);
            w.0.extend(n.version.to_be_bytes());
        }
        w.u32(edges.len() as u32);
        for e in edges {
            w.text(&e.id);
            w.text(&e.source_id);
            w.text(&e.target_id);
            w.0.push(e.kind as u8);
            w.text(&e.provenance_ref);
            w.i64(e.observed_at_ms);
            w.0.extend(e.version.to_be_bytes());
        }
        if w.0.len() > 32 * 1024 * 1024 {
            return Err("snapshot exceeds IPC limit".into());
        }
        Ok(w.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextQuery {
    pub request_id: String,
    pub question: String,
    pub active_project_id: String,
    pub snapshot_id: String,
    pub permitted_sensitivities: Vec<Sensitivity>,
    pub token_budget: u32,
    pub maximum_items: u16,
    pub maximum_expansion_hops: u8,
    pub requested_at_ms: i64,
    #[serde(default)]
    pub retrieval_cues: Vec<RetrievalCue>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalCue {
    pub grounding_node_id: String,
    pub question: String,
}
impl ContextQuery {
    pub fn encode(&self, focus: Option<&FocusRecord>) -> Result<Vec<u8>, String> {
        if !reference(&self.request_id)
            || !reference(&self.snapshot_id)
            || self.question.is_empty()
            || self.question.len() > 16384
            || self.active_project_id.len() > 192
            || self.permitted_sensitivities.is_empty()
            || self.permitted_sensitivities.len() > 6
            || !(64..=32768).contains(&self.token_budget)
            || self.maximum_items == 0
            || self.maximum_expansion_hops > 3
            || self.requested_at_ms <= 0
        {
            return Err("invalid ContextQuery".into());
        }
        let mut seen = BTreeSet::new();
        if self.retrieval_cues.len() > 8
            || self.retrieval_cues.iter().any(|c| {
                !reference(&c.grounding_node_id)
                    || c.question.is_empty()
                    || c.question.len() > 256
                    || !seen.insert((&c.grounding_node_id, &c.question))
            })
        {
            return Err("invalid bounded retrieval cues".into());
        }
        let version = if !self.retrieval_cues.is_empty() {
            3
        } else if focus.is_some() {
            2
        } else {
            1
        };
        let mut w = Wire(vec![version, 2]);
        w.text(&self.request_id);
        w.text(&self.question);
        w.text(&self.active_project_id);
        w.text(&self.snapshot_id);
        w.u32(self.permitted_sensitivities.len() as u32);
        w.0.extend(self.permitted_sensitivities.iter().map(|s| *s as u8));
        w.u32(self.token_budget);
        w.0.extend(self.maximum_items.to_be_bytes());
        w.0.push(self.maximum_expansion_hops);
        w.i64(self.requested_at_ms);
        if version == 3 {
            w.0.push(u8::from(focus.is_some()));
        }
        if let Some(f) = focus {
            f.validate()?;
            if f.updated_at_ms > self.requested_at_ms {
                return Err("future Focus cannot enter historical query".into());
            }
            w.text(&f.focus_id);
            w.0.push(u8::from(f.suspended));
            w.i64(f.half_life_ms);
            w.u32(f.activations.len() as u32);
            for a in &f.activations {
                w.text(&a.node_id);
                w.u32((a.strength * 1_000_000.0) as u32);
                w.i64(a.activated_at_ms);
                w.0.push(u8::from(a.pinned));
            }
        }
        if version == 3 {
            w.u32(self.retrieval_cues.len() as u32);
            for cue in &self.retrieval_cues {
                w.text(&cue.grounding_node_id);
                w.text(&cue.question);
            }
        }
        Ok(w.0)
    }
}
struct Wire(Vec<u8>);
impl Wire {
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_be_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend(v.to_be_bytes());
    }
    fn text(&mut self, v: &str) {
        self.u32(v.len() as u32);
        self.0.extend(v.as_bytes());
    }
}
