//! The native display's loopback control channel, for every Rust caller.
//!
//! One JSON line in, one line out, on `127.0.0.1:7668`: `ok` or `error: <reason>`. The CLI
//! and the voice both go through here, so there is one client and one list of the names
//! the display accepts (the display checks them again on its side). Nothing here can run
//! commands, reach Stage or write to displayd.

use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Where the native display listens (loopback only).
pub const ADDRESS: &str = "127.0.0.1:7668";
/// How long a caller waits for the display before giving up.
pub const TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_REPLY: u64 = 64 * 1024;

/// Observatory views.
pub const SCENES: [&str; 3] = ["orb", "forge", "world"];
/// Look presets. `field` ("Regiones vivas") is the original look.
pub const PRESETS: [&str; 4] = ["field", "sphere", "orb", "neural"];
/// Older camera framings the display still accepts.
pub const CINEMATICS: [&str; 4] = ["field", "crown", "atlas", "dense"];
/// Layers that can be switched on and off.
pub const OVERLAYS: [&str; 7] = [
    "hud", "shell", "ambient", "guides", "links", "pulses", "bloom",
];
/// Shell panels (only the native shell accepts these).
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

/// Whether the display accepts `action value` (`enabled` is required for overlays only).
pub fn valid(action: &str, value: &str, enabled: Option<bool>) -> bool {
    match action {
        "scene" => SCENES.contains(&value),
        "preset" => PRESETS.contains(&value),
        "cinematic" => CINEMATICS.contains(&value),
        "panel" => PANELS.contains(&value),
        "overlay" => enabled.is_some() && OVERLAYS.contains(&value),
        _ => false,
    }
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

/// Checks the command, sends it and reads the display's answer.
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
    let line = format!("{request}\n");
    let reply = tokio::time::timeout(TIMEOUT, async {
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
    .map_err(|_| ControlError::Timeout)??;
    outcome(&reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_list_for_every_caller() {
        assert!(valid("scene", "world", None));
        assert!(valid("preset", "sphere", None));
        for name in CINEMATICS {
            assert!(valid("cinematic", name, None));
        }
        assert!(valid("overlay", "hud", Some(false)));
        assert!(!valid("overlay", "hud", None));
        assert!(valid("panel", "inspector", None));
        assert!(!valid("panel", "execute", None));
        assert!(!valid("preset", "../crown", None));
    }

    #[test]
    fn replies_are_read_as_the_display_writes_them() {
        assert_eq!(outcome("ok"), Ok(()));
        assert_eq!(
            outcome("error: invalid visual command"),
            Err(ControlError::Refused("invalid visual command".into()))
        );
        assert!(matches!(outcome("okay"), Err(ControlError::Malformed(_))));
    }

    #[tokio::test]
    async fn an_invalid_command_never_connects() {
        assert!(matches!(
            send("panel", "execute", None).await,
            Err(ControlError::Invalid { .. })
        ));
    }
}
