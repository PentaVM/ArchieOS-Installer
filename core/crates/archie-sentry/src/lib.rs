//! Sentry coordination belongs to Core; proposals and adapters cannot grant authority.
#![forbid(unsafe_code)]

pub mod adapters;
pub mod contracts;
pub mod engine;
pub mod journal;
mod postgres_journal;
pub mod registry;
