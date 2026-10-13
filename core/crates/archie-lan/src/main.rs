//! Sentry's bounded LAN inventory. Discovery never grants device control.
use archie_lan::{
    inventory::{scan_with, ScanOptions},
    presence::Subnet,
};
use clap::{Args, Parser, Subcommand};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

#[derive(Parser)]
#[command(about = "Discover local devices using Sentry; no device control or credentials")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// One bounded inventory on stdout. Diagnostics are written separately.
    Scan(Options),
    /// Refresh an inventory atomically; does not enable any system service.
    Watch {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(30..))]
        every: u64,
        #[command(flatten)]
        options: Options,
    },
    /// List local IPv4 interface addresses usable by --bind (no device inventory).
    Interfaces,
}

#[derive(Args)]
struct Options {
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u64).range(1..=30))]
    seconds: u64,
    /// Select a private local IPv4 interface instead of the default route.
    #[arg(long, value_parser = local_address)]
    bind: Option<Ipv4Addr>,
    /// Connection-only TCP discovery on this interface's subnet, at most /24.
    #[arg(long, conflicts_with = "subnet")]
    probe_local: bool,
    /// Explicit private IPv4/CIDR scope containing the local interface, /24..=/32.
    #[arg(long)]
    subnet: Option<Subnet>,
    /// TCP discovery deadline; partial coverage is explicitly reported.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=120))]
    probe_seconds: u64,
    /// Private JSON sidecar: source errors, multicast availability and probe coverage.
    #[arg(long)]
    diagnostics: Option<PathBuf>,
}

fn local_address(raw: &str) -> Result<Ipv4Addr, String> {
    let address: Ipv4Addr = raw.parse().map_err(|_| "invalid IPv4 interface address")?;
    archie_lan::is_lan_v4(address)
        .then_some(address)
        .ok_or_else(|| "interface must be private or link-local".into())
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

fn run(options: &Options) -> std::io::Result<Vec<u8>> {
    let report = scan_with(
        ScanOptions {
            bind: options.bind,
            probe_local: options.probe_local,
            subnet: options.subnet,
            probe_seconds: options.probe_seconds,
        },
        Duration::from_secs(options.seconds),
    )?;
    for (name, source) in &report.diagnostics.sources {
        if let Some(error) = &source.error {
            eprintln!("archie-lan: {name}: {error}");
        }
    }
    if let Some(probe) = &report.diagnostics.presence {
        if !probe.completed {
            eprintln!(
                "archie-lan: TCP discovery reached its deadline; coverage is partial ({} / {} hosts, {} / {} port probes)",
                probe.hosts_attempted, probe.hosts_total, probe.probes_attempted, probe.probes_total
            );
        }
    }
    if let Some(path) = &options.diagnostics {
        write_private(path, &serde_json::to_vec_pretty(&report.diagnostics)?)?;
    }
    if report
        .diagnostics
        .sources
        .values()
        .all(|source| !source.available)
    {
        return Err(std::io::Error::other(
            "no discovery source could start; see diagnostics",
        ));
    }
    Ok(serde_json::to_vec_pretty(&report.inventory)?)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Scan(options) => {
            run(&options).map(|bytes| println!("{}", String::from_utf8_lossy(&bytes)))
        }
        Command::Watch {
            out,
            every,
            options,
        } => loop {
            match run(&options) {
                Ok(bytes) => {
                    let partial = out.with_extension("partial");
                    if let Err(error) = write_private(&partial, &bytes)
                        .and_then(|()| std::fs::rename(&partial, &out))
                    {
                        eprintln!("archie-lan: {}: {error}", out.display());
                    }
                }
                Err(error) => eprintln!("archie-lan: {error}"),
            }
            std::thread::sleep(Duration::from_secs(every));
        },
        Command::Interfaces => if_addrs::get_if_addrs().map(|interfaces| {
            for interface in interfaces {
                if let if_addrs::IfAddr::V4(address) = interface.addr {
                    if archie_lan::is_lan_v4(address.ip) {
                        println!("{}\t{}\t{}", interface.name, address.ip, address.netmask);
                    }
                }
            }
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("archie-lan: {error}");
            ExitCode::FAILURE
        }
    }
}
