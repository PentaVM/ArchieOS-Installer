//! # archie-toolpack
//!
//! Tools Archie installs for itself.
//!
//! When a task needs software the node does not have (a document renderer, a PDF tool, a
//! solver), Archie can fetch it from a release, check it, and put it in place without a
//! person running an installer. This crate is that path, and the places it can go wrong
//! are the ones it is built around.
//!
//! ```text
//! catalog.toml + catalog.toml.sig
//!   │  signature verified against trusted keys before a byte is parsed
//!   ▼
//! Catalog ── every URL, digest, size and path validated
//!   │
//!   ▼
//! InstallGate ── a one-operation lease to write the tool's directory
//!   │           └ risk class: read-only and workspace tools install on the lease;
//!   │             network and device tools wait for a person; system tools never
//!   │  AuthorizedInstall   (no public constructor)
//!   ▼
//! download ── size enforced while streaming, SHA-256 at the end, redirects only to
//!   │         catalog-named HTTPS hosts; a failed download leaves no file
//!   ▼
//! ToolRoot::install ── unpack in staging (no `..`, no absolute paths, no links, capped),
//!                      one rename into place, one rename of the `current` pointer,
//!                      rollback to `previous`
//! ```
//!
//! What this crate does not do: run the tools (the executor does that, inside a workspace
//! the task was granted), or install system packages. Those come from the system package
//! manager, behind administrative approval, and a catalog that lists one is invalid.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod fetch;
pub mod gate;
pub mod install;
pub mod signature;
pub mod unpack;

pub use catalog::{Artifact, Catalog, CatalogError, Format, Risk, ToolEntry, CATALOG_VERSION};
pub use fetch::{download, FetchError};
pub use gate::{
    tool_resource, AuthorizedInstall, InstallAdmission, InstallGate, InstallProposal,
    InstallRefusal,
};
pub use install::{InstallError, Installed, ToolRoot};
pub use signature::{open_catalog, sign, OpenError, SignatureError, TrustedKeys};
pub use unpack::{unpack_tar_gz, Limits, UnpackError, Unpacked};
