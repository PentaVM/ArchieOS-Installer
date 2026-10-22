//! # archie-memory
//!
//! The librarian: provenance, evidence, and what counts as corroboration.
//!
//! Archie's source of truth is not a model's weights; it is the record store, and the
//! most consequential judgement that store makes is *how many sources actually said a
//! thing*. Twelve outlets carrying one wire story produce twelve records and one fact.
//! Getting that wrong does not merely inflate a number — it lets a single report acquire
//! the authority of a consensus, on a screen that looks authoritative either way.
//!
//! * [`evidence`] — provenance chains, republication collapse, independent-source
//!   counting, and great-circle geometry.
//! * [`cluster`] — grouping observations into candidate occurrences, conservatively.
//! * [`detector`] — versioned, auditable rules over that evidence, which ship in shadow
//!   mode and carry the version that produced every finding.
//! * [`claim`] — what Archie believes, on two orthogonal axes: how far verification has
//!   come, and what qualifications attach. Operator overrides are an overlay that never
//!   edits the canonical record.
//!
//! Both produce *candidates and counts*, never conclusions. An Event is written by the
//! layer above, and it may only claim `corroborated` when the count here supports it.

#![forbid(unsafe_code)]

pub mod admission;
pub mod analysis;
pub mod claim;
pub mod cluster;
pub mod cognitive;
pub mod correction;
pub mod detector;
pub mod evidence;
pub mod execution;
pub mod floor;
pub mod graph;
pub mod inference;
pub mod resume;
pub mod shared_thought;
pub mod storage;
pub mod utility;

pub use admission::{
    admit_learning, AdmissionBatch, AdmissionDecision, AdmissionError, AdmissionPolicy,
    AdmissionReason, ContentClass, CuratedTrainingRecord, LearningCandidate, MemoryDisposition,
    SensorSource, Supervision,
};
pub use analysis::{
    assess_hypothesis, gate_model_proposal, AnalysisError, AnalysisPolicy, AnalyticalAssessment,
    AnalyticalSignal, AssessmentState, EvidenceStance, ModelProposal, ProposalDecision,
    ProposalDisposition, ProposalError, ProposalReason, SignalDecision, SignalReason,
};
pub use claim::{
    Claim, ClaimFlag, OverrideContext, OverrideScope, PremiseView, UserOverride, VerificationStage,
};
pub use cluster::{cluster, contributing_sources, corroborated_clusters, Cluster, ClusterWindow};
pub use correction::{
    CorrectionError, CorrectionEvent, CorrectionKind, CorrectionTrajectory, GraphCorrectionReceipt,
};
pub use evidence::{assess_independence, GeoPoint, IndependenceReport, ObservationRef, SourceRef};
