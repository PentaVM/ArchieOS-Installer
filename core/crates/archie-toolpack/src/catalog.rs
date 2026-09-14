//! The catalog: which tools exist, where each one's artifact is, and what it may do.
//!
//! A catalog is data an agent acts on, so it is checked like input from outside, because it
//! is: every URL, digest, size and path in it is validated before anything can be built
//! from it, and fields this build does not know are an error rather than ignored.

use reqwest::Url;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub const CATALOG_VERSION: u32 = 1;
/// The largest catalog read at all.
pub const MAX_CATALOG_BYTES: usize = 1024 * 1024;
/// The largest artifact a catalog may declare.
pub const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// What a tool can do once installed, in the executor's risk classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    ReadOnly,
    WorkspaceWrite,
    NetworkEgress,
    DeviceAccess,
    SystemAdmin,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WorkspaceWrite => "workspace_write",
            Self::NetworkEgress => "network_egress",
            Self::DeviceAccess => "device_access",
            Self::SystemAdmin => "system_admin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// A gzip-compressed tarball.
    TarGz,
    /// The artifact is the binary itself.
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub url: String,
    /// Lowercase hex SHA-256 of the artifact as downloaded.
    pub sha256: String,
    /// Its exact size in bytes.
    pub size: u64,
    pub format: Format,
    /// Executables the tool provides, as paths inside the unpacked artifact. A binary
    /// artifact names one: the file name it is installed as.
    pub binaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolEntry {
    pub id: String,
    pub title: String,
    pub version: String,
    pub license: String,
    pub risk: Risk,
    /// Archie capabilities the tool satisfies, such as `document.render`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub artifact: Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub name: String,
    /// Hosts a download may be redirected to, such as the object store behind a release
    /// page. The digest decides what the bytes are; this decides where they may come from.
    #[serde(default)]
    pub redirect_hosts: Vec<String>,
    #[serde(rename = "tool", default)]
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("the catalog is larger than {MAX_CATALOG_BYTES} bytes")]
    TooLarge,
    #[error("the catalog does not parse: {0}")]
    Syntax(String),
    #[error("catalog version {0} is not one this build reads")]
    Version(u32),
    #[error("tool id {0:?} is not lowercase letters, digits and dashes")]
    Id(String),
    #[error("tool {0} appears more than once")]
    Duplicate(String),
    #[error("tool {0}: version {1:?} is not a plain version string")]
    VersionString(String, String),
    #[error("tool {0}: the artifact URL must be https without credentials, or http to this machine")]
    Url(String),
    #[error("tool {0}: sha256 must be 64 lowercase hex characters")]
    Digest(String),
    #[error("tool {0}: the declared size must be between 1 byte and 2 GiB")]
    Size(String),
    #[error("tool {0}: binary {1:?} must be a relative path that stays inside the tool")]
    Binary(String, String),
    #[error("tool {0}: a binary artifact names exactly one binary, as a plain file name")]
    BinaryArtifact(String),
    #[error("tool {0}: system administration tools come from the system package manager, not the toolpack")]
    SystemAdmin(String),
    #[error("tool {0}: capability {1:?} is not dotted lowercase words")]
    Capability(String, String),
    #[error("redirect host {0:?} is not a host name")]
    RedirectHost(String),
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self, CatalogError> {
        if text.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let catalog: Catalog = toml::from_str(text).map_err(|e| CatalogError::Syntax(e.message().to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn tool(&self, id: &str) -> Option<&ToolEntry> {
        self.tools.iter().find(|tool| tool.id == id)
    }

    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.version != CATALOG_VERSION {
            return Err(CatalogError::Version(self.version));
        }
        for host in &self.redirect_hosts {
            if host.is_empty()
                || host.len() > 253
                || !host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
            {
                return Err(CatalogError::RedirectHost(host.clone()));
            }
        }
        let mut seen = BTreeSet::new();
        for tool in &self.tools {
            validate_tool(tool)?;
            if !seen.insert(tool.id.as_str()) {
                return Err(CatalogError::Duplicate(tool.id.clone()));
            }
        }
        Ok(())
    }
}

/// Whether `text` is a name this crate will use as a directory: a tool id or a version.
pub(crate) fn plain_name(text: &str, extra: &[u8]) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text != "."
        && text != ".."
        && text.as_bytes()[0].is_ascii_alphanumeric()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || extra.contains(&b))
}

pub(crate) fn plain_version(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.as_bytes()[0].is_ascii_alphanumeric()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

fn validate_tool(tool: &ToolEntry) -> Result<(), CatalogError> {
    let id = &tool.id;
    if !plain_name(id, b"") {
        return Err(CatalogError::Id(id.clone()));
    }
    if !plain_version(&tool.version) {
        return Err(CatalogError::VersionString(id.clone(), tool.version.clone()));
    }
    if tool.risk == Risk::SystemAdmin {
        return Err(CatalogError::SystemAdmin(id.clone()));
    }
    for capability in &tool.capabilities {
        let well_formed = capability.split('.').all(|word| {
            !word.is_empty()
                && word.as_bytes()[0].is_ascii_lowercase()
                && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        });
        if !well_formed {
            return Err(CatalogError::Capability(id.clone(), capability.clone()));
        }
    }

    let artifact = &tool.artifact;
    if !url_allowed(&artifact.url) {
        return Err(CatalogError::Url(id.clone()));
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(CatalogError::Digest(id.clone()));
    }
    if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
        return Err(CatalogError::Size(id.clone()));
    }
    if artifact.binaries.is_empty() {
        return Err(CatalogError::Binary(id.clone(), String::new()));
    }
    for binary in &artifact.binaries {
        if !inside(binary) {
            return Err(CatalogError::Binary(id.clone(), binary.clone()));
        }
    }
    if artifact.format == Format::Binary
        && (artifact.binaries.len() != 1 || Path::new(&artifact.binaries[0]).components().count() != 1)
    {
        return Err(CatalogError::BinaryArtifact(id.clone()));
    }
    Ok(())
}

/// A relative path of plain components: no root, no `..`, no `.`, no backslash.
pub(crate) fn inside(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && !path.contains('\\')
        && !path.contains('\0')
        && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

pub(crate) fn url_allowed(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        // A mirror on this machine. Nothing leaves it, and the digest still decides.
        "http" => matches!(parsed.host_str(), Some("127.0.0.1") | Some("localhost") | Some("[::1]")),
        _ => false,
    }
}
