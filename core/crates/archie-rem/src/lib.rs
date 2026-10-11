//! # archie-rem
//!
//! REM sleep (RFC-014): what the node does at night with nobody at it.
//!
//! During the day a person who opted in has the windows of their verified work kept,
//! encrypted, on the node. At night the supervisor pauses whatever long jobs are running,
//! hands those windows to the IRIS Foundry trainer, which trains and judges a skill
//! adapter against a sealed benchmark, then deletes the day's windows and resumes the
//! jobs. The trainer is the one place Python is allowed; everything here is the Rust
//! around it.
//!
//! Three promises shape the code, and each has a test:
//!
//! * **Retention is never extended.** The day's windows are deleted at the end of every
//!   cycle: trained, skipped, interrupted, or cut short by a crash and found again at
//!   start-up. Deleting is destroying the day's key ([`buffer::DayBuffer::shred`]), so
//!   ciphertext a failing disk kept is unreadable too.
//! * **Paused jobs come back.** The list of what was paused is on disk before anything is
//!   paused ([`jobs::ResumeList`]), and a job counts as resumed when its heartbeat moves,
//!   not when its process exists.
//! * **The person comes first.** REM starts only inside the night window, on mains power,
//!   with the machine untouched for a while ([`gate::Gate`]), and it stops when they come
//!   back: the trainer reads the activity file this crate writes ([`activity`]) at every
//!   step, and is killed if it has not left after a grace period.

#![forbid(unsafe_code)]

pub mod activity;
pub mod buffer;
pub mod cycle;
pub mod gate;
pub mod jobs;

pub use activity::Activity;
pub use buffer::{BufferError, DayBuffer};
pub use cycle::{Cycle, Plan, Report, Running, TrainerEnd, World};
pub use gate::{Conditions, Gate, NightWindow, Wait};
pub use jobs::{Job, JobError, ResumeList};
