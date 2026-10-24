//! Sentry's own bounded, interface-scoped ARP cache observation. Cache records
//! are not proof of current reachability, model, manufacturer or ownership.
use crate::is_lan_v4;
use std::{
    collections::BTreeSet,
    io::Read,
    net::Ipv4Addr,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct Neighbor {
    pub address: Ipv4Addr,
    pub mac: String,
    pub status: String,
}

fn mac(raw: &str) -> Option<String> {
    let bytes: Vec<u8> = raw
        .split(['-', ':'])
        .map(|part| {
            if part.is_empty() || part.len() > 2 {
                None
            } else {
                u8::from_str_radix(part, 16).ok()
            }
        })
        .collect::<Option<_>>()?;
    if bytes.len() != 6 || bytes[0] & 1 != 0 || bytes.iter().all(|b| *b == 0) {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

pub fn parse_windows(text: &str, local: Ipv4Addr) -> Vec<Neighbor> {
    let mut selected = false;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if line.contains("---") {
            selected = fields
                .iter()
                .filter_map(|s| s.parse::<Ipv4Addr>().ok())
                .any(|a| a == local);
            continue;
        }
        if !selected || fields.len() < 3 || out.len() >= 4096 {
            continue;
        }
        let Some(address) = fields[0].parse::<Ipv4Addr>().ok().filter(|a| is_lan_v4(*a)) else {
            continue;
        };
        let Some(mac) = mac(fields[1]) else { continue };
        if seen.insert((address, mac.clone())) {
            out.push(Neighbor {
                address,
                mac,
                status: fields[2].chars().take(32).collect(),
            });
        }
    }
    out
}

pub fn parse_linux(text: &str, interface: &str) -> Vec<Neighbor> {
    let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(text) else {
        return Vec::new();
    };
    rows.into_iter()
        .take(4096)
        .filter_map(|row| {
            if row.get("dev")?.as_str()? != interface {
                return None;
            }
            let address = row.get("dst")?.as_str()?.parse::<Ipv4Addr>().ok()?;
            if !is_lan_v4(address) {
                return None;
            }
            let mac = mac(row.get("lladdr")?.as_str()?)?;
            let states = row.get("state")?.as_array()?;
            let status = states
                .iter()
                .filter_map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",");
            if status.contains("FAILED") || status.contains("INCOMPLETE") {
                return None;
            }
            Some(Neighbor {
                address,
                mac,
                status: status.chars().take(64).collect(),
            })
        })
        .collect()
}

pub fn parse_macos(text: &str, interface: &str) -> Vec<Neighbor> {
    text.lines()
        .take(4096)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if !fields.windows(2).any(|pair| pair == ["on", interface]) {
                return None;
            }
            let address = fields
                .iter()
                .find_map(|s| s.trim_matches(['(', ')']).parse::<Ipv4Addr>().ok())?;
            if !is_lan_v4(address) {
                return None;
            }
            let at = fields.iter().position(|s| *s == "at")?;
            let mac = mac(fields.get(at + 1)?)?;
            Some(Neighbor {
                address,
                mac,
                status: "cached".into(),
            })
        })
        .collect()
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("neighbor source could not start: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let until = Instant::now() + Duration::from_secs(3);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("neighbor source timeout or failure: {other:?}"));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "neighbor output worker failed")?
        .map_err(|e| e.to_string())?;
    if !status?.success() || bytes.len() > 1_048_576 {
        return Err("neighbor source failed or exceeded its output limit".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn collect(local: Ipv4Addr) -> Result<Vec<Neighbor>, String> {
    if cfg!(windows) {
        return output("arp", &["-a"]).map(|text| parse_windows(&text, local));
    }
    let interface = if_addrs::get_if_addrs()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.addr.ip() == std::net::IpAddr::V4(local))
        .ok_or("local interface not found")?
        .name;
    if cfg!(target_os = "linux") {
        output("ip", &["-j", "-4", "neigh", "show"]).map(|text| parse_linux(&text, &interface))
    } else if cfg!(target_os = "macos") {
        output("arp", &["-an", "-i", &interface]).map(|text| parse_macos(&text, &interface))
    } else {
        Err("neighbor cache source unavailable on this platform".into())
    }
}
