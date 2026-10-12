//! `archie-netwatch` — Sentry's continuous audit of the traffic on the owner's network.
//!
//! ```text
//! archie-netwatch interfaces
//! archie-netwatch live --interface eth0 --state-dir DIR [--learn 600] [--gateway 192.168.1.1] [--filter BPF]
//! archie-netwatch read FILE.pcapng --state-dir DIR [--gateway 192.168.1.1]
//! archie-netwatch report --state-dir DIR [--alerts 50]
//! ```
//!
//! `live` runs until stopped: TShark captures, each frame is audited, alerts are appended
//! to `DIR/alerts.jsonl` as they are raised, what is known of each device is kept in
//! `DIR/state.json`, and which alerts were already raised in `DIR/told.json`, so a
//! restart does not repeat them. `read` audits a capture file the same way. `report`
//! prints both. Everything is written for its owner only: which names a device asks for
//! is private.

use archie_lan::audit::{Alert, Audit, Settings, State, Told};
use archie_lan::capture::{field_arguments, parse, SNAPLEN};
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const MAX_ALERTS_BYTES: u64 = 4 * 1024 * 1024;
const SAVE_EVERY: Duration = Duration::from_secs(30);

fn take(arguments: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = arguments.iter().position(|a| a == flag)?;
    (at + 1 < arguments.len()).then(|| {
        arguments.remove(at);
        arguments.remove(at)
    })
}

fn private(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).append(append).truncate(!append);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Writes a file in one move: a partial write is never mistaken for the whole.
fn save(directory: &Path, name: &str, value: &impl serde::Serialize) -> std::io::Result<()> {
    let partial = directory.join(format!("{name}.partial"));
    private(&partial, false)?.write_all(&serde_json::to_vec(value).expect("it serializes"))?;
    std::fs::rename(partial, directory.join(name))
}

fn load(directory: &Path) -> Result<State, String> {
    match std::fs::read(directory.join("state.json")) {
        Ok(bytes) => State::parse(&bytes).map_err(|e| format!("state.json is not a state this program wrote: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// What was already told. A told.json that will not load is set aside rather than
/// fatal: the worst that follows is an alert told once more, and that is said.
fn load_told(directory: &Path) -> Told {
    match std::fs::read(directory.join("told.json")) {
        Ok(bytes) => match Told::parse(&bytes) {
            Ok(told) => told,
            Err(error) => {
                eprintln!("archie-netwatch: told.json is set aside ({error}); some alerts may be told once more");
                Told::default()
            }
        },
        Err(_) => Told::default(),
    }
}

fn append(directory: &Path, alerts: &[Alert]) -> std::io::Result<()> {
    let path = directory.join("alerts.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_ALERTS_BYTES) {
        std::fs::rename(&path, directory.join("alerts.1.jsonl"))?;
    }
    let mut file = private(&path, true)?;
    for alert in alerts {
        writeln!(file, "{}", serde_json::to_string(alert).expect("an alert serializes"))?;
    }
    Ok(())
}

/// The router, from the kernel's own routing table.
fn default_gateway() -> Option<Ipv4Addr> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.len() > 2 && fields[1] == "00000000").then(|| u32::from_str_radix(fields[2], 16).ok().map(|raw| Ipv4Addr::from(raw.swap_bytes())))?
    })
}

fn audit(source: &[String], directory: &Path, settings: Settings) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let mut audit = Audit::new(load(directory)?, settings);
    audit.told = load_told(directory);
    let mut tshark = Command::new("tshark")
        .args(source)
        .args(field_arguments())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("tshark could not be started ({e}); it comes with the wireshark-cli package"))?;
    let lines = BufReader::new(tshark.stdout.take().expect("piped"));
    let (mut frames, mut raised, mut saved) = (0u64, 0u64, Instant::now());
    for line in lines.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Some(packet) = parse(&line) else { continue };
        frames += 1;
        let alerts = audit.see(&packet);
        if !alerts.is_empty() {
            raised += alerts.len() as u64;
            append(directory, &alerts).map_err(|e| e.to_string())?;
            // What was told is written with the alerts themselves, so even a crash
            // between saves does not repeat them.
            save(directory, "told.json", &audit.told).map_err(|e| e.to_string())?;
            for alert in &alerts {
                eprintln!("archie-netwatch: {:?} {} {}: {}", alert.severity, alert.rule, alert.device, alert.detail);
            }
        }
        if saved.elapsed() >= SAVE_EVERY {
            save(directory, "state.json", &audit.state).map_err(|e| e.to_string())?;
            save(directory, "told.json", &audit.told).map_err(|e| e.to_string())?;
            saved = Instant::now();
        }
    }
    save(directory, "state.json", &audit.state).map_err(|e| e.to_string())?;
    save(directory, "told.json", &audit.told).map_err(|e| e.to_string())?;
    let status = tshark.wait().map_err(|e| e.to_string())?;
    eprintln!("archie-netwatch: {frames} frames, {raised} alerts, {} devices known", audit.state.devices.len());
    if status.success() {
        Ok(())
    } else {
        Err(format!("tshark ended with {status}"))
    }
}

fn report(directory: &Path, last: usize) -> Result<(), String> {
    let state = load(directory)?;
    let text = std::fs::read_to_string(directory.join("alerts.jsonl")).unwrap_or_default();
    let alerts: Vec<serde_json::Value> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let recent = &alerts[alerts.len().saturating_sub(last)..];
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "state": state, "alerts": recent })).expect("a report serializes"));
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let directory = take(&mut arguments, "--state-dir").map(PathBuf::from);
    let gateway = take(&mut arguments, "--gateway").and_then(|g| g.parse().ok());
    let learn = take(&mut arguments, "--learn").and_then(|s| s.parse::<u64>().ok());
    let outcome = match (arguments.first().map(String::as_str), directory) {
        (Some("interfaces"), _) => Command::new("tshark").arg("-D").status().map(|_| ()).map_err(|e| format!("tshark could not be started: {e}")),
        (Some("live"), Some(directory)) => {
            let Some(interface) = take(&mut arguments, "--interface") else {
                eprintln!("archie-netwatch: live needs --interface NAME; `archie-netwatch interfaces` lists them");
                return ExitCode::from(2);
            };
            let mut source = vec!["-i".to_string(), interface, "-s".into(), SNAPLEN.into()];
            if let Some(filter) = take(&mut arguments, "--filter") {
                source.extend(["-f".to_string(), filter]);
            }
            let settings = Settings { learn_ms: learn.unwrap_or(600) * 1000, gateway: gateway.or_else(default_gateway) };
            audit(&source, &directory, settings)
        }
        (Some("read"), Some(directory)) if arguments.len() == 2 => {
            // A file is audited from its first frame: nothing in it is news to be learned first.
            audit(&["-r".to_string(), arguments[1].clone()], &directory, Settings { learn_ms: learn.unwrap_or(0) * 1000, gateway })
        }
        (Some("report"), Some(directory)) => report(&directory, take(&mut arguments, "--alerts").and_then(|n| n.parse().ok()).unwrap_or(50)),
        _ => {
            eprintln!(
                "usage: archie-netwatch interfaces\n       archie-netwatch live --interface NAME --state-dir DIR [--learn SECONDS] [--gateway IP] [--filter BPF]\n       archie-netwatch read FILE --state-dir DIR [--gateway IP]\n       archie-netwatch report --state-dir DIR [--alerts N]"
            );
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("archie-netwatch: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_lan::audit::Limits;
    use archie_lan::capture::Packet;

    const TV: &str = "54:4e:f0:07:3d:e8";

    /// A directory of one's own, emptied before and after.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archie-netwatch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn from(mac: &str, source: &str, at_ms: u64) -> Packet {
        Packet { at_ms, length: 100, source_mac: Some(mac.into()), source: Some(source.parse().unwrap()), ..Packet::default() }
    }

    #[test]
    fn a_restart_repeats_nothing_that_was_told() {
        let dir = dir("restart");
        let settings = Settings { learn_ms: 0, gateway: None };
        let telnet = Packet { destination: Some("192.168.1.1".parse().unwrap()), tcp: Some((50_000, 23)), opening: true, ..from(TV, "192.168.1.84", 2_000) };
        let mut first = Audit::new(load(&dir).unwrap(), settings.clone());
        first.told = load_told(&dir);
        assert_eq!(first.see(&from(TV, "192.168.1.84", 1_000)).len(), 1, "new-device is told");
        assert_eq!(first.see(&telnet).len(), 1, "cleartext-login is told");
        save(&dir, "state.json", &first.state).unwrap();
        save(&dir, "told.json", &first.told).unwrap();

        // The process stops and starts. The same frames arrive again.
        let mut second = Audit::new(load(&dir).unwrap(), settings);
        second.told = load_told(&dir);
        assert_eq!(second.state, first.state, "the state survives the restart");
        assert!(second.see(&from(TV, "192.168.1.84", 3_000)).is_empty(), "the device is known");
        assert!(second.see(&telnet).is_empty(), "the alert was already told before the restart");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_state_file_is_an_error_naming_why_never_a_panic() {
        let dir = dir("broken");
        std::fs::write(dir.join("state.json"), b"{ not json").unwrap();
        let error = load(&dir).unwrap_err();
        assert!(error.contains("not the JSON this program writes"), "{error}");

        // One device past the cap of devices: refused, naming the cap.
        let device = serde_json::json!({
            "first_seen_ms": 1, "last_seen_ms": 1, "addresses": ["192.168.1.84"],
            "hostname": null, "frames": 1, "bytes": 100, "asked": [], "sent_to": {}
        });
        let cap = Limits::default().devices;
        let devices: serde_json::Map<String, serde_json::Value> =
            (0..=cap).map(|i| (format!("aa:bb:cc:{:02x}:{:02x}:{:02x}", i / 65536, i / 256 % 256, i % 256), device.clone())).collect();
        let state = serde_json::json!({ "format": "archie-netwatch-state-v1", "started_ms": 1, "devices": devices, "owners": {} });
        std::fs::write(dir.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
        let error = load(&dir).unwrap_err();
        assert!(error.contains(&format!("is more than the {cap}")), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_told_file_over_the_cap_is_set_aside_and_a_sound_one_kept() {
        let dir = dir("told-cap");
        // Over the cap of remembered alerts: set aside, so the run goes on.
        let cap = Limits::default().told;
        let records: serde_json::Map<String, serde_json::Value> =
            (0..=cap).map(|i| (format!("alert {i}"), serde_json::json!({ "at_ms": i, "count": 1 }))).collect();
        let told = serde_json::json!({ "format": "archie-netwatch-told-v1", "records": records });
        std::fs::write(dir.join("told.json"), serde_json::to_vec(&told).unwrap()).unwrap();
        assert!(load_told(&dir).records.is_empty(), "over the cap it starts fresh");

        // A sound one comes back whole.
        let mut audit = Audit::new(State::default(), Settings { learn_ms: 0, gateway: None });
        audit.see(&from(TV, "192.168.1.84", 1_000));
        save(&dir, "told.json", &audit.told).unwrap();
        assert_eq!(load_told(&dir), audit.told);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn the_files_written_are_the_owners_own() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir("private");
        let audit = Audit::new(State::default(), Settings::default());
        save(&dir, "state.json", &audit.state).unwrap();
        save(&dir, "told.json", &audit.told).unwrap();
        append(&dir, &[Alert { at_ms: 1, severity: archie_lan::audit::Severity::Notice, rule: "new-device".into(), device: TV.into(), detail: "first seen".into() }]).unwrap();
        for name in ["state.json", "told.json", "alerts.jsonl"] {
            let mode = std::fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name} is for its owner only");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
