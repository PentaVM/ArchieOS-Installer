//! IRIS is Archie's typed perception fabric.
//!
//! It publishes a compact world model. It grants no authority and executes no action:
//! Cerebra interprets, policy authorizes, and an executor acts.

#![forbid(unsafe_code)]

pub mod cortex;
pub mod damage;
pub mod geometry;
pub mod reflex;
pub mod scene;
pub mod source;
pub mod specialist;

pub use cortex::{
    CortexError, CortexEscalationGate, CortexProposal, CortexRequest, KeyframeReference,
    VisualCortex,
};
pub use damage::{DamageConfig, DamageRegion, EphemeralFrame, PixelFormat, SemanticRoi};
pub use geometry::{Affine2D, Point, Rect};
pub use reflex::{
    DeterministicReflexBaseline, ModelManifest, ReflexBatchResult, ReflexDetection, ReflexError,
    ReflexSample, VisualReflexModel, VisualReflexRuntime,
};
pub use scene::{
    NodeId, NodeKind, Relation, RelationKind, SceneError, SceneEvent, SceneGraph, SceneNode,
    SourceId, TrackId,
};
pub use source::{PerceptionSource, SourceKind, SourceObservation, SourceSnapshot};
pub use specialist::{
    BiometricVerifier, GroundingSpecialist, NativeTextAdapter, ObjectSpecialist, OcrSpecialist,
    PoseSpecialist, SpecialistDescriptor, SpecialistError, SpecialistKind, UnconfiguredSpecialist,
    VisualRequest,
};
