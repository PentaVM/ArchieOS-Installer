//! archie-shell-bridge watch [--displayd 127.0.0.1:7666] [--every SECS] [--lan-file FILE] [--workspace DIR]... [--workspaces-root DIR] [--once]
//!
//! One JSON line per poll on standard output (`archie_shell_bridge::Frame`). The shell
//! starts this with a fixed argument list and reads the lines. It writes nothing but
//! frames to standard output; diagnostics go to standard error.

#![forbid(unsafe_code)]

use archie_shell_bridge::{poll_with, Client, Sources};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser)]
#[command(about = "Read-only adapter between the shell and archie-displayd")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Poll archie-displayd and print one JSON frame per poll.
    Watch {
        /// Loopback address of archie-displayd. Any other address is refused.
        #[arg(long, default_value = "127.0.0.1:7666")]
        displayd: SocketAddr,
        /// Seconds between polls (1 to 300).
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..=300))]
        every: u64,
        /// The inventory file `archie-lan watch` writes. Read only; no scan is started.
        /// Without it the frame says the LAN inventory is not configured.
        #[arg(long, value_name = "FILE")]
        lan_file: Option<PathBuf>,
        /// A workspace whose long-job status (`.archie/jobs`) is shown. Repeatable (up to 8).
        /// Read only: jobs are never cancelled from here.
        #[arg(long = "workspace", value_name = "DIR")]
        workspaces: Vec<PathBuf>,
        /// A folder whose subfolders are workspaces, e.g. /archie/workspaces. Listed on every
        /// poll, symlinks not followed.
        #[arg(long, value_name = "DIR")]
        workspaces_root: Option<PathBuf>,
        /// Print one frame and exit.
        #[arg(long)]
        once: bool,
    },
}

fn main() -> ExitCode {
    let Command::Watch {
        displayd,
        every,
        lan_file,
        workspaces,
        workspaces_root,
        once,
    } = Cli::parse().command;
    let client = match Client::new(displayd) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("archie-shell-bridge: {e}");
            return ExitCode::from(64);
        }
    };
    let sources = Sources {
        lan_file,
        workspaces,
        workspaces_root,
    };
    let stdout = std::io::stdout();
    loop {
        let frame = poll_with(&client, &sources);
        let mut out = stdout.lock();
        let line = match serde_json::to_string(&frame) {
            Ok(line) => line,
            Err(e) => {
                eprintln!("archie-shell-bridge: could not encode a frame: {e}");
                return ExitCode::FAILURE;
            }
        };
        // A closed pipe means the shell went away: stop quietly.
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            return ExitCode::SUCCESS;
        }
        drop(out);
        if once {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(Duration::from_secs(every));
    }
}
