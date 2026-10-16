//! `archie-skill-node` — one person's GPU, training IRIS skill adapters for the shared board.
//!
//! ```text
//! archie-skill-node mcp --config node.toml        serve the tools over stdio (an agent runs this through ssh)
//! archie-skill-node init --config node.toml --contributor NAME [--gpu "RTX 3070 8GB"]
//!                                                 write a config to fill in
//! archie-skill-node client --host person@machine --config PATH [--name iris-NAME]
//!                                                 print the lines that connect an agent to this node
//! ```

use archie_skill_node::hub::GitRepo;
use archie_skill_node::mcp;
use archie_skill_node::node::{valid_name, NodeConfig};
use archie_skill_node::tools::{now, Node};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|a| a == flag)?;
    args.remove(at);
    (at < args.len()).then(|| args.remove(at))
}

fn serve(config: NodeConfig) -> ExitCode {
    if !config.board_repo.join(".git").exists() {
        eprintln!("archie-skill-node: {} is not a clone of the shared repository (git clone it there first)",
                  config.board_repo.display());
        return ExitCode::from(2);
    }
    let hub = GitRepo::new(&config.board_repo, &config.board_branch);
    eprintln!(
        "archie-skill-node: ready for {} on {}",
        config.contributor,
        config.board_repo.display()
    );
    let mut node = Node {
        config,
        hub,
        clock: now,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = if line.len() > mcp::MAX_MESSAGE_BYTES {
            Some(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"message too large"}}"#.to_string())
        } else {
            mcp::handle(&mut node, line.trim_end_matches('\r'))
        };
        if let Some(reply) = reply {
            if writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break; // the agent went away
            }
        }
    }
    ExitCode::SUCCESS
}

const TEMPLATE: &str = r#"# An IRIS skill node. Fill in the paths. board_repo is your own clone of the shared
# repository: pushes use your git identity, so the history credits you.
contributor = "{contributor}"
gpu = "{gpu}"
python = "C:/iris/venv/Scripts/python.exe"
foundry = "C:/iris/archie-iris"
weights = "C:/iris/base/iris-beta2.pt"
skills_dir = "C:/iris/skills"
work_dir = "C:/iris/node"
board_repo = "C:/iris/iris-skills"
board_branch = "main"
# Where the shared repository's files are served over HTTPS, for the catalog IRIS downloads
# from; leave empty until the repository is public.
public_url = ""
"#;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let command = if args.is_empty() {
        String::new()
    } else {
        args.remove(0)
    };
    let config_path = take(&mut args, "--config").map(PathBuf::from);
    match command.as_str() {
        "mcp" => {
            let Some(path) = config_path else {
                eprintln!("archie-skill-node mcp --config node.toml");
                return ExitCode::from(2);
            };
            match NodeConfig::load(&path) {
                Ok(config) => serve(config),
                Err(message) => {
                    eprintln!("archie-skill-node: {message}");
                    ExitCode::from(2)
                }
            }
        }
        "init" => {
            let (Some(path), Some(contributor)) = (config_path, take(&mut args, "--contributor"))
            else {
                eprintln!(
                    "archie-skill-node init --config node.toml --contributor NAME [--gpu TEXT]"
                );
                return ExitCode::from(2);
            };
            if !valid_name(&contributor) {
                eprintln!(
                    "archie-skill-node: the contributor name is 1-32 letters, digits, '-' or '_'"
                );
                return ExitCode::from(2);
            }
            if path.exists() {
                eprintln!(
                    "archie-skill-node: {} exists; not overwritten",
                    path.display()
                );
                return ExitCode::from(2);
            }
            let gpu = take(&mut args, "--gpu")
                .unwrap_or_else(|| "RTX 3060 Ti 8GB".into())
                .replace('"', "");
            let text = TEMPLATE
                .replace("{contributor}", &contributor)
                .replace("{gpu}", &gpu);
            match std::fs::write(&path, text) {
                Ok(()) => {
                    println!(
                        "wrote {}; fill in the paths and clone the shared repository to board_repo",
                        path.display()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("archie-skill-node: {}: {error}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        "client" => {
            let (Some(path), Some(host)) = (config_path, take(&mut args, "--host")) else {
                eprintln!("archie-skill-node client --host person@machine --config PATH [--name iris-NAME]");
                return ExitCode::from(2);
            };
            let name = take(&mut args, "--name").unwrap_or_else(|| "iris-node".into());
            let remote = format!("archie-skill-node mcp --config {}", path.display());
            println!("Claude Code:\n  claude mcp add {name} -- ssh -T {host} \"{remote}\"\n");
            println!("Any MCP client (JSON):");
            let mut servers = serde_json::Map::new();
            servers.insert(
                name,
                serde_json::json!({"command": "ssh", "args": ["-T", host, remote]}),
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"mcpServers": servers}))
                    .unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: archie-skill-node mcp|init|client --config node.toml ...");
            ExitCode::from(2)
        }
    }
}
