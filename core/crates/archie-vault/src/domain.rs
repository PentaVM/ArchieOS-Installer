//! Task context boundaries. A domain narrows retrieval; it grants no authority.
use crate::VaultRef;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContextDomain(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("context domain requires bounded personal/owner or work/organisation/project identifiers")]
pub struct DomainError;

impl ContextDomain {
    pub fn personal(owner: &str) -> Result<Self, DomainError> {
        segment(owner)?;
        Ok(Self(format!("personal/{owner}")))
    }

    pub fn work(organisation: &str, project: &str) -> Result<Self, DomainError> {
        segment(organisation)?;
        segment(project)?;
        Ok(Self(format!("work/{organisation}/{project}")))
    }

    pub fn is_work(&self) -> bool {
        self.0.starts_with("work/")
    }

    pub fn path(&self) -> &str {
        &self.0
    }

    pub fn contains(&self, reference: &VaultRef) -> bool {
        reference
            .path()
            .strip_prefix(&self.0)
            .is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
    }

    pub fn reference(&self, relative: &str) -> Result<VaultRef, DomainError> {
        // Validate the suffix separately so an empty or absolute suffix cannot escape.
        VaultRef::from_path(relative).map_err(|_| DomainError)?;
        VaultRef::from_path(&format!("{}/{relative}", self.0)).map_err(|_| DomainError)
    }
}

fn segment(value: &str) -> Result<(), DomainError> {
    if value.is_empty()
        || value.len() > 64
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(DomainError);
    }
    Ok(())
}

impl TryFrom<String> for ContextDomain {
    type Error = DomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parts: Vec<_> = value.split('/').collect();
        match parts.as_slice() {
            ["personal", owner] => Self::personal(owner),
            ["work", organisation, project] => Self::work(organisation, project),
            _ => Err(DomainError),
        }
    }
}

impl From<ContextDomain> for String {
    fn from(value: ContextDomain) -> Self {
        value.0
    }
}

impl fmt::Display for ContextDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
