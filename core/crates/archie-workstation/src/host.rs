use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    FileImport,
    FileExport,
    NotificationShow,
    MicrophoneStream,
    ClipboardReadAuthorized,
    AppLaunchAuthorized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOperation {
    pub capability: HostCapability,
    pub workspace_id: String,
    pub relative_path: Option<PathBuf>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HostBoundaryError {
    #[error("workspace identifier is invalid")]
    InvalidWorkspace,
    #[error("host bridge paths must be relative and traversal-free")]
    PathEscape,
}

impl HostOperation {
    pub fn validate(&self) -> Result<(), HostBoundaryError> {
        if self.workspace_id.is_empty()
            || !self
                .workspace_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(HostBoundaryError::InvalidWorkspace);
        }
        if let Some(path) = &self.relative_path {
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(HostBoundaryError::PathEscape);
            }
        }
        Ok(())
    }

    pub fn workspace_path(&self, root: &Path) -> Result<Option<PathBuf>, HostBoundaryError> {
        self.validate()?;
        Ok(self
            .relative_path
            .as_ref()
            .map(|path| root.join(&self.workspace_id).join(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_rejects_host_and_parent_paths() {
        for path in [PathBuf::from("../secret"), PathBuf::from("C:\\secret")] {
            let operation = HostOperation {
                capability: HostCapability::FileImport,
                workspace_id: "demo".into(),
                relative_path: Some(path),
            };
            assert_eq!(operation.validate(), Err(HostBoundaryError::PathEscape));
        }
    }
}
