//! The realtime speech plane.
//!
//! Speech is a sensory and motor interface to Cerebra, not a second assistant. Nothing
//! here decides what Archie means or what Archie may do — it decides who has the floor,
//! when to speak, and how much. What to *say* comes from the cognitive plane, and what
//! Archie is permitted to *do* comes from Policy.
//!
//! The separation matters because the alternative is a voice model that answers, acts and
//! remembers on its own, and then Archie's behaviour is whatever that checkpoint happens
//! to do this month.

pub mod adapter;
pub mod cognitive;
pub mod estimator;
pub mod floor;
pub mod session;

pub use adapter::{
    deduplicate_tool_intents, SpeechCapabilities, SpeechToolIntentCandidate, SpokenContentSource,
};
pub use cognitive::{
    validate_request, CancelReason, CognitiveError, CognitiveEvent, CognitiveModelAdapter,
    CognitiveRequest, ResponseCommitLog, ResponseGate, ResponseSegment, ScriptedCognitiveModel,
    ScriptedReply,
};
pub use floor::{
    ConversationalFloorState, FloorAction, FloorDecision, FloorPolicy, InterruptionDirection,
    OverlapState, Probability, SilenceScope, Speaker, SpeechBudget,
};
pub use session::{SessionError, SessionEvent, SessionPlan, SpeechSession, SpokenSegment};
