//! Sentry coordination belongs to Core; proposals and adapters cannot grant authority.
#![forbid(unsafe_code)]

pub mod contracts;
pub mod engine;
pub mod journal;
pub mod registry;
