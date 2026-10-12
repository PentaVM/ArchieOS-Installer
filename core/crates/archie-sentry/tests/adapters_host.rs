//! Host-over-SSH adapter tests: a FakeRunner (CommandRunner) records exact
//! argv and answers by scripted argv-suffix match, like the router tests.
//! Every remote output here is a recorded fixture shape, not a live host.
use archie_sentry::{
    adapters::{
        host_ssh::{
            base64_encode, nircmd_volume, parse_amixer_volume, parse_locked_hint,
            parse_osascript_volume, parse_pactl_volume, parse_process_name, parse_ps_bool,
            parse_ps_volume, parse_session_ids, probe_host, ps_audio_get, ps_audio_set,
            ps_encoded_command, ps_foreground, ps_lock_state, HostCapabilities, HostOs, HostSsh,
            HostTarget, LockSupport, VolumeTool,
        },
        process::{CommandOutput, CommandRunner},
        router::ssh::SshConfig,
    },
    contracts::*,
    engine::*,
    registry::ResourceDescriptor,
};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::Duration,
};

// ---------- FakeRunner: scripted by exact argv-suffix match ----------

struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
    scripted: Vec<(Vec<String>, i32, Vec<u8>)>,
}
impl FakeRunner {
    fn new(scripted: Vec<(&[&str], i32, &str)>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(vec![]),
            scripted: scripted
                .into_iter()
                .map(|(tail, status, stdout)| {
                    (
                        tail.iter().map(|s| s.to_string()).collect(),
                        status,
                        stdout.as_bytes().to_vec(),
                    )
                })
                .collect(),
        })
    }
    fn script(tails: &[&[&str]]) -> Arc<Self> {
        Self::new(tails.iter().map(|t| (*t, 0, "ok")).collect())
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
    /// Tails of every recorded call (the remote argv, after user@host).
    fn remote_tails(&self) -> Vec<Vec<String>> {
        self.calls()
            .iter()
            .map(|argv| {
                let at = argv.iter().position(|a| a.contains('@')).unwrap();
                argv[at + 1..].to_vec()
            })
            .collect()
    }
}
impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], _timeout: Duration) -> Result<CommandOutput, AdapterError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        for (tail, status, stdout) in &self.scripted {
            if argv.len() >= tail.len() && argv[argv.len() - tail.len()..] == tail[..] {
                return Ok(CommandOutput {
                    status: *status,
                    stdout: stdout.clone(),
                    stderr: vec![],
                });
            }
        }
        // Unscripted: like a host without the tool. No command may ever
        // silently succeed in a test.
        Ok(CommandOutput {
            status: 255,
            stdout: vec![],
            stderr: b"command not found".to_vec(),
        })
    }
}

fn ssh_path() -> &'static str {
    if cfg!(windows) {
        "C:/tools/ssh.exe"
    } else {
        "/usr/bin/ssh"
    }
}
// `Path::is_absolute` is per-platform: `C:/…` is relative on Unix.
fn key_path() -> &'static str {
    if cfg!(windows) {
        "C:/keys/host_ed25519"
    } else {
        "/etc/archie/keys/host_ed25519"
    }
}
fn ssh_config() -> SshConfig {
    SshConfig::new(
        ssh_path(),
        "archie",
        Ipv4Addr::new(192, 168, 1, 40),
        Some(key_path().into()),
    )
    .unwrap()
}
fn mk_resource(caps: &[CommandKind]) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "workstation".into(),
        edge_id: "edge".into(),
        adapter_id: "host-ssh".into(),
        paired: true,
        capabilities: caps.to_vec(),
    }
}
fn target(allow_power: bool) -> HostTarget {
    HostTarget {
        ssh: ssh_config(),
        apps: BTreeMap::from([
            ("notes".into(), "C:\\Tools\\notes.exe".into()),
            ("firefox".into(), "/usr/bin/firefox".into()),
            ("Firefox".into(), "Firefox".into()),
        ]),
        allow_power,
    }
}
fn windows_caps() -> HostCapabilities {
    HostCapabilities {
        ssh: true,
        os: Some(HostOs::Windows),
        lock: LockSupport::LockOnly,
        volume: Some(VolumeTool::Nircmd),
        foreground: true,
        app_launch: true,
        power: true,
    }
}
fn linux_caps() -> HostCapabilities {
    HostCapabilities {
        ssh: true,
        os: Some(HostOs::Linux),
        lock: LockSupport::LockUnlock,
        volume: Some(VolumeTool::Pactl),
        foreground: true,
        app_launch: true,
        power: true,
    }
}
fn macos_caps() -> HostCapabilities {
    HostCapabilities {
        ssh: true,
        os: Some(HostOs::MacOs),
        lock: LockSupport::LockOnly,
        volume: Some(VolumeTool::Osascript),
        foreground: true,
        app_launch: true,
        power: true,
    }
}
fn mk_adapter(caps: HostCapabilities, runner: Arc<FakeRunner>, allow_power: bool) -> HostSsh {
    HostSsh::new(
        "edge",
        "host-ssh",
        "workstation",
        target(allow_power),
        caps,
        runner,
    )
    .unwrap()
}

// ---------- fixtures (recorded shapes, labeled as fixtures) ----------

const WINDOWS_VER: &str = "\r\nMicrosoft Windows [Version 10.0.22631.4460]\r\n";
const LOGINCTL_LIST: &str = "   3 1000 archie seat0 tty2\n   7 1000 archie  pts/4\n";
const PACTL_VOLUME: &str =
    "Volume: front-left: 39321 /  60% / -15.94 dB,   front-right: 39321 /  60% / -15.94 dB\n\
     balance 0.00\n";
const AMIXER_VOLUME: &str = "Simple mixer control 'Master',0\n\
     Capabilities: pvolume pswitch\n\
     Playback channels: Front Left - Front Right\n\
     Limits: Playback 0 - 65536\n\
     Front Left: Playback 39322 [60%] [on]\n\
     Front Right: Playback 39322 [60%] [on]\n";
const OSASCRIPT_VOLUME: &str =
    "output volume:35, input volume:74, alert volume:100, output muted:false\n";

// ---------- parsers ----------

#[test]
fn base64_and_encoded_command_are_exact() {
    assert_eq!(base64_encode(b"H"), "SA==");
    assert_eq!(base64_encode(b"He"), "SGU=");
    assert_eq!(base64_encode(b"Hel"), "SGVs");
    assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
    assert_eq!(base64_encode(b""), "");
    // "A" in UTF-16LE is 0x41 0x00.
    assert_eq!(ps_encoded_command("A"), "QQA=");
    // The encoded payload never needs shell quoting.
    let encoded = ps_encoded_command("Add-Type -TypeDefinition 'x \"y\"';[A]::B(1)");
    assert!(encoded
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='));
}

#[test]
fn powershell_parsers() {
    assert_eq!(parse_ps_bool("True\r\n"), Some(true));
    assert_eq!(parse_ps_bool("False\n"), Some(false));
    assert_eq!(parse_ps_bool("true"), None);
    assert_eq!(parse_ps_bool(""), None);
    assert_eq!(parse_ps_volume("53\r\n"), Some(53));
    assert_eq!(parse_ps_volume("0"), Some(0));
    assert_eq!(parse_ps_volume("100"), Some(100));
    assert_eq!(
        parse_ps_volume("121"),
        None,
        "over-amplification is unknown, not clamped"
    );
    assert_eq!(parse_ps_volume("loud"), None);
}

#[test]
fn linux_parsers() {
    assert_eq!(parse_session_ids(LOGINCTL_LIST), vec!["3", "7"]);
    assert_eq!(parse_session_ids(""), Vec::<String>::new());
    assert_eq!(parse_locked_hint("LockedHint=yes\n"), Some(true));
    assert_eq!(parse_locked_hint("LockedHint=no\n"), Some(false));
    assert_eq!(parse_locked_hint("LockedHint=maybe\n"), None);
    assert_eq!(parse_pactl_volume(PACTL_VOLUME), Some(60));
    assert_eq!(
        parse_pactl_volume("Volume: mono: 65536 / 100%\n"),
        Some(100)
    );
    assert_eq!(parse_pactl_volume("Volume: mono: 98304 / 150%\n"), None);
    assert_eq!(parse_pactl_volume("no volume here"), None);
    assert_eq!(parse_amixer_volume(AMIXER_VOLUME), Some(60));
    assert_eq!(
        parse_amixer_volume("Front Left: Playback 0 [0%] [off]\n"),
        Some(0)
    );
    assert_eq!(parse_amixer_volume("junk [x%]"), None);
}

#[test]
fn macos_and_name_parsers() {
    assert_eq!(parse_osascript_volume(OSASCRIPT_VOLUME), Some(35));
    assert_eq!(
        parse_osascript_volume("output volume:0, output muted:true"),
        Some(0)
    );
    assert_eq!(
        parse_osascript_volume("output volume:missing value, input volume:74"),
        None
    );
    assert_eq!(parse_osascript_volume("no fields"), None);
    assert_eq!(parse_process_name("firefox\n"), Some("firefox".to_string()));
    assert_eq!(
        parse_process_name("loginwindow\n"),
        Some("loginwindow".to_string())
    );
    assert_eq!(
        parse_process_name("Visual Studio Code\n"),
        None,
        "spaces are not a contract id"
    );
    assert_eq!(parse_process_name("\n"), None);
}

#[test]
fn nircmd_scale() {
    assert_eq!(nircmd_volume(0), 0);
    assert_eq!(nircmd_volume(50), 32768);
    assert_eq!(nircmd_volume(100), 65535);
}

// ---------- probe ----------

#[tokio::test]
async fn probe_windows_full() {
    let runner = FakeRunner::new(vec![
        (&["cmd.exe", "/c", "ver"], 0, WINDOWS_VER),
        (
            &["where.exe", "rundll32.exe"],
            0,
            "C:\\Windows\\System32\\rundll32.exe\r\n",
        ),
        (
            &["where.exe", "powershell.exe"],
            0,
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\r\n",
        ),
        (&["where.exe", "nircmd.exe"], 0, "C:\\Tools\\nircmd.exe\r\n"),
        (
            &["where.exe", "shutdown.exe"],
            0,
            "C:\\Windows\\System32\\shutdown.exe\r\n",
        ),
    ]);
    let caps = probe_host(ssh_config(), runner.clone()).await;
    assert_eq!(caps, windows_caps());
    // uname is tried first and fails; no command beyond the probe ran.
    let tails = runner.remote_tails();
    assert_eq!(tails[0], vec!["uname", "-s"]);
    assert_eq!(tails[1], vec!["cmd.exe", "/c", "ver"]);
    assert_eq!(tails.len(), 6);
}

#[tokio::test]
async fn probe_windows_without_powershell_offers_nothing_measurable() {
    let runner = FakeRunner::new(vec![
        (&["cmd.exe", "/c", "ver"], 0, WINDOWS_VER),
        (&["where.exe", "rundll32.exe"], 0, "found"),
        (&["where.exe", "shutdown.exe"], 0, "found"),
    ]);
    let caps = probe_host(ssh_config(), runner).await;
    assert_eq!(caps.os, Some(HostOs::Windows));
    assert_eq!(
        caps.lock,
        LockSupport::None,
        "lock without its read-back is not offered"
    );
    assert_eq!(caps.volume, None);
    assert!(!caps.foreground);
    assert!(!caps.app_launch);
    assert!(caps.power);
}

#[tokio::test]
async fn probe_linux_full() {
    let runner = FakeRunner::new(vec![
        (&["uname", "-s"], 0, "Linux\n"),
        (&["command", "-v", "loginctl"], 0, "/usr/bin/loginctl\n"),
        (&["command", "-v", "pactl"], 0, "/usr/bin/pactl\n"),
        (
            &["command", "-v", "systemd-run"],
            0,
            "/usr/bin/systemd-run\n",
        ),
        (&["command", "-v", "xdotool"], 0, "/usr/bin/xdotool\n"),
        (&["command", "-v", "shutdown"], 0, "/usr/bin/shutdown\n"),
    ]);
    let caps = probe_host(ssh_config(), runner).await;
    assert_eq!(caps, linux_caps());
    assert_eq!(
        caps.command_kinds(),
        vec![
            CommandKind::SessionLock,
            CommandKind::VolumeSet,
            CommandKind::AppLaunch
        ]
    );
}

#[tokio::test]
async fn probe_linux_amixer_fallback_and_no_foreground() {
    let runner = FakeRunner::new(vec![
        (&["uname", "-s"], 0, "Linux\n"),
        (&["command", "-v", "loginctl"], 0, "ok"),
        (&["command", "-v", "amixer"], 0, "ok"),
        (&["command", "-v", "systemd-run"], 0, "ok"),
    ]);
    let caps = probe_host(ssh_config(), runner).await;
    assert_eq!(caps.volume, Some(VolumeTool::Amixer));
    assert!(!caps.foreground);
    assert!(
        !caps.app_launch,
        "launch without a foreground read-back is not offered"
    );
    assert_eq!(caps.lock, LockSupport::LockUnlock);
}

#[tokio::test]
async fn probe_macos_full() {
    // The real CGSession path contains a space; the probe must send it
    // single-quoted as one remote argument.
    let cgsession =
        "'/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession'";
    let runner = FakeRunner::new(vec![
        (&["uname", "-s"], 0, "Darwin\n"),
        (&["command", "-v", "osascript"], 0, "/usr/bin/osascript\n"),
        (&["command", "-v", "open"], 0, "/usr/bin/open\n"),
        (&["test", "-x", cgsession], 0, ""),
        (&["command", "-v", "shutdown"], 0, "/sbin/shutdown\n"),
    ]);
    let caps = probe_host(ssh_config(), runner).await;
    assert_eq!(caps, macos_caps());
}

#[tokio::test]
async fn probe_unreachable_or_unknown_os_reports_nothing() {
    // Nothing scripted: every command fails, no OS detected.
    let runner = FakeRunner::new(vec![]);
    let caps = probe_host(ssh_config(), runner.clone()).await;
    assert_eq!(caps, HostCapabilities::default());
    assert!(caps.command_kinds().is_empty());
    assert_eq!(runner.remote_tails().len(), 2, "uname, then ver, then stop");
}

// ---------- argv construction through the channel ----------

fn assert_ssh_prefix(call: &[String]) {
    assert_eq!(call[0], ssh_path());
    assert!(call[1..].windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
    assert!(call.iter().any(|a| a == "archie@192.168.1.40"));
    assert!(call.iter().any(|a| a == key_path()));
}

#[tokio::test]
async fn windows_lock_and_unlock_honesty() {
    let runner = FakeRunner::script(&[&["rundll32.exe", "user32.dll,LockWorkStation"]]);
    let mut adapter = mk_adapter(windows_caps(), runner.clone(), false);
    let resource = mk_resource(&[CommandKind::SessionLock]);
    adapter
        .apply(&resource, &Command::SessionLock { locked: true })
        .await
        .unwrap();
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    assert_ssh_prefix(&calls[0]);
    assert_eq!(
        runner.remote_tails()[0],
        vec!["rundll32.exe", "user32.dll,LockWorkStation"]
    );
    // Windows cannot be unlocked remotely: refused before any process exists.
    assert_eq!(
        adapter
            .apply(&resource, &Command::SessionLock { locked: false })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert_eq!(runner.calls().len(), 1, "no blind unlock attempt");
}

#[tokio::test]
async fn windows_volume_via_nircmd_with_powershell_readback() {
    let runner = FakeRunner::script(&[
        &["nircmd.exe", "setsysvolume", "32768"],
        &[
            "powershell.exe",
            "-NoProfile",
            "-EncodedCommand",
            &ps_audio_get(),
        ],
    ]);
    let mut adapter = mk_adapter(windows_caps(), runner.clone(), false);
    let resource = mk_resource(&[CommandKind::VolumeSet]);
    adapter
        .apply(&resource, &Command::VolumeSet { percent: 50 })
        .await
        .unwrap();
    assert_eq!(
        runner.remote_tails()[0],
        vec!["nircmd.exe", "setsysvolume", "32768"]
    );
    // The read-back path is exercised in observe via the fixture below.
    let _ = adapter;
}

#[tokio::test]
async fn windows_observe_measures_lock_volume_and_foreground() {
    let runner = FakeRunner::new(vec![
        (
            &[
                "powershell.exe",
                "-NoProfile",
                "-EncodedCommand",
                &ps_lock_state(),
            ],
            0,
            "True\r\n",
        ),
        (
            &[
                "powershell.exe",
                "-NoProfile",
                "-EncodedCommand",
                &ps_audio_get(),
            ],
            0,
            "53\r\n",
        ),
        (
            &[
                "powershell.exe",
                "-NoProfile",
                "-EncodedCommand",
                &ps_foreground(),
            ],
            0,
            "notepad\r\n",
        ),
    ]);
    let mut adapter = mk_adapter(windows_caps(), runner.clone(), false);
    let resource = mk_resource(&[
        CommandKind::SessionLock,
        CommandKind::VolumeSet,
        CommandKind::AppLaunch,
    ]);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Host {
            reachable: true,
            awake: Some(true),
            session_locked: Some(true),
            volume_percent: Some(53),
            foreground_app: Some("notepad".into()),
        }
    );
    assert!(observation.evidence_id.starts_with("host-"));
    assert!(Command::SessionLock { locked: true }.matches(&observation.state));
    assert!(Command::VolumeSet { percent: 53 }.matches(&observation.state));
    assert_eq!(runner.remote_tails().len(), 3);
}

#[tokio::test]
async fn linux_lock_unlock_volume_and_launch_argv() {
    let runner = FakeRunner::script(&[
        &["loginctl", "lock-session"],
        &["loginctl", "unlock-session"],
        &["pactl", "set-sink-volume", "@DEFAULT_SINK@", "50%"],
        &["systemd-run", "--user", "'/usr/bin/firefox'"],
    ]);
    let mut adapter = mk_adapter(linux_caps(), runner.clone(), false);
    let resource = mk_resource(&[
        CommandKind::SessionLock,
        CommandKind::VolumeSet,
        CommandKind::AppLaunch,
    ]);
    adapter
        .apply(&resource, &Command::SessionLock { locked: true })
        .await
        .unwrap();
    adapter
        .apply(&resource, &Command::SessionLock { locked: false })
        .await
        .unwrap();
    adapter
        .apply(&resource, &Command::VolumeSet { percent: 50 })
        .await
        .unwrap();
    adapter
        .apply(
            &resource,
            &Command::AppLaunch {
                app_id: "firefox".into(),
            },
        )
        .await
        .unwrap();
    let tails = runner.remote_tails();
    assert_eq!(tails[0], vec!["loginctl", "lock-session"]);
    assert_eq!(tails[1], vec!["loginctl", "unlock-session"]);
    assert_eq!(
        tails[2],
        vec!["pactl", "set-sink-volume", "@DEFAULT_SINK@", "50%"]
    );
    assert_eq!(
        tails[3],
        vec!["systemd-run", "--user", "'/usr/bin/firefox'"]
    );
}

#[tokio::test]
async fn linux_observe_agrees_across_sessions() {
    let runner = FakeRunner::new(vec![
        (
            &["loginctl", "list-sessions", "--no-legend"],
            0,
            LOGINCTL_LIST,
        ),
        (
            &["loginctl", "show-session", "3", "-p", "LockedHint"],
            0,
            "LockedHint=no\n",
        ),
        (
            &["loginctl", "show-session", "7", "-p", "LockedHint"],
            0,
            "LockedHint=no\n",
        ),
        (
            &["pactl", "get-sink-volume", "@DEFAULT_SINK@"],
            0,
            PACTL_VOLUME,
        ),
        (
            &["xdotool", "getactivewindow", "getwindowname"],
            0,
            "ignored",
        ),
        (
            &["xdotool", "getactivewindow", "getwindowclassname"],
            0,
            "firefox\n",
        ),
    ]);
    let mut adapter = mk_adapter(linux_caps(), runner.clone(), false);
    let resource = mk_resource(&[
        CommandKind::SessionLock,
        CommandKind::VolumeSet,
        CommandKind::AppLaunch,
    ]);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Host {
            reachable: true,
            awake: Some(true),
            session_locked: Some(false),
            volume_percent: Some(60),
            foreground_app: Some("firefox".into()),
        }
    );
    assert!(Command::AppLaunch {
        app_id: "firefox".into()
    }
    .matches(&observation.state));
    assert!(Command::SessionLock { locked: false }.matches(&observation.state));

    // A session that cannot say leaves the lock state unknown, not guessed.
    let runner = FakeRunner::new(vec![
        (
            &["loginctl", "list-sessions", "--no-legend"],
            0,
            LOGINCTL_LIST,
        ),
        (
            &["loginctl", "show-session", "3", "-p", "LockedHint"],
            0,
            "LockedHint=yes\n",
        ),
        (
            &["loginctl", "show-session", "7", "-p", "LockedHint"],
            0,
            "garbage\n",
        ),
        (
            &["pactl", "get-sink-volume", "@DEFAULT_SINK@"],
            0,
            PACTL_VOLUME,
        ),
        (
            &["xdotool", "getactivewindow", "getwindowclassname"],
            0,
            "firefox\n",
        ),
    ]);
    let mut adapter = mk_adapter(linux_caps(), runner, false);
    let observation = adapter.observe(&resource).await.unwrap();
    match observation.state {
        ResourceState::Host { session_locked, .. } => assert_eq!(session_locked, None),
        other => panic!("expected Host, got {other:?}"),
    }
}

#[tokio::test]
async fn macos_lock_is_measured_via_loginwindow_frontmost() {
    let cgsession =
        "'/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession'";
    let runner = FakeRunner::new(vec![
        (&[cgsession, "-suspend"], 0, ""),
        (
            &["osascript", "-e", "'tell application \"System Events\" to get name of first application process whose frontmost is true'"],
            0,
            "loginwindow\n",
        ),
        (&["osascript", "-e", "'get volume settings'"], 0, OSASCRIPT_VOLUME),
    ]);
    let mut adapter = mk_adapter(macos_caps(), runner.clone(), false);
    let resource = mk_resource(&[CommandKind::SessionLock, CommandKind::VolumeSet]);
    adapter
        .apply(&resource, &Command::SessionLock { locked: true })
        .await
        .unwrap();
    assert_eq!(runner.remote_tails()[0], vec![cgsession, "-suspend"]);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Host {
            reachable: true,
            awake: Some(true),
            session_locked: Some(true),
            volume_percent: Some(35),
            foreground_app: Some("loginwindow".into()),
        }
    );
    assert!(Command::SessionLock { locked: true }.matches(&observation.state));
    assert!(Command::VolumeSet { percent: 35 }.matches(&observation.state));
    // macOS cannot be unlocked remotely either.
    assert_eq!(
        adapter
            .apply(&resource, &Command::SessionLock { locked: false })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
}

// ---------- allowlists, gates, and honest refusals ----------

#[tokio::test]
async fn app_launch_is_allowlisted_per_os() {
    // Windows token, quoted for cmd.
    let runner =
        FakeRunner::script(&[&["cmd.exe", "/c", "start", "\"\"", "\"C:\\Tools\\notes.exe\""]]);
    let mut adapter = mk_adapter(windows_caps(), runner.clone(), false);
    let resource = mk_resource(&[CommandKind::AppLaunch]);
    adapter
        .apply(
            &resource,
            &Command::AppLaunch {
                app_id: "notes".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runner.remote_tails()[0],
        vec!["cmd.exe", "/c", "start", "\"\"", "\"C:\\Tools\\notes.exe\""]
    );
    // An app_id outside the table never reaches a process.
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "notepad".into()
                }
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert_eq!(runner.calls().len(), 1);
}

#[tokio::test]
async fn unprobed_capabilities_are_unsupported_without_acting() {
    // Probe found no volume tool and no foreground read.
    let caps = HostCapabilities {
        ssh: true,
        os: Some(HostOs::Linux),
        lock: LockSupport::LockUnlock,
        volume: None,
        foreground: false,
        app_launch: false,
        power: false,
    };
    let runner = FakeRunner::new(vec![]);
    let mut adapter = mk_adapter(caps, runner.clone(), false);
    let resource = mk_resource(&[
        CommandKind::VolumeSet,
        CommandKind::AppLaunch,
        CommandKind::SessionLock,
    ]);
    assert_eq!(
        adapter
            .apply(&resource, &Command::VolumeSet { percent: 50 })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "firefox".into()
                }
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    // Commands the adapter never serves, even when declared.
    let mut rich = mk_resource(&[CommandKind::SwitchSet, CommandKind::WakeHost]);
    rich.capabilities.push(CommandKind::MediaPause);
    for command in [
        Command::SwitchSet { on: true },
        Command::WakeHost,
        Command::MediaPause,
    ] {
        assert_eq!(
            adapter.apply(&rich, &command).await.unwrap_err(),
            AdapterError::Unsupported
        );
    }
    assert!(runner.calls().is_empty(), "refusals never spawn ssh");
}

#[tokio::test]
async fn power_actions_are_doubly_gated() {
    // Config gate off: refused without spawning.
    let runner = FakeRunner::new(vec![]);
    let adapter = mk_adapter(windows_caps(), runner.clone(), false);
    assert_eq!(
        adapter.shutdown().await.unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty());
    // Probe gate off: refused without spawning.
    let mut caps = windows_caps();
    caps.power = false;
    let runner = FakeRunner::new(vec![]);
    let adapter = mk_adapter(caps, runner.clone(), true);
    assert_eq!(
        adapter.reboot().await.unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty());
    // Both gates open: exact argv per OS.
    let runner = FakeRunner::script(&[
        &["shutdown.exe", "/s", "/t", "0"],
        &["shutdown.exe", "/r", "/t", "0"],
    ]);
    let adapter = mk_adapter(windows_caps(), runner.clone(), true);
    adapter.shutdown().await.unwrap();
    adapter.reboot().await.unwrap();
    let tails = runner.remote_tails();
    assert_eq!(tails[0], vec!["shutdown.exe", "/s", "/t", "0"]);
    assert_eq!(tails[1], vec!["shutdown.exe", "/r", "/t", "0"]);

    let runner = FakeRunner::script(&[&["shutdown", "-h", "now"], &["shutdown", "-r", "now"]]);
    let adapter = mk_adapter(linux_caps(), runner.clone(), true);
    adapter.shutdown().await.unwrap();
    adapter.reboot().await.unwrap();
    let tails = runner.remote_tails();
    assert_eq!(tails[0], vec!["shutdown", "-h", "now"]);
    assert_eq!(tails[1], vec!["shutdown", "-r", "now"]);
}

#[tokio::test]
async fn unreachable_host_and_bad_output_are_honest_errors() {
    // Every command fails: observe is Unavailable, never an invented state.
    let runner = FakeRunner::new(vec![]);
    let mut adapter = mk_adapter(linux_caps(), runner.clone(), false);
    let resource = mk_resource(&[CommandKind::SessionLock]);
    assert_eq!(
        adapter.observe(&resource).await.unwrap_err(),
        AdapterError::Unavailable
    );

    // Unparseable lock read-back: the field is unknown, volume still measured.
    let runner = FakeRunner::new(vec![
        (
            &["loginctl", "list-sessions", "--no-legend"],
            0,
            "not a session list\n",
        ),
        (
            &["pactl", "get-sink-volume", "@DEFAULT_SINK@"],
            0,
            PACTL_VOLUME,
        ),
        (
            &["xdotool", "getactivewindow", "getwindowclassname"],
            0,
            "firefox\n",
        ),
    ]);
    let mut adapter = mk_adapter(linux_caps(), runner, false);
    let observation = adapter.observe(&resource).await.unwrap();
    match observation.state {
        ResourceState::Host {
            session_locked,
            volume_percent,
            ..
        } => {
            assert_eq!(session_locked, None);
            assert_eq!(volume_percent, Some(60));
        }
        other => panic!("expected Host, got {other:?}"),
    }
}

#[tokio::test]
async fn configuration_is_validated() {
    let runner = FakeRunner::new(vec![]);
    // Bad app token (quote would escape the remote quoting).
    let mut bad = target(false);
    bad.apps.insert("evil".into(), "x'; rm -rf /; '".into());
    assert!(HostSsh::new(
        "edge",
        "host-ssh",
        "workstation",
        bad,
        linux_caps(),
        runner.clone()
    )
    .is_err());
    let mut bad = target(false);
    bad.apps.insert("evil".into(), "C:\\%PATH%\\x.exe".into());
    assert!(HostSsh::new(
        "edge",
        "host-ssh",
        "workstation",
        bad,
        linux_caps(),
        runner.clone()
    )
    .is_err());
    let mut bad = target(false);
    bad.apps.insert("bad id".into(), "fine".into());
    assert!(HostSsh::new(
        "edge",
        "host-ssh",
        "workstation",
        bad,
        linux_caps(),
        runner.clone()
    )
    .is_err());
    // Unpaired resource is refused before anything runs.
    let mut adapter = mk_adapter(linux_caps(), runner.clone(), false);
    let mut unpaired = mk_resource(&[CommandKind::SessionLock]);
    unpaired.paired = false;
    assert_eq!(
        adapter
            .apply(&unpaired, &Command::SessionLock { locked: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty());
}

#[tokio::test]
async fn windows_volume_set_via_powershell_when_nircmd_absent() {
    let mut caps = windows_caps();
    caps.volume = Some(VolumeTool::PowerShell);
    let runner = FakeRunner::script(&[&[
        "powershell.exe",
        "-NoProfile",
        "-EncodedCommand",
        &ps_audio_set(80),
    ]]);
    let mut adapter = mk_adapter(caps, runner.clone(), false);
    let resource = mk_resource(&[CommandKind::VolumeSet]);
    adapter
        .apply(&resource, &Command::VolumeSet { percent: 80 })
        .await
        .unwrap();
    assert_eq!(
        runner.remote_tails()[0],
        vec![
            "powershell.exe",
            "-NoProfile",
            "-EncodedCommand",
            &ps_audio_set(80)
        ]
    );
}
