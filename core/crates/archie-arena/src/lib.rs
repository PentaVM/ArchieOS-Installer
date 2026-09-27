//! Archie Arena: benchmarks of Archie using a computer.
//!
//! A task is said the way a person would say it — "copy this picture in Photopea" — with
//! whatever it needs beside it (a reference picture, documentation) and a time limit. Archie
//! then does it on the Archie Stage with the mouse and the keyboard alone: it looks at the
//! window, a model decides the next actions from the task, the capture, what its last
//! actions did and what it learned in earlier runs, and every action goes through the
//! Stage's gate like any other. There is no painter, player or pilot written for the task;
//! changing the task is changing the words and the application.
//!
//! When the time is up the result is judged — for a picture, against the reference, by
//! measures and by a critic model — and a reflection turns the run into lessons the next run
//! is given. Whether the scores rise run after run is what the arena is for: the RSI loop,
//! measured.
//!
//! ```text
//!  task ─► reset the application ─► ┌ look (Stage capture) ─► decide (model) ─► act (Stage gate) ┐
//!                                   └───────────── until the tier's seconds are spent ◄──────────┘
//!                                          ─► judge ─► reflect ─► lessons ─► the next run
//! ```

pub mod image;
pub mod memory;
pub mod model;
pub mod moves;
pub mod practice;
pub mod seat;
pub mod task;
