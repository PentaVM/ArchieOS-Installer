//! The video bridge against the real melt and ffprobe.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=video cargo test -p archie-bridges --test video_e2e -- --nocapture
//! ```

use archie_bridges::{open, Programs};
use serde_json::{json, Map, Value};
use std::process::Command;

fn enabled() -> bool {
    std::env::var("ARCHIE_BRIDGE_E2E")
        .map(|v| v.split(',').any(|b| b.trim() == "video"))
        .unwrap_or(false)
}

fn call(bridge: &dyn archie_bridges::Bridge, tool: &str, arguments: Value) -> Value {
    let arguments: Map<String, Value> = arguments.as_object().unwrap().clone();
    let outcome = bridge.call(tool, &arguments);
    assert!(!outcome.is_error, "{tool}: {}", outcome.text);
    serde_json::from_str(&outcome.text).unwrap()
}

#[test]
fn two_sources_become_one_titled_video() {
    if !enabled() {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name video");
        return;
    }
    let media = std::env::temp_dir().join(format!("archie-video-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&media);
    std::fs::create_dir_all(&media).unwrap();
    // Two three-second sources, made by melt itself.
    for (name, colour) in [("red.mp4", "red"), ("blue.mp4", "blue")] {
        let status = Command::new("melt")
            .args([
                &format!("color:{colour}"),
                "out=74",
                "-profile",
                "atsc_720p_25",
                "-quiet",
                "-consumer",
            ])
            .arg(format!("avformat:{}", media.join(name).display()))
            .args(["vcodec=libx264", "preset=ultrafast"])
            .status()
            .expect("melt runs");
        assert!(status.success(), "making {name}");
    }
    let bridge = open("video", &media, &Programs::default()).unwrap();
    let bridge = bridge.as_ref();

    let sources = call(bridge, "video_sources", json!({}));
    assert_eq!(sources["files"].as_array().unwrap().len(), 2);
    let probe = call(bridge, "video_probe", json!({"file": "red.mp4"}));
    let seconds = probe["seconds"].as_f64().unwrap();
    assert!((2.5..3.5).contains(&seconds), "{probe}");

    let made = call(
        bridge,
        "video_render",
        json!({"output": "cut.mp4", "timeline": {
            "width": 1280, "height": 720, "fps": 25,
            "clips": [{"source": "red.mp4", "in_s": 0.5, "out_s": 2.0}, {"source": "blue.mp4", "out_s": 1.5}],
            "titles": [{"text": "Archie", "start_s": 0.2, "duration_s": 1.0}],
            "fade_s": 0.3
        }}),
    );
    assert_eq!(made["video"], "renders/cut.mp4");
    let rendered = call(bridge, "video_probe", json!({"file": "renders/cut.mp4"}));
    let length = rendered["seconds"].as_f64().unwrap();
    assert!((2.7..3.3).contains(&length), "1.5 s + 1.5 s: {rendered}");
    assert!(
        media.join("renders/cut.mlt").is_file(),
        "the project stays for Kdenlive"
    );

    let again = bridge.call(
        "video_render",
        json!({"output": "cut.mp4", "timeline": {"clips": [{"source": "red.mp4", "out_s": 1.0}]}})
            .as_object()
            .unwrap(),
    );
    assert!(again.is_error, "an existing render is never replaced");
    let _ = std::fs::remove_dir_all(&media);
}
