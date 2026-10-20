//! XaviCottier's delivered Lumenfield design, ported from the 2026-10-18 handoff.
//! Presentation only: no observation, filesystem scan, inference or authority APIs.
//! Source: codigo fuente/src/{field,hybrid-materials,quality-modes}.js and visual-presets/.

pub mod catalog;
mod ffi;
pub mod mesh;
pub mod policy;
pub mod shell;
pub mod simulation;
pub mod surface;

pub use catalog::*;
pub use ffi::*;
pub use simulation::{DesignBody, DesignMotion, DesignVec3};

#[cfg(test)]
mod tests;
