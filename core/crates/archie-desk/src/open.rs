//! Opening a delivered file in the person's own application, only as their `desk.toml`
//! allows:
//!
//! ```toml
//! [open]
//! step = "default"                 # whatever the system opens .step with (e.g. Fusion 360)
//! pdf = "default"
//! kicad_pcb = { program = 'C:\Program Files\KiCad\10.0\bin\pcbnew.exe' }
//! blend = { program = "/usr/bin/blender", args = ["{file}"] }
//! ```
//!
//! A kind of file not listed is delivered and never opened. The program runs directly,
//! with the file as an argument; no shell reads anything.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Rule {
    /// `"default"`: the operating system's own association.
    Default(String),
    Program {
        program: PathBuf,
        #[serde(default = "file_only")]
        args: Vec<String>,
    },
}

fn file_only() -> Vec<String> {
    vec!["{file}".into()]
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub open: BTreeMap<String, Rule>,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Self, String> {
        let rules: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        for (extension, rule) in &rules.open {
            if let Rule::Default(word) = rule {
                if word != "default" {
                    return Err(format!(
                        "open.{extension} is \"default\" or {{ program = … }}"
                    ));
                }
            }
        }
        Ok(rules)
    }

    /// The command that would open `path`, if a rule covers its kind.
    pub fn command(&self, path: &Path) -> Option<Command> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        let rule = self.open.get(&extension)?;
        let mut command = match rule {
            Rule::Default(_) => system_opener(path),
            Rule::Program { program, args } => {
                let mut command = Command::new(program);
                for arg in args {
                    if arg == "{file}" {
                        command.arg(path);
                    } else {
                        command.arg(arg);
                    }
                }
                command
            }
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Some(command)
    }

    /// Opens `path` if a rule allows it. Returns whether it started something.
    pub fn open(&self, path: &Path) -> bool {
        match self.command(path).map(|mut c| c.spawn()) {
            Some(Ok(_)) => true,
            Some(Err(e)) => {
                eprintln!("archie-desk: could not open {}: {e}", path.display());
                false
            }
            None => false,
        }
    }
}

fn system_opener(path: &Path) -> Command {
    #[cfg(windows)]
    let mut command = Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_kinds_open() {
        let rules = Rules::parse(
            r#"
[open]
step = "default"
kicad_pcb = { program = "/opt/kicad/pcbnew", args = ["--frame", "{file}"] }
"#,
        )
        .unwrap();
        assert!(
            rules.command(Path::new("/d/a.STEP")).is_some(),
            "extensions ignore case"
        );
        assert!(rules.command(Path::new("/d/a.exe")).is_none());
        assert!(rules.command(Path::new("/d/noext")).is_none());
        let command = rules.command(Path::new("/d/b.kicad_pcb")).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--frame", "/d/b.kicad_pcb"]);
        assert!(Rules::parse("[open]\nstep = \"always\"").is_err());
    }
}
