//! # archie-skill-node
//!
//! IRIS learns programs and games as skills: small LoRA adapters on its frozen base
//! (RFC-014). This crate turns one person's GPU machine into a node that trains and checks
//! them for a board everyone shares, and hands the work to an AI agent as MCP tools.
//!
//! * The agent connects through SSH (`ssh person@machine archie-skill-node mcp`), so the
//!   machine opens no port, and any agent that speaks MCP can use it.
//! * The board and the packages live in one cloud repository. Each change is a commit made
//!   with the token of the person whose node made it: the history is the credit.
//! * A package is used only after a node other than the one that trained it has checked it
//!   on its own sealed windows. Nobody promotes their own work.

#![forbid(unsafe_code)]

pub mod board;
pub mod hub;
pub mod mcp;
pub mod node;
pub mod tools;
