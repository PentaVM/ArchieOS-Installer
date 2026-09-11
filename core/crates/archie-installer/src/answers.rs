//! What the person told the wizard, and whether it can be used.
//!
//! Validation happens here rather than at the prompt because the prompt is the one part
//! of an installer that cannot be tested, and "the name you typed is rejected by
//! `useradd` three minutes into the install" is a bad way to find out.

use archie_types::NodeProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidAnswer {
    EmptyHostname,
    HostnameTooLong { length: usize },
    HostnameCharacter { found: char },
    HostnameEdgeHyphen,
    EmptyUsername,
    UsernameTooLong { length: usize },
    UsernameCharacter { found: char },
    UsernameStart { found: char },
    UsernameReserved { name: String },
}

impl std::fmt::Display for InvalidAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyHostname => f.write_str("a hostname cannot be empty"),
            Self::HostnameTooLong { length } => write!(
                f,
                "a hostname can be at most 63 characters, and this one is {length}"
            ),
            Self::HostnameCharacter { found } => write!(
                f,
                "a hostname may only contain letters, digits and hyphens, and this one \
                 contains {found:?}"
            ),
            Self::HostnameEdgeHyphen => f.write_str("a hostname cannot begin or end with a hyphen"),
            Self::EmptyUsername => f.write_str("a username cannot be empty"),
            Self::UsernameTooLong { length } => write!(
                f,
                "a username can be at most 32 characters, and this one is {length}"
            ),
            Self::UsernameCharacter { found } => write!(
                f,
                "a username may only contain lowercase letters, digits, underscores and \
                 hyphens, and this one contains {found:?}"
            ),
            Self::UsernameStart { found } => write!(
                f,
                "a username must begin with a lowercase letter or an underscore, and this \
                 one begins with {found:?}"
            ),
            Self::UsernameReserved { name } => write!(
                f,
                "{name:?} is an account the system already owns; pick another name"
            ),
        }
    }
}

/// Accounts that already exist on an ArchieOS install, or that the base system creates.
///
/// Letting somebody claim `archie` would give a human login the identity the daemon runs
/// under, which is not a naming collision but a privilege one.
const RESERVED_USERNAMES: &[&str] = &[
    "root",
    "bin",
    "daemon",
    "mail",
    "ftp",
    "http",
    "nobody",
    "systemd-network",
    "archie",
    "archie-cortex",
    "archie-display",
];

/// A hostname, checked against what the kernel and `systemd-hostnamed` will accept.
pub fn validate_hostname(name: &str) -> Result<(), InvalidAnswer> {
    if name.is_empty() {
        return Err(InvalidAnswer::EmptyHostname);
    }
    if name.len() > 63 {
        return Err(InvalidAnswer::HostnameTooLong { length: name.len() });
    }
    if let Some(found) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-'))
    {
        return Err(InvalidAnswer::HostnameCharacter { found });
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(InvalidAnswer::HostnameEdgeHyphen);
    }
    Ok(())
}

/// A login name, checked against what `useradd` will accept.
pub fn validate_username(name: &str) -> Result<(), InvalidAnswer> {
    if name.is_empty() {
        return Err(InvalidAnswer::EmptyUsername);
    }
    if name.len() > 32 {
        return Err(InvalidAnswer::UsernameTooLong { length: name.len() });
    }
    let first = name.chars().next().expect("checked non-empty");
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err(InvalidAnswer::UsernameStart { found: first });
    }
    if let Some(found) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
    {
        return Err(InvalidAnswer::UsernameCharacter { found });
    }
    if RESERVED_USERNAMES.contains(&name) {
        return Err(InvalidAnswer::UsernameReserved { name: name.into() });
    }
    Ok(())
}

/// Everything the wizard asked, once it is known to be usable.
///
/// Constructed only through [`Answers::new`], so an `Answers` that exists has already
/// been checked and nothing downstream has to re-ask the question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers {
    hostname: String,
    username: String,
    profile: NodeProfile,
    enable_ai_module: bool,
}

impl Answers {
    pub fn new(
        hostname: &str,
        username: &str,
        profile: NodeProfile,
        enable_ai_module: bool,
    ) -> Result<Self, InvalidAnswer> {
        validate_hostname(hostname)?;
        validate_username(username)?;
        Ok(Self {
            hostname: hostname.to_string(),
            username: username.to_string(),
            profile,
            enable_ai_module,
        })
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn profile(&self) -> NodeProfile {
        self.profile
    }
    pub fn enable_ai_module(&self) -> bool {
        self.enable_ai_module
    }
}
