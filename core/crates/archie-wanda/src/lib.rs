//! Wanda is an advisory mode of Archie, not a graph or model authority.
//! Person data is projected by the existing C++ GraphStore; private context
//! remains domain-scoped in Vault and inference uses the existing router.
#![forbid(unsafe_code)]

pub mod advice;
pub mod chat;
pub mod client;
pub mod config;
pub mod decision;
pub mod graph;
pub mod runtime;
pub mod server;
pub mod social;
pub mod store;
pub mod types;

#[derive(Debug, thiserror::Error)]
pub enum WandaError {
    #[error("invalid Wanda configuration: {0}")]
    Config(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("this context domain is not configured")]
    UnknownDomain,
    #[error("event refused: {0}")]
    InvalidEvent(&'static str),
    #[error("input exceeds the Wanda budget")]
    TooLarge,
    #[error("Wanda storage is unavailable")]
    Storage,
    #[error("the person graph is unavailable")]
    GraphUnavailable,
    #[error("no eligible reasoning model is available")]
    NoModel,
    #[error("the model response did not cite admitted evidence")]
    InvalidAdvice,
    #[error("Wanda transport failed")]
    Transport,
    #[error("Wanda is already reasoning; try again later")]
    Busy,
}

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
pub const MAX_BATCH: usize = 32;
pub const MAX_CONTEXT: usize = 64;
/// How long a recommendation may stand, and so how long the evidence it cites is kept.
pub const ADVICE_HORIZON_MS: i64 = 24 * 3_600_000;
/// The share of one query's context the person graph may take.
pub const GRAPH_CONTEXT_BYTES: usize = 16 * 1024;
/// One query's whole context: question, graph and events.
pub const QUERY_CONTEXT_BYTES: usize = 32 * 1024;
