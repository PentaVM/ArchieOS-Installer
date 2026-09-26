//! # archie-mcp-server
//!
//! Archie as a Model Context Protocol server, over stdio.
//!
//! An IDE agent (GitHub Copilot's agent mode in VS Code, Claude Code, Zed's agent panel)
//! calls this server's tools the way it calls any MCP server, and so works with the node as
//! a collaborator. It can see whether Archie runs, read the node's status, see which
//! modules loaded and why, and, when the operator allows it, operate applications on the
//! Archie Stage while a person watches. No IDE extension is involved: the IDE starts
//! this binary and speaks JSON-RPC over its standard input and output.
//!
//! # Whatever this server returns leaves the node
//!
//! A tool result goes into the IDE agent's context, and from there to whatever model that
//! agent uses. Archie cannot see that model, choose it, or verify what it retains. So this
//! server is treated the way routing treats an unverified cloud provider. Every tool
//! declares the most sensitive grade its result can carry. The server runs with a
//! [`Clearance`], and a tool above it is neither listed nor callable. The default is
//! `public`. `internal` is for an operator who has verified the IDE model's retention,
//! and it is the ceiling: sensitive, restricted and secret data never leave through here.
//!
//! # Actions
//!
//! Operating applications is off unless the server is started with it. Even then, it goes
//! through [`archie_stage::Operator`] like every other caller: the stage policy, a
//! one-operation lease, the gate, sealing, and the compositor's own checks. Those checks
//! include a person's takeover, which no tool can clear. The IDE agent is a principal of
//! its own and is never taken for a person at the machine, so nothing the policy holds
//! for approval runs from here.
//!
//! # Bridges
//!
//! `--bridge <id>=<folder>` serves one of [`archie_bridges`]' plugins: KiCad, video and
//! the others, each confined to the folder it is given. `archie_bridge_guide` hands a
//! client the guide to each before its first call.

#![forbid(unsafe_code)]

pub mod archie;
pub mod delivery;
pub mod server;
pub mod toolbox;

pub use archie::ArchieToolbox;
pub use server::{
    McpServer, MAX_IMAGE_BYTES, MAX_MESSAGE_BYTES, MAX_RESULT_BYTES, SUPPORTED_VERSIONS,
};
pub use toolbox::{Clearance, Effect, ToolImage, ToolOutcome, ToolSpec, Toolbox};
