//! Node-control adapters: Wake-on-LAN and Android adb. All process interaction
//! goes through a FakeRunner that records argv and replays fixtures; the only
//! real I/O is the loopback UDP socket that captures the magic packet.
use archie_sentry::{
    adapters::{
        adb::{
            device_online, index_for_percent, locked_state, parse_foreground, parse_keyguard,
            parse_volume, parse_wakefulness, percent_for_index, Adb, AndroidTarget,
        },
        process::{CommandOutput, CommandRunner},
        wol::{magic_packet, parse_arp_windows, parse_ip_neigh, parse_mac, Wol, WolTarget},
    },
    contracts::*,
    engine::*,
    registry::*,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

// ---------------------------------------------------------------------------
// FakeRunner: records every argv, replays scripted replies in order.
// A call beyond the script gets status 1, which the adapters treat as failure -
// so an unexpected extra command can never silently pass a test.
// ---------------------------------------------------------------------------
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
    replies: Mutex<VecDeque<Result<CommandOutput, AdapterError>>>,
}
impl FakeRunner {
    fn with(replies: Vec<Result<CommandOutput, AdapterError>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(vec![]),
            replies: Mutex::new(replies.into()),
        })
    }
    fn ok(stdout: &str) -> Result<CommandOutput, AdapterError> {
        Ok(CommandOutput {
            status: 0,
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        })
    }
    fn fail(status: i32) -> Result<CommandOutput, AdapterError> {
        Ok(CommandOutput {
            status,
            stdout: vec![],
            stderr: b"tool failed".to_vec(),
        })
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}
impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], _timeout: Duration) -> Result<CommandOutput, AdapterError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Self::fail(1))
    }
}

fn adb_path() -> &'static str {
    if cfg!(windows) {
        "C:\\platform-tools\\adb.exe"
    } else {
        "/usr/bin/adb"
    }
}
fn arp_path() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32\\ARP.EXE"
    } else {
        "/usr/sbin/ip"
    }
}

const MAC: &str = "7c:10:c9:55:aa:01";
const IP: &str = "192.168.1.50";
const SERIAL: &str = "192.168.1.70:5555";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------
const ARP_WINDOWS: &str = "\r\nInterface: 192.168.1.10 --- 0x6\r\n\
  Internet Address      Physical Address      Type\r\n\
  192.168.1.1           a4-b1-c1-89-2e-01     dynamic\r\n\
  192.168.1.50          7c-10-c9-55-aa-01     dynamic\r\n\
  192.168.1.77          00-00-00-00-00-00     invalid\r\n\
  224.0.0.22            01-00-5e-00-00-16     static\r\n";

const IP_NEIGH: &str = "\
192.168.1.1 dev eth0 lladdr a4:b1:c1:89:2e:01 REACHABLE\n\
192.168.1.50 dev eth0 lladdr 7c:10:c9:55:aa:01 STALE\n\
192.168.1.60 dev eth0  INCOMPLETE\n\
192.168.1.70 dev eth0 FAILED\n";

const ADB_DEVICES_ONLINE: &str = "List of devices attached\n192.168.1.70:5555\tdevice\n\n";
const ADB_DEVICES_OTHER: &str = "List of devices attached\nRZ8N90ABCDE\tdevice\n\n";
const ADB_DEVICES_OFFLINE: &str = "List of devices attached\n192.168.1.70:5555\toffline\n\n";

const DUMPSYS_POWER_AWAKE: &str = "\
POWER MANAGER (dumpsys power)\n\
\n\
Power Manager State:\n\
  mDirty=0x0\n\
  mWakefulness=Awake\n\
  mWakefulnessChanging=false\n\
  mIsPowered=true\n";
const DUMPSYS_POWER_ASLEEP: &str = "\
POWER MANAGER (dumpsys power)\n\
\n\
Power Manager State:\n\
  mDirty=0x0\n\
  mWakefulness=Asleep\n\
  mIsPowered=true\n";

const DUMPSYS_WINDOW_UNLOCKED: &str = "\
WINDOW MANAGER POLICY STATE (dumpsys window)\n\
    mSafeMode=false mSystemReady=true mSystemBooted=true\n\
    mDreamingLockscreen=false mShowingDream=false\n\
\n\
WINDOW MANAGER WINDOWS (dumpsys window windows)\n\
  Window #3 Window{7a3b2c1 u0 com.example.notes/com.example.notes.MainActivity}:\n\
    mDisplayId=0 stackId=1 mSession=Session{abc}\n\
\n\
  mCurrentFocus=Window{7a3b2c1 u0 com.example.notes/com.example.notes.MainActivity}\n\
  mFocusedApp=null\n";
const DUMPSYS_WINDOW_LOCKED: &str = "\
WINDOW MANAGER POLICY STATE (dumpsys window)\n\
    mSafeMode=false mSystemReady=true mSystemBooted=true\n\
    mDreamingLockscreen=true mShowingDream=false\n\
\n\
WINDOW MANAGER WINDOWS (dumpsys window windows)\n\
  mCurrentFocus=Window{de4f5a6 u0 com.android.systemui/com.android.systemui.keyguard.KeyguardViewMediator}\n\
  mFocusedApp=null\n";

const DUMPSYS_AUDIO: &str = "\
AudioManagerService (dumpsys audio)\n\
\n\
Stream volumes (device: index)\n\
- STREAM_MUSIC:\n\
   Muted: false\n\
   Muted Internally: false\n\
   Min volume index: 0\n\
   Max volume index: 15\n\
   Current: 2 (speaker): 8, 16 (headphone): 10\n\
   Devices: speaker\n\
- STREAM_ALARM:\n\
   Muted: false\n\
   Min volume index: 0\n\
   Max volume index: 7\n\
   Current: 2 (speaker): 6\n\
   Devices: speaker\n";

// ---------------------------------------------------------------------------
// Shared builders
// ---------------------------------------------------------------------------
fn wol_resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "pc".into(),
        edge_id: "edge".into(),
        adapter_id: "wol".into(),
        paired: true,
        capabilities: vec![CommandKind::WakeHost],
    }
}
fn wol_adapter(broadcast: &str, runner: Arc<FakeRunner>) -> Wol {
    Wol::new(
        "edge",
        "wol",
        broadcast,
        arp_path(),
        BTreeMap::from([(
            "pc".into(),
            WolTarget {
                mac: MAC.into(),
                ip: IP.into(),
            },
        )]),
        runner,
    )
    .unwrap()
}
fn adb_resource(capabilities: Vec<CommandKind>) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "phone".into(),
        edge_id: "edge".into(),
        adapter_id: "adb".into(),
        paired: true,
        capabilities,
    }
}
fn adb_targets(network: bool) -> BTreeMap<String, AndroidTarget> {
    BTreeMap::from([(
        "phone".into(),
        AndroidTarget {
            serial: SERIAL.into(),
            network,
        },
    )])
}
fn adb_adapter(runner: Arc<FakeRunner>, network: bool) -> Adb {
    Adb::new("edge", "adb", adb_path(), adb_targets(network), runner).unwrap()
}

// ---------------------------------------------------------------------------
// WoL: magic packet and MAC validation
// ---------------------------------------------------------------------------
#[test]
fn magic_packet_is_sync_stream_plus_sixteen_macs() {
    let packet = magic_packet([0x7c, 0x10, 0xc9, 0x55, 0xaa, 0x01]);
    assert_eq!(packet.len(), 102);
    assert!(packet[..6].iter().all(|b| *b == 0xFF));
    for repeat in 0..16 {
        assert_eq!(
            &packet[6 + repeat * 6..6 + (repeat + 1) * 6],
            &[0x7c, 0x10, 0xc9, 0x55, 0xaa, 0x01],
            "repeat {repeat} must carry the target MAC"
        );
    }
}

#[test]
fn mac_must_be_lowercase_hex_with_colons() {
    assert_eq!(parse_mac(MAC), Some([0x7c, 0x10, 0xc9, 0x55, 0xaa, 0x01]));
    for bad in [
        "7C:10:C9:55:AA:01",    // uppercase
        "7c-10-c9-55-aa-01",    // hyphens
        "7c:10:c9:55:aa",       // too short
        "7c:10:c9:55:aa:01:02", // too long
        "7c:10:c9:55:aa:gg",    // not hex
        "7c:10:c9:55:aa:1",     // single digit
        "7c:10:c9:55:aa:01 ",   // trailing space
    ] {
        assert_eq!(parse_mac(bad), None, "{bad} must be rejected");
    }
}

#[test]
fn wol_configuration_is_validated() {
    let runner = FakeRunner::with(vec![]);
    let target = || WolTarget {
        mac: MAC.into(),
        ip: IP.into(),
    };
    let one = |t: WolTarget| BTreeMap::from([("pc".into(), t)]);
    // Bad MAC, bad IP, relative tool path, empty targets, bad broadcast.
    assert!(Wol::new(
        "edge",
        "wol",
        "192.168.1.255",
        arp_path(),
        one({
            let mut t = target();
            t.mac = "7C-10-C9-55-AA-01".into();
            t
        }),
        runner.clone()
    )
    .is_err());
    assert!(Wol::new(
        "edge",
        "wol",
        "192.168.1.255",
        arp_path(),
        one({
            let mut t = target();
            t.ip = "not-an-ip".into();
            t
        }),
        runner.clone()
    )
    .is_err());
    assert!(Wol::new(
        "edge",
        "wol",
        "192.168.1.255",
        "arp",
        one(target()),
        runner.clone()
    )
    .is_err());
    assert!(Wol::new(
        "edge",
        "wol",
        "192.168.1.255",
        arp_path(),
        BTreeMap::new(),
        runner.clone()
    )
    .is_err());
    assert!(Wol::new(
        "edge",
        "wol",
        "example.com",
        arp_path(),
        one(target()),
        runner.clone()
    )
    .is_err());
    assert!(Wol::new(
        "edge",
        "wol",
        "192.168.1.255:0",
        arp_path(),
        one(target()),
        runner
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// WoL: neighbor-cache parsing (both OS formats)
// ---------------------------------------------------------------------------
#[test]
fn windows_arp_parsing() {
    let ip = |s: &str| s.parse().unwrap();
    assert!(parse_arp_windows(ARP_WINDOWS, &ip("192.168.1.50"))); // dynamic
    assert!(parse_arp_windows(ARP_WINDOWS, &ip("192.168.1.1"))); // dynamic
                                                                 // An "invalid" row is a failed resolution, not a reachable host.
    assert!(!parse_arp_windows(ARP_WINDOWS, &ip("192.168.1.77")));
    // Multicast/static entries for other IPs are not evidence for our host.
    assert!(!parse_arp_windows(ARP_WINDOWS, &ip("192.168.1.99")));
    assert!(!parse_arp_windows("", &ip("192.168.1.50")));
}

#[test]
fn linux_neigh_parsing() {
    let ip = |s: &str| s.parse().unwrap();
    assert!(parse_ip_neigh(IP_NEIGH, &ip("192.168.1.1"))); // REACHABLE
    assert!(parse_ip_neigh(IP_NEIGH, &ip("192.168.1.50"))); // STALE still resolved
    assert!(!parse_ip_neigh(IP_NEIGH, &ip("192.168.1.60"))); // INCOMPLETE
    assert!(!parse_ip_neigh(IP_NEIGH, &ip("192.168.1.70"))); // FAILED
    assert!(!parse_ip_neigh(IP_NEIGH, &ip("192.168.1.99"))); // absent
}

// ---------------------------------------------------------------------------
// WoL: apply sends the packet; observe reads the cache
// ---------------------------------------------------------------------------
#[tokio::test]
async fn apply_broadcasts_the_exact_magic_packet() {
    let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = receiver.local_addr().unwrap();
    let runner = FakeRunner::with(vec![]); // apply must not run any tool
    let mut adapter = wol_adapter(&address.to_string(), runner.clone());
    let ack = adapter
        .apply(&wol_resource(), &Command::WakeHost)
        .await
        .unwrap();
    assert!(ack.accepted);
    let mut buffer = [0u8; 128];
    let (size, _) = tokio::time::timeout(Duration::from_secs(2), receiver.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(size, 102);
    assert_eq!(
        &buffer[..size],
        &magic_packet([0x7c, 0x10, 0xc9, 0x55, 0xaa, 0x01])
    );
    assert!(runner.calls().is_empty(), "apply must be pure UDP");
}

#[tokio::test]
async fn wol_rejects_everything_but_wake_host() {
    let runner = FakeRunner::with(vec![]);
    let mut adapter = wol_adapter("192.168.1.255", runner.clone());
    // Not declared in the resource's capabilities.
    let volume = Command::VolumeSet { percent: 50 };
    assert_eq!(
        adapter.apply(&wol_resource(), &volume).await.unwrap_err(),
        AdapterError::Unsupported
    );
    // Declared but not WoL's command.
    let mut resource = wol_resource();
    resource.capabilities.push(CommandKind::VolumeSet);
    assert_eq!(
        adapter.apply(&resource, &volume).await.unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty());
}

#[tokio::test]
async fn wol_observe_reports_reachability_from_the_neighbor_cache() {
    let (fixture, expected_tail): (&str, &[&str]) = if cfg!(windows) {
        (ARP_WINDOWS, &["-a"])
    } else {
        (IP_NEIGH, &["neigh", "show"])
    };
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(fixture),
        FakeRunner::ok(""), // empty cache: nobody answers
        FakeRunner::fail(1),
    ]);
    let mut adapter = wol_adapter("192.168.1.255", runner.clone());

    let observation = adapter.observe(&wol_resource()).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Host {
            reachable: true,
            awake: None,
            session_locked: None,
            volume_percent: None,
            foreground_app: None,
        }
    );
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert!(observation.evidence_id.starts_with("wol-"));
    assert!(Command::WakeHost.matches(&observation.state));

    let unreachable = adapter.observe(&wol_resource()).await.unwrap();
    match unreachable.state {
        ResourceState::Host { reachable, .. } => assert!(!reachable),
        other => panic!("expected Host, got {other:?}"),
    }

    // A failing cache tool makes the observation Unavailable, never guessed.
    assert_eq!(
        adapter.observe(&wol_resource()).await.unwrap_err(),
        AdapterError::Unavailable
    );

    // The tool argv is the configured absolute path plus the OS's listing verb.
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0][0], arp_path());
    assert_eq!(&calls[0][1..], expected_tail);
}

// ---------------------------------------------------------------------------
// adb: argv construction and fixtures parsing
// ---------------------------------------------------------------------------
fn expect_device_calls(calls: &[Vec<String>], network: bool) -> usize {
    let mut index = 0;
    if network {
        assert_eq!(
            calls[0],
            vec![adb_path().to_string(), "connect".into(), SERIAL.into()]
        );
        index = 1;
    }
    assert_eq!(calls[index], vec![adb_path().to_string(), "devices".into()]);
    index + 1
}

#[tokio::test]
async fn app_launch_invokes_monkey_with_exact_argv() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok("  arg: \"-p\"\nEvents injected: 1\r\n"),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![CommandKind::AppLaunch]);
    let ack = adapter
        .apply(
            &resource,
            &Command::AppLaunch {
                app_id: "com.example.notes".into(),
            },
        )
        .await
        .unwrap();
    assert!(ack.accepted);
    let calls = runner.calls();
    assert_eq!(calls.len(), 2);
    expect_device_calls(&calls, false);
    assert_eq!(
        calls[1],
        vec![
            adb_path().to_string(),
            "-s".into(),
            SERIAL.into(),
            "shell".into(),
            "monkey".into(),
            "-p".into(),
            "com.example.notes".into(),
            "-c".into(),
            "android.intent.category.LAUNCHER".into(),
            "1".into(),
        ]
    );
}

#[tokio::test]
async fn app_launch_with_no_activity_is_a_dispatch_failure() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok("** No activities found to run, monkey aborted."),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![CommandKind::AppLaunch]);
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "com.example.empty".into(),
                },
            )
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(runner.calls().len(), 2);
}

#[tokio::test]
async fn network_target_is_connected_before_acting() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok("connected to 192.168.1.70:5555\n"),
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok("Events injected: 1\r\n"),
    ]);
    let mut adapter = adb_adapter(runner.clone(), true);
    let resource = adb_resource(vec![CommandKind::AppLaunch]);
    adapter
        .apply(
            &resource,
            &Command::AppLaunch {
                app_id: "com.example.notes".into(),
            },
        )
        .await
        .unwrap();
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    expect_device_calls(&calls, true);

    // A refused wireless connection stops everything after `adb connect`.
    let refused = FakeRunner::with(vec![FakeRunner::ok(
        "failed to connect to 192.168.1.70:5555\n",
    )]);
    let mut adapter = adb_adapter(refused.clone(), true);
    assert_eq!(
        adapter
            .apply(
                &adb_resource(vec![CommandKind::AppLaunch]),
                &Command::AppLaunch {
                    app_id: "com.example.notes".into(),
                },
            )
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(refused.calls().len(), 1);
}

#[tokio::test]
async fn device_not_present_is_unavailable_without_acting() {
    // The serial is missing entirely...
    let runner = FakeRunner::with(vec![FakeRunner::ok(ADB_DEVICES_OTHER)]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![CommandKind::AppLaunch, CommandKind::VolumeSet]);
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "com.example.notes".into(),
                },
            )
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(
        runner.calls(),
        vec![vec![adb_path().to_string(), "devices".into()]],
        "nothing may run after a missing device"
    );

    // ...or present but offline: still not acted upon.
    let runner = FakeRunner::with(vec![FakeRunner::ok(ADB_DEVICES_OFFLINE)]);
    let mut adapter = adb_adapter(runner.clone(), false);
    assert_eq!(
        adapter
            .apply(&resource, &Command::VolumeSet { percent: 30 })
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(runner.calls().len(), 1);
}

#[tokio::test]
async fn observe_measures_awake_unlocked_foreground_and_volume() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok(DUMPSYS_POWER_AWAKE),
        FakeRunner::ok(DUMPSYS_WINDOW_UNLOCKED),
        FakeRunner::ok(DUMPSYS_AUDIO),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![
        CommandKind::AppLaunch,
        CommandKind::SessionLock,
        CommandKind::VolumeSet,
    ]);
    let observation = adapter.observe(&resource).await.unwrap();
    let state = ResourceState::Host {
        reachable: true,
        awake: Some(true),
        session_locked: Some(false),
        volume_percent: Some(53), // index 8 of max 15
        foreground_app: Some("com.example.notes".into()),
    };
    assert_eq!(observation.state, state);
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert!(observation.evidence_id.starts_with("adb-"));
    // The measured state must satisfy the commands' postconditions.
    assert!(Command::AppLaunch {
        app_id: "com.example.notes".into()
    }
    .matches(&observation.state));
    assert!(Command::SessionLock { locked: false }.matches(&observation.state));
    assert!(Command::VolumeSet { percent: 53 }.matches(&observation.state));

    // devices, then the three dumpsys calls in order.
    let calls = runner.calls();
    assert_eq!(calls.len(), 4);
    expect_device_calls(&calls, false);
    for (call, dump) in calls[1..].iter().zip(["power", "window", "audio"]) {
        assert_eq!(
            call,
            &vec![
                adb_path().to_string(),
                "-s".into(),
                SERIAL.into(),
                "shell".into(),
                "dumpsys".into(),
                dump.into(),
            ]
        );
    }
}

// ---------------------------------------------------------------------------
// adb: parsers
// ---------------------------------------------------------------------------
#[test]
fn wakefulness_parsing() {
    assert_eq!(parse_wakefulness(DUMPSYS_POWER_AWAKE), Some(true));
    assert_eq!(parse_wakefulness(DUMPSYS_POWER_ASLEEP), Some(false));
    assert_eq!(
        parse_wakefulness("  mWakefulness=Dozing\n"),
        Some(false),
        "dozing is not usable-awake"
    );
    assert_eq!(parse_wakefulness("  mWakefulness=Dreaming\n"), Some(false));
    assert_eq!(parse_wakefulness("no such field here"), None);
    assert_eq!(parse_wakefulness("  mWakefulness=Bewildered\n"), None);
}

#[test]
fn keyguard_and_locked_state_both_directions() {
    assert_eq!(parse_keyguard(DUMPSYS_WINDOW_LOCKED), Some(true));
    assert_eq!(parse_keyguard(DUMPSYS_WINDOW_UNLOCKED), Some(false));
    assert_eq!(parse_keyguard("no markers"), None);
    // The module's locked definition, both ways.
    assert_eq!(locked_state(Some(true), Some(false)), Some(false)); // usable
    assert_eq!(locked_state(Some(true), Some(true)), Some(true)); // keyguard
    assert_eq!(locked_state(Some(false), Some(false)), Some(true)); // asleep
    assert_eq!(locked_state(Some(false), None), Some(true)); // asleep
    assert_eq!(locked_state(Some(true), None), None); // cannot say
    assert_eq!(locked_state(None, None), None);
}

#[test]
fn foreground_parsing_from_realistic_dumpsys_window() {
    assert_eq!(
        parse_foreground(DUMPSYS_WINDOW_UNLOCKED),
        Some("com.example.notes".to_string())
    );
    assert_eq!(
        parse_foreground(DUMPSYS_WINDOW_LOCKED),
        Some("com.android.systemui".to_string())
    );
    // A null focus (sleeping / transient) is "no foreground app".
    assert_eq!(parse_foreground("  mCurrentFocus=null\n"), None);
    // mFocusedApp is the fallback when mCurrentFocus is absent.
    assert_eq!(
        parse_foreground(
            "  mFocusedApp=ActivityRecord{a1b2c3 u0 com.example.player/.ui.MainActivity t42}\n"
        ),
        Some("com.example.player".to_string())
    );
    assert_eq!(parse_foreground("no focus lines at all"), None);
}

#[test]
fn volume_index_mapping_and_round_trip() {
    // Parsing the fixture: current 8 of max 15 from the STREAM_MUSIC block.
    assert_eq!(parse_volume(DUMPSYS_AUDIO), Some((8, 15)));
    // A section-less dump, or a partial one, is unknown - never guessed.
    assert_eq!(parse_volume("AudioManagerService\n"), None);
    assert_eq!(
        parse_volume("- STREAM_MUSIC:\n   Max volume index: 15\n"),
        None
    );
    // Older key style is also accepted.
    assert_eq!(
        parse_volume("- STREAM_MUSIC:\n   streamVolumeMax:25\n   streamVolume:5\n"),
        Some((5, 25))
    );

    // Percent -> index -> percent round trip.
    assert_eq!(index_for_percent(0, 15), 0);
    assert_eq!(index_for_percent(50, 15), 8); // 7.5 rounds to nearest
    assert_eq!(index_for_percent(100, 15), 15);
    assert_eq!(percent_for_index(0, 15), Some(0));
    assert_eq!(percent_for_index(8, 15), Some(53));
    assert_eq!(percent_for_index(15, 15), Some(100));
    for percent in [0u8, 7, 25, 50, 75, 100] {
        let index = index_for_percent(percent, 15);
        assert!(index <= 15);
        let back = percent_for_index(index, 15).unwrap();
        assert!(
            back.abs_diff(percent) <= 4,
            "percent {percent} -> index {index} -> percent {back} drifted"
        );
    }
    // A fixed-silent stream (max 0) has exactly one state: 0%.
    assert_eq!(percent_for_index(0, 0), Some(0));
    assert_eq!(percent_for_index(1, 0), None);
}

// ---------------------------------------------------------------------------
// adb: commands
// ---------------------------------------------------------------------------
#[tokio::test]
async fn volume_set_maps_percent_onto_the_parsed_stream_range() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok(DUMPSYS_AUDIO),
        FakeRunner::ok(""),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![CommandKind::VolumeSet]);
    let ack = adapter
        .apply(&resource, &Command::VolumeSet { percent: 50 })
        .await
        .unwrap();
    assert!(ack.accepted);
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    expect_device_calls(&calls, false);
    assert_eq!(
        calls[2],
        vec![
            adb_path().to_string(),
            "-s".into(),
            SERIAL.into(),
            "shell".into(),
            "cmd".into(),
            "media_session".into(),
            "volume".into(),
            "--stream".into(),
            "3".into(),
            "--set".into(),
            "8".into(), // 50% of max index 15
        ]
    );

    // When the range cannot be parsed, nothing is set and no index is guessed.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok("AudioManagerService: unexpected format\n"),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    assert_eq!(
        adapter
            .apply(&resource, &Command::VolumeSet { percent: 50 })
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(runner.calls().len(), 2, "no volume command may be sent");
}

#[tokio::test]
async fn session_lock_only_toggles_when_the_state_differs() {
    // Already locked (asleep + keyguard): the toggle must NOT be sent.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok(DUMPSYS_POWER_ASLEEP),
        FakeRunner::ok(DUMPSYS_WINDOW_LOCKED),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![CommandKind::SessionLock]);
    let ack = adapter
        .apply(&resource, &Command::SessionLock { locked: true })
        .await
        .unwrap();
    assert!(ack.accepted);
    assert_eq!(
        runner.calls().len(),
        3,
        "devices + power + window, and no keyevent"
    );

    // Awake and unlocked, lock requested: exactly one KEYCODE_POWER.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok(DUMPSYS_POWER_AWAKE),
        FakeRunner::ok(DUMPSYS_WINDOW_UNLOCKED),
        FakeRunner::ok(""),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    adapter
        .apply(&resource, &Command::SessionLock { locked: true })
        .await
        .unwrap();
    let calls = runner.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls[3],
        vec![
            adb_path().to_string(),
            "-s".into(),
            SERIAL.into(),
            "shell".into(),
            "input".into(),
            "keyevent".into(),
            "KEYCODE_POWER".into(),
        ]
    );

    // Unmeasurable state: a toggle could go the wrong way, so it is refused.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(ADB_DEVICES_ONLINE),
        FakeRunner::ok("unrecognized power dump"),
        FakeRunner::ok("unrecognized window dump"),
    ]);
    let mut adapter = adb_adapter(runner.clone(), false);
    assert_eq!(
        adapter
            .apply(&resource, &Command::SessionLock { locked: false })
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(runner.calls().len(), 3, "no blind toggle");
}

#[tokio::test]
async fn unsupported_commands_are_refused_without_acting() {
    let runner = FakeRunner::with(vec![]);
    let mut adapter = adb_adapter(runner.clone(), false);
    let resource = adb_resource(vec![
        CommandKind::MediaPause,
        CommandKind::MediaPlay,
        CommandKind::WakeHost,
        CommandKind::SwitchSet,
        CommandKind::AppLaunch,
    ]);
    for command in [
        Command::MediaPause,
        Command::MediaPlay,
        Command::WakeHost,
        Command::SwitchSet { on: true },
    ] {
        assert_eq!(
            adapter.apply(&resource, &command).await.unwrap_err(),
            AdapterError::Unsupported,
            "{command:?} must be refused"
        );
    }
    // Even an unsupported command on an unpaired resource is refused.
    let mut unpaired = adb_resource(vec![CommandKind::AppLaunch]);
    unpaired.paired = false;
    assert_eq!(
        adapter
            .apply(
                &unpaired,
                &Command::AppLaunch {
                    app_id: "com.example.notes".into(),
                },
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty(), "refusals never spawn adb");
}

#[test]
fn adb_devices_and_configuration_are_validated() {
    assert!(device_online(ADB_DEVICES_ONLINE, SERIAL));
    assert!(!device_online(ADB_DEVICES_OFFLINE, SERIAL));
    assert!(!device_online(ADB_DEVICES_OTHER, SERIAL));
    assert!(!device_online("daemon not running; starting now\n", SERIAL));

    let runner = FakeRunner::with(vec![]);
    // Relative tool path, bad serial, network serial that is not ip:port.
    assert!(Adb::new("edge", "adb", "adb", adb_targets(false), runner.clone()).is_err());
    assert!(Adb::new(
        "edge",
        "adb",
        adb_path(),
        BTreeMap::from([(
            "phone".into(),
            AndroidTarget {
                serial: "bad serial;rm -rf".into(),
                network: false,
            }
        )]),
        runner.clone(),
    )
    .is_err());
    assert!(Adb::new(
        "edge",
        "adb",
        adb_path(),
        BTreeMap::from([(
            "phone".into(),
            AndroidTarget {
                serial: "not-an-ip:5555".into(),
                network: true,
            }
        )]),
        runner.clone(),
    )
    .is_err());
}

/// A tool that hangs: it answers only when its own timeout would have killed it.
struct StalledRunner;
impl CommandRunner for StalledRunner {
    fn run(&self, _argv: &[String], timeout: Duration) -> Result<CommandOutput, AdapterError> {
        std::thread::sleep(timeout.min(Duration::from_secs(3)));
        Err(AdapterError::Unavailable)
    }
}

#[tokio::test]
async fn a_stalled_tool_does_not_hold_the_engine_past_its_deadline() {
    // adb's observe is several calls of seconds each. Run inline on the async thread,
    // a deadline around it could only fire once all of them had finished.
    let mut adapter = Adb::new(
        "edge",
        "adb",
        adb_path(),
        adb_targets(true),
        Arc::new(StalledRunner),
    )
    .unwrap();
    let resource = adb_resource(vec![CommandKind::SessionLock]);
    let started = std::time::Instant::now();
    let outcome =
        tokio::time::timeout(Duration::from_millis(200), adapter.observe(&resource)).await;
    assert!(outcome.is_err(), "the deadline fires");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "and fires on time: {:?}",
        started.elapsed()
    );
}
