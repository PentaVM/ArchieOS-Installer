use archie_policy::{Action, Lease, LeaseRejection, Operation, Principal, Resource};
use archie_types::{TaskId, Timestamp};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum FreeCadTool {
    CreateDocument {
        name: String,
    },
    OpenDocument {
        path: PathBuf,
    },
    InspectDocument {
        path: PathBuf,
    },
    CreatePrimitive {
        kind: String,
        parameters_permille: Vec<i64>,
    },
    ModifyParameter {
        name: String,
        value_permille: i64,
    },
    RunPython {
        script: PathBuf,
    },
    Recompute,
    Validate,
    ExportStep {
        path: PathBuf,
    },
    ExportStl {
        path: PathBuf,
    },
    RenderPreview {
        path: PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum FreeCadError {
    #[error("FreeCAD path escapes the authorised workspace")]
    PathEscape,
    #[error("FreeCAD tool input is invalid")]
    InvalidInput,
    #[error("capability lease rejected the FreeCAD operation: {0}")]
    Lease(LeaseRejection),
    #[error("FreeCAD I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("FreeCAD command failed")]
    CommandFailed,
}

pub struct AuthorizedWorkspace {
    root: PathBuf,
    principal: Principal,
    task_id: Option<TaskId>,
}

impl AuthorizedWorkspace {
    pub fn from_lease(
        root: impl AsRef<Path>,
        principal: Principal,
        task_id: Option<TaskId>,
        lease: &mut Lease,
    ) -> Result<Self, FreeCadError> {
        let root = root.as_ref().to_path_buf();
        let operation = Operation {
            principal: principal.clone(),
            resource: Resource::Filesystem {
                path: root.display().to_string(),
            },
            action: Action::Write,
            task_id,
            user_present: false,
        };
        lease
            .authorises(&operation, Timestamp::now())
            .map_err(FreeCadError::Lease)?;
        lease.consume();
        Ok(Self {
            root,
            principal,
            task_id,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    pub fn resolve(&self, relative: &Path) -> Result<PathBuf, FreeCadError> {
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(FreeCadError::PathEscape);
        }
        Ok(self.root.join(relative))
    }
}

pub struct FreeCadExecutor {
    command: PathBuf,
}

impl FreeCadExecutor {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn run_script(
        &self,
        workspace: &AuthorizedWorkspace,
        script: &Path,
    ) -> Result<(), FreeCadError> {
        let script = workspace.resolve(script)?;
        let canonical_root = fs::canonicalize(workspace.root())?;
        let canonical_script = fs::canonicalize(&script)?;
        if !canonical_script.starts_with(&canonical_root) {
            return Err(FreeCadError::PathEscape);
        }
        let status = Command::new(&self.command)
            .arg(&canonical_script)
            .current_dir(&canonical_root)
            .stdin(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(FreeCadError::CommandFailed);
        }
        Ok(())
    }
}

/// A deterministic parametric bracket demo. Critical dimensions are inputs, not guesses.
pub fn motor_bracket_script(
    width_mm: u32,
    height_mm: u32,
    depth_mm: u32,
    hole_mm: u32,
) -> Result<String, FreeCadError> {
    if [width_mm, height_mm, depth_mm, hole_mm].contains(&0) || hole_mm >= width_mm.min(height_mm) {
        return Err(FreeCadError::InvalidInput);
    }
    Ok(format!(
        r#"import FreeCAD as App
import Part
import Import
import Mesh

doc = App.newDocument("MotorBracket")
params = doc.addObject("App::FeaturePython", "Parameters")
params.addProperty("App::PropertyLength", "Width").Width = {width_mm}
params.addProperty("App::PropertyLength", "Height").Height = {height_mm}
params.addProperty("App::PropertyLength", "Depth").Depth = {depth_mm}
params.addProperty("App::PropertyLength", "HoleDiameter").HoleDiameter = {hole_mm}

body = doc.addObject("PartDesign::Feature", "Bracket")
base = Part.makeBox(params.Width, params.Depth, params.Height)
hole = Part.makeCylinder(params.HoleDiameter / 2, params.Depth,
                         App.Vector(params.Width / 2, 0, params.Height / 2),
                         App.Vector(0, 1, 0))
body.Shape = base.cut(hole)
doc.recompute()
if body.Shape.isNull() or not body.Shape.isValid() or body.Shape.Solids.__len__() != 1:
    raise RuntimeError("invalid solid")
doc.saveAs("motor-bracket.FCStd")
Import.export([body], "motor-bracket.step")
Mesh.export([body], "motor-bracket.stl")
print("ARCHIE_FREECAD_OK solid=1 parametric=1")
"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_escape() {
        let workspace = AuthorizedWorkspace {
            root: PathBuf::from("/archie/workspaces/demo"),
            principal: Principal::Human {
                id: "test".into(),
                authenticated: true,
            },
            task_id: None,
        };
        assert!(matches!(
            workspace.resolve(Path::new("../vault/key")),
            Err(FreeCadError::PathEscape)
        ));
        assert!(workspace.resolve(Path::new("exports/part.step")).is_ok());
    }

    #[test]
    fn demo_requires_critical_dimensions() {
        assert!(motor_bracket_script(0, 40, 5, 8).is_err());
        let script = motor_bracket_script(80, 40, 5, 8).unwrap();
        assert!(script.contains("App::PropertyLength"));
        assert!(script.contains("isValid"));
        assert!(script.contains("motor-bracket.step"));
    }
}
