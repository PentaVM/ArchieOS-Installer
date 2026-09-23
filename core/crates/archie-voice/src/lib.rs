//! Archie's voice: what stands behind the speech plane.
//!
//! [`archie_speech`] decides who has the floor, when to speak and how much. It does not
//! decide what to say, and it does not act. This crate supplies those backends:
//!
//! - [`nemotron::NemotronCognitive`] — the cognitive plane, a Nemotron model on Nebius that
//!   answers in speakable pieces and proposes tools.
//! - [`tools::os_control`] — the tools it may propose: the stage's own vocabulary, so every
//!   proposal maps to an action the stage already checks and can refuse.
//! - [`segment::Segmenter`] — streamed text cut where a voice can stop.
//!
//! The shape follows the voice agents Hugging Face runs in production: listening, thinking
//! and speaking are separate stages rather than one network, so that the thinking can be a
//! model that uses tools and speaks Spanish, and so that each stage can be replaced without
//! the others noticing.

pub mod nemotron;
pub mod segment;
pub mod tools;

pub use nemotron::{NebiusSource, NemotronCognitive, TurnSource, TurnStream, Update, PERSONA_ES};
pub use segment::{Segmenter, Spoken};
