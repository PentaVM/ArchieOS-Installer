//! CLI surface tests for the IoT commands: command mapping, offline behaviour
//! of `context-feed --once` against fixture directories, and local print
//! staging. Deterministic: no network, no daemon, no real devices.
use std::{fs, process::Command};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archie-cli-iot-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn archie(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn sentry_iot_commands_map_onto_the_reviewed_flow_and_daemon_queries() {
    // Keys and launches prepare intents: without a daemon they fail at connect
    // time with EX_UNAVAILABLE, never at parse time and never by bypassing.
    for args in [
        vec!["sentry", "key", "living-room-tv", "power_on"],
        vec!["sentry", "key", "living-room-tv", "home"],
        vec!["sentry", "launch", "living-room-tv", "12"],
        vec!["sentry", "printer-doctor", "office-printer"],
    ] {
        let output = archie(&args);
        assert_eq!(
            output.status.code(),
            Some(69),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // A key outside the contract vocabulary is refused by the CLI itself.
    let output = archie(&["sentry", "key", "living-room-tv", "volume_up"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
    // --pages enforces the contract's 1..=9999 bound at the CLI.
    let output = archie(&["sentry", "print", "office-printer", "doc.pdf", "--pages", "0"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn context_feed_once_ingests_fixtures_and_is_idempotent_across_runs() {
    let dir = temp_dir("feed");
    let watchdog = dir.join("watchdog");
    let out = dir.join("out");
    fs::create_dir_all(&watchdog).unwrap();
    fs::write(
        watchdog.join("capture-1.jsonl"),
        "{\"ip_src\":\"192.168.1.40\",\"udp_srcport\":5353,\"dns_ptr_names\":[\"Roku._roku-ecp._tcp.local\"],\"frame_time_epoch\":1790000000.5}\n\
         not-json-at-all\n\
         {\"ip_src\":\"192.168.1.60\",\"udp_srcport\":5353,\"dns_ptr_names\":[\"Printer._ipp._tcp.local\"]}\n",
    )
    .unwrap();
    let lan = dir.join("lan.json");
    fs::write(
        &lan,
        serde_json::json!({
            "format": "archie-lan-inventory-v1",
            "scanned_unix": 1_790_000_000u64,
            "devices": [{
                "address": "192.168.1.40",
                "class": "television",
                "name": "Living",
                "manufacturer": "Roku",
                "model": "3941X",
                "services": ["_roku-ecp._tcp"],
                "seen_by": ["mdns"],
                "state": {}
            }]
        })
        .to_string(),
    )
    .unwrap();
    let passive = dir.join("passive.json");
    fs::write(
        &passive,
        serde_json::json!({
            "source": "offline_mdns_metadata",
            "trusted": false,
            "assets": [{"ipv4_address": "192.168.1.3", "candidates": [
                {"class": "television", "score": 80, "supporting_signals": 1}
            ]}],
            "changes": []
        })
        .to_string(),
    )
    .unwrap();

    let args = vec![
        "--json".to_owned(),
        "context-feed".to_owned(),
        "--watchdog-dir".to_owned(),
        watchdog.to_string_lossy().into_owned(),
        "--out".to_owned(),
        out.to_string_lossy().into_owned(),
        "--lan".to_owned(),
        lan.to_string_lossy().into_owned(),
        "--passive".to_owned(),
        passive.to_string_lossy().into_owned(),
        "--once".to_owned(),
    ];
    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(&args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // 2 watchdog hosts + 1 lan device + 1 passive asset.
    assert_eq!(summary["events_written"], 4);
    assert_eq!(summary["malformed_lines"], 1);
    // The segment is one event per line, and every line parses back as an event.
    let segment = fs::read_to_string(out.join("segment-000000.jsonl")).unwrap();
    let lines: Vec<&str> = segment.lines().collect();
    assert_eq!(lines.len(), 4);
    for line in &lines {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(event["schema_version"], 1);
        assert!(event["event_id"].as_str().unwrap().len() == 64);
    }
    assert!(out.join("context-feed-state.json").is_file());

    // A second run re-reads the fixtures but writes nothing: replays dedupe.
    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(&args)
        .output()
        .unwrap();
    assert!(output.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["events_written"], 0);
    assert_eq!(fs::read_to_string(out.join("segment-000000.jsonl")).unwrap().lines().count(), 4);

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn context_feed_rejects_missing_directories_and_bad_poll_bounds() {
    let dir = temp_dir("feed-args");
    let out = dir.join("out");
    // No watchdog dir anywhere (a fresh root has no [sentry.context_feed]).
    let output = archie(&[
        "--root",
        dir.to_str().unwrap(),
        "context-feed",
        "--out",
        out.to_str().unwrap(),
        "--once",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("watchdog"));
    // Poll bound is enforced.
    let watchdog = dir.join("watchdog");
    fs::create_dir_all(&watchdog).unwrap();
    let output = archie(&[
        "context-feed",
        "--watchdog-dir",
        watchdog.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--poll-ms",
        "50",
        "--once",
    ]);
    assert!(!output.status.success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sentry_print_stages_locally_before_the_daemon_is_ever_contacted() {
    let dir = temp_dir("print");
    let spool = dir.join("spool");
    fs::create_dir_all(&spool).unwrap();
    let etc = dir.join("etc");
    fs::create_dir_all(&etc).unwrap();
    fs::write(
        etc.join("archie.toml"),
        format!(
            "[[sentry.printers]]\nresource_id='office-printer'\nendpoint='ipp://192.168.1.232/ipp/print'\nexpected_uuid='urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b'\nspool_dir='{}'\n",
            spool.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let document = dir.join("letter.pdf");
    fs::write(&document, b"%PDF-1.4 fixture").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(["--root"])
        .arg(&dir)
        .args(["sentry", "print", "office-printer"])
        .arg(&document)
        .args(["--pages", "2"])
        .output()
        .unwrap();
    // Staging succeeded; the daemon is down, so the reviewed flow stops at connect.
    assert_eq!(output.status.code(), Some(69));
    // Exactly one staged file, named by content hash, holding the exact bytes.
    let staged: Vec<_> = fs::read_dir(&spool)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(staged.len(), 1);
    let name = &staged[0];
    assert_eq!(name.len(), 64);
    assert!(name.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(
        fs::read(spool.join(name)).unwrap(),
        b"%PDF-1.4 fixture"
    );

    // A non-printable format is refused before anything is staged.
    let blob = dir.join("blob.bin");
    fs::write(&blob, [0u8, 1, 2, 3]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(["--root"])
        .arg(&dir)
        .args(["sentry", "print", "office-printer"])
        .arg(&blob)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not printable"));
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 1);

    // A resource with no spool_dir configured cannot print.
    let bare = temp_dir("print-bare");
    fs::create_dir_all(bare.join("etc")).unwrap();
    fs::write(
        bare.join("etc/archie.toml"),
        "[[sentry.printers]]\nresource_id='office-printer'\nendpoint='ipp://192.168.1.232/ipp/print'\nexpected_uuid='urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b'\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archie"))
        .args(["--root"])
        .arg(&bare)
        .args(["sentry", "print", "office-printer"])
        .arg(&document)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("spool_dir"));

    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(bare).unwrap();
}
