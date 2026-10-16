//! Sentry coordination belongs to Core; proposals and adapters cannot grant authority.
#![forbid(unsafe_code)]

pub mod adapters;
pub mod contracts;
pub mod engine;
pub mod feed;
pub mod journal;
pub mod page;
mod postgres_journal;
pub mod registry;
