use crate::{NodeId, Rect, SourceId};
use archie_types::{Locality, PrivacyGrade};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_REFERENCES: usize = 64;
const MAX_TASK_CLASS: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyframeReference {
    pub source: SourceId,
    pub captured_at_ms: i64,
    pub roi: Rect,
    pub content_sha256: String,
}

impl KeyframeReference {
    fn validate(&self) -> bool {
        self.captured_at_ms >= 0
            && self.roi.validate().is_ok()
            && self.content_sha256.len() == 64
            && self
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CortexRequest {
    pub task_class: String,
    pub scene_revision: u64,
    pub nodes: Vec<NodeId>,
    pub keyframes: Vec<KeyframeReference>,
    pub privacy: PrivacyGrade,
}

impl CortexRequest {
    pub fn validate(&self) -> Result<(), CortexError> {
        if self.task_class.is_empty()
            || self.task_class.len() > MAX_TASK_CLASS
            || !self.task_class.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
            || self.nodes.len() > MAX_REFERENCES
            || self.keyframes.len() > MAX_REFERENCES
            || self.nodes.is_empty() && self.keyframes.is_empty()
            || !self.keyframes.iter().all(KeyframeReference::validate)
        {
            return Err(CortexError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CortexProposal {
    pub referenced_nodes: Vec<NodeId>,
    pub relation_codes: Vec<String>,
    pub confidence_permille: u16,
    pub uncertainty_permille: u16,
}

impl CortexProposal {
    pub fn validate_for(&self, request: &CortexRequest) -> Result<(), CortexError> {
        if self.referenced_nodes.is_empty()
            || self.referenced_nodes.len() > MAX_REFERENCES
            || self.confidence_permille > 1_000
            || self.uncertainty_permille > 1_000
            || self
                .referenced_nodes
                .iter()
                .any(|node| !request.nodes.contains(node))
            || self.relation_codes.len() > MAX_REFERENCES
            || self.relation_codes.iter().any(|code| {
                code.is_empty()
                    || code.len() > 64
                    || !code.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'_' | b'-')
                    })
            })
        {
            return Err(CortexError::InvalidProposal);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CortexError {
    #[error("visual cortex request is invalid")]
    InvalidRequest,
    #[error("visual cortex proposal is invalid")]
    InvalidProposal,
    #[error("selected locality cannot receive this privacy grade")]
    PrivacyDenied,
    #[error("visual cortex provider is unavailable")]
    NotConfigured,
}

pub trait VisualCortex: Send + Sync {
    fn locality(&self) -> Locality;
    fn infer(&self, request: &CortexRequest) -> Result<CortexProposal, CortexError>;
}

/// Event gate for the large model. It is called only for changed, task-relevant state whose
/// uncertainty remains above the configured bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CortexEscalationGate {
    pub uncertainty_permille: u16,
}

impl CortexEscalationGate {
    pub fn should_invoke(
        &self,
        scene_changed: bool,
        task_relevant: bool,
        uncertainty_permille: u16,
    ) -> bool {
        self.uncertainty_permille <= 1_000
            && uncertainty_permille <= 1_000
            && scene_changed
            && task_relevant
            && uncertainty_permille >= self.uncertainty_permille
    }

    pub fn invoke<P: VisualCortex>(
        &self,
        provider: &P,
        request: &CortexRequest,
    ) -> Result<CortexProposal, CortexError> {
        request.validate()?;
        if !request.privacy.permits(provider.locality()) {
            return Err(CortexError::PrivacyDenied);
        }
        let proposal = provider.infer(request)?;
        proposal.validate_for(request)?;
        Ok(proposal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Provider {
        locality: Locality,
        calls: AtomicUsize,
    }

    impl VisualCortex for Provider {
        fn locality(&self) -> Locality {
            self.locality
        }

        fn infer(&self, request: &CortexRequest) -> Result<CortexProposal, CortexError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CortexProposal {
                referenced_nodes: vec![request.nodes[0].clone()],
                relation_codes: vec!["holding".into()],
                confidence_permille: 800,
                uncertainty_permille: 200,
            })
        }
    }

    fn request(privacy: PrivacyGrade) -> CortexRequest {
        CortexRequest {
            task_class: "scene-relation".into(),
            scene_revision: 1,
            nodes: vec![NodeId::new("person:1").unwrap()],
            keyframes: Vec::new(),
            privacy,
        }
    }

    #[test]
    fn cortex_is_event_driven() {
        let gate = CortexEscalationGate {
            uncertainty_permille: 500,
        };
        assert!(!gate.should_invoke(false, true, 900));
        assert!(!gate.should_invoke(true, false, 900));
        assert!(!gate.should_invoke(true, true, 499));
        assert!(gate.should_invoke(true, true, 500));
    }

    #[test]
    fn restricted_scene_cannot_reach_cloud_provider() {
        let provider = Provider {
            locality: Locality::Cloud,
            calls: AtomicUsize::new(0),
        };
        let result = CortexEscalationGate {
            uncertainty_permille: 500,
        }
        .invoke(&provider, &request(PrivacyGrade::Restricted));
        assert_eq!(result, Err(CortexError::PrivacyDenied));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn proposal_cannot_add_unrequested_scene_nodes() {
        let request = request(PrivacyGrade::Public);
        let proposal = CortexProposal {
            referenced_nodes: vec![NodeId::new("secret:2").unwrap()],
            relation_codes: Vec::new(),
            confidence_permille: 900,
            uncertainty_permille: 100,
        };
        assert_eq!(
            proposal.validate_for(&request),
            Err(CortexError::InvalidProposal)
        );
    }
}
