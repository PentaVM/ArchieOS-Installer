//! Whether an install may happen.
//!
//! Installing puts executables on the node, so it is a write, and it needs a one-operation
//! lease on the tool's own directory like any other write. What a tool can do once
//! installed decides whether a person must also approve: a tool that only reads, or writes
//! inside the workspace it is given, is installed on the lease; a tool that can reach the
//! network or a device waits for a person.

use crate::catalog::{Artifact, Catalog, Risk, ToolEntry};
use archie_policy::{Action, Lease, LeaseRejection, Operation, Principal, Resource};
use archie_types::{TaskId, Timestamp};
use std::path::Path;

/// The resource an install writes: the tool's directory under the tool root.
pub fn tool_resource(root: &Path, id: &str) -> Resource {
    Resource::Filesystem {
        path: root.join(id).to_string_lossy().into_owned(),
    }
}

#[derive(Debug, Clone)]
pub struct InstallProposal {
    pub tool_id: String,
    pub operation_id: u64,
    pub task_id: Option<TaskId>,
}

/// An install the gate admitted. There is no public constructor.
#[derive(Debug, Clone)]
pub struct AuthorizedInstall {
    tool: ToolEntry,
    redirect_hosts: Vec<String>,
    operation_id: u64,
}

impl AuthorizedInstall {
    pub fn tool(&self) -> &ToolEntry {
        &self.tool
    }

    pub fn artifact(&self) -> &Artifact {
        &self.tool.artifact
    }

    pub fn redirect_hosts(&self) -> &[String] {
        &self.redirect_hosts
    }

    pub fn operation_id(&self) -> u64 {
        self.operation_id
    }
}

#[derive(Debug)]
pub enum InstallAdmission {
    Allowed(AuthorizedInstall),
    NeedsApproval {
        install: AuthorizedInstall,
        reason: &'static str,
    },
    Refused(InstallRefusal),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InstallRefusal {
    #[error("an install must name the operation its lease was granted for")]
    NoOperation,
    #[error("the catalog has no tool {0:?}")]
    UnknownTool(String),
    #[error("the lease does not authorise this install: {0:?}")]
    Lease(LeaseRejection),
    #[error("system administration tools are not installed by the toolpack")]
    SystemAdmin,
}

impl InstallRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoOperation => "no_operation",
            Self::UnknownTool(_) => "unknown_tool",
            Self::Lease(_) => "lease_refused",
            Self::SystemAdmin => "system_admin_tool",
        }
    }
}

pub struct InstallGate;

impl InstallGate {
    /// The only way to obtain an [`AuthorizedInstall`]. Checks the lease without consuming
    /// it: the install spends it once the tool is actually in place.
    pub fn authorize(
        catalog: &Catalog,
        root: &Path,
        lease: &Lease,
        principal: &Principal,
        user_present: bool,
        proposal: &InstallProposal,
        now: Timestamp,
    ) -> InstallAdmission {
        if proposal.operation_id == 0 {
            return InstallAdmission::Refused(InstallRefusal::NoOperation);
        }
        let Some(tool) = catalog.tool(&proposal.tool_id) else {
            return InstallAdmission::Refused(InstallRefusal::UnknownTool(proposal.tool_id.clone()));
        };
        let operation = Operation {
            principal: principal.clone(),
            resource: tool_resource(root, &tool.id),
            action: Action::Write,
            task_id: proposal.task_id,
            user_present,
        };
        if let Err(rejection) = lease.authorises_one_operation(&operation, proposal.operation_id, now) {
            return InstallAdmission::Refused(InstallRefusal::Lease(rejection));
        }
        let install = AuthorizedInstall {
            tool: tool.clone(),
            redirect_hosts: catalog.redirect_hosts.clone(),
            operation_id: proposal.operation_id,
        };
        match tool.risk {
            Risk::ReadOnly | Risk::WorkspaceWrite => InstallAdmission::Allowed(install),
            Risk::NetworkEgress => InstallAdmission::NeedsApproval {
                install,
                reason: "the tool can reach the network",
            },
            Risk::DeviceAccess => InstallAdmission::NeedsApproval {
                install,
                reason: "the tool can reach devices",
            },
            // A validated catalog cannot hold one; a hand-built catalog still does not pass.
            Risk::SystemAdmin => InstallAdmission::Refused(InstallRefusal::SystemAdmin),
        }
    }
}
