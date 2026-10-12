//! archie-shell-bridge watch [--displayd 127.0.0.1:7666] [--every SECS] [--once]
//!
//! One JSON line per poll on standard output (`archie_shell_bridge::Frame`). The shell
//! starts this with a fixed argument list and reads the lines. It writes nothing but
//! frames to standard output; diagnostics go to standard error.

#![forbid(unsafe_code)]

use archie_shell_bridge::{poll, Client};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::net::SocketAddr;
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
        /// Print one frame and exit.
        #[arg(long)]
        once: bool,
    },
}

fn main() -> ExitCode {
    let Command::Watch {
        displayd,
        every,
        once,
    } = Cli::parse().command;
    let client = match Client::new(displayd) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("archie-shell-bridge: {e}");
            return ExitCode::from(64);
        }
    };
    let stdout = std::io::stdout();
    loop {
        let frame = poll(&client);
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
