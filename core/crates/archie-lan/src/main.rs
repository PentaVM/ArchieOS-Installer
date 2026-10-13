//! Sentry's bounded LAN inventory. Discovery never grants device control.
use archie_lan::{
    dns_sd::Backend,
    inventory::{scan_with, ScanOptions, ScanReport},
    presence::Subnet,
};
mod watch;
use clap::{Args, Parser, Subcommand};
use std::{net::Ipv4Addr, path::PathBuf, process::ExitCode, time::Duration};

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
        #[arg(long, value_parser = clap::value_parser!(u64).range(30..=3600))]
        every: Option<u64>,
        /// Recover observation history and schedule light/deep discovery independently.
        #[arg(long)]
        autonomous: bool,
        /// Perform one bounded iteration and exit, useful for validation.
        #[arg(long)]
        once: bool,
        #[arg(long, requires = "autonomous")]
        history: Option<PathBuf>,
        #[arg(long, requires = "autonomous")]
        health: Option<PathBuf>,
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(30..=86400))]
        deep_every: u64,
        #[command(flatten)]
        options: Options,
    },
    /// List local IPv4 interface addresses usable by --bind (no device inventory).
    Interfaces,
}

#[derive(Args, Clone)]
struct Options {
    /// Keep native discovery, use upstream DNS-SD, or combine both.
    #[arg(long, value_enum)]
    mdns_backend: Option<Backend>,
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
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=120))]
    probe_seconds: Option<u64>,
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

fn run_report(options: &Options) -> std::io::Result<ScanReport> {
    let report = scan_with(
        ScanOptions {
            bind: options.bind,
            probe_local: options.probe_local,
            subnet: options.subnet,
            probe_seconds: options.probe_seconds.unwrap_or(15),
            mdns_backend: options.mdns_backend.unwrap_or_default(),
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
        watch::write_atomic(path, &serde_json::to_vec_pretty(&report.diagnostics)?)?;
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
    Ok(report)
}

fn run(options: &Options) -> std::io::Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&run_report(options)?.inventory)?)
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
            autonomous,
            once,
            history,
            health,
            deep_every,
            options,
        } => watch::run(
            watch::Settings {
                out,
                every,
                autonomous,
                once,
                history,
                health,
                deep_every,
            },
            options,
        ),
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
