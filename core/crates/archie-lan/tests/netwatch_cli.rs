//! Reject mistyped capture limits before starting a subprocess or creating state.
use std::process::Command;

#[test]
fn invalid_capture_limits_never_start_an_unbounded_capture() {
    let root = std::env::temp_dir().join(format!("archie-netwatch-limits-{}", std::process::id()));
    assert!(!root.exists(), "test directory must be fresh");
    let cases: &[&[&str]] = &[
        &["--seconds", "0"],
        &["--seconds", "-1"],
        &["--seconds", "0.5"],
        &["--seconds", "tomorrow"],
        &["--seconds", "86401"],
        &["--seconds", "18446744073709551615"],
        &["--seconds"],
        &["--duration", "2"],
        &["--learn", "--seconds"],
        &["--gateway", "--seconds"],
        &["--learn", "--seconds", "2"],
        &["--gateway", "--seconds", "2"],
    ];
    for (index, flags) in cases.iter().enumerate() {
        let state = root.join(index.to_string());
        let output = Command::new(env!("CARGO_BIN_EXE_archie-netwatch"))
            .args([
                "live",
                "--interface",
                "unavailable-test-interface",
                "--state-dir",
            ])
            .arg(&state)
            .args(*flags)
            .env("PATH", "")
            .output()
            .expect("CLI starts");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !state.exists(),
            "{flags:?}: validation must precede state creation"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains(flags[0]));
    }
}
