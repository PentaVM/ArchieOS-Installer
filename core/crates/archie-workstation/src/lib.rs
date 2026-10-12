//! Workstation boundary for ArchieOS.
//!
//! This crate connects replaceable model and tool processes to the trusted control
//! plane. It does not plan, grant capabilities, or decide what is remembered.

#![forbid(unsafe_code)]

pub mod board_api;
pub mod enterprise_bridge;
pub mod freecad;
pub mod harness;
pub mod health;
pub mod host;
pub mod provider;
pub mod speech;
pub mod trace;

pub use health::{check_component, doctor, Check, CheckStatus};
