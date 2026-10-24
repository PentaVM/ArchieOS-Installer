//! Spotify's published ZeroConf getInfo, observation only. No login or account data.
use crate::http::{get, LanUrl, MAX_BODY};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Snapshot {
    pub class: Option<String>,
    pub state: BTreeMap<String, String>,
}

pub fn parse(bytes: &[u8]) -> Option<Snapshot> {
    if bytes.len() > MAX_BODY {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value.get("status")?.as_u64()? != 101 || value.get("spotifyError")?.as_i64()? != 0 {
        return None;
    }
    let device_type = value.get("deviceType")?.as_str()?;
    let class = match device_type {
        "Smartphone" | "Tablet" => Some("mobile"),
        "Computer" => Some("host"),
        "TV" => Some("television"),
        _ => None,
    }
    .map(str::to_string);
    let mut state = BTreeMap::new();
    for (source, dest) in [
        ("deviceType", "spotify.device_type"),
        ("remoteName", "spotify.name"),
        ("brandDisplayName", "spotify.manufacturer"),
        ("modelDisplayName", "spotify.model"),
    ] {
        if let Some(text) = value.get(source).and_then(|v| v.as_str()) {
            let text: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
            if !text.is_empty() {
                state.insert(dest.to_string(), text);
            }
        }
    }
    Some(Snapshot { class, state })
}

pub fn observe(endpoint: &str) -> Result<Snapshot, String> {
    // endpoint was formed from a matching SRV/TXT and same-host A/AAAA record.
    let url = LanUrl::parse(&format!("{endpoint}?action=getInfo&version=2.10.0"))
        .ok_or("invalid Spotify local endpoint")?;
    let bytes = get(&url)?;
    parse(&bytes).ok_or_else(|| {
        // Only public protocol fields explain a rejected reply. Never include
        // activeUser, deviceID, publicKey or the raw response in diagnostics.
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => format!(
                "invalid Spotify getInfo: status={:?}, spotifyError={:?}, deviceType={}",
                value.get("status").and_then(|v| v.as_i64()),
                value.get("spotifyError").and_then(|v| v.as_i64()),
                value
                    .get("deviceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("missing")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(32)
                    .collect::<String>()
            ),
            Err(error) => format!("invalid JSON from Spotify getInfo: {error}"),
        }
    })
}
