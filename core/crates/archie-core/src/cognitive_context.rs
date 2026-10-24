//! Compatibility exports for the backend-independent worker-resumption boundary.
//! Authority remains with the trusted Core policy adapter.
pub use archie_memory::resume::{
    collaboration_context, resume_context, CollaborationContext, ConstraintSource,
    WorkerResumeContext,
};
