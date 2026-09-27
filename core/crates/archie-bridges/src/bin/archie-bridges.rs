//! `archie-bridges` — the bridges from a shell.
//!
//! ```text
//! archie-bridges list
//! archie-bridges guide <bridge>
//! archie-bridges call <bridge> <tool> [<json arguments>] --workspace DIR [--program NAME=PATH]…
//! archie-bridges run-job --workspace DIR --job ID [--program …] [--deliver-key FILE]
//! archie-bridges index [--write FILE]
//! archie-bridges new <id> [--root REPO]
//! ```

use archie_bridges::{index, jobs, open, Manifest, Programs, BUILTIN};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  archie-bridges list                         the bridges in this build
  archie-bridges guide <bridge>               a bridge's SKILL.md
  archie-bridges call <bridge> <tool> [JSON] --workspace DIR [--program NAME=PATH]...
                                              run one tool; prints its answer, exit 1 if it failed
  archie-bridges run-job --workspace DIR --job ID [--program ...] [--deliver-key FILE --deliver-to ADDR]
                                              a long job a bridge started (run by the bridge, detached)
  archie-bridges index [--write FILE]         docs/bridges/INDEX.md, generated from the bridges
  archie-bridges new <id> [--root REPO]       the skeleton of a new bridge, per docs/BRIDGES.md";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("archie-bridges: {message}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<ExitCode, String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("list") => {
            for builtin in BUILTIN {
                let manifest = Manifest::parse(builtin.manifest).map_err(|e| e.to_string())?;
                println!("{:<10} {}", manifest.id, manifest.summary);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("guide") => {
            let id = args.next().ok_or("guide needs a bridge")?;
            let builtin = BUILTIN
                .iter()
                .find(|b| b.id == id)
                .ok_or_else(|| format!("no bridge called {id}"))?;
            print!("{}", builtin.guide);
            Ok(ExitCode::SUCCESS)
        }
        Some("call") => call(args.collect()),
        Some("run-job") => run_job(args.collect()),
        Some("index") => {
            let page = index::render();
            match (args.next().as_deref(), args.next()) {
                (Some("--write"), Some(file)) => {
                    std::fs::write(&file, page).map_err(|e| format!("{file}: {e}"))?
                }
                (None, _) => print!("{page}"),
                _ => return Err("index takes --write FILE or nothing".into()),
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("new") => scaffold(args.collect()),
        Some("--help" | "-h" | "help") | None => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("unknown command {other:?}")),
    }
}

/// The flags `call` and `run-job` share: the workspace, programs and the desk.
struct Setup {
    positional: Vec<String>,
    workspace: Option<PathBuf>,
    job: Option<String>,
    programs: Programs,
}

fn setup(args: Vec<String>) -> Result<Setup, String> {
    let mut found = Setup {
        positional: Vec::new(),
        workspace: None,
        job: None,
        programs: Programs::default(),
    };
    let mut deliver_key = None;
    let mut deliver_to: std::net::SocketAddr =
        ([127, 0, 0, 1], archie_bridges::delivery::DEFAULT_PORT).into();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| format!("{flag} needs a value"));
        match arg.as_str() {
            "--workspace" => found.workspace = Some(PathBuf::from(value("--workspace")?)),
            "--job" => found.job = Some(value("--job")?),
            "--program" => {
                let pair = value("--program")?;
                let (name, path) = pair.split_once('=').ok_or("--program is NAME=PATH")?;
                found.programs.set(name, path);
            }
            "--deliver-key" => deliver_key = Some(PathBuf::from(value("--deliver-key")?)),
            "--deliver-to" => {
                deliver_to = value("--deliver-to")?
                    .parse()
                    .map_err(|_| "--deliver-to is an address such as 127.0.0.1:7447")?
            }
            _ => found.positional.push(arg),
        }
    }
    found.programs.delivery = deliver_key.map(|key| (key, deliver_to));
    Ok(found)
}

fn call(args: Vec<String>) -> Result<ExitCode, String> {
    let Setup {
        positional,
        workspace,
        programs,
        ..
    } = setup(args)?;
    let [bridge, tool, rest @ ..] = positional.as_slice() else {
        return Err("call needs a bridge and a tool".into());
    };
    let arguments: Map<String, Value> = match rest {
        [] => Map::new(),
        [json] => match serde_json::from_str(json) {
            Ok(Value::Object(map)) => map,
            _ => return Err("the arguments are one JSON object".into()),
        },
        _ => return Err("the arguments are one JSON object, quoted".into()),
    };
    let workspace = workspace.ok_or("call needs --workspace DIR")?;
    let bridge = open(bridge, &workspace, &programs).map_err(|e| e.to_string())?;
    if !bridge.tools().iter().any(|t| t.name == tool) {
        let names: Vec<_> = bridge.tools().iter().map(|t| t.name).collect();
        return Err(format!("{tool} is not one of {}", names.join(", ")));
    }
    let outcome = bridge.call(tool, &arguments);
    println!("{}", outcome.text);
    for image in &outcome.images {
        eprintln!("(and a {} image of {} bytes)", image.mime, image.data.len());
    }
    Ok(if outcome.is_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// A long job, started detached by a bridge's tool.
fn run_job(args: Vec<String>) -> Result<ExitCode, String> {
    let Setup {
        workspace,
        job,
        programs,
        ..
    } = setup(args)?;
    let folder = workspace.ok_or("run-job needs --workspace DIR")?;
    let id = job.ok_or("run-job needs --job ID")?;
    let workspace = archie_bridges::Workspace::open(&folder).map_err(|e| e.to_string())?;
    let (mut runner, spec) = jobs::Runner::open(&workspace, &id).map_err(|e| e.to_string())?;
    let bridge = open(&spec.bridge, &folder, &programs).map_err(|e| e.to_string())?;
    eprintln!("archie-bridges: job {id} ({} {}) started", spec.bridge, spec.kind);
    let result = bridge.run_job(&spec, &mut runner);
    let cancelled = runner.cancelled();
    let (state, error) = match result {
        Ok(()) if cancelled => (jobs::State::Cancelled, None),
        Ok(()) => (jobs::State::Done, None),
        Err(why) => (jobs::State::Failed, Some(why)),
    };
    eprintln!("archie-bridges: job {id} ended: {state:?} {}", error.as_deref().unwrap_or(""));
    let failed = state == jobs::State::Failed;
    runner.finish(state, error).map_err(|e| e.to_string())?;
    Ok(if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

const MANIFEST_TEMPLATE: &str = r#"# The bridge's manifest. docs/BRIDGES.md explains every field.
id = "{id}"
title = "{Title}"
summary = "One sentence: what a planner can do with {Title} through this bridge."
programs = ["{id}-cli"]
workspace = "What the folder the operator names holds."
e2e = "ARCHIE_BRIDGE_E2E={id} cargo test -p archie-bridges --test {id}_e2e"

[app]
name = "{Title}"
homepage = "https://"
license = ""
versions = ""

[[surfaces]]
kind = "cli"
name = "{id}-cli"
doc = "https://"

[install]
arch = "pacman -S {id}"

[vision]
app_ids = ["{id}"]
when = "What the tools do not cover, which a planner does in the GUI on the Stage."
"#;

const GUIDE_TEMPLATE: &str = r#"---
name: bridge-{id}
description: Drive {Title} through Archie's {id} bridge. Use when a task needs {Title}.
---

# {Title} through Archie

## Before the first call

## The tools, in the order that works

## Traps

## When to use the GUI instead
"#;

fn scaffold(args: Vec<String>) -> Result<ExitCode, String> {
    let mut args = args.into_iter();
    let id = args.next().ok_or("new needs an id")?;
    let root = match (args.next().as_deref(), args.next()) {
        (Some("--root"), Some(root)) => PathBuf::from(root),
        (None, _) => PathBuf::from("."),
        _ => return Err("new takes --root REPO or nothing".into()),
    };
    let title = {
        let mut chars = id.chars();
        chars
            .next()
            .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default()
    };
    let fill = |text: &str| text.replace("{id}", &id).replace("{Title}", &title);
    // The template must be a manifest this crate accepts once its blanks are filled.
    let manifest = fill(MANIFEST_TEMPLATE).replace("\"https://\"", "\"https://example.org\"");
    Manifest::parse(&manifest).map_err(|e| format!("{id} is not a usable bridge id: {e}"))?;
    let dir = root.join("bridges").join(&id);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("bridge.toml"), fill(MANIFEST_TEMPLATE)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("SKILL.md"), fill(GUIDE_TEMPLATE)).map_err(|e| e.to_string())?;
    println!(
        "wrote {0}/bridge.toml and {0}/SKILL.md.\nNext, per docs/BRIDGES.md: fill the manifest from the \
         application's official docs, write core/crates/archie-bridges/src/{id}.rs, add it to BUILTIN, \
         write the end-to-end test, then `archie-bridges index --write docs/bridges/INDEX.md`.",
        dir.display()
    );
    Ok(ExitCode::SUCCESS)
}
