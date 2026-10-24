//! Single-owner inventory writer; sensing never grants actuation authority.
use super::{run_report, Options};
use archie_lan::{
    autonomy::{Controller, Settings as HistorySettings},
    dns_sd::Backend,
    presence::Subnet,
};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct Settings {
    pub out: PathBuf,
    pub every: Option<u64>,
    pub autonomous: bool,
    pub once: bool,
    pub history: Option<PathBuf>,
    pub health: Option<PathBuf>,
    pub deep_every: u64,
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Resolve existing ancestors before removing '..' across directory symlinks.
    let mut parent = path.as_path();
    let mut suffix = Vec::new();
    while !parent.exists() {
        suffix.push(
            parent
                .file_name()
                .ok_or_else(|| io::Error::other("invalid output path"))?
                .to_os_string(),
        );
        parent = parent
            .parent()
            .ok_or_else(|| io::Error::other("invalid output parent"))?;
    }
    let mut resolved = parent.canonicalize()?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    let mut clean = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other.as_os_str()),
        }
    }
    Ok(clean)
}

fn key(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

fn validate_paths(paths: &[&Path]) -> io::Result<()> {
    let mut keys = std::collections::BTreeSet::new();
    for path in paths {
        for candidate in [path.to_path_buf(), lock_path(path)] {
            if std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(io::Error::other("monitor files cannot be symlinks"));
            }
            if !keys.insert(key(&absolute(&candidate)?)) {
                return Err(io::Error::other(
                    "monitor output, state, diagnostics or lock paths collide",
                ));
            }
        }
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

fn lock(paths: &[&Path]) -> io::Result<Vec<File>> {
    let mut ordered: Vec<PathBuf> = paths
        .iter()
        .map(|p| absolute(p))
        .collect::<io::Result<_>>()?;
    ordered.sort_by_key(|p| key(p));
    let mut locks = Vec::new();
    for path in ordered {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = private_options().truncate(false).open(lock_path(&path))?;
        file.try_lock_exclusive()
            .map_err(|_| io::Error::other("another monitor owns an output/state file"))?;
        locks.push(file);
    }
    Ok(locks)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".{}.partial", uuid::Uuid::new_v4()));
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let mut file = private_options().create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn controller(settings: HistorySettings, path: &Path) -> io::Result<Controller> {
    match File::open(path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            Controller::restore(settings, &bytes).map_err(io::Error::other)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Controller::new(settings).map_err(io::Error::other)
        }
        Err(error) => Err(error),
    }
}

pub fn run(settings: Settings, mut options: Options) -> io::Result<()> {
    let every = settings
        .every
        .unwrap_or(if settings.autonomous { 30 } else { 300 });
    let history = settings
        .history
        .unwrap_or_else(|| settings.out.with_extension("history.json"));
    let health = settings
        .health
        .unwrap_or_else(|| settings.out.with_extension("health.json"));
    let mut paths = vec![settings.out.as_path()];
    if settings.autonomous {
        paths.extend([history.as_path(), health.as_path()]);
    }
    if let Some(path) = &options.diagnostics {
        paths.push(path);
    }
    validate_paths(&paths)?;
    let _locks = lock(&paths)?;
    let mut controller = if settings.autonomous {
        options.mdns_backend.get_or_insert(Backend::Hybrid);
        options.probe_seconds.get_or_insert(75);
        Some(controller(
            HistorySettings {
                light_seconds: every,
                deep_seconds: settings.deep_every,
                ..Default::default()
            },
            &history,
        )?)
    } else {
        None
    };
    loop {
        let mut pass = options.clone();
        let mut deep_skip = None;
        let mut deep = false;
        let prepared = (|| -> io::Result<()> {
            if let Some(controller) = &mut controller {
                if let Ok(address) = pass.bind.map(Ok).unwrap_or_else(archie_lan::lan_address) {
                    pass.bind = Some(address);
                    if let Some(interface) = if_addrs::get_if_addrs()?
                        .into_iter()
                        .find(|i| i.addr.ip() == std::net::IpAddr::V4(address))
                    {
                        let scope = format!(
                            "{}|{}|{}|{}",
                            interface.name,
                            address,
                            match interface.addr {
                                if_addrs::IfAddr::V4(a) => a.netmask.to_string(),
                                _ => String::new(),
                            },
                            options
                                .subnet
                                .map(|s| s.cidr())
                                .unwrap_or_else(|| if options.probe_local {
                                    "local-tcp"
                                } else {
                                    "discovery-only"
                                }
                                .into())
                        );
                        controller.set_scope(&scope).map_err(io::Error::other)?;
                    }
                }
                deep = controller.deep_due(now());
                if !deep {
                    // A light pass is the native listeners only: no TCP sweep and no
                    // DNS-SD library daemon. What those saw on the last deep pass stands.
                    pass.probe_local = false;
                    pass.subnet = None;
                    if matches!(pass.mdns_backend, Some(Backend::Hybrid)) {
                        pass.mdns_backend = Some(Backend::Native);
                    }
                }
                if pass.probe_local || pass.subnet.is_some() {
                    match pass.bind.map(Ok).unwrap_or_else(archie_lan::lan_address) {
                        Ok(address) => {
                            pass.bind = Some(address);
                            if pass.probe_local {
                                if let Err(error) = Subnet::for_interface(address) {
                                    deep_skip = Some(error);
                                    pass.probe_local = false;
                                }
                            }
                        }
                        Err(error) => deep_skip = Some(error.to_string()),
                    }
                }
            }
            Ok(())
        })();
        let result = prepared
            .and_then(|()| run_report(&pass))
            .and_then(|report| {
                let mut next = controller.clone();
                let output = if let Some(candidate) = &mut next {
                    candidate
                        .record_success(&report.inventory, deep)
                        .map_err(io::Error::other)?
                } else {
                    report.inventory
                };
                // Validate/serialize all documents before publishing any successful result.
                let bytes = serde_json::to_vec_pretty(&output)?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(io::Error::other("inventory exceeds 8 MiB"));
                }
                let sidecars = next
                    .as_ref()
                    .map(|candidate| -> io::Result<_> {
                        let history_bytes = candidate.encode().map_err(io::Error::other)?;
                        let mut status = candidate.health(every);
                        status["deep_skip_reason"] = serde_json::json!(deep_skip);
                        status["sources"] = serde_json::to_value(&report.diagnostics.sources)?;
                        Ok((history_bytes, serde_json::to_vec_pretty(&status)?))
                    })
                    .transpose()?;
                // Out is the consumer contract. Never mark success/deep before it is visible.
                write_atomic(&settings.out, &bytes)?;
                if let Some((history_bytes, health_bytes)) = sidecars {
                    write_atomic(&history, &history_bytes)?;
                    write_atomic(&health, &health_bytes)?;
                }
                controller = next;
                Ok(())
            });
        let mut delay = every;
        match result {
            Ok(()) => {
                if settings.once {
                    return Ok(());
                }
            }
            Err(error) => {
                if let Some(controller) = &mut controller {
                    delay = controller
                        .record_failure(&error.to_string())
                        .map_err(io::Error::other)?;
                    let mut status = controller.health(delay);
                    status["deep_skip_reason"] = serde_json::json!(deep_skip);
                    // A full/unwritable disk may prevent health persistence too. Keep
                    // retrying in memory and expose that failure on stderr instead of exiting.
                    let persisted = controller
                        .encode()
                        .map_err(io::Error::other)
                        .and_then(|bytes| write_atomic(&history, &bytes));
                    if let Err(state_error) = persisted {
                        eprintln!("archie-lan: history: {state_error}");
                    }
                    if let Err(health_error) = serde_json::to_vec_pretty(&status)
                        .map_err(io::Error::other)
                        .and_then(|bytes| write_atomic(&health, &bytes))
                    {
                        eprintln!("archie-lan: health: {health_error}");
                    }
                }
                if settings.once {
                    return Err(error);
                }
                eprintln!("archie-lan: {error}; retry in {delay}s");
            }
        }
        std::thread::sleep(Duration::from_secs(delay));
    }
}

#[cfg(all(test, unix))]
mod path_tests {
    use super::*;
    #[test]
    fn symlink_parent_traversal_cannot_hide_a_path_collision() {
        let dir = std::env::temp_dir().join(format!("sentry-paths-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("real/inner")).unwrap();
        std::os::unix::fs::symlink(dir.join("real/inner"), dir.join("alias")).unwrap();
        assert!(
            validate_paths(&[&dir.join("alias/../lan.json"), &dir.join("real/lan.json")]).is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
