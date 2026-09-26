//! The one folder a bridge works in.
//!
//! The operator names it when the bridge is served. Everything a planner names is a path
//! relative to it, checked twice: by its components, so no `..` or root takes part, and by
//! its real path once links are followed, so a link inside cannot lead out. Answers name
//! files relative to it too, so a planner learns nothing about the rest of the disk.
//!
//! New files never replace old ones, with one exception a bridge must ask for by name:
//! [`Workspace::replace`], which keeps the previous version under `.archie/history/`.

use std::path::{Component, Path, PathBuf};

/// Where a bridge keeps what is its own: reports, job files, history.
pub const SCRATCH: &str = ".archie";

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0} is not a folder that exists")]
    NotAFolder(String),
    #[error("{0:?} is not a path inside the workspace")]
    Outside(String),
    #[error("{0} does not exist in the workspace")]
    Missing(String),
    #[error("{0} already exists; this tool never replaces a file")]
    Exists(String),
    #[error("{what}: {detail}")]
    Io { what: String, detail: String },
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> WorkspaceError {
    let what = what.into();
    move |error| WorkspaceError::Io {
        what,
        detail: error.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn open(folder: &Path) -> Result<Self, WorkspaceError> {
        let root = folder
            .canonicalize()
            .ok()
            .filter(|root| root.is_dir())
            .ok_or_else(|| WorkspaceError::NotAFolder(folder.display().to_string()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn plain(name: &str) -> Result<&Path, WorkspaceError> {
        let relative = Path::new(name);
        // Components alone would let `a/./b` through as `a/b`; a name is written plainly.
        let normal = !name.is_empty()
            && !name.contains('\0')
            && name
                .split(['/', '\\'])
                .all(|part| !part.is_empty() && part != "." && part != "..")
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
        if normal {
            Ok(relative)
        } else {
            Err(WorkspaceError::Outside(name.to_string()))
        }
    }

    /// `name`, a file that exists inside the workspace.
    pub fn file(&self, name: &str) -> Result<PathBuf, WorkspaceError> {
        let relative = Self::plain(name)?;
        let real = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|_| WorkspaceError::Missing(name.to_string()))?;
        if !real.starts_with(&self.root) {
            return Err(WorkspaceError::Outside(name.to_string()));
        }
        if !real.is_file() {
            return Err(WorkspaceError::Missing(name.to_string()));
        }
        Ok(real)
    }

    /// `name`, a file that exists inside the workspace and ends in `.<extension>`.
    pub fn file_with(&self, name: &str, extension: &str) -> Result<PathBuf, WorkspaceError> {
        if Path::new(name).extension().and_then(|e| e.to_str()) != Some(extension) {
            return Err(WorkspaceError::Missing(format!(
                "{name} (a .{extension} file)"
            )));
        }
        self.file(name)
    }

    /// A folder that does not exist yet, `<parent>/<stem>-<n>`, created inside the
    /// workspace. For a tool that writes several files at once, such as gerbers.
    pub fn fresh_folder(&self, parent: &str, stem: &str) -> Result<PathBuf, WorkspaceError> {
        let parent_path = self.root.join(Self::plain(parent)?);
        std::fs::create_dir_all(&parent_path).map_err(io(format!("creating {parent}")))?;
        let parent_real = parent_path
            .canonicalize()
            .map_err(io(format!("reading {parent}")))?;
        if !parent_real.starts_with(&self.root) {
            return Err(WorkspaceError::Outside(parent.to_string()));
        }
        for n in 1..10_000 {
            let candidate = parent_real.join(format!("{stem}-{n}"));
            match std::fs::create_dir(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io(format!("creating {parent}/{stem}-{n}"))(e)),
            }
        }
        Err(WorkspaceError::Exists(format!("{parent}/{stem}-*")))
    }

    /// A new file `<parent>/<stem>-<n>.<extension>` that does not exist yet. The file is
    /// not created; the caller's program writes it.
    pub fn fresh_file(
        &self,
        parent: &str,
        stem: &str,
        extension: &str,
    ) -> Result<PathBuf, WorkspaceError> {
        let parent_path = self.root.join(Self::plain(parent)?);
        std::fs::create_dir_all(&parent_path).map_err(io(format!("creating {parent}")))?;
        let parent_real = parent_path
            .canonicalize()
            .map_err(io(format!("reading {parent}")))?;
        if !parent_real.starts_with(&self.root) {
            return Err(WorkspaceError::Outside(parent.to_string()));
        }
        (1..10_000)
            .map(|n| parent_real.join(format!("{stem}-{n}.{extension}")))
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| WorkspaceError::Exists(format!("{parent}/{stem}-*.{extension}")))
    }

    /// The bridge's own scratch folder, `.archie/<what>`, created if needed.
    pub fn scratch(&self, what: &str) -> Result<PathBuf, WorkspaceError> {
        let dir = self.root.join(SCRATCH).join(Self::plain(what)?);
        std::fs::create_dir_all(&dir).map_err(io("creating the scratch folder"))?;
        Ok(dir)
    }

    /// Replaces an existing file inside the workspace with `bytes`, atomically, after
    /// copying the current version to `.archie/history/<name>.<n>`.
    pub fn replace(&self, name: &str, bytes: &[u8]) -> Result<String, WorkspaceError> {
        let path = self.file(name)?;
        let history = self.scratch("history")?;
        let flat = self.relative(&path).replace(['/', '\\'], "__");
        let kept = (1..100_000)
            .map(|n| history.join(format!("{flat}.{n}")))
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| WorkspaceError::Exists(format!("{SCRATCH}/history/{flat}.*")))?;
        std::fs::copy(&path, &kept).map_err(io(format!("keeping the old {name}")))?;
        let staged = path.with_extension(format!(
            "{}.archie-new",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        std::fs::write(&staged, bytes).map_err(io(format!("writing the new {name}")))?;
        std::fs::rename(&staged, &path).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            io(format!("putting the new {name} in place"))(e)
        })?;
        Ok(self.relative(&kept))
    }

    /// `path` as the planner names it: relative to the workspace, with `/`.
    pub fn relative(&self, path: &Path) -> String {
        let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        match real
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.root))
        {
            Ok(rest) => rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => "(outside the workspace)".into(),
        }
    }

    /// Files inside the workspace whose extension is one of `extensions`, at most `depth`
    /// folders down and at most `limit` of them, sorted. Links and hidden folders are not
    /// followed. The second value says whether the list was cut at `limit`.
    pub fn find(&self, extensions: &[&str], depth: usize, limit: usize) -> (Vec<String>, bool) {
        let mut found = Vec::new();
        let mut truncated = false;
        let mut pending = vec![(self.root.clone(), 0usize)];
        while let Some((dir, level)) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if kind.is_dir() && !hidden && level < depth {
                    pending.push((path, level + 1));
                } else if kind.is_file() {
                    let wanted = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| extensions.contains(&e.to_ascii_lowercase().as_str()));
                    if wanted {
                        if found.len() == limit {
                            truncated = true;
                            continue;
                        }
                        found.push(self.relative(&path));
                    }
                }
            }
        }
        found.sort();
        (found, truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> (Workspace, PathBuf) {
        let dir = std::env::temp_dir().join(format!("archie-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/board.kicad_pcb"), b"(kicad_pcb)").unwrap();
        std::fs::write(dir.join("top.kicad_sch"), b"(kicad_sch)").unwrap();
        (Workspace::open(&dir).unwrap(), dir)
    }

    #[test]
    fn names_that_leave_the_folder_are_refused() {
        let (ws, dir) = folder("outside");
        for name in [
            "../x",
            "/etc/passwd",
            "a/../../x",
            "",
            "a/./b/board.kicad_pcb",
        ] {
            assert!(ws.file(name).is_err(), "{name} was accepted");
        }
        assert!(ws.file("a/b/board.kicad_pcb").is_ok());
        assert!(matches!(
            ws.file("nope.kicad_sch"),
            Err(WorkspaceError::Missing(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_cannot_lead_out() {
        let (ws, dir) = folder("link");
        std::os::unix::fs::symlink("/etc/hostname", dir.join("host")).unwrap();
        assert!(matches!(ws.file("host"), Err(WorkspaceError::Outside(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fresh_names_never_collide_and_answers_are_relative() {
        let (ws, dir) = folder("fresh");
        let one = ws.fresh_folder("exports", "gerbers").unwrap();
        let two = ws.fresh_folder("exports", "gerbers").unwrap();
        assert_ne!(one, two);
        assert_eq!(ws.relative(&one), "exports/gerbers-1");
        let file = ws.fresh_file("renders", "board", "png").unwrap();
        assert_eq!(ws.relative(&file), "renders/board-1.png");
        let (found, truncated) = ws.find(&["kicad_sch", "kicad_pcb"], 4, 10);
        assert_eq!(found, vec!["a/b/board.kicad_pcb", "top.kicad_sch"]);
        assert!(!truncated);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_keeps_the_old_version() {
        let (ws, dir) = folder("replace");
        let kept = ws.replace("top.kicad_sch", b"(kicad_sch new)").unwrap();
        assert_eq!(kept, ".archie/history/top.kicad_sch.1");
        assert_eq!(std::fs::read(dir.join(&kept)).unwrap(), b"(kicad_sch)");
        assert_eq!(
            std::fs::read(dir.join("top.kicad_sch")).unwrap(),
            b"(kicad_sch new)"
        );
        let hidden = ws.find(&["1"], 4, 10).0;
        assert!(hidden.is_empty(), "history is not listed: {hidden:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
