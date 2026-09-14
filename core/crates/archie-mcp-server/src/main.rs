//! `archie-mcp-server` — Archie for IDE agents, over stdio.
//!
//! ```text
//! archie-mcp-server [--clearance public|internal] [--root DIR]
//!                   [--stage-socket PATH]
//!                   [--allow-stage-actions --stage-key FILE [--stage-policy FILE]]
//! ```
//!
//! Standard output carries protocol messages and nothing else; diagnostics go to standard
//! error, where IDEs show a server's log.

use archie_mcp_server::{ArchieToolbox, Clearance, McpServer, MAX_MESSAGE_BYTES};
use archie_policy::PolicyEngine;
use archie_stage::{CommandKey, Operator, StagePolicy};
use archie_state::StatePaths;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};

const USAGE: &str = "usage: archie-mcp-server [--clearance public|internal] [--root DIR]
                         [--stage-socket PATH]
                         [--allow-stage-actions --stage-key FILE [--stage-policy FILE]]

  --clearance            the most sensitive data a tool may return (default: public).
                         internal only when the IDE's model provider retention is verified.
  --root                 an ArchieOS tree other than the system one, as archied --root.
  --stage-socket         the Archie Stage control socket
                         (default: $XDG_RUNTIME_DIR/archie-stage/control.sock).
  --allow-stage-actions  offer stage_act, which operates applications. Needs --stage-key.
  --stage-key            the stage command key file (64 hex characters, mode 0600).
  --stage-policy         the stage policy (default: /etc/archie/stage/apps.toml).";

struct Config {
    clearance: Clearance,
    root: Option<PathBuf>,
    stage_socket: Option<PathBuf>,
    allow_stage_actions: bool,
    stage_key: Option<PathBuf>,
    stage_policy: PathBuf,
}

fn parse(args: Vec<String>) -> Result<Option<Config>, String> {
    let mut config = Config {
        clearance: Clearance::Public,
        root: None,
        stage_socket: None,
        allow_stage_actions: false,
        stage_key: None,
        stage_policy: PathBuf::from("/etc/archie/stage/apps.toml"),
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| format!("{flag} needs a value"));
        match arg.as_str() {
            "--clearance" => {
                config.clearance = Clearance::parse(&value("--clearance")?).ok_or(
                    "clearance is public or internal: nothing more sensitive leaves through this server",
                )?;
            }
            "--root" => config.root = Some(PathBuf::from(value("--root")?)),
            "--stage-socket" => config.stage_socket = Some(PathBuf::from(value("--stage-socket")?)),
            "--stage-key" => config.stage_key = Some(PathBuf::from(value("--stage-key")?)),
            "--stage-policy" => config.stage_policy = PathBuf::from(value("--stage-policy")?),
            "--allow-stage-actions" => config.allow_stage_actions = true,
            "--help" | "-h" => return Ok(None),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if !cfg!(unix) && (config.stage_socket.is_some() || config.allow_stage_actions) {
        return Err("the Archie Stage runs on Linux; stage tools are not available here".into());
    }
    if cfg!(unix) && config.stage_socket.is_none() {
        config.stage_socket = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(|dir| PathBuf::from(dir).join("archie-stage/control.sock"));
    }
    if config.allow_stage_actions && config.stage_key.is_none() {
        return Err("--allow-stage-actions needs --stage-key".into());
    }
    if config.allow_stage_actions && config.stage_socket.is_none() {
        return Err("--allow-stage-actions needs --stage-socket, or XDG_RUNTIME_DIR set".into());
    }
    Ok(Some(config))
}

fn load_key(path: &PathBuf) -> Result<CommandKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{}: a stage key is 64 hex characters", path.display()));
    }
    let mut bytes = [0_u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| "not hex".to_string())?;
    }
    Ok(CommandKey::from_bytes(&bytes))
}

enum Line {
    Complete,
    TooLong,
    End,
}

/// Reads one line into `out`, keeping at most `limit` bytes of it. A longer line is read to
/// its end and dropped, so the next message still starts where it should.
async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R, out: &mut Vec<u8>, limit: usize) -> std::io::Result<Line> {
    out.clear();
    let mut too_long = false;
    let mut read_any = false;
    loop {
        let (consumed, newline) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(match (read_any, too_long) {
                    (false, _) => Line::End,
                    (true, true) => Line::TooLong,
                    (true, false) => Line::Complete,
                });
            }
            read_any = true;
            let (end, newline) = match available.iter().position(|b| *b == b'\n') {
                Some(at) => (at, true),
                None => (available.len(), false),
            };
            if !too_long {
                if out.len() + end > limit {
                    too_long = true;
                    out.clear();
                } else {
                    out.extend_from_slice(&available[..end]);
                }
            }
            (if newline { end + 1 } else { end }, newline)
        };
        reader.consume(consumed);
        if newline {
            return Ok(if too_long { Line::TooLong } else { Line::Complete });
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let config = match parse(std::env::args().skip(1).collect()) {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("archie-mcp-server: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let operator = if config.allow_stage_actions {
        let key = match config.stage_key.as_ref().map(load_key) {
            Some(Ok(key)) => key,
            Some(Err(message)) => {
                eprintln!("archie-mcp-server: {message}");
                return ExitCode::from(2);
            }
            None => unreachable!("parse requires a key with --allow-stage-actions"),
        };
        let policy = match StagePolicy::load(&config.stage_policy) {
            Ok(policy) => policy,
            Err(error) => {
                eprintln!("archie-mcp-server: {}: {error}", config.stage_policy.display());
                return ExitCode::from(2);
            }
        };
        Some(Operator::new(key, policy, PolicyEngine::default()))
    } else {
        None
    };
    let paths = match &config.root {
        Some(root) => StatePaths::rooted(root),
        None => StatePaths::from_env(),
    };
    let toolbox = ArchieToolbox::new(paths, config.stage_socket.clone(), operator);
    let mut server = McpServer::new(toolbox, config.clearance);
    eprintln!(
        "archie-mcp-server: ready, clearance {:?}, stage actions {}",
        config.clearance,
        if config.allow_stage_actions { "on" } else { "off" }
    );

    let mut reader = BufReader::with_capacity(64 * 1024, tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut line = Vec::with_capacity(4096);
    loop {
        let reply = match read_line(&mut reader, &mut line, MAX_MESSAGE_BYTES).await {
            Ok(Line::End) => break,
            Ok(Line::TooLong) => Some(
                "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"message too large\"}}".to_string(),
            ),
            Ok(Line::Complete) => match std::str::from_utf8(&line) {
                Ok(text) if text.trim().is_empty() => None,
                Ok(text) => server.handle(text.trim_end_matches('\r')).await,
                Err(_) => Some(
                    "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"not UTF-8\"}}".to_string(),
                ),
            },
            Err(error) => {
                eprintln!("archie-mcp-server: stdin: {error}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(reply) = reply {
            let written = async {
                stdout.write_all(reply.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await
            }
            .await;
            if written.is_err() {
                // The IDE went away.
                break;
            }
        }
    }
    ExitCode::SUCCESS
}
