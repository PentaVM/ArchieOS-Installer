//! What an MCP server is allowed to be, written down before it is reached.
//!
//! Same bargain the module and provider registries make: a server that cannot be
//! described in a manifest is a server Archie will not call. An operator can read the
//! file and see every external thing their node can be made to do, which is not true of
//! a tool list fetched at runtime from the server itself.
//!
//! # The server does not get to say what it is allowed to do
//!
//! `tools/list` is the server describing itself. Useful, and not authority: a server
//! that adds `filesystem.delete` to its own tool list next Tuesday has not thereby been
//! granted it. The allowlist here is the operator's list, and the intersection is what
//! Archie will call.

use archie_types::{Capability, CapabilitySet, Locality, PrivacyGrade};
use archie_vault::VaultRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How the server is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum McpTransport {
    /// A child process speaking JSON-RPC over stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// An HTTP endpoint speaking the streamable HTTP binding.
    StreamableHttp { url: String },
}

impl McpTransport {
    /// Where the work physically happens, which is what the privacy rules turn on.
    ///
    /// A stdio server is a local child process. An HTTP one is only local if it is
    /// pointed at loopback — and a manifest claiming otherwise is caught by
    /// [`McpServerManifest::validate`] rather than trusted.
    pub fn implied_locality(&self) -> Locality {
        match self {
            Self::Stdio { .. } => Locality::Local,
            Self::StreamableHttp { url } => {
                if is_loopback_url(url) {
                    Locality::Local
                } else {
                    Locality::Cloud
                }
            }
        }
    }
}

fn is_loopback_url(url: &str) -> bool {
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest,
        None => url,
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or_else(|| rest.split(['/', '?', '#']).next().unwrap_or(""));
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Which of a server's tools Archie may call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum ToolAllowlist {
    /// Exactly these names, whatever else the server advertises.
    Named { tools: BTreeSet<String> },
    /// Nothing. A server can be registered and reachable while granted nothing, which
    /// is the right state for one that has just been added and not yet reviewed.
    None,
}

impl ToolAllowlist {
    pub fn named<I, S>(tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Named {
            tools: tools.into_iter().map(Into::into).collect(),
        }
    }

    pub fn permits(&self, tool: &str) -> bool {
        match self {
            Self::Named { tools } => tools.contains(tool),
            Self::None => false,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Named { tools } => tools.len(),
            Self::None => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One MCP server, as the operator declared it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerManifest {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransport,

    /// The NAME of the credential this server needs, never the credential. Resolved at
    /// the execution boundary through the vault's broker, so a manifest can be
    /// committed and reviewed without carrying anything sensitive.
    #[serde(default)]
    pub secret_handle: Option<VaultRef>,

    /// What Archie may use this server for. Checked against the caller's grants.
    #[serde(default)]
    pub capabilities: CapabilitySet,

    pub tools: ToolAllowlist,

    /// The most sensitive material that may appear in arguments to this server.
    ///
    /// Defaults to [`PrivacyGrade::Public`]: a newly added server is trusted with
    /// nothing until someone raises this deliberately.
    #[serde(default = "public_grade")]
    pub max_privacy_grade: PrivacyGrade,

    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u32,

    /// A registered server that is switched off. Distinct from absent: the operator
    /// disabled this one, and the reason belongs in review, not in a deleted file.
    #[serde(default)]
    pub enabled: bool,
}

fn public_grade() -> PrivacyGrade {
    PrivacyGrade::Public
}

fn default_timeout_ms() -> u32 {
    30_000
}

fn default_max_response_bytes() -> u32 {
    1_048_576
}

impl McpServerManifest {
    /// Where calls to this server physically go.
    pub fn locality(&self) -> Locality {
        self.transport.implied_locality()
    }

    /// Whether this server may receive arguments graded `grade`.
    ///
    /// Both gates, as everywhere else: the operator's ceiling for this server, and the
    /// locality the grade permits. A server declared `sensitive` that turns out to be a
    /// remote URL is still refused sensitive arguments.
    pub fn permits_privacy(&self, grade: PrivacyGrade) -> bool {
        grade <= self.max_privacy_grade && grade.permits(self.locality())
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.is_empty() || self.id.len() > 64 {
            return Err(ManifestError::Id);
        }
        if !self
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        {
            return Err(ManifestError::Id);
        }
        match &self.transport {
            McpTransport::Stdio { command, .. } if command.is_empty() => {
                return Err(ManifestError::Transport("command is empty"));
            }
            // Plaintext to a remote host would put every argument on the wire.
            McpTransport::StreamableHttp { url }
                if !url.starts_with("https://") && !is_loopback_url(url) =>
            {
                return Err(ManifestError::Transport(
                    "a non-loopback server must be reached over https",
                ));
            }
            _ => {}
        }
        if self.max_privacy_grade == PrivacyGrade::Secret {
            // Nothing may be graded secret in a tool argument. Secrets travel as a
            // credential handle resolved at the boundary, which is what secret_handle is.
            return Err(ManifestError::SecretArguments);
        }
        if !self.max_privacy_grade.permits(self.locality()) {
            return Err(ManifestError::GradeExceedsLocality {
                grade: self.max_privacy_grade,
                locality: self.locality(),
            });
        }
        if self.timeout_ms == 0 || self.timeout_ms > 600_000 {
            return Err(ManifestError::Timeout);
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > 64 * 1_048_576 {
            return Err(ManifestError::ResponseBudget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("server id must be 1..=64 characters of [a-z0-9-_.]")]
    Id,
    #[error("invalid transport: {0}")]
    Transport(&'static str),
    #[error("a tool argument may never be graded secret; use secret_handle")]
    SecretArguments,
    #[error("declared grade {grade} cannot be honoured at {locality:?}")]
    GradeExceedsLocality {
        grade: PrivacyGrade,
        locality: Locality,
    },
    #[error("timeout must be 1..=600000 ms")]
    Timeout,
    #[error("response budget must be 1..=64 MiB")]
    ResponseBudget,
    #[error("duplicate server id {0}")]
    Duplicate(String),
}

/// Every MCP server this node knows about.
#[derive(Debug, Default, Clone)]
pub struct McpRegistry {
    servers: std::collections::BTreeMap<String, McpServerManifest>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: McpServerManifest) -> Result<(), ManifestError> {
        manifest.validate()?;
        if self.servers.contains_key(&manifest.id) {
            return Err(ManifestError::Duplicate(manifest.id));
        }
        self.servers.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&McpServerManifest> {
        self.servers.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &McpServerManifest> {
        self.servers.values()
    }

    pub fn len(&self) -> usize {
        self.servers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Servers that are enabled and advertise `capability`.
    pub fn providers_of(&self, capability: &Capability) -> Vec<&McpServerManifest> {
        self.servers
            .values()
            .filter(|s| s.enabled && s.capabilities.provides(capability))
            .collect()
    }

    /// Parse a TOML document holding one manifest.
    pub fn parse_manifest(toml_text: &str) -> Result<McpServerManifest, String> {
        let manifest: McpServerManifest = toml::from_str(toml_text).map_err(|e| e.to_string())?;
        manifest.validate().map_err(|e| e.to_string())?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(url: &str) -> McpServerManifest {
        McpServerManifest {
            id: "github".into(),
            display_name: "GitHub".into(),
            transport: McpTransport::StreamableHttp { url: url.into() },
            secret_handle: None,
            capabilities: CapabilitySet::new(),
            tools: ToolAllowlist::named(["create_issue"]),
            max_privacy_grade: PrivacyGrade::Public,
            timeout_ms: 30_000,
            max_response_bytes: 1_048_576,
            enabled: true,
        }
    }

    #[test]
    fn loopback_is_local_and_anything_else_is_not() {
        assert_eq!(
            McpTransport::StreamableHttp {
                url: "http://127.0.0.1:9000/mcp".into()
            }
            .implied_locality(),
            Locality::Local
        );
        assert_eq!(
            McpTransport::StreamableHttp {
                url: "https://api.example.com/mcp".into()
            }
            .implied_locality(),
            Locality::Cloud
        );
        assert_eq!(
            McpTransport::Stdio {
                command: "mcp-server".into(),
                args: vec![]
            }
            .implied_locality(),
            Locality::Local
        );
    }

    #[test]
    fn a_hostname_that_merely_contains_localhost_is_not_loopback() {
        // `localhost.evil.com` resolves wherever its owner wants it to.
        assert_eq!(
            McpTransport::StreamableHttp {
                url: "https://localhost.evil.com/mcp".into()
            }
            .implied_locality(),
            Locality::Cloud
        );
    }

    #[test]
    fn a_remote_server_must_be_reached_over_https() {
        let mut manifest = http("http://api.example.com/mcp");
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::Transport(_))
        ));
        manifest.transport = McpTransport::StreamableHttp {
            url: "https://api.example.com/mcp".into(),
        };
        assert!(manifest.validate().is_ok());
        // Loopback plaintext is fine: it never reaches a wire.
        assert!(http("http://127.0.0.1:9000/mcp").validate().is_ok());
    }

    #[test]
    fn a_remote_server_cannot_be_cleared_for_restricted_arguments() {
        let mut manifest = http("https://api.example.com/mcp");
        manifest.max_privacy_grade = PrivacyGrade::Restricted;
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::GradeExceedsLocality { .. })
        ));
    }

    #[test]
    fn no_server_may_be_cleared_for_secret_arguments() {
        let mut manifest = http("http://127.0.0.1:9000/mcp");
        manifest.max_privacy_grade = PrivacyGrade::Secret;
        assert_eq!(
            manifest.validate().unwrap_err(),
            ManifestError::SecretArguments
        );
    }

    #[test]
    fn a_manifest_round_trips_through_toml() {
        let text = r#"
id = "github"
display_name = "GitHub"
max_privacy_grade = "internal"
enabled = true
secret_handle = "vault:github/main"

[transport]
kind = "streamable-http"
url = "https://api.githubcopilot.com/mcp/"

[tools]
mode = "named"
tools = ["create_issue", "list_issues"]
"#;
        let manifest = McpRegistry::parse_manifest(text).unwrap();
        assert_eq!(manifest.id, "github");
        assert!(manifest.tools.permits("create_issue"));
        assert!(!manifest.tools.permits("delete_repository"));
        assert_eq!(
            manifest.secret_handle.as_ref().unwrap().to_string(),
            "vault:github/main"
        );
        // Defaults arrive without being written down.
        assert_eq!(manifest.timeout_ms, 30_000);
    }

    #[test]
    fn an_unreviewed_server_is_granted_nothing_by_default() {
        let text = r#"
id = "new-server"
display_name = "Newly added"

[transport]
kind = "stdio"
command = "some-mcp-server"

[tools]
mode = "none"
"#;
        let manifest = McpRegistry::parse_manifest(text).unwrap();
        assert!(
            !manifest.enabled,
            "a server must be switched on deliberately"
        );
        assert_eq!(manifest.max_privacy_grade, PrivacyGrade::Public);
        assert!(manifest.tools.is_empty());
    }
}
