//! `archie visual` — the Observatory's look is controlled from here, never from an overlay.
//!
//! Every visual parameter lives in a named preset; a scene is one of the Observatory views.
//! The CLI sends one JSON line to the native display's loopback control channel
//! (`127.0.0.1:7668`), which accepts only the allowlisted names below. Nothing here can
//! run commands, reach Stage or write to displayd.

use clap::Subcommand;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Where the native display listens (loopback only).
pub const CONTROL_ADDRESS: &str = "127.0.0.1:7668";

/// Observatory views.
pub const SCENES: &[&str] = &["orb", "forge", "world"];

/// Look presets. `field` ("Regiones vivas") is the original look and the default.
pub const PRESETS: &[&str] = &["field", "sphere", "orb", "neural"];

#[derive(Debug, Subcommand)]
pub enum VisualCommand {
    /// Switch the Observatory view: orb, forge or world.
    Scene { name: String },
    /// Apply a look preset to the orb (opens Orb): field, sphere, orb or neural.
    Preset { name: String },
    /// List the scenes and presets this build accepts.
    List,
}

/// The JSON object sent to the display for a command, or why it is not valid.
pub fn request_for(command: &VisualCommand) -> Result<Option<Value>, String> {
    match command {
        VisualCommand::Scene { name } if SCENES.contains(&name.as_str()) => {
            Ok(Some(json!({"action": "scene", "value": name})))
        }
        VisualCommand::Scene { name } => Err(format!(
            "unknown scene `{name}` (expected one of: {})",
            SCENES.join(", ")
        )),
        VisualCommand::Preset { name } if PRESETS.contains(&name.as_str()) => {
            Ok(Some(json!({"action": "preset", "value": name})))
        }
        VisualCommand::Preset { name } => Err(format!(
            "unknown preset `{name}` (expected one of: {})",
            PRESETS.join(", ")
        )),
        VisualCommand::List => Ok(None),
    }
}

async fn send(request: &Value) -> Result<String, String> {
    let line = format!("{request}\n");
    tokio::time::timeout(Duration::from_millis(1500), async {
        let mut stream = TcpStream::connect(CONTROL_ADDRESS).await.map_err(|error| {
            format!("the Observatory is not listening on {CONTROL_ADDRESS}: {error}")
        })?;
        stream
            .write_all(line.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        let mut reply = String::new();
        stream
            .read_to_string(&mut reply)
            .await
            .map_err(|error| error.to_string())?;
        Ok(reply.trim().to_owned())
    })
    .await
    .map_err(|_| "the Observatory did not answer".to_owned())?
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
        Ok(Some(request)) => match send(&request).await {
            Ok(reply) if reply == "ok" => {
                println!("ok");
                std::process::ExitCode::SUCCESS
            }
            Ok(reply) => {
                eprintln!("archie: {reply}");
                std::process::ExitCode::FAILURE
            }
            Err(message) => {
                eprintln!("archie: {message}");
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
        assert_eq!(
            request_for(&preset).unwrap(),
            Some(json!({"action": "preset", "value": "sphere"}))
        );
        let scene = VisualCommand::Scene {
            name: "world".into(),
        };
        assert_eq!(
            request_for(&scene).unwrap(),
            Some(json!({"action": "scene", "value": "world"}))
        );
        assert!(request_for(&VisualCommand::List).unwrap().is_none());
    }

    #[test]
    fn the_retired_cinematic_profiles_and_overlays_are_rejected() {
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
}
