//! `archie-netwatch leases` reads a dnsmasq lease file into the state: quietly the first
//! time (the state is new, so it is learning), and a lease for a device not seen before is
//! told once the learning time is over.

use std::process::Command;

fn netwatch(state: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dnsmasq.leases");
    Command::new(env!("CARGO_BIN_EXE_archie-netwatch"))
        .arg("leases")
        .arg(&fixture)
        .arg("--state-dir")
        .arg(state)
        .args(extra)
        .output()
        .expect("CLI starts")
}

#[test]
fn the_router_s_leases_fill_the_inventory() {
    let state = std::env::temp_dir().join(format!("netwatch-leases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    let first = netwatch(&state, &[]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(
        stderr.contains("3 leases, 0 alerts, 3 devices known"),
        "{stderr}"
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("state.json")).unwrap()).unwrap();
    let tv = &saved["devices"]["54:4e:f0:07:3d:e8"];
    assert_eq!(tv["hostname"], "Living-Room-TV");
    assert_eq!(tv["leased_until_s"], 1_791_750_000u64);
    assert_eq!(saved["devices"]["00:50:56:12:34:56"]["leased_until_s"], 0);
    // The same file again, with no learning time: every device is known, nothing is new.
    let again = netwatch(&state, &["--learn", "0"]);
    assert!(String::from_utf8_lossy(&again.stderr).contains("3 leases, 0 alerts"));
    // Not a lease file: refused, nothing written.
    let other = std::env::temp_dir().join(format!("netwatch-leases-big-{}", std::process::id()));
    std::fs::write(&other, vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_archie-netwatch"))
        .arg("leases")
        .arg(&other)
        .arg("--state-dir")
        .arg(state.join("other"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!state.join("other").exists());
    std::fs::remove_file(&other).unwrap();
    std::fs::remove_dir_all(&state).unwrap();
}
