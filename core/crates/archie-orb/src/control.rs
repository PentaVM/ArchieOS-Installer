//! The native Observatory's loopback control channel.
//!
//! One JSON line in, one line out, on `127.0.0.1:7668`. The display is the only authority on
//! what it can show: its scenes and its look presets (the JSON files embedded in its
//! resources). Callers ask it with [`catalog`] instead of keeping their own copy of the
//! names, and every request is validated by the display, which answers `ok` or
//! `error: <reason>`. Nothing here can run commands, reach Stage or write to displayd.

use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Where the native display listens (loopback only).
pub const ADDRESS: &str = "127.0.0.1:7668";

/// How long a caller waits for the display before giving up.
pub const TIMEOUT: Duration = Duration::from_millis(1500);

/// The longest reply line accepted; the catalog is the largest and is a few hundred bytes.
const MAX_REPLY: u64 = 64 * 1024;

/// What the running display can show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Catalog {
    pub scenes: Vec<String>,
    pub presets: Vec<String>,
}

/// Legacy camera framings; these do not replace the display's preset catalog.
pub const CINEMATICS: [&str; 4] = ["field", "crown", "atlas", "dense"];
/// Presentation layers that can be switched on and off.
pub const OVERLAYS: [&str; 7] = [
    "hud", "shell", "ambient", "guides", "links", "pulses", "bloom",
];
/// Local panels supported by the native shell.
pub const PANELS: [&str; 7] = [
    "forge",
    "inspector",
    "settings",
    "sentry",
    "activity",
    "notifications",
    "search",
];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("`{action} {value}` is not a visual command the display accepts")]
    Invalid { action: String, value: String },
    #[error("the Observatory is not listening on {ADDRESS}: {0}")]
    Unreachable(String),
    #[error("the Observatory did not answer within {}ms", TIMEOUT.as_millis())]
    Timeout,
    #[error("the Observatory refused: {0}")]
    Refused(String),
    #[error("the Observatory answered something unexpected: {0}")]
    Malformed(String),
}

/// Check presentation command shape; scene and preset membership belongs to the display.
pub fn valid(action: &str, value: &str, enabled: Option<bool>) -> bool {
    match action {
        "scene" | "preset" => {
            !value.trim().is_empty()
                && value.len() <= 128
                && !value
                    .chars()
                    .any(|c| c.is_control() || c == '/' || c == '\\')
                && value != "."
                && value != ".."
                && enabled.is_none()
        }
        "cinematic" => CINEMATICS.contains(&value) && enabled.is_none(),
        "panel" => PANELS.contains(&value) && enabled.is_none(),
        "overlay" => OVERLAYS.contains(&value) && enabled.is_some(),
        _ => false,
    }
}

async fn exchange(request: &Value) -> Result<String, ControlError> {
    let line = format!("{request}\n");
    tokio::time::timeout(TIMEOUT, async {
        let stream = TcpStream::connect(ADDRESS)
            .await
            .map_err(|error| ControlError::Unreachable(error.to_string()))?;
        let (read, mut write) = stream.into_split();
        write
            .write_all(line.as_bytes())
            .await
            .map_err(|error| ControlError::Unreachable(error.to_string()))?;
        let mut reply = String::new();
        BufReader::new(tokio::io::AsyncReadExt::take(read, MAX_REPLY))
            .read_line(&mut reply)
            .await
            .map_err(|error| ControlError::Malformed(error.to_string()))?;
        Ok(reply.trim_end().to_owned())
    })
    .await
    .map_err(|_| ControlError::Timeout)?
}

/// `ok` is success, `error: <reason>` a refusal, anything else a protocol fault.
fn outcome(reply: &str) -> Result<(), ControlError> {
    if reply == "ok" {
        Ok(())
    } else if let Some(reason) = reply.strip_prefix("error:") {
        Err(ControlError::Refused(reason.trim().to_owned()))
    } else {
        Err(ControlError::Malformed(reply.chars().take(200).collect()))
    }
}

fn parse_catalog(reply: &str) -> Result<Catalog, ControlError> {
    if let Some(reason) = reply.strip_prefix("error:") {
        return Err(ControlError::Refused(reason.trim().to_owned()));
    }
    serde_json::from_str(reply).map_err(|error| ControlError::Malformed(error.to_string()))
}

/// Send a bounded presentation command; the display validates its current capabilities.
pub async fn send(action: &str, value: &str, enabled: Option<bool>) -> Result<(), ControlError> {
    if !valid(action, value, enabled) {
        return Err(ControlError::Invalid {
            action: action.chars().take(64).collect(),
            value: value.chars().take(64).collect(),
        });
    }
    let mut request = json!({"action": action, "value": value});
    if let Some(enabled) = enabled {
        request["enabled"] = json!(enabled);
    }
    outcome(&exchange(&request).await?)
}

/// The scenes and presets the running display accepts.
pub async fn catalog() -> Result<Catalog, ControlError> {
    parse_catalog(&exchange(&json!({"action": "list"})).await?)
}

/// Switch the Observatory to a scene.
pub async fn scene(name: &str) -> Result<(), ControlError> {
    outcome(&exchange(&json!({"action": "scene", "value": name})).await?)
}

/// Apply a look preset to the orb (the display opens Orb first).
pub async fn preset(name: &str) -> Result<(), ControlError> {
    outcome(&exchange(&json!({"action": "preset", "value": name})).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_controls_preserve_dynamic_scene_and_preset_names() {
        assert!(valid("scene", "future-scene", None));
        assert!(valid("preset", "future-preset", None));
        for name in CINEMATICS {
            assert!(valid("cinematic", name, None));
        }
        for name in OVERLAYS {
            assert!(valid("overlay", name, Some(false)));
            assert!(!valid("overlay", name, None));
        }
        assert!(valid("panel", "inspector", None));
        assert!(!valid("panel", "execute", None));
        assert!(!valid("preset", "../crown", None));
        assert!(!valid("preset", "", None));
        assert!(!valid("scene", "orb\n", None));
    }

    #[tokio::test]
    async fn invalid_controls_are_rejected_before_connecting() {
        assert!(matches!(
            send("panel", "execute", None).await,
            Err(ControlError::Invalid { .. })
        ));
        assert!(matches!(
            send("overlay", "hud", None).await,
            Err(ControlError::Invalid { .. })
        ));
    }

    #[test]
    fn replies_are_read_as_the_display_writes_them() {
        assert_eq!(outcome("ok"), Ok(()));
        assert_eq!(
            outcome("error: unknown preset sparkle"),
            Err(ControlError::Refused("unknown preset sparkle".into()))
        );
        assert!(matches!(outcome("okay"), Err(ControlError::Malformed(_))));
        assert_eq!(
            parse_catalog(r#"{"scenes":["orb"],"presets":["field","neural"]}"#).unwrap(),
            Catalog {
                scenes: vec!["orb".into()],
                presets: vec!["field".into(), "neural".into()]
            }
        );
        assert!(matches!(
            parse_catalog("[]"),
            Err(ControlError::Malformed(_))
        ));
    }
}
