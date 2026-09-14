//! Putting a verified tool in place, and taking it back out.
//!
//! Layout, under a tool root such as `/opt/archie/tools`:
//!
//! ```text
//! <id>/<version>/...        one directory per installed version, never modified after
//! <id>/current              the version in use, one line
//! <id>/previous             the version before it, for rollback
//! .staging/                 unpacking happens here, then one rename into place
//! ```
//!
//! A version directory appears in one rename, so it is either absent or complete, and the
//! `current` pointer changes in one rename too. An install that fails at any point leaves
//! the tool exactly as it was. A running program keeps the version it started with.

use crate::catalog::{plain_version, Format};
use crate::gate::AuthorizedInstall;
use crate::unpack::{unpack_tar_gz, Limits, UnpackError};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("installing: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Unpack(#[from] UnpackError),
    #[error("the artifact does not contain {0:?}")]
    MissingBinary(String),
    #[error("{0} has no previous version to roll back to")]
    NothingToRollBack(String),
    #[error("{id}'s previous version {version} is no longer installed")]
    PreviousMissing { id: String, version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub previous: Option<String>,
    pub binaries: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ToolRoot {
    root: PathBuf,
}

impl ToolRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Where an authorised install's artifact is downloaded to before it is installed.
    pub fn download_path(&self, install: &AuthorizedInstall) -> PathBuf {
        let tool = install.tool();
        self.root.join(".downloads").join(format!(
            "{}-{}-{}",
            tool.id,
            tool.version,
            install.operation_id()
        ))
    }

    pub fn current(&self, id: &str) -> Option<String> {
        read_pointer(&self.root.join(id).join("current"))
    }

    pub fn previous(&self, id: &str) -> Option<String> {
        read_pointer(&self.root.join(id).join("previous"))
    }

    /// The path of one of a tool's binaries in its current version.
    pub fn binary(&self, id: &str, binary: &str) -> Option<PathBuf> {
        let path = self.root.join(id).join(self.current(id)?).join(binary);
        path.is_file().then_some(path)
    }

    /// Installed tool ids, in order.
    pub fn tools(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.'))
            .collect();
        ids.sort();
        ids
    }

    /// Installs a verified artifact for an authorised install and makes it current.
    pub fn install(
        &self,
        install: &AuthorizedInstall,
        artifact_file: &Path,
    ) -> Result<Installed, InstallError> {
        let tool = install.tool();
        let tool_dir = self.root.join(&tool.id);
        let version_dir = tool_dir.join(&tool.version);
        fs::create_dir_all(&tool_dir)?;

        if !version_dir.is_dir() {
            let staging = self.root.join(".staging").join(format!(
                "{}-{}-{}",
                tool.id,
                tool.version,
                install.operation_id()
            ));
            if staging.exists() {
                fs::remove_dir_all(&staging)?;
            }
            fs::create_dir_all(&staging)?;
            if let Err(error) = self.stage(install, artifact_file, &staging) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            if let Err(error) = fs::rename(&staging, &version_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error.into());
            }
        }

        for binary in &tool.artifact.binaries {
            if !version_dir.join(binary).is_file() {
                return Err(InstallError::MissingBinary(binary.clone()));
            }
        }
        let previous = self.current(&tool.id);
        if previous.as_deref() != Some(tool.version.as_str()) {
            if let Some(previous) = &previous {
                write_pointer(&tool_dir.join("previous"), previous)?;
            }
            write_pointer(&tool_dir.join("current"), &tool.version)?;
        }
        Ok(Installed {
            id: tool.id.clone(),
            version: tool.version.clone(),
            previous: previous.filter(|p| p != &tool.version),
            binaries: tool
                .artifact
                .binaries
                .iter()
                .map(|b| version_dir.join(b))
                .collect(),
        })
    }

    fn stage(
        &self,
        install: &AuthorizedInstall,
        artifact_file: &Path,
        staging: &Path,
    ) -> Result<(), InstallError> {
        let artifact = install.artifact();
        match artifact.format {
            Format::TarGz => {
                unpack_tar_gz(artifact_file, staging, Limits::default())?;
            }
            Format::Binary => {
                fs::copy(artifact_file, staging.join(&artifact.binaries[0]))?;
            }
        }
        for binary in &artifact.binaries {
            let path = staging.join(binary);
            if !path.is_file() {
                return Err(InstallError::MissingBinary(binary.clone()));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    }

    /// Makes the previous version current again, and the current one previous, so a second
    /// rollback undoes the first.
    pub fn rollback(&self, id: &str) -> Result<String, InstallError> {
        let tool_dir = self.root.join(id);
        let previous = self
            .previous(id)
            .ok_or_else(|| InstallError::NothingToRollBack(id.to_string()))?;
        if !tool_dir.join(&previous).is_dir() {
            return Err(InstallError::PreviousMissing {
                id: id.to_string(),
                version: previous,
            });
        }
        if let Some(current) = self.current(id) {
            write_pointer(&tool_dir.join("previous"), &current)?;
        }
        write_pointer(&tool_dir.join("current"), &previous)?;
        Ok(previous)
    }
}

fn read_pointer(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let version = text.trim();
    plain_version(version).then(|| version.to_string())
}

/// Writes `version` to a temporary file and renames it over `path`: a reader sees the old
/// pointer or the new one, never half of either.
fn write_pointer(path: &Path, version: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("next");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(version.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}
