//! Durable collaboration state refers to evidence, never transcripts or hidden CoT.
//! Proposed actions are references, not executor requests or policy permissions.
use crate::cognitive::reference;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtKind {
    Entity,
    Fact,
    Assumption,
    Hypothesis,
    Constraint,
    ProposedAction,
    Decision,
    Correction,
    Disagreement,
    UnresolvedReference,
    OpenQuestion,
    Artifact,
    ToolResult,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtStatus {
    Tentative,
    Accepted,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThoughtItem {
    pub id: String,
    pub kind: ThoughtKind,
    pub content_ref: String,
    pub status: ThoughtStatus,
    pub confidence: f64,
    pub evidence_refs: Vec<String>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedThoughtState {
    pub schema_version: u32,
    pub thought_id: String,
    pub revision: u64,
    pub updated_at_ms: i64,
    pub objective_ref: String,
    pub topic_ref: String,
    pub focus_id: String,
    pub execution_branch_ref: Option<String>,
    pub items: Vec<ThoughtItem>,
}
impl SharedThoughtState {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.revision == 0
            || self.revision > i64::MAX as u64
            || self.updated_at_ms <= 0
            || [
                &self.thought_id,
                &self.objective_ref,
                &self.topic_ref,
                &self.focus_id,
            ]
            .iter()
            .any(|r| !reference(r))
            || self
                .execution_branch_ref
                .as_ref()
                .is_some_and(|r| !reference(r))
            || self.items.len() > 128
        {
            return Err("invalid SharedThought envelope".into());
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            if !reference(&item.id)
                || !ids.insert(&item.id)
                || !reference(&item.content_ref)
                || !item.confidence.is_finite()
                || !(0.0..=1.0).contains(&item.confidence)
                || item.evidence_refs.is_empty()
                || item.evidence_refs.len() > 16
                || item.evidence_refs.iter().any(|r| !reference(r))
            {
                return Err("invalid thought item, confidence or provenance".into());
            }
            // Supersession edges point backward in this bounded append-only list.
            if let Some(old) = &item.supersedes {
                if old == &item.id
                    || !self
                        .items
                        .iter()
                        .take_while(|x| x.id != item.id)
                        .any(|x| &x.id == old && x.status == ThoughtStatus::Superseded)
                {
                    return Err("invalid supersession lineage".into());
                }
            }
        }
        Ok(())
    }
    pub fn validate_successor(&self, previous: &Self) -> Result<(), String> {
        self.validate()?;
        if self.thought_id != previous.thought_id
            || self.focus_id != previous.focus_id
            || self.objective_ref != previous.objective_ref
            || self.revision != previous.revision + 1
            || self.updated_at_ms < previous.updated_at_ms
            || self.items.len() < previous.items.len()
        {
            return Err("stale thought revision or changed continuity".into());
        }
        for (old, new) in previous.items.iter().zip(&self.items) {
            let mut permitted = old.clone();
            if old.status != ThoughtStatus::Superseded && new.status == ThoughtStatus::Superseded {
                permitted.status = ThoughtStatus::Superseded;
                if !self.items[previous.items.len()..]
                    .iter()
                    .any(|x| x.supersedes.as_ref() == Some(&old.id))
                {
                    return Err("supersession needs a new evidence-backed replacement".into());
                }
            }
            if &permitted != new {
                return Err("thought history cannot be rewritten".into());
            }
        }
        Ok(())
    }
    /// Local repair preserves objective, topic, Focus, execution and other items.
    pub fn repair(
        &self,
        old_id: &str,
        mut replacement: ThoughtItem,
        at_ms: i64,
    ) -> Result<Self, String> {
        self.validate()?;
        let mut next = self.clone();
        let old = next
            .items
            .iter_mut()
            .find(|x| x.id == old_id && x.status != ThoughtStatus::Superseded)
            .ok_or("repair target is missing or already superseded")?;
        old.status = ThoughtStatus::Superseded;
        replacement.supersedes = Some(old_id.to_owned());
        next.items.push(replacement);
        next.revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        next.updated_at_ms = at_ms;
        next.validate_successor(self)?;
        Ok(next)
    }
}
