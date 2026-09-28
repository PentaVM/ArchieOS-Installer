//! The obs bridge against a running OBS Studio with its WebSocket server on.
//!
//! OBS must already be open, on this machine, with Tools › WebSocket Server Settings
//! enabled. `ARCHIE_PROGRAM_OBS_PASSWORD` names the file holding its password, if it has
//! one.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=obs cargo test -p archie-bridges --test obs_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;

#[test]
fn an_overlay_is_added_changed_and_seen() {
    if !enabled("obs") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name obs");
        return;
    }
    let dir = workspace("obs");
    let bridge = bridge("obs", &dir);
    let bridge = bridge.as_ref();

    let status = call(bridge, "obs_status", json!({}));
    let scene = status["scene"].as_str().unwrap().to_string();
    let name = format!("archie-e2e-{}", std::process::id());
    call(
        bridge,
        "obs_add_overlay",
        json!({"scene": scene, "name": name, "text": "Archie", "x": 80, "y": 80, "size": 64, "color": "#2f6fde"}),
    );
    call(
        bridge,
        "obs_set_text",
        json!({"input": name, "text": "Archie · live test"}),
    );
    call(
        bridge,
        "obs_show",
        json!({"scene": scene, "source": name, "visible": true}),
    );
    let shot = outcome(bridge, "obs_screenshot", json!({}));
    assert!(!shot.is_error && !shot.images.is_empty(), "{}", shot.text);
    call(
        bridge,
        "obs_show",
        json!({"scene": scene, "source": name, "visible": false}),
    );
    finish(dir);
}
