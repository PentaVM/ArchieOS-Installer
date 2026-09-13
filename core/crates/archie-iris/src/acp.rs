use crate::{NodeId, NodeKind, Rect, SceneGraph, SourceId};
use archie_types::PrivacyGrade;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const ACP_VERSION: u16 = 1;
const MAX_NODES: usize = 256;
const MAX_GOAL_CODE: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpNode {
    #[serde(rename = "i")]
    pub id: NodeId,
    #[serde(rename = "s")]
    pub source: SourceId,
    #[serde(rename = "k")]
    pub kind: NodeKind,
    #[serde(rename = "b", skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeId>,
    #[serde(rename = "c")]
    pub confidence_permille: u16,
    #[serde(rename = "u")]
    pub uncertainty_permille: u16,
    #[serde(rename = "l", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpCheckpoint {
    pub version: u16,
    pub goal: String,
    pub scene_revision: u64,
    pub nodes: Vec<AcpNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpDelta {
    pub version: u16,
    pub goal: String,
    pub base_revision: u64,
    pub scene_revision: u64,
    pub upserts: Vec<AcpNode>,
    pub deletes: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionHorizon {
    pub scene_revision: u64,
    pub action_codes: Vec<String>,
    pub node_ids: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AcpError {
    #[error("ACP request is malformed or outside its bound")]
    InvalidRequest,
    #[error("ACP requested a missing scene node")]
    MissingNode,
    #[error("secret material cannot reach ACP")]
    SecretDenied,
    #[error("ACP encoding failed")]
    Encoding,
    #[error("ACP checkpoint does not belong to the same goal")]
    GoalMismatch,
}

pub struct AcpCompiler {
    include_labels: bool,
}

impl AcpCompiler {
    pub const fn new(include_labels: bool) -> Self {
        Self { include_labels }
    }

    pub fn checkpoint(
        &self,
        scene: &SceneGraph,
        goal: &str,
        requested: &[NodeId],
        privacy: PrivacyGrade,
    ) -> Result<AcpCheckpoint, AcpError> {
        validate_request(goal, requested, privacy)?;
        let mut seen = BTreeSet::new();
        let mut nodes = Vec::with_capacity(requested.len());
        for id in requested {
            if !seen.insert(id.clone()) {
                continue;
            }
            let node = scene.node(id).ok_or(AcpError::MissingNode)?;
            nodes.push(AcpNode {
                id: node.id.clone(),
                source: node.source.clone(),
                kind: node.kind,
                bbox: node.bbox,
                parent: node
                    .parent
                    .as_ref()
                    .filter(|parent| requested.contains(parent))
                    .cloned(),
                confidence_permille: node.confidence_permille,
                uncertainty_permille: node.uncertainty_permille,
                label: self.include_labels.then(|| node.label.clone()).flatten(),
            });
        }
        Ok(AcpCheckpoint {
            version: ACP_VERSION,
            goal: goal.to_owned(),
            scene_revision: scene.revision(),
            nodes,
        })
    }

    pub fn delta(
        &self,
        previous: &AcpCheckpoint,
        current: &AcpCheckpoint,
    ) -> Result<AcpDelta, AcpError> {
        if previous.goal != current.goal {
            return Err(AcpError::GoalMismatch);
        }
        let old: BTreeMap<_, _> = previous
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        let new: BTreeMap<_, _> = current
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        let upserts = current
            .nodes
            .iter()
            .filter(|node| old.get(&node.id).is_none_or(|before| **before != **node))
            .cloned()
            .collect();
        let deletes = previous
            .nodes
            .iter()
            .filter(|node| !new.contains_key(&node.id))
            .map(|node| node.id.clone())
            .collect();
        Ok(AcpDelta {
            version: ACP_VERSION,
            goal: current.goal.clone(),
            base_revision: previous.scene_revision,
            scene_revision: current.scene_revision,
            upserts,
            deletes,
        })
    }
}

fn validate_request(
    goal: &str,
    requested: &[NodeId],
    privacy: PrivacyGrade,
) -> Result<(), AcpError> {
    if privacy == PrivacyGrade::Secret {
        return Err(AcpError::SecretDenied);
    }
    if goal.is_empty()
        || goal.len() > MAX_GOAL_CODE
        || !goal.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
        || requested.is_empty()
        || requested.len() > MAX_NODES
    {
        return Err(AcpError::InvalidRequest);
    }
    Ok(())
}

pub fn encode_checkpoint(checkpoint: &AcpCheckpoint) -> Result<Vec<u8>, AcpError> {
    serde_json::to_vec(checkpoint).map_err(|_| AcpError::Encoding)
}

pub fn encode_delta(delta: &AcpDelta) -> Result<Vec<u8>, AcpError> {
    serde_json::to_vec(delta).map_err(|_| AcpError::Encoding)
}

pub trait TokenCounter {
    fn tokenizer_id(&self) -> &str;
    fn count_tokens(&self, bytes: &[u8]) -> Result<u32, AcpError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpSizeBenchmark {
    pub tokenizer_id: Option<String>,
    pub checkpoint_bytes: usize,
    pub delta_bytes: usize,
    pub checkpoint_tokens: Option<u32>,
    pub delta_tokens: Option<u32>,
}

pub fn benchmark_sizes(
    checkpoint: &AcpCheckpoint,
    delta: &AcpDelta,
    tokenizer: Option<&dyn TokenCounter>,
) -> Result<AcpSizeBenchmark, AcpError> {
    let checkpoint_bytes = encode_checkpoint(checkpoint)?;
    let delta_bytes = encode_delta(delta)?;
    let (tokenizer_id, checkpoint_tokens, delta_tokens) = if let Some(tokenizer) = tokenizer {
        (
            Some(tokenizer.tokenizer_id().to_owned()),
            Some(tokenizer.count_tokens(&checkpoint_bytes)?),
            Some(tokenizer.count_tokens(&delta_bytes)?),
        )
    } else {
        (None, None, None)
    };
    Ok(AcpSizeBenchmark {
        tokenizer_id,
        checkpoint_bytes: checkpoint_bytes.len(),
        delta_bytes: delta_bytes.len(),
        checkpoint_tokens,
        delta_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SceneEvent, SceneNode};

    fn scene() -> SceneGraph {
        let mut scene = SceneGraph::default();
        let source = SourceId::new("browser:1").unwrap();
        let nodes = [
            ("button:1", NodeKind::Button, "Create account"),
            ("input:1", NodeKind::Input, "email"),
            ("secret:1", NodeKind::Text, "private note"),
        ];
        scene
            .apply(
                0,
                &nodes.map(|(id, kind, label)| {
                    SceneEvent::Upsert(SceneNode {
                        id: NodeId::new(id).unwrap(),
                        source: source.clone(),
                        kind,
                        bbox: Some(Rect::new(0, 0, 10, 10).unwrap()),
                        parent: None,
                        track_id: None,
                        confidence_permille: 1_000,
                        uncertainty_permille: 0,
                        last_seen_ms: 1,
                        label: Some(label.into()),
                        revision: 0,
                    })
                }),
            )
            .unwrap();
        scene
    }

    #[test]
    fn compiler_never_adds_an_unrequested_node() {
        let checkpoint = AcpCompiler::new(true)
            .checkpoint(
                &scene(),
                "create-account",
                &[NodeId::new("button:1").unwrap()],
                PrivacyGrade::Internal,
            )
            .unwrap();
        assert_eq!(checkpoint.nodes.len(), 1);
        assert_eq!(checkpoint.nodes[0].id.as_str(), "button:1");
        let encoded = String::from_utf8(encode_checkpoint(&checkpoint).unwrap()).unwrap();
        assert!(!encoded.contains("private note"));
        assert!(!encoded.contains("secret:1"));
    }

    #[test]
    fn delta_contains_only_changed_and_deleted_state() {
        let compiler = AcpCompiler::new(false);
        let old = compiler
            .checkpoint(
                &scene(),
                "create-account",
                &[
                    NodeId::new("button:1").unwrap(),
                    NodeId::new("input:1").unwrap(),
                ],
                PrivacyGrade::Internal,
            )
            .unwrap();
        let new = compiler
            .checkpoint(
                &scene(),
                "create-account",
                &[NodeId::new("button:1").unwrap()],
                PrivacyGrade::Internal,
            )
            .unwrap();
        let delta = compiler.delta(&old, &new).unwrap();
        assert!(delta.upserts.is_empty());
        assert_eq!(delta.deletes, vec![NodeId::new("input:1").unwrap()]);
    }

    struct FixtureTokenizer;

    impl TokenCounter for FixtureTokenizer {
        fn tokenizer_id(&self) -> &str {
            "fixture-byte-pairs-v1"
        }

        fn count_tokens(&self, bytes: &[u8]) -> Result<u32, AcpError> {
            Ok(bytes.chunks(4).count() as u32)
        }
    }

    #[test]
    fn token_claim_exists_only_with_a_named_tokenizer() {
        let checkpoint = AcpCompiler::new(false)
            .checkpoint(
                &scene(),
                "create-account",
                &[NodeId::new("button:1").unwrap()],
                PrivacyGrade::Internal,
            )
            .unwrap();
        let delta = AcpCompiler::new(false)
            .delta(&checkpoint, &checkpoint)
            .unwrap();
        let bytes_only = benchmark_sizes(&checkpoint, &delta, None).unwrap();
        assert_eq!(bytes_only.checkpoint_tokens, None);
        let measured = benchmark_sizes(&checkpoint, &delta, Some(&FixtureTokenizer)).unwrap();
        assert_eq!(
            measured.tokenizer_id.as_deref(),
            Some("fixture-byte-pairs-v1")
        );
        assert!(measured.checkpoint_tokens.unwrap() > 0);
    }

    #[test]
    fn secret_grade_is_refused_before_encoding() {
        assert_eq!(
            AcpCompiler::new(false).checkpoint(
                &scene(),
                "create-account",
                &[NodeId::new("button:1").unwrap()],
                PrivacyGrade::Secret,
            ),
            Err(AcpError::SecretDenied)
        );
    }
}
