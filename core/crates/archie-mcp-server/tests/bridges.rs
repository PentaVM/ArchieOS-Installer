//! Bridges served over MCP: listed at internal clearance, hidden at public, with their
//! guide one call away.

use archie_bridges::Programs;
use archie_mcp_server::{ArchieToolbox, Clearance, McpServer};
use archie_state::StatePaths;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("archie-mcp-bridges-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("blinky")).unwrap();
    std::fs::write(dir.join("blinky/blinky.kicad_pro"), "{}").unwrap();
    std::fs::write(
        dir.join("blinky/blinky.kicad_sch"),
        "(kicad_sch (version 20250114))",
    )
    .unwrap();
    dir
}

async fn server(dir: &Path, clearance: Clearance) -> McpServer<ArchieToolbox> {
    let bridge = archie_bridges::open("kicad", dir, &Programs::default()).unwrap();
    let toolbox = ArchieToolbox::new(StatePaths::rooted(dir.join(".node")), None, None)
        .with_bridge(bridge)
        .unwrap();
    let mut server = McpServer::new(toolbox, clearance);
    server
        .handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}).to_string())
        .await
        .unwrap();
    server
}

async fn reply(server: &mut McpServer<ArchieToolbox>, message: Value) -> Value {
    serde_json::from_str(&server.handle(&message.to_string()).await.unwrap()).unwrap()
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_bridge_is_listed_guided_and_called() {
    let dir = folder("served");
    let mut internal = server(&dir, Clearance::Internal).await;
    let list = reply(
        &mut internal,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names = names(&list);
    for tool in [
        "archie_bridge_guide",
        "kicad_projects",
        "kicad_drc",
        "kicad_set_fields",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "{tool} missing from {names:?}"
        );
    }

    let guide = reply(
        &mut internal,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "archie_bridge_guide", "arguments": {"bridge": "kicad"}}}),
    )
    .await;
    assert!(guide["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("# KiCad through Archie"));

    let projects = reply(
        &mut internal,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "kicad_projects", "arguments": {}}}),
    )
    .await;
    assert_eq!(projects["result"]["isError"], false, "{projects}");
    let text: Value =
        serde_json::from_str(projects["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["projects"][0]["project"], "blinky/blinky.kicad_pro");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn file_bearing_tools_stay_hidden_at_public_clearance() {
    let dir = folder("public");
    let mut public = server(&dir, Clearance::Public).await;
    let list = reply(
        &mut public,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names = names(&list);
    assert!(
        !names
            .iter()
            .any(|n| n == "kicad_projects" || n == "kicad_set_fields"),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n == "kicad_version"),
        "the version names no file"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_same_bridge_twice_is_refused() {
    let dir = folder("twice");
    let open = || archie_bridges::open("kicad", &dir, &Programs::default()).unwrap();
    let toolbox = ArchieToolbox::new(StatePaths::rooted(dir.join(".node")), None, None)
        .with_bridge(open())
        .unwrap();
    assert!(toolbox.with_bridge(open()).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
