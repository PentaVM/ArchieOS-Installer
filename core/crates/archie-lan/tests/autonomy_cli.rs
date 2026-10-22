#[test]
fn once_mode_runs_autonomous_watch_and_persists_private_state_without_a_hardcoded_host() {
    let dir = std::env::temp_dir().join(format!("sentry-autonomy-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
        .args([
            "watch",
            "--autonomous",
            "--once",
            "--seconds",
            "1",
            "--mdns-backend",
            "native",
            "--out",
        ])
        .arg(dir.join("lan.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("lan.json")).unwrap()).unwrap();
    assert_eq!(inventory["format"], "archie-lan-inventory-v1");
    let health: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("lan.health.json")).unwrap()).unwrap();
    assert_eq!(health["consecutive_failures"], 0);
    assert!(dir.join("lan.history.json").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unsafe_path_collisions_are_rejected() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
        .args([
            "watch",
            "--autonomous",
            "--once",
            "--seconds",
            "1",
            "--out",
            "same.json",
            "--history",
            "same.json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("paths collide"));
    assert!(!std::path::Path::new("same.json").exists());
}

#[test]
fn failures_preserve_the_inventory_and_backoff_survives_restart() {
    let dir =
        std::env::temp_dir().join(format!("sentry-autonomy-failure-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("lan.json");
    std::fs::write(&out, b"last successful inventory").unwrap();
    let interfaces = if_addrs::get_if_addrs().unwrap();
    let address = ["10.254.254.254", "192.168.255.254"]
        .into_iter()
        .find(|raw| !interfaces.iter().any(|i| i.addr.ip().to_string() == *raw))
        .unwrap();
    for (failures, delay) in [(1, 30), (2, 60)] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
            .args([
                "watch",
                "--autonomous",
                "--once",
                "--seconds",
                "1",
                "--bind",
                address,
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(std::fs::read(&out).unwrap(), b"last successful inventory");
        let health: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("lan.health.json")).unwrap()).unwrap();
        assert_eq!(health["consecutive_failures"], failures);
        assert_eq!(health["next_retry_seconds"], delay);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupt_history_is_reported_without_overwriting_it() {
    let dir =
        std::env::temp_dir().join(format!("sentry-autonomy-corrupt-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lan.history.json"), b"corrupt history").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
        .args(["watch", "--autonomous", "--once", "--out"])
        .arg(dir.join("lan.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid history JSON"));
    assert_eq!(
        std::fs::read(dir.join("lan.history.json")).unwrap(),
        b"corrupt history"
    );
    assert!(!dir.join("lan.json").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn simultaneous_monitors_cannot_share_a_state_file() {
    use fs2::FileExt;
    let dir = std::env::temp_dir().join(format!("sentry-autonomy-lock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let lock = std::fs::File::create(dir.join("shared.history.json.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
        .args(["watch", "--autonomous", "--once", "--out"])
        .arg(dir.join("other.json"))
        .arg("--history")
        .arg(dir.join("shared.history.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("another monitor owns"));
    assert!(!dir.join("other.json").exists());
    drop(lock);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn publication_failure_records_failed_health_without_committing_successful_history() {
    let dir =
        std::env::temp_dir().join(format!("sentry-autonomy-publish-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("lan.json")).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
        .args([
            "watch",
            "--autonomous",
            "--once",
            "--seconds",
            "1",
            "--mdns-backend",
            "native",
            "--out",
        ])
        .arg(dir.join("lan.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let health: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("lan.health.json")).unwrap()).unwrap();
    assert_eq!(health["consecutive_failures"], 1);
    assert!(health["last_success_unix"].is_null());
    assert!(dir.join("lan.json").is_dir());
    std::fs::remove_dir_all(&dir).unwrap();
}
