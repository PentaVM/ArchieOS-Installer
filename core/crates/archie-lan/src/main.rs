//! `archie-lan` — what is on the owner's network.
//!
//! ```text
//! archie-lan scan  [--seconds 4]                       one inventory, as JSON on standard output
//! archie-lan watch --out FILE [--every 300] [--seconds 4]   the service: rewrite FILE with each inventory
//! ```
//!
//! The inventory is `archie-lan-inventory-v1`: one entry per device with its address, its
//! family when it said enough to tell, its name and model, and what it reports of itself.

use archie_lan::inventory::scan;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

fn take(arguments: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = arguments.iter().position(|a| a == flag)?;
    (at + 1 < arguments.len()).then(|| {
        arguments.remove(at);
        arguments.remove(at)
    })
}

/// The inventory says what is in the house and what it is doing: for its owner only.
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

fn main() -> ExitCode {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let seconds = take(&mut arguments, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(4u64).clamp(1, 30);
    let listen = Duration::from_secs(seconds);
    match arguments.first().map(String::as_str) {
        Some("scan") => match scan(listen) {
            Ok(inventory) => {
                println!("{}", serde_json::to_string_pretty(&inventory).expect("an inventory serializes"));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("archie-lan: {error}");
                ExitCode::FAILURE
            }
        },
        Some("watch") => {
            let Some(out) = take(&mut arguments, "--out").map(PathBuf::from) else {
                eprintln!("archie-lan: watch needs --out FILE");
                return ExitCode::from(2);
            };
            let every = take(&mut arguments, "--every").and_then(|s| s.parse().ok()).unwrap_or(300u64).max(30);
            loop {
                match scan(listen) {
                    Ok(inventory) => {
                        // Beside the file, then renamed: a reader never sees half an inventory.
                        let partial = out.with_extension("partial");
                        let written = write_private(&partial, &serde_json::to_vec_pretty(&inventory).expect("an inventory serializes"))
                            .and_then(|()| std::fs::rename(&partial, &out));
                        if let Err(error) = written {
                            eprintln!("archie-lan: {}: {error}", out.display());
                        }
                    }
                    Err(error) => eprintln!("archie-lan: {error}"),
                }
                std::thread::sleep(Duration::from_secs(every));
            }
        }
        _ => {
            eprintln!("usage: archie-lan scan [--seconds 4]\n       archie-lan watch --out FILE [--every 300] [--seconds 4]");
            ExitCode::from(2)
        }
    }
}
