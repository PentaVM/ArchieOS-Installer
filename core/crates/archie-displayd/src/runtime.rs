//! Bounded, read-only inventory of this machine for the orbital projection.
//!
//! Process names, parenthood, CPU tick deltas, installed service unit names, and device
//! class names show the architecture of a working OS. Fixed system directories are
//! checked for existence only. Command lines, environment, user paths, open files,
//! and process memory never enter the display daemon's orb feed.

use archie_orb::OrbEntity;
#[cfg(target_os = "linux")]
use archie_orb::{semantic_features, EntityKind};
#[cfg(target_os = "linux")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
const MAX_PROCESSES: usize = 256;
#[cfg(target_os = "linux")]
const MAX_SERVICES: usize = 256;
#[cfg(target_os = "linux")]
const MAX_DEVICES: usize = 64;

#[derive(Default)]
pub struct RuntimeInventory {
    #[cfg(target_os = "linux")]
    previous_ticks: BTreeMap<String, u64>,
}

#[cfg(target_os = "linux")]
struct ProcessFact {
    pid: u32,
    ppid: u32,
    id: String,
    name: String,
    state: char,
    ticks: u64,
    service: Option<String>,
}

impl RuntimeInventory {
    pub fn read(&mut self) -> Vec<OrbEntity> {
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
        #[cfg(target_os = "linux")]
        {
            self.read_linux()
        }
    }

    #[cfg(target_os = "linux")]
    fn read_linux(&mut self) -> Vec<OrbEntity> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut pids = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .collect::<Vec<_>>();
        pids.sort_unstable();
        let mut facts = pids
            .into_iter()
            .filter_map(read_process)
            .collect::<Vec<_>>();
        let next_ticks = facts
            .iter()
            .map(|fact| (fact.id.clone(), fact.ticks))
            .collect::<BTreeMap<_, _>>();
        let rank = |fact: &ProcessFact| {
            (
                u8::from(fact.pid == 1),
                u8::from(fact.name.starts_with("archie")),
                u8::from(fact.state == 'R'),
                fact.ticks.saturating_sub(
                    self.previous_ticks
                        .get(&fact.id)
                        .copied()
                        .unwrap_or(fact.ticks),
                ),
                u8::from(fact.service.is_some()),
                std::cmp::Reverse(fact.pid),
            )
        };
        facts.sort_by_key(|fact| std::cmp::Reverse(rank(fact)));
        facts.truncate(MAX_PROCESSES);
        facts.sort_unstable_by_key(|fact| fact.pid);
        let ids = facts
            .iter()
            .map(|fact| (fact.pid, fact.id.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut entities = Vec::with_capacity(facts.len() + MAX_SERVICES + MAX_DEVICES + 8);
        let mut service_members = BTreeMap::<String, Vec<String>>::new();
        for fact in facts {
            let previous = self
                .previous_ticks
                .get(&fact.id)
                .copied()
                .unwrap_or(fact.ticks);
            let delta = fact.ticks.saturating_sub(previous);
            let activity = if fact.state == 'R' {
                (0.25 + (delta as f32 / 25.0).min(0.7)).min(1.0)
            } else {
                (delta as f32 / 25.0).min(1.0)
            };
            if let Some(service) = &fact.service {
                service_members
                    .entry(service.clone())
                    .or_default()
                    .push(fact.id.clone());
            }
            let mut classes = BTreeMap::from([("process".into(), 1.0)]);
            if fact.pid == 1 {
                classes.insert("boot".into(), 0.8);
            }
            if fact.service.is_some() {
                classes.insert("service".into(), 0.55);
            }
            entities.push(OrbEntity {
                id: fact.id,
                parent_id: ids
                    .get(&fact.ppid)
                    .cloned()
                    .or_else(|| Some("system:archieos".into())),
                related_ids: fact.service.as_ref().map_or_else(
                    || vec!["directory:/proc".into()],
                    |service| vec![format!("service:{service}"), "directory:/proc".into()],
                ),
                kind: EntityKind::Process,
                token: semantic_features(&format!(
                    "process {} {}",
                    fact.name,
                    fact.service.as_deref().unwrap_or("")
                )),
                class_probs: classes,
                size_bytes: 0,
                activity,
                importance: if fact.name.starts_with("archie") {
                    0.65
                } else {
                    0.2
                },
                uncertainty: 0.05,
                runtime: Vec::new(),
                label: fact.name,
            });
        }
        self.previous_ticks = next_ticks;
        let mut service_units = BTreeSet::new();
        for directory in [
            "/usr/lib/systemd/system",
            "/etc/systemd/system",
            "/run/systemd/system",
        ] {
            if let Ok(entries) = std::fs::read_dir(directory) {
                for entry in entries.flatten() {
                    let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    if entry.path().is_file()
                        && name.ends_with(".service")
                        && name.len() <= 96
                        && !name.chars().any(char::is_control)
                    {
                        service_units.insert(name);
                    }
                }
            }
        }
        // Keep running units ahead of installed but inactive ones when the
        // inventory cap is reached on a larger machine.
        let active_names = service_members.keys().cloned().collect::<BTreeSet<_>>();
        let prioritized_units = active_names.iter().cloned().chain(
            service_units
                .into_iter()
                .filter(|name| !active_names.contains(name)),
        );
        for service in prioritized_units.take(MAX_SERVICES) {
            let members = service_members.remove(&service).unwrap_or_default();
            let active = !members.is_empty();
            let label = service.trim_end_matches(".service");
            entities.push(OrbEntity {
                id: format!("service:{service}"),
                parent_id: Some("system:archieos".into()),
                related_ids: members.into_iter().take(64).collect(),
                kind: EntityKind::Service,
                token: semantic_features(&format!("service {label}")),
                class_probs: BTreeMap::from([
                    ("service".into(), 1.0),
                    (if active { "active" } else { "installed" }.into(), 0.8),
                ]),
                size_bytes: 0,
                activity: if active { 0.25 } else { 0.0 },
                importance: if label.starts_with("archie") && active {
                    0.75
                } else if active {
                    0.35
                } else {
                    0.12
                },
                uncertainty: 0.05,
                runtime: Vec::new(),
                label: label.chars().take(64).collect(),
            });
        }
        let mut seen = BTreeSet::new();
        for (directory, class) in [
            ("/sys/class/net", "network"),
            ("/sys/block", "storage"),
            ("/sys/class/drm", "display"),
            ("/sys/class/power_supply", "power"),
            ("/sys/class/hwmon", "sensor"),
            ("/sys/class/thermal", "thermal"),
            ("/sys/devices/system/cpu", "compute"),
        ] {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                if seen.len() >= MAX_DEVICES {
                    break;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if name.len() > 64
                    || name.chars().any(char::is_control)
                    || (class == "compute"
                        && !name.strip_prefix("cpu").is_some_and(|number| {
                            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                        }))
                    || !seen.insert((class, name.clone()))
                {
                    continue;
                }
                entities.push(OrbEntity {
                    id: format!("device:{class}:{name}"),
                    parent_id: Some("system:archieos".into()),
                    related_ids: vec!["directory:/sys".into()],
                    kind: EntityKind::Device,
                    token: semantic_features(&format!("device {class} {name}")),
                    class_probs: BTreeMap::from([("device".into(), 1.0), (class.into(), 0.8)]),
                    size_bytes: 0,
                    activity: 0.05,
                    importance: 0.3,
                    uncertainty: 0.1,
                    runtime: Vec::new(),
                    label: name,
                });
            }
        }
        for directory in [
            "/proc", "/sys", "/dev", "/run", "/etc", "/usr", "/var", "/tmp",
        ] {
            if !Path::new(directory).is_dir() {
                continue;
            }
            entities.push(OrbEntity {
                id: format!("directory:{directory}"),
                parent_id: Some("system:archieos".into()),
                related_ids: Vec::new(),
                kind: EntityKind::Directory,
                token: semantic_features(&format!("system directory {directory}")),
                class_probs: BTreeMap::from([("directory".into(), 1.0)]),
                size_bytes: 0,
                activity: 0.0,
                importance: 0.3,
                uncertainty: 0.0,
                runtime: Vec::new(),
                label: directory.into(),
            });
        }
        entities
    }
}

#[cfg(target_os = "linux")]
fn read_process(pid: u32) -> Option<ProcessFact> {
    let root = Path::new("/proc").join(pid.to_string());
    let stat = std::fs::read_to_string(root.join("stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    let fields = after_name.split_whitespace().collect::<Vec<_>>();
    if fields.len() <= 19 {
        return None;
    }
    let state = fields[0].chars().next()?;
    let ppid = fields[1].parse().ok()?;
    let ticks = fields[11]
        .parse::<u64>()
        .ok()?
        .saturating_add(fields[12].parse::<u64>().ok()?);
    let start = fields[19].parse::<u64>().ok()?;
    let name = std::fs::read_to_string(root.join("comm"))
        .ok()?
        .trim()
        .to_string();
    if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return None;
    }
    let service = std::fs::read_to_string(root.join("cgroup"))
        .ok()
        .and_then(|text| {
            text.lines()
                .flat_map(|line| line.split('/'))
                .find(|part| {
                    part.ends_with(".service")
                        && part.len() <= 96
                        && !part.chars().any(char::is_control)
                })
                .map(str::to_string)
        });
    Some(ProcessFact {
        pid,
        ppid,
        id: format!("process:{pid}:{start}"),
        name,
        state,
        ticks,
        service,
    })
}
