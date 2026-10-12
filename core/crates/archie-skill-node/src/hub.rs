//! The repository the nodes share: the board, the catalog IRIS reads, and the packages.
//!
//! It is a git repository, and the node works on its own clone of it. Every change is one
//! commit on top of the revision the node read, pushed with the git identity and
//! credentials of the person whose machine it is: the history is the credit, and this
//! program never sees a token. A push that is not a fast-forward is refused by the remote,
//! which is what keeps two nodes from overwriting each other's changes to the board.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A package larger than this does not belong in a git history.
pub const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubError {
    /// Someone pushed since this node read the board: read it again and retry.
    Moved,
    NotFound,
    TooLarge(usize),
    Failed(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Moved => {
                f.write_str("the board changed since it was read: read it again and retry")
            }
            Self::NotFound => f.write_str("not found in the repository"),
            Self::TooLarge(bytes) => write!(
                f,
                "a file of {bytes} bytes is too large for the shared repository"
            ),
            Self::Failed(message) => write!(f, "shared repository: {message}"),
        }
    }
}

/// What a node needs from the shared repository.
pub trait Hub {
    /// The head revision of the shared branch.
    fn head(&self) -> Result<String, HubError>;
    fn read(&self, path: &str) -> Result<Vec<u8>, HubError>;
    /// One commit of `files` on top of `parent`, pushed as this node's person.
    fn commit(
        &self,
        summary: &str,
        files: &[(String, Vec<u8>)],
        parent: &str,
    ) -> Result<String, HubError>;
}

/// The node's clone of the shared repository.
pub struct GitRepo {
    pub dir: PathBuf,
    pub branch: String,
}

impl GitRepo {
    pub fn new(dir: &Path, branch: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            branch: branch.into(),
        }
    }

    fn git(&self, args: &[&str]) -> Result<(bool, Vec<u8>, String), HubError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|e| HubError::Failed(format!("git: {e}")))?;
        Ok((
            output.status.success(),
            output.stdout,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, HubError> {
        match self.git(args)? {
            (true, stdout, _) => Ok(stdout),
            (false, _, stderr) => Err(HubError::Failed(format!(
                "git {}: {}",
                args[0],
                short(&stderr)
            ))),
        }
    }

    fn remote_ref(&self) -> String {
        format!("origin/{}", self.branch)
    }
}

fn short(text: &str) -> String {
    text.chars().take(300).collect()
}

/// A path the node itself made up, kept inside the clone whatever it is.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && part != ".git")
}

impl Hub for GitRepo {
    fn head(&self) -> Result<String, HubError> {
        self.run(&["fetch", "--quiet", "origin", &self.branch])?;
        let sha = self.run(&["rev-parse", &self.remote_ref()])?;
        Ok(String::from_utf8_lossy(&sha).trim().to_string())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, HubError> {
        if !safe_path(path) {
            return Err(HubError::NotFound);
        }
        match self.git(&["show", &format!("{}:{path}", self.remote_ref())])? {
            (true, stdout, _) => Ok(stdout),
            (false, _, stderr)
                if stderr.contains("does not exist")
                    || stderr.contains("exists on disk, but not in") =>
            {
                Err(HubError::NotFound)
            }
            (false, _, stderr) => Err(HubError::Failed(format!("git show: {}", short(&stderr)))),
        }
    }

    fn commit(
        &self,
        summary: &str,
        files: &[(String, Vec<u8>)],
        parent: &str,
    ) -> Result<String, HubError> {
        for (path, bytes) in files {
            if !safe_path(path) {
                return Err(HubError::Failed(format!("refused path {path}")));
            }
            if bytes.len() > MAX_PACKAGE_BYTES {
                return Err(HubError::TooLarge(bytes.len()));
            }
        }
        // The clone is this node's alone: its work tree is put exactly at the revision read.
        self.run(&["checkout", "--quiet", "--force", "--detach", parent])?;
        for (path, bytes) in files {
            let target = self.dir.join(path);
            if let Some(folder) = target.parent() {
                std::fs::create_dir_all(folder).map_err(|e| HubError::Failed(e.to_string()))?;
            }
            std::fs::write(&target, bytes).map_err(|e| HubError::Failed(e.to_string()))?;
            self.run(&["add", "--", path])?;
        }
        self.run(&["commit", "--quiet", "-m", summary])?;
        let sha = String::from_utf8_lossy(&self.run(&["rev-parse", "HEAD"])?)
            .trim()
            .to_string();
        match self.git(&[
            "push",
            "--quiet",
            "origin",
            &format!("HEAD:{}", self.branch),
        ])? {
            (true, _, _) => Ok(sha),
            (false, _, stderr)
                if stderr.contains("rejected")
                    || stderr.contains("fetch first")
                    || stderr.contains("non-fast-forward") =>
            {
                Err(HubError::Moved)
            }
            (false, _, stderr) => Err(HubError::Failed(format!("git push: {}", short(&stderr)))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_the_clone_are_written() {
        for good in ["board.json", "skills/csgo/1-nelson-abc.skill"] {
            assert!(safe_path(good), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../board.json",
            "skills/../../x",
            ".git/config",
            "a//b",
            "C:/x",
            "a\\b",
        ] {
            assert!(!safe_path(bad), "{bad}");
        }
    }
}
