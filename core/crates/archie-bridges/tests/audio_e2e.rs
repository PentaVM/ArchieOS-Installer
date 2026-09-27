//! The audio bridge against the real FFmpeg: measure, clean, level, mix, picture.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=audio cargo test -p archie-bridges --test audio_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;
use std::process::Command;

#[test]
fn a_voice_is_cleaned_levelled_and_mixed_with_music() {
    if !enabled("audio") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name audio");
        return;
    }
    let dir = workspace("audio");
    // A quiet "voice" (a 220 Hz tone with noise) and louder "music", made by FFmpeg itself.
    for (name, source) in [
        ("voice.wav", "sine=frequency=220:duration=6,volume=0.05"),
        ("music.wav", "sine=frequency=660:duration=8,volume=0.5"),
    ] {
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                source,
            ])
            .arg(dir.join(name))
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
    }
    let bridge = bridge("audio", &dir);
    let bridge = bridge.as_ref();

    let before = call(bridge, "audio_inspect", json!({"file": "voice.wav"}));
    let quiet = before["loudness"]["integrated_lufs"].as_f64().unwrap();
    assert!(quiet < -25.0, "{quiet}");

    let cleaned = call(
        bridge,
        "audio_process",
        json!({"file": "voice.wav", "steps": [{"highpass": {"hz": 80}}, "denoise", {"normalize": {"lufs": -16}}, {"fade_out": {"seconds": 1}}]}),
    );
    let level = cleaned["loudness"]["integrated_lufs"].as_f64().unwrap();
    assert!(
        (level + 16.0).abs() < 2.0,
        "normalized to about -16 LUFS: {level}"
    );
    assert!(dir.join(cleaned["waveform"].as_str().unwrap()).is_file());

    let mixed = call(
        bridge,
        "audio_mix",
        json!({"name": "episode", "format": "mp3", "tracks": [
            {"file": cleaned["file"], "start_s": 1.0},
            {"file": "music.wav", "gain_db": -18}
        ]}),
    );
    assert!(mixed["seconds"].as_f64().unwrap() >= 7.0);
    finish(dir);
}
