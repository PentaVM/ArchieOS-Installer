//! Wanda is an advisory mode of Archie, not a graph or model authority.
//! Person data is projected by the existing C++ GraphStore; private context
//! remains domain-scoped in Vault and inference uses the existing router.
#![forbid(unsafe_code)]

pub mod advice;
pub mod client;
pub mod config;
pub mod graph;
pub mod runtime;
pub mod server;
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
