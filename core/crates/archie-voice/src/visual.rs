//! Allowlisted local presentation controls. No Core operations or model authority.
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) fn valid_command(action: &str, value: &str, enabled: Option<bool>) -> bool {
    match action {
        "scene" => matches!(value, "orb" | "forge" | "world"),
        "cinematic" => matches!(value, "field" | "crown" | "atlas" | "dense"),
        "preset" => matches!(value, "field" | "sphere" | "orb" | "neural"),
        "panel" => matches!(
            value,
            "forge" | "inspector" | "settings" | "sentry" | "activity" | "notifications" | "search"
        ),
        "overlay" => {
            enabled.is_some()
                && matches!(
                    value,
                    "hud" | "shell" | "ambient" | "guides" | "links" | "pulses" | "bloom"
                )
        }
        _ => false,
    }
}

pub(crate) async fn visual_command(action: &str, value: &str, enabled: Option<bool>) -> String {
    let valid = valid_command(action, value, enabled);
    if !valid {
        return "No cambié el Observatory: la opción visual no es válida.".into();
    }
    let mut command = json!({"action": action, "value": value});
    if let Some(enabled) = enabled {
        command["enabled"] = json!(enabled);
    }
    let request = format!("{command}\n");
    let sent = tokio::time::timeout(Duration::from_millis(750), async {
        let mut stream = TcpStream::connect("127.0.0.1:7668")
            .await
            .map_err(|e| e.to_string())?;
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut reply = [0u8; 3];
        stream
            .read_exact(&mut reply)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<bool, String>(&reply == b"ok\n")
    })
    .await;
    match sent {
        Ok(Ok(true)) => format!(
            "Observatory actualizado: {action} {value}{}.",
            enabled
                .map(|on| if on { " on" } else { " off" })
                .unwrap_or("")
        ),
        Ok(Ok(false)) => "El Observatory rechazó ese cambio visual.".into(),
        Ok(Err(_)) | Err(_) => {
            "No pude cambiar el Observatory: la ventana local no responde.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unsupported_action_is_rejected_before_connecting() {
        assert!(visual_command("panel", "execute", None)
            .await
            .contains("no es válida"));
    }
    #[test]
    fn preserves_legacy_controls() {
        assert!(valid_command("scene", "world", None));
        for name in ["field", "crown", "atlas", "dense"] {
            assert!(valid_command("cinematic", name, None));
        }
        assert!(valid_command("overlay", "hud", Some(false)));
        assert!(!valid_command("overlay", "hud", None));
    }
    #[test]
    fn additive_presets_and_shell_panels() {
        for name in ["field", "sphere", "orb", "neural"] {
            assert!(valid_command("preset", name, None));
        }
        assert!(valid_command("panel", "inspector", None));
        assert!(!valid_command("panel", "execute", None));
        assert!(!valid_command("preset", "../crown", None));
    }
}
