use std::{fs, process::Command};

#[test]
fn offline_network_command_works_without_daemon_and_rejects_outside_scope() {
    let dir = std::env::temp_dir().join(format!("archie-passive-network-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.tsv");
    let now = dir.join("now.tsv");
    fs::write(&old, "192.168.1.2\t5353\tprinter._ipp._tcp.local\n").unwrap();
    fs::write(&now, "192.168.1.3\t5353\ttv._googlecast._tcp.local\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args([
            "--json",
            "sentry-network-analyze",
            "--scope",
            "192.168.1.0/24",
            "--input",
        ])
        .arg(&now)
        .arg("--baseline")
        .arg(&old)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["trusted"], false);
    assert_eq!(value["assets"][0]["ipv4_address"], "192.168.1.3");
    assert_eq!(value["changes"].as_array().unwrap().len(), 2);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("tv._googlecast"));
    let denied = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args([
            "sentry-network-analyze",
            "--scope",
            "192.168.2.0/24",
            "--input",
        ])
        .arg(&now)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    fs::remove_file(old).unwrap();
    fs::remove_file(now).unwrap();
    fs::remove_dir(dir).unwrap();
}
