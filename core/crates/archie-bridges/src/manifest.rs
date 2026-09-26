//! `bridge.toml`: what a bridge drives, through which of the application's own surfaces,
//! with which programs, and how a node gets them.
//!
//! The file is written for two readers. A person reviewing a bridge sees every program it
//! can run. A planner reading `docs/bridges/INDEX.md` learns what the bridge is for and
//! when to use the Stage instead.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Lowercase letters, digits and hyphens; every tool is named `<id>_…`.
    pub id: String,
    pub title: String,
    /// One sentence: what a planner can do with it.
    pub summary: String,
    pub app: App,
    /// The application's automation surfaces this bridge uses, or plans to.
    pub surfaces: Vec<Surface>,
    /// The programs the bridge runs, by the name `--program <name>=<path>` overrides.
    pub programs: Vec<String>,
    /// How a node installs the application, per platform.
    pub install: Install,
    /// What the operator's folder holds.
    pub workspace: String,
    pub vision: Vision,
    /// The command that runs this bridge's end-to-end tests against the real application.
    pub e2e: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    pub homepage: String,
    pub license: String,
    /// The versions the bridge was tested against.
    pub versions: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    /// cli, file-format, ipc, dbus, http or script.
    pub kind: String,
    pub name: String,
    /// The official documentation of the surface.
    pub doc: String,
    #[serde(default = "used")]
    pub status: String,
}

fn used() -> String {
    "used".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
    pub arch: String,
    #[serde(default)]
    pub ubuntu: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vision {
    /// Wayland app ids of the application's windows on the Stage.
    pub app_ids: Vec<String>,
    /// When a planner should operate the GUI instead of calling tools.
    pub when: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("bridge.toml does not parse: {0}")]
    Parse(String),
    #[error("bridge.toml: {0}")]
    Invalid(String),
}

const SURFACE_KINDS: [&str; 6] = ["cli", "file-format", "ipc", "dbus", "http", "script"];
const STATUSES: [&str; 2] = ["used", "planned"];

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let invalid = |message: String| Err(ManifestError::Invalid(message));
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 32
            && self.id.starts_with(|c: char| c.is_ascii_lowercase())
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            return invalid(format!(
                "id {:?} is not lowercase letters, digits and hyphens",
                self.id
            ));
        }
        if self.summary.trim().is_empty() || self.summary.len() > 300 {
            return invalid("summary is one sentence of at most 300 bytes".into());
        }
        if self.surfaces.is_empty() {
            return invalid("a bridge names at least one surface of the application".into());
        }
        for surface in &self.surfaces {
            if !SURFACE_KINDS.contains(&surface.kind.as_str()) {
                return invalid(format!(
                    "surface kind {:?} is not one of {SURFACE_KINDS:?}",
                    surface.kind
                ));
            }
            if !STATUSES.contains(&surface.status.as_str()) {
                return invalid(format!(
                    "surface status {:?} is used or planned",
                    surface.status
                ));
            }
            if !surface.doc.starts_with("https://") {
                return invalid(format!(
                    "{}: doc is the official https documentation",
                    surface.name
                ));
            }
        }
        for program in &self.programs {
            let plain = !program.is_empty()
                && program
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
            if !plain {
                return invalid(format!(
                    "program {program:?} is a bare program name, not a path"
                ));
            }
        }
        if !self.app.homepage.starts_with("https://") {
            return invalid("app.homepage is an https URL".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
id = "demo"
title = "Demo"
summary = "Does a demo."
programs = ["demo-cli"]
workspace = "A folder."
e2e = "cargo test"
[app]
name = "Demo"
homepage = "https://example.org"
license = "MIT"
versions = "1.x"
[[surfaces]]
kind = "cli"
name = "demo-cli"
doc = "https://example.org/cli"
[install]
arch = "pacman -S demo"
[vision]
app_ids = ["demo"]
when = "Never."
"#;

    #[test]
    fn a_complete_manifest_parses() {
        let manifest = Manifest::parse(GOOD).unwrap();
        assert_eq!(manifest.id, "demo");
        assert_eq!(manifest.surfaces[0].status, "used");
    }

    #[test]
    fn a_program_is_a_name_never_a_path() {
        let bad = GOOD.replace(r#"["demo-cli"]"#, r#"["/tmp/evil"]"#);
        assert!(Manifest::parse(&bad).is_err());
    }

    #[test]
    fn unknown_fields_and_kinds_are_refused() {
        assert!(Manifest::parse(&format!("{GOOD}\nextra = 1")).is_err());
        assert!(Manifest::parse(&GOOD.replace("kind = \"cli\"", "kind = \"magic\"")).is_err());
        assert!(Manifest::parse(&GOOD.replace("id = \"demo\"", "id = \"Demo X\"")).is_err());
    }
}
