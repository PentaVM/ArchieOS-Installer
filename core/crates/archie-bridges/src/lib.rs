//! # archie-bridges
//!
//! One maintained plugin per open-source application, so that any planner — Archie's own
//! local model, Claude Code, Codex, an API model — can work in KiCad, Kdenlive, FreeCAD or
//! LibreOffice through tools instead of through pixels.
//!
//! A bridge is three things, kept together and tested together:
//!
//! * `bridges/<id>/bridge.toml` — what the bridge drives and how: the application's own
//!   free automation surfaces (a CLI, a file format, an IPC API), the programs it runs, how
//!   a node installs them, and which windows to look at when a task needs the GUI.
//! * the Rust here, a [`Bridge`] — typed tools confined to one folder the operator names.
//! * `bridges/<id>/SKILL.md` — the guide a planner reads before its first call: what each
//!   tool is for, the order that works, and the traps.
//!
//! `archie-mcp-server --bridge <id>=<folder>` serves bridges over MCP;
//! `archie-bridges call` runs one tool from a shell. `docs/BRIDGES.md` is the procedure
//! for writing a new one, and `docs/bridges/INDEX.md`, generated from the manifests, is
//! what a node reads to know what it can drive.
//!
//! # Tools, then eyes
//!
//! A tool is exact and cheap; a GUI seen through the Stage is general and slow. A planner
//! uses the bridge for what it covers and the Stage for the rest — a dialog, interactive
//! routing — and a bridge's renders let it check GUI work without reading pixels.
//!
//! # Applications whose only automation language is Python
//!
//! FreeCAD, Blender, GIMP, QGIS and Inkscape extensions are automated in their own
//! embedded Python. ArchieOS carries no Python, so their bridges hold each script as a
//! constant in Rust ([`script`]), write it out at run time and hand it its arguments as a
//! JSON file. No argument is ever formatted into code.

#![forbid(unsafe_code)]

pub mod delivery;
pub mod freecad;
pub mod index;
pub mod jobs;
pub mod kicad;
pub mod manifest;
pub mod preview;
pub mod run;
pub mod script;
pub mod sexpr;
pub mod video;
pub mod workspace;

pub use manifest::{Manifest, ManifestError};
pub use workspace::{Workspace, WorkspaceError};

use archie_types::PrivacyGrade;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Whether calling a tool can change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, and changes nothing a person would see. A report written to the bridge's
    /// scratch folder counts as reading.
    ReadOnly,
    /// Writes into the workspace: an export, a render, an edited design.
    Writes,
}

/// A tool a bridge offers. Its name starts with the bridge's id and an underscore.
#[derive(Debug, Clone)]
pub struct BridgeTool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    /// The most sensitive grade the result can carry.
    pub grade: PrivacyGrade,
    pub effect: Effect,
}

/// An image a tool returns beside its text, such as a render of a board.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// A tool's answer. `is_error` marks a failure the planner should read and can act on.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub is_error: bool,
    pub images: Vec<Image>,
}

impl Outcome {
    pub fn json(value: &Value) -> Self {
        Self {
            text: serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into()),
            is_error: false,
            images: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            text: message.into(),
            is_error: true,
            images: Vec::new(),
        }
    }

    pub fn with_image(mut self, image: Image) -> Self {
        self.images.push(image);
        self
    }
}

/// A plugin over one application.
///
/// Calls block: a bridge runs the application's programs and waits for them. A server
/// calls it off its protocol thread.
pub trait Bridge: Send + Sync {
    fn manifest(&self) -> &Manifest;

    /// The folder the bridge works in.
    fn workspace(&self) -> &Workspace;

    /// The guide a planner reads before calling the tools.
    fn guide(&self) -> &'static str;

    fn tools(&self) -> Vec<BridgeTool>;

    /// Runs a tool the caller has checked is one of [`Bridge::tools`].
    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome;

    /// Runs a long job one of its tools started, inside `archie-bridges run-job`.
    fn run_job(&self, spec: &jobs::Spec, _runner: &mut jobs::Runner) -> Result<(), String> {
        Err(format!("the {} bridge runs no {} jobs", self.manifest().id, spec.kind))
    }
}

/// Where each program a bridge runs is, by the name its manifest gives it, and where a
/// long job delivers. A program not named here is looked up beside this executable, then
/// on `PATH`.
#[derive(Debug, Clone, Default)]
pub struct Programs {
    paths: BTreeMap<String, PathBuf>,
    /// The desk a long job delivers its stages to: the key file and the tunnel's address.
    pub delivery: Option<(PathBuf, std::net::SocketAddr)>,
}

impl Programs {
    pub fn set(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        self.paths.insert(name.into(), path.into());
    }

    pub fn get(&self, name: &str) -> PathBuf {
        if let Some(path) = self.paths.get(name) {
            return path.clone();
        }
        // Archie's own programs are installed together.
        if name.starts_with("archie-") {
            let beside = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
                .map(|p| if cfg!(windows) { p.with_extension("exe") } else { p });
            if let Some(beside) = beside.filter(|p| p.is_file()) {
                return beside;
            }
        }
        PathBuf::from(name)
    }

    /// The same settings as `archie-bridges` flags, for a job process.
    pub fn as_args(&self) -> Vec<std::ffi::OsString> {
        let mut args = Vec::new();
        for (name, path) in &self.paths {
            args.push("--program".into());
            let mut pair = std::ffi::OsString::from(format!("{name}="));
            pair.push(path);
            args.push(pair);
        }
        if let Some((key, desk)) = &self.delivery {
            args.push("--deliver-key".into());
            args.push(key.clone().into_os_string());
            args.push("--deliver-to".into());
            args.push(desk.to_string().into());
        }
        args
    }

    /// The deliverer a job uses, when delivery is set up.
    pub fn deliverer(&self) -> Option<Result<delivery::Deliverer, delivery::DeliveryError>> {
        self.delivery.as_ref().map(|(key, desk)| {
            delivery::DeliveryKey::load(key).and_then(|key| delivery::Deliverer::new(*desk, key))
        })
    }
}

/// A bridge built into this crate: its id, manifest text and guide.
pub struct Builtin {
    pub id: &'static str,
    pub manifest: &'static str,
    pub guide: &'static str,
    open: fn(Manifest, Workspace, &Programs) -> Box<dyn Bridge>,
}

/// Every bridge this build carries, in the order the index lists them.
pub const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "kicad",
        manifest: kicad::MANIFEST,
        guide: kicad::GUIDE,
        open: |manifest, workspace, programs| {
            Box::new(kicad::Kicad::new(manifest, workspace, programs))
        },
    },
    Builtin {
        id: "freecad",
        manifest: freecad::MANIFEST,
        guide: freecad::GUIDE,
        open: |manifest, workspace, programs| {
            Box::new(freecad::Freecad::new(manifest, workspace, programs))
        },
    },
    Builtin {
        id: "video",
        manifest: video::MANIFEST,
        guide: video::GUIDE,
        open: |manifest, workspace, programs| {
            Box::new(video::Video::new(manifest, workspace, programs))
        },
    },
];

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("there is no bridge called {0:?}; the bridges are: {1}")]
    Unknown(String, String),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

/// The built-in bridge `id`, working in `folder`.
pub fn open(id: &str, folder: &Path, programs: &Programs) -> Result<Box<dyn Bridge>, OpenError> {
    let builtin = BUILTIN.iter().find(|b| b.id == id).ok_or_else(|| {
        OpenError::Unknown(
            id.to_string(),
            BUILTIN.iter().map(|b| b.id).collect::<Vec<_>>().join(", "),
        )
    })?;
    let manifest = Manifest::parse(builtin.manifest)?;
    let workspace = Workspace::open(folder)?;
    Ok((builtin.open)(manifest, workspace, programs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("archie-bridges-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_builtin_opens_and_keeps_its_own_names() {
        let dir = scratch("builtins");
        for builtin in BUILTIN {
            let bridge = open(builtin.id, &dir, &Programs::default()).unwrap();
            assert_eq!(bridge.manifest().id, builtin.id);
            let prefix = format!("{}_", builtin.id);
            let tools = bridge.tools();
            assert!(!tools.is_empty(), "{} offers no tools", builtin.id);
            for tool in &tools {
                assert!(
                    tool.name.starts_with(&prefix),
                    "{} is not named {prefix}…",
                    tool.name
                );
                assert!(
                    builtin.guide.contains(tool.name),
                    "{}'s guide never mentions {}",
                    builtin.id,
                    tool.name
                );
                assert_eq!(
                    tool.input_schema["type"], "object",
                    "{} takes an object",
                    tool.name
                );
            }
            let mut names: Vec<_> = tools.iter().map(|t| t.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), tools.len(), "{} repeats a tool", builtin.id);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unknown_bridge_names_the_ones_that_exist() {
        let dir = scratch("unknown");
        let error = open("photoshop", &dir, &Programs::default())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("kicad"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
