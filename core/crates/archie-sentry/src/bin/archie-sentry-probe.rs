//! Diagnostics only: no physical writes and no deserializable authority.
use archie_sentry::adapters::{
    ipp::IppClient,
    process::ProcessRunner,
    router::{probe_router, IgdClient, SshChannel, SshConfig},
};
use std::{net::Ipv4Addr, sync::Arc};

fn usage() -> ! {
    eprintln!(
        "usage:\n  archie-sentry-probe ipp <configured LAN printer URI>\n  \
         archie-sentry-probe router [--control-url <URI>] \
         [--ssh-host <IPv4> --ssh-user <USER> [--ssh-key <PATH>] [--ssh-port <PORT>] [--ssh-binary <PATH>]]"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "ipp" => {
            if args.len() != 3 {
                usage();
            }
            let result = match IppClient::new(&args[2]) {
                Ok(client) => client.get_printer_attributes(1).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(snapshot) => println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot).expect("typed snapshot encoding")
                ),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
        }
        "router" => {
            let mut control_url: Option<String> = None;
            let mut ssh_host: Option<String> = None;
            let mut ssh_user: Option<String> = None;
            let mut ssh_key: Option<String> = None;
            let mut ssh_port: Option<u16> = None;
            let mut ssh_binary: Option<String> = None;
            let mut idx = 2;
            while idx < args.len() {
                let flag = args[idx].as_str();
                let Some(val) = args.get(idx + 1) else {
                    usage();
                };
                match flag {
                    "--control-url" => control_url = Some(val.clone()),
                    "--ssh-host" => ssh_host = Some(val.clone()),
                    "--ssh-user" => ssh_user = Some(val.clone()),
                    "--ssh-key" => ssh_key = Some(val.clone()),
                    "--ssh-port" => {
                        ssh_port = Some(val.parse::<u16>().unwrap_or_else(|_| usage()));
                    }
                    "--ssh-binary" => ssh_binary = Some(val.clone()),
                    _ => usage(),
                }
                idx += 2;
            }
            if control_url.is_none() && ssh_host.is_none() {
                usage();
            }
            if ssh_host.is_some() != ssh_user.is_some() {
                usage();
            }
            if ssh_host.is_none()
                && (ssh_key.is_some() || ssh_port.is_some() || ssh_binary.is_some())
            {
                usage();
            }
            let igd = match &control_url {
                Some(url) => match IgdClient::new(url) {
                    Ok(client) => Some(client),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(2);
                    }
                },
                None => None,
            };
            let ssh = match (&ssh_host, &ssh_user) {
                (Some(host_str), Some(user)) => {
                    let host: Ipv4Addr = host_str.parse().unwrap_or_else(|_| {
                        eprintln!("invalid router IPv4 address");
                        std::process::exit(2);
                    });
                    let default_binary = if cfg!(windows) {
                        "C:/Windows/System32/OpenSSH/ssh.exe"
                    } else {
                        "/usr/bin/ssh"
                    };
                    let binary = ssh_binary.as_deref().unwrap_or(default_binary);
                    let mut config = match SshConfig::new(binary, user, host, ssh_key) {
                        Ok(cfg) => cfg,
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(2);
                        }
                    };
                    if let Some(port) = ssh_port {
                        config = match config.with_port(port) {
                            Ok(cfg) => cfg,
                            Err(error) => {
                                eprintln!("{error}");
                                std::process::exit(2);
                            }
                        };
                    }
                    Some(SshChannel::new(config, Arc::new(ProcessRunner::default())))
                }
                _ => None,
            };
            let capabilities = probe_router(igd.as_ref(), ssh.as_ref()).await;
            println!(
                "{}",
                serde_json::to_string_pretty(&capabilities).expect("typed capabilities encoding")
            );
            if !capabilities.igd && !capabilities.ssh {
                std::process::exit(1);
            }
        }
        _ => usage(),
    }
}
