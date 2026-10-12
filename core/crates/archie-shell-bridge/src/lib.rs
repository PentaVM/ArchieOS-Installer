//! # archie-shell-bridge
//!
//! The adapter between the professional shell (Quickshell/QML) and the node, phase C of
//! the GUI work (`docs/gui/ARCHITECTURE_GUI.md`, ADR-GUI-005).
//!
//! It is a **client with no authority**. It reads what `archie-displayd` already projects
//! (a loopback, read-only HTTP bridge whose snapshot says whether it is `live` or a
//! `fixture`) and turns it into one JSON line per poll with an explicit link state. It
//! has no request that mutates anything, opens no socket for others to connect to, and
//! refuses any address that is not loopback.
//!
//! The shell runs it as a child process with a fixed argument list and reads its standard
//! output. QML never builds a command line from user input.
//!
//! What it does not do, on purpose: answer Policy escalations, pause authority, list
//! tasks, touch files. `archied` exposes no contract for those yet (see
//! `GUI-POLICY-001`, `GUI-POLICY-002`, `GUI-EXEC-001` in the backlog), and a control
//! drawn without one would be a control that does nothing.

#![forbid(unsafe_code)]

pub mod client;
pub mod protocol;

pub use client::{Client, ClientError};
pub use protocol::{poll, Frame, Link, System, SCHEMA};
