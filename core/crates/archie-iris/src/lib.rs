//! IRIS is Archie's typed perception fabric.
//!
//! It publishes a compact world model. It grants no authority and executes no action:
//! Cerebra interprets, policy authorizes, and an executor acts.

#![forbid(unsafe_code)]

pub mod acp;
pub mod cortex;
pub mod damage;
pub mod foundry;
pub mod geometry;
pub mod motor;
pub mod multisource;
pub mod reflex;
pub mod scene;
pub mod source;
pub mod specialist;
pub mod working_memory;

pub use acp::{
    benchmark_sizes, encode_checkpoint, encode_delta, AcpCheckpoint, AcpCompiler, AcpDelta,
    AcpError, AcpNode, AcpSizeBenchmark, ActionHorizon, TokenCounter,
};
pub use cortex::{
    CortexError, CortexEscalationGate, CortexProposal, CortexRequest, KeyframeReference,
    VisualCortex,
};
pub use damage::{DamageConfig, DamageRegion, EphemeralFrame, PixelFormat, SemanticRoi};
pub use foundry::FoundryReflexModel;
pub use geometry::{Affine2D, Point, Rect};
pub use motor::{
    MotorController, MotorError, MotorExecutor, MotorOutcome, PreparedMotorAction, SemanticAction,
};
pub use multisource::{
    InferenceDemand, MicroBatchPlan, MultiSourceError, MultiSourceRuntime, PerceptionSessionId,
    PlannedRoi, QueuedRoi,
};
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
pub use working_memory::{
    DurablePromotionCandidate, MemoryBounds, SceneDeltaRecord, SemanticEpisode, WorkingMemoryError,
    WorkingSceneMemory,
};
