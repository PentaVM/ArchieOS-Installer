//! A Roku's own account of itself, over the External Control Protocol it serves on port
//! 8060 to anything on the LAN. Two questions are asked, both read-only: what the device
//! is, and what is on its screen. Nothing is sent that changes it; the keypress and launch
//! endpoints of the same protocol are not used here.

use crate::http::{get, LanUrl};
use crate::upnp::first;
use std::collections::BTreeMap;
use std::net::IpAddr;

/// What a Roku reports, as plain facts for the inventory.
pub fn state_of(device_info: &[u8], active_app: &[u8]) -> BTreeMap<String, String> {
    let info = String::from_utf8_lossy(device_info);
    let app = String::from_utf8_lossy(active_app);
    let mut state = BTreeMap::new();
    for (name, tag) in [
        ("name", "user-device-name"),
        ("model", "model-name"),
        ("software", "software-version"),
        ("power", "power-mode"),
        ("is_tv", "is-tv"),
    ] {
        let value = first(&info, tag);
        if !value.is_empty() {
            state.insert(name.to_string(), value);
        }
    }
    // `<app id="12" type="appl" version="…">Netflix</app>`: the name is the element's text.
    if let Some(open) = app.find("<app") {
        if let Some(close) = app[open..].find('>') {
            let rest = &app[open + close + 1..];
            if let Some(end) = rest.find("</app>") {
                let name: String = rest[..end].chars().filter(|c| !c.is_control()).take(100).collect();
                if !name.trim().is_empty() {
                    state.insert("showing".into(), name.trim().to_string());
                }
            }
        }
    }
    state
}

/// Asks the Roku at `address`. An unanswered question leaves its facts out.
pub fn observe(address: IpAddr) -> BTreeMap<String, String> {
    let ask = |path: &str| {
        LanUrl::parse(&format!("http://{}/{path}", std::net::SocketAddr::new(address, 8060)))
            .and_then(|url| get(&url).ok())
            .unwrap_or_default()
    };
    state_of(&ask("query/device-info"), &ask("query/active-app"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "<?xml version=\"1.0\"?><device-info><model-name>Roku Express 4K</model-name>\
        <user-device-name>Living</user-device-name><software-version>12.5.0</software-version>\
        <power-mode>PowerOn</power-mode><is-tv>false</is-tv><serial-number>X00000000000</serial-number></device-info>";
    const APP: &str = "<?xml version=\"1.0\"?><active-app><app id=\"12\" type=\"appl\" version=\"5.1\">Netflix</app></active-app>";

    #[test]
    fn a_roku_says_what_it_is_and_what_is_on() {
        let state = state_of(INFO.as_bytes(), APP.as_bytes());
        assert_eq!(state["name"], "Living");
        assert_eq!(state["model"], "Roku Express 4K");
        assert_eq!(state["power"], "PowerOn");
        assert_eq!(state["showing"], "Netflix");
        assert!(!state.contains_key("serial"), "the serial number is not kept");
    }

    #[test]
    fn a_roku_that_does_not_answer_adds_nothing() {
        assert!(state_of(b"", b"").is_empty());
        assert_eq!(state_of(INFO.as_bytes(), b"<active-app><app>").get("showing"), None);
    }
}
