//! `ARCHIE.md` against the node it describes.
//!
//! The contract is what a planner reads before it does anything, so a tool, a bridge or a
//! model it does not mention does not exist as far as that planner knows. These tests fail
//! when the code gains one and the page does not.

use archie_bridges::Programs;
use archie_mcp_server::archie::CONTRACT;
use archie_mcp_server::{delivery, jobs, ArchieToolbox, Clearance, McpServer, Toolbox};
use archie_state::StatePaths;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate sits three levels below the repository root")
        .to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-contract-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_contract_names_every_tool_the_server_can_offer() {
    let dir = scratch("tools");
    let bridge = archie_bridges::open("kicad", &dir, &Programs::default()).unwrap();
    let bridge_tools: Vec<&str> = bridge.tools().iter().map(|t| t.name).collect();
    let toolbox = ArchieToolbox::new(
        StatePaths::rooted(dir.join(".node")),
        Some(dir.join("control.sock")),
        None,
    )
    .with_bridge(bridge)
    .unwrap();

    // What this configuration lists, less the bridge's own tools (the bridge index names
    // those), plus what only another configuration lists: delivery, and acting on the stage.
    let mut names: Vec<&str> = toolbox
        .tools()
        .iter()
        .map(|t| t.name)
        .filter(|name| !bridge_tools.contains(name))
        .collect();
    names.extend(delivery::NAMES);
    names.extend(jobs::NAMES);
    names.extend(["stage_surfaces", "stage_act"]);
    for name in names {
        assert!(
            CONTRACT.contains(&format!("`{name}`")),
            "ARCHIE.md does not mention the tool {name}"
        );
    }
}

#[test]
fn the_contract_names_every_bridge_and_every_shipped_model() {
    for bridge in archie_bridges::BUILTIN {
        assert!(
            CONTRACT.contains(&format!("`{}`", bridge.id)),
            "ARCHIE.md does not mention the {} bridge",
            bridge.id
        );
    }
    let mut manifests = Vec::new();
    let mut pending = vec![repo_root().join("models")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "toml") {
                manifests.push(path);
            }
        }
    }
    assert!(!manifests.is_empty(), "no model manifests are shipped");
    for manifest in manifests {
        let text = std::fs::read_to_string(&manifest).unwrap();
        let id = text
            .lines()
            .find_map(|line| line.strip_prefix("id = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("{} has no id", manifest.display()));
        assert!(
            CONTRACT.contains(&format!("`{id}`")),
            "ARCHIE.md does not mention the model {id}"
        );
    }
}

/// The plan in the contract is the one IRIS's runner reads: a planner that copies the
/// example must be understood.
#[test]
fn the_plan_the_contract_shows_has_the_fields_the_runner_reads() {
    let start = CONTRACT.find("```json").expect("the contract shows a plan") + "```json".len();
    let end = start + CONTRACT[start..].find("```").unwrap();
    let plan: Value = serde_json::from_str(&CONTRACT[start..end]).expect("the plan is valid JSON");
    assert!(plan["skill"].is_string());
    let steps = plan["steps"].as_array().expect("steps is a list");
    assert!(!steps.is_empty());
    for step in steps {
        assert!(step["do"].is_string(), "a step says what to do");
        assert!(step["seconds"].is_number(), "a step says for how long");
    }
}

#[tokio::test]
async fn any_client_can_read_the_contract_and_is_told_to() {
    let dir = scratch("read");
    let toolbox = ArchieToolbox::new(StatePaths::rooted(dir.join(".node")), None, None);
    let mut server = McpServer::new(toolbox, Clearance::Public);
    let init: Value = serde_json::from_str(
        &server
            .handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(init["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("archie_contract"));

    let reply: Value = serde_json::from_str(
        &server
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "archie_contract", "arguments": {}}}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("the contract comes back as text");
    assert!(text.starts_with("# Archie"));
    assert!(
        text.contains("Models propose, Cerebra decides, Policy authorises, the Stage executes.")
    );
}
