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
//! to `DIR/alerts.jsonl` as they are raised and what is known of each device is kept in
//! `DIR/state.json`. `read` audits a capture file the same way. `report` prints both.
//! Everything is written for its owner only: which names a device asks for is private.

use archie_lan::audit::{Alert, Audit, Settings, State};
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

fn save(directory: &Path, state: &State) -> std::io::Result<()> {
    let partial = directory.join("state.partial");
    private(&partial, false)?.write_all(&serde_json::to_vec(state).expect("the state serializes"))?;
    std::fs::rename(partial, directory.join("state.json"))
}

fn load(directory: &Path) -> Result<State, String> {
    match std::fs::read(directory.join("state.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("state.json is not a state this program wrote: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.to_string()),
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
            for alert in &alerts {
                eprintln!("archie-netwatch: {:?} {} {}: {}", alert.severity, alert.rule, alert.device, alert.detail);
            }
        }
        if saved.elapsed() >= SAVE_EVERY {
            save(directory, &audit.state).map_err(|e| e.to_string())?;
            saved = Instant::now();
        }
    }
    save(directory, &audit.state).map_err(|e| e.to_string())?;
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
