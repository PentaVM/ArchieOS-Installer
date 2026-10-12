//! `archie-desk` — where Archie's results land on the person's computer.
//!
//! ```text
//! archie-desk pair  [--key FILE]
//! archie-desk serve [--root DIR] [--listen 127.0.0.1:7447] [--key FILE] [--config FILE]
//! ```

use archie_bridges::delivery::{DeliveryKey, DEFAULT_PORT};
use archie_desk::{open::Rules, Desk};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  archie-desk pair  [--key FILE]      make the key this computer and an Archie node share
  archie-desk serve [--root DIR] [--listen 127.0.0.1:7447] [--key FILE] [--config FILE]
                                      receive deliveries into DIR (default ~/Archie)

Defaults: key ~/.archie/desk.key, config ~/.archie/desk.toml (optional; its [open] table
says which kinds of file to open, and with what).";

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = match args.split_first() {
        Some((command, rest)) => (command.as_str(), rest),
        None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
    };
    let mut key = home().join(".archie").join("desk.key");
    let mut root = home().join("Archie");
    let mut config = home().join(".archie").join("desk.toml");
    let mut listen: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        let Some(value) = rest.next() else {
            eprintln!("archie-desk: {flag} needs a value\n{USAGE}");
            return ExitCode::from(2);
        };
        match flag.as_str() {
            "--key" => key = PathBuf::from(value),
            "--root" => root = PathBuf::from(value),
            "--config" => config = PathBuf::from(value),
            "--listen" => match value.parse() {
                Ok(address) => listen = address,
                Err(_) => {
                    eprintln!("archie-desk: --listen is an address such as 127.0.0.1:7447");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!("archie-desk: unknown flag {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    match command {
        "pair" => pair(&key),
        "serve" => serve(root, key, config, listen),
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("archie-desk: unknown command {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn pair(key_file: &Path) -> ExitCode {
    if key_file.exists() {
        eprintln!(
            "archie-desk: {} exists; it already pairs this computer. Delete it first to pair anew.",
            key_file.display()
        );
        return ExitCode::from(2);
    }
    if let Some(dir) = key_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let key = DeliveryKey::generate();
    if let Err(e) = key.save(key_file) {
        eprintln!("archie-desk: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "Paired key written to {file}.\n\n\
         On the Archie node, put the same 64 characters in a file only its owner can read:\n\n\
         \x20 install -m 600 /dev/null /etc/archie/deliver.key && cat > /etc/archie/deliver.key\n\n\
         (paste the contents of {file}, then Ctrl-D), and serve bridges with\n\n\
         \x20 archie-mcp-server --deliver-key /etc/archie/deliver.key …\n\n\
         Then, from this computer, keep the tunnel open while Archie works:\n\n\
         \x20 ssh -N -R {port}:127.0.0.1:{port} <you>@<node>\n\n\
         and run `archie-desk serve`.",
        file = key_file.display(),
        port = DEFAULT_PORT,
    );
    ExitCode::SUCCESS
}

fn serve(root: PathBuf, key_file: PathBuf, config: PathBuf, listen: SocketAddr) -> ExitCode {
    if !listen.ip().is_loopback() {
        eprintln!(
            "archie-desk: {listen} is not loopback. The desk listens on loopback only; the node \
             reaches it through `ssh -R`."
        );
        return ExitCode::from(2);
    }
    let key = match DeliveryKey::load(&key_file) {
        Ok(key) => key,
        Err(e) => {
            eprintln!("archie-desk: {e}. Run `archie-desk pair` first.");
            return ExitCode::from(2);
        }
    };
    let rules = match std::fs::read_to_string(&config) {
        Ok(text) => match Rules::parse(&text) {
            Ok(rules) => rules,
            Err(e) => {
                eprintln!("archie-desk: {}: {e}", config.display());
                return ExitCode::from(2);
            }
        },
        Err(_) => Rules::default(),
    };
    let desk = match Desk::new(root, key, rules.clone()) {
        Ok(desk) => desk,
        Err(e) => {
            eprintln!("archie-desk: {e}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("archie-desk: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(listen).await {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("archie-desk: {listen}: {e}");
                return ExitCode::FAILURE;
            }
        };
        eprintln!(
            "archie-desk: receiving on {listen} into {}; opens {}",
            desk.root().display(),
            if rules.open.is_empty() {
                "nothing (no [open] rules)".to_string()
            } else {
                rules
                    .open
                    .keys()
                    .map(|k| format!(".{k}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        );
        match axum::serve(listener, desk.router()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("archie-desk: {e}");
                ExitCode::FAILURE
            }
        }
    })
}
