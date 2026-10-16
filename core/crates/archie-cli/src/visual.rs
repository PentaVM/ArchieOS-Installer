//! `archie visual` — the Observatory's look is controlled from here, never from an overlay.
//!
//! Every visual parameter lives in a named preset; a scene is one of the Observatory views.
//! The CLI is a thin client of [`archie_orb::control`], the same one the voice uses: one
//! client and one list of names for both. Nothing here can run commands, reach Stage or
//! write to displayd.

use archie_orb::control::{self, PRESETS, SCENES};
use clap::Subcommand;
#[cfg(test)]
use serde_json::Value;

#[derive(Debug, Subcommand)]
pub enum VisualCommand {
    /// Switch the Observatory view: orb, forge or world.
    Scene { name: String },
    /// Apply a look preset to the orb (opens Orb): field, sphere, orb or neural.
    Preset { name: String },
    /// List the scenes and presets this build accepts.
    List,
}

/// The `(action, value)` sent to the display for a command, or why it is not valid.
fn request_for(command: &VisualCommand) -> Result<Option<(&'static str, &str)>, String> {
    let (action, name, names): (_, &str, &[&str]) = match command {
        VisualCommand::Scene { name } => ("scene", name, &SCENES),
        VisualCommand::Preset { name } => ("preset", name, &PRESETS),
        VisualCommand::List => return Ok(None),
    };
    if control::valid(action, name, None) {
        Ok(Some((action, name)))
    } else {
        Err(format!(
            "unknown {action} `{name}` (expected one of: {})",
            names.join(", ")
        ))
    }
}

pub async fn run(command: &VisualCommand) -> std::process::ExitCode {
    match request_for(command) {
        Err(message) => {
            eprintln!("archie: {message}");
            std::process::ExitCode::FAILURE
        }
        Ok(None) => {
            println!("scenes:  {}", SCENES.join(" "));
            println!("presets: {}", PRESETS.join(" "));
            std::process::ExitCode::SUCCESS
        }
        Ok(Some((action, value))) => match control::send(action, value, None).await {
            Ok(()) => {
                println!("ok");
                std::process::ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("archie: {error}");
                std::process::ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_scenes_are_allowlisted() {
        let preset = VisualCommand::Preset {
            name: "sphere".into(),
        };
        assert_eq!(request_for(&preset).unwrap(), Some(("preset", "sphere")));
        let scene = VisualCommand::Scene {
            name: "world".into(),
        };
        assert_eq!(request_for(&scene).unwrap(), Some(("scene", "world")));
        assert!(request_for(&VisualCommand::List).unwrap().is_none());
    }

    #[test]
    fn preset_names_are_distinct_from_cinematic_and_overlay_controls() {
        for name in ["crown", "atlas", "dense", "bloom", "hud", ""] {
            assert!(
                request_for(&VisualCommand::Preset { name: name.into() }).is_err(),
                "{name}"
            );
        }
        assert!(request_for(&VisualCommand::Scene {
            name: "overlay".into()
        })
        .is_err());
    }
    #[test]
    fn every_preset_has_a_file_in_the_display() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../display/native/presets");
        for name in PRESETS {
            let text = std::fs::read_to_string(dir.join(format!("{name}.json")))
                .unwrap_or_else(|error| panic!("preset file for {name}: {error}"));
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["name"], *name);
            assert!(value["settings"].is_object(), "{name}");
        }
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(
            files,
            PRESETS.len(),
            "a preset file the CLI does not accept"
        );
    }
}
