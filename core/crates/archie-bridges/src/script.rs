//! Scripts for applications whose automation language is their own embedded one.
//!
//! FreeCAD (`FreeCADCmd`), Blender (`blender --background --python`), GIMP
//! (`gimp-console --batch-interpreter`), QGIS and Inkscape extensions are driven by a
//! script in the language they embed. The rule for writing one here:
//!
//! * the script is a `&'static str` constant in the bridge's Rust, reviewed like code;
//! * it reads its arguments from the JSON file named by `ARCHIE_JOB_ARGS` and writes its
//!   answer as JSON to the file named by `ARCHIE_JOB_RESULT`;
//! * nothing a planner sends is ever formatted into the script's text.
//!
//! The third rule is what keeps a tool call from becoming code execution: an argument can
//! be any string at all and it is still only data the script reads.

use crate::run::{Ran, Run, RunError};
use crate::workspace::{Workspace, WorkspaceError};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

/// A script and how its application runs one.
#[derive(Debug, Clone, Copy)]
pub struct Script {
    /// A name for the job files, such as `freecad-inspect`.
    pub name: &'static str,
    /// The file extension the interpreter expects, such as `py` or `scm`.
    pub extension: &'static str,
    pub text: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("writing the job: {0}")]
    Write(String),
    #[error("the script failed: {0}")]
    Failed(String),
    #[error("the script's answer is not JSON: {0}")]
    Answer(String),
}

/// A finished job: the script's JSON answer and what the program printed.
#[derive(Debug, Clone)]
pub struct Answer {
    pub value: Value,
    pub ran: Ran,
}

impl Script {
    /// Runs the script with `arguments` in `workspace`. `argv` builds the program's
    /// arguments from the path of the written script, e.g.
    /// `|script| vec!["--background".into(), "--python".into(), script.into()]`.
    pub fn run(
        &self,
        workspace: &Workspace,
        program: PathBuf,
        argv: impl FnOnce(&std::path::Path) -> Vec<std::ffi::OsString>,
        arguments: &Value,
        limit: Duration,
    ) -> Result<Answer, ScriptError> {
        let jobs = workspace.scratch("jobs")?;
        let stamp = format!(
            "{}-{}-{}",
            self.name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let script = jobs.join(format!("{stamp}.{}", self.extension));
        let args = jobs.join(format!("{stamp}.args.json"));
        let result = jobs.join(format!("{stamp}.result.json"));
        let write = |path: &PathBuf, bytes: &[u8]| {
            std::fs::write(path, bytes).map_err(|e| ScriptError::Write(e.to_string()))
        };
        write(&script, self.text.as_bytes())?;
        write(
            &args,
            serde_json::to_string(arguments)
                .map_err(|e| ScriptError::Write(e.to_string()))?
                .as_bytes(),
        )?;
        let ran = Run::new(program)
            .args(argv(&script))
            .cwd(workspace.root())
            .env("ARCHIE_JOB_SCRIPT", &script)
            .env("ARCHIE_JOB_ARGS", &args)
            .env("ARCHIE_JOB_RESULT", &result)
            .env("ARCHIE_WORKSPACE", workspace.root())
            .limit(limit)
            .run();
        let answer = std::fs::read_to_string(&result);
        for path in [&script, &args, &result] {
            let _ = std::fs::remove_file(path);
        }
        let ran = ran?;
        let text = answer.map_err(|_| {
            ScriptError::Failed(if ran.said().is_empty() {
                format!("it exited with {:?} and gave no answer", ran.code)
            } else {
                ran.said()
            })
        })?;
        let value = serde_json::from_str(&text).map_err(|e| ScriptError::Answer(e.to_string()))?;
        Ok(Answer { value, ran })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // A shell stands in for an embedded interpreter: it reads its arguments from the file
    // and answers through the other, like a FreeCAD or Blender script does.
    const ECHO: Script = Script {
        name: "echo",
        extension: "sh",
        text: "cp \"$ARCHIE_JOB_ARGS\" \"$ARCHIE_JOB_RESULT\"\n",
    };

    #[test]
    fn arguments_are_data_never_code() {
        let dir = std::env::temp_dir().join(format!("archie-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::open(&dir).unwrap();
        let hostile = serde_json::json!({"name": "\"; rm -rf / #", "n": 3});
        let answer = ECHO
            .run(
                &ws,
                PathBuf::from("/bin/sh"),
                |script| vec![script.as_os_str().to_os_string()],
                &hostile,
                Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(answer.value, hostile);
        let left: Vec<_> = std::fs::read_dir(dir.join(".archie/jobs"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "job files are removed");
        let _ = std::fs::remove_dir_all(dir);
    }
}
