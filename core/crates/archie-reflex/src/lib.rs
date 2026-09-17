//! # archie-reflex
//!
//! A small local model that finds an application's controls in a picture of its window.
//!
//! Archie prefers what an application says about itself: the accessibility bus gives exact
//! boxes, exact names and costs milliseconds. Not everything says anything — a canvas, a
//! game, a remote desktop, a toolkit with no bridge — and for those the only other local
//! answer is to look. This is that answer: IRIS's Visual Reflex, trained on what the
//! applications that do declare themselves already give away.
//!
//! That is the whole idea. Every window that declares its controls is a labelled example
//! of what a control looks like, free of charge and free of anybody's data: Archie captures
//! the window through the stage gate, asks the application where its controls are, and
//! keeps the pair. Nothing about the corpus is anybody's screen but the machine's own test
//! windows, and a tile is 160x96 greys — too small to read, large enough to see a button.
//!
//! What it does not do is read. A detector says "there is a button here"; it never says
//! "that button says Guardar". Naming is what declarations are for, and what a grounding
//! model is asked when nothing declares. So the reflex earns its place three ways:
//!
//! - it puts a grounding model's answer on a real control, instead of near one;
//! - it finds a control again after the window redrew it, without asking anything;
//! - it says what there is to click at all, so the slow question can be asked about a
//!   small part of a window instead of all of it.
//!
//! It advises. Cerebra decides, policy authorises, and the stage still has to be told.

#![forbid(unsafe_code)]

mod corpus;
mod model;
mod net;
mod tile;

pub use corpus::{Box2, ControlKind, Corpus, Sample, MAX_SAMPLES};
pub use model::{kind_of, Deployed};
pub use net::{Detector, Epoch, Prediction, Training, CELL, GRID_HEIGHT, GRID_WIDTH, MAX_CONTROLS};
pub use tile::{Tile, TILE_HEIGHT, TILE_WIDTH};

#[derive(Debug, thiserror::Error)]
pub enum ReflexError {
    #[error("the capture could not be read")]
    Unreadable,
    #[error("the corpus is not what it says it is")]
    Corpus,
    #[error("the checkpoint does not match its manifest")]
    Checkpoint,
    #[error("that is not a model this runtime knows")]
    Shape,
    #[error("reading or writing: {0}")]
    Io(#[from] std::io::Error),
}
