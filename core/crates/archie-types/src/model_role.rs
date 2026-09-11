//! What a model is *for*, as opposed to which model it is.
//!
//! Archie runs several models at once with different jobs: one hears, one thinks, one
//! speaks. A request has to name the job — "I need something that can render Spanish
//! speech" — because naming the checkpoint is how a system ends up with `if model ==
//! "qwen"` scattered through it, and then a new model means a rewrite.
//!
//! # Why a role is a capability and not a new field
//!
//! The obvious implementation is a `required_role` beside `required_capabilities` on
//! every routing request. It is the wrong one. The router already selects by capability,
//! with hierarchical matching, privacy gating and a documented refusal code for a missing
//! one; a second axis would mean two selection mechanisms in one router, two places to
//! check, and two ways for them to disagree about the same model.
//!
//! So a role *compiles to* a capability. `ModelRole::CognitiveResponse` is
//! `cognition.response`, and asking for it is `RoutingRequest::requiring(role.into())`.
//! Nothing in the router changes, and a model advertises a role by listing its capability
//! like any other.
//!
//! The hierarchy comes free. `Capability::satisfies` is segment-wise, so a model that
//! advertises `speech.frontend.duplex` satisfies a request for `speech.frontend` while a
//! request for the duplex form still refuses everything that only listens.

use crate::capability::Capability;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A job a model can hold in the fabric.
///
/// A checkpoint may hold several: a unified speech model is a frontend *and* a renderer,
/// and says so by advertising both capabilities. Nothing here assumes one model has one
/// job, because the models that combine roles are exactly the ones worth having.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// Hears. Streaming understanding, prosody, overlap, floor evidence.
    SpeechFrontend,
    /// Speaks. Turns text into audio, and can be told to pause, duck or yield.
    SpeechRenderer,
    /// Hears and speaks in one network, listening while it talks.
    ///
    /// Distinct from holding both of the above: a model that merely does each in turn is
    /// not full duplex, and the difference is audible.
    SpeechUnified,
    /// Thinks. The substantive answer.
    CognitiveResponse,
    /// Thinks harder and slower, for work that earns the wait.
    ReasoningWorker,
    /// Sees.
    VisionModel,
    EmbeddingModel,
    Reranker,
    /// Speech to text, without generating a reply.
    Asr,
    /// Text to speech, without deciding what to say.
    Tts,
    /// Whether this is the owner speaking.
    SpeakerVerification,
    /// Who spoke when.
    Diarization,
}

impl ModelRole {
    /// Every role, for menus, manifests and exhaustiveness tests.
    pub const ALL: [ModelRole; 12] = [
        Self::SpeechFrontend,
        Self::SpeechRenderer,
        Self::SpeechUnified,
        Self::CognitiveResponse,
        Self::ReasoningWorker,
        Self::VisionModel,
        Self::EmbeddingModel,
        Self::Reranker,
        Self::Asr,
        Self::Tts,
        Self::SpeakerVerification,
        Self::Diarization,
    ];

    /// The capability that means "can do this job".
    ///
    /// Two families, chosen so the hierarchy is useful rather than decorative: everything
    /// in the realtime speech plane sits under `speech.`, and everything that reasons
    /// sits under `cognition.`. A node that wants "any speech model at all" can ask for
    /// the family; a node that needs a renderer asks for the leaf.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpeechFrontend => "speech.frontend",
            Self::SpeechRenderer => "speech.renderer",
            Self::SpeechUnified => "speech.unified",
            Self::CognitiveResponse => "cognition.response",
            Self::ReasoningWorker => "cognition.reasoning",
            Self::VisionModel => "cognition.vision",
            Self::EmbeddingModel => "cognition.embedding",
            Self::Reranker => "cognition.rerank",
            Self::Asr => "speech.asr",
            Self::Tts => "speech.tts",
            Self::SpeakerVerification => "speech.speaker_verification",
            Self::Diarization => "speech.diarization",
        }
    }

    /// The capability a model advertises to claim this role.
    ///
    /// Infallible by construction: every string above is a valid two-segment capability,
    /// and a test walks all of them so a new role with a bad name cannot compile past it.
    pub fn capability(self) -> Capability {
        Capability::parse(self.as_str())
            .expect("every model role name is a valid capability by construction")
    }

    /// The role a capability names, when it names one.
    ///
    /// `None` for a capability outside the role families — `compute.inference` is a real
    /// capability and not a role, and quietly turning it into one would let a model claim
    /// a job nobody gave it.
    pub fn from_capability(capability: &Capability) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|role| capability.satisfies(&role.capability()))
    }
}

impl fmt::Display for ModelRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ModelRole> for Capability {
    fn from(role: ModelRole) -> Self {
        role.capability()
    }
}

/// How much is actually known about whether a model runs on a given node.
///
/// The distinction this type exists for is between *measured* and *guessed*. A wizard
/// that shows "compatible" because 27 billion parameters at four bits fits in twenty-four
/// gigabytes on paper has told somebody a number, not a fact, and they will find out
/// which it was after the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    /// It ran here, on this node, and was observed to work.
    SupportedMeasured,
    /// The vendor documents this hardware as supported. Nobody here has run it.
    SupportedDocumented,
    /// It looks like it should fit. This is arithmetic, not evidence.
    CompatibleEstimated,
    /// The vendor documents this hardware as unsupported.
    UnsupportedDocumented,
    /// Not enough memory, disk or accelerator for the variant selected.
    InsufficientResources,
    /// The engine it needs is not installed.
    MissingRuntime,
    /// A dependency is absent — a codec, a driver, an architecture-specific wheel.
    MissingDependency,
    /// Nothing has been determined.
    Unknown,
}

impl CompatibilityStatus {
    /// Whether somebody watched this work.
    ///
    /// The only status that may be promoted to "this node runs this model". Everything
    /// else is a prediction, including the vendor's own documentation: a model card
    /// listing an H100 is evidence about NVIDIA's test rig, not about this machine.
    pub const fn is_measured(self) -> bool {
        matches!(self, Self::SupportedMeasured)
    }

    /// Whether it is worth offering to install.
    ///
    /// Deliberately wider than `is_measured`: refusing to offer anything unmeasured means
    /// nothing can ever be measured, because it could never be installed to try.
    pub const fn is_installable(self) -> bool {
        matches!(
            self,
            Self::SupportedMeasured | Self::SupportedDocumented | Self::CompatibleEstimated
        )
    }

    /// What to tell somebody looking at the wizard.
    pub const fn explain(self) -> &'static str {
        match self {
            Self::SupportedMeasured => "measured on this node",
            Self::SupportedDocumented => "documented as supported; not measured here",
            Self::CompatibleEstimated => "estimated to fit; not measured",
            Self::UnsupportedDocumented => "documented as unsupported",
            Self::InsufficientResources => "this node does not have the memory it needs",
            Self::MissingRuntime => "the engine it runs on is not installed",
            Self::MissingDependency => "a dependency it needs is missing",
            Self::Unknown => "not determined",
        }
    }
}
