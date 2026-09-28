//! The kicad bridge against the real KiCad, on one of the demo projects KiCad ships.
//!
//! Runs only with `ARCHIE_BRIDGE_E2E` naming `kicad`, on a machine with KiCad installed:
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=kicad cargo test -p archie-bridges --test kicad_e2e -- --nocapture
//! ```
//!
//! `ARCHIE_KICAD_CLI` points at kicad-cli if it is not on PATH, and `ARCHIE_KICAD_DEMO` at
//! a demo project folder other than the packaged `pic_programmer`.

use archie_bridges::{open, Programs};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

fn enabled() -> bool {
    std::env::var("ARCHIE_BRIDGE_E2E")
        .map(|v| v.split(',').any(|b| b.trim() == "kicad"))
        .unwrap_or(false)
}

fn demo() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARCHIE_KICAD_DEMO") {
        return PathBuf::from(dir);
    }
    [
        "/usr/share/kicad/demos/pic_programmer",
        "/usr/share/kicad-demos/pic_programmer",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_dir())
    .expect("no KiCad demo found: install kicad-demos or set ARCHIE_KICAD_DEMO")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn call(bridge: &dyn archie_bridges::Bridge, tool: &str, arguments: Value) -> Value {
    let arguments: Map<String, Value> = arguments.as_object().unwrap().clone();
    let outcome = bridge.call(tool, &arguments);
    assert!(!outcome.is_error, "{tool}: {}", outcome.text);
    println!(
        "{tool}: {} bytes, {} images",
        outcome.text.len(),
        outcome.images.len()
    );
    serde_json::from_str(&outcome.text).unwrap()
}

#[test]
fn a_demo_project_is_read_checked_edited_exported_and_rendered() {
    if !enabled() {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name kicad");
        return;
    }
    let workspace = std::env::temp_dir().join(format!("archie-kicad-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workspace);
    copy(&demo(), &workspace.join("demo"));

    let mut programs = Programs::default();
    if let Ok(cli) = std::env::var("ARCHIE_KICAD_CLI") {
        programs.set("kicad-cli", cli);
    }
    let bridge = open("kicad", &workspace, &programs).unwrap();
    let bridge = bridge.as_ref();

    let version = call(bridge, "kicad_version", json!({}));
    println!("KiCad {}", version["kicad_cli"]);

    let projects = call(bridge, "kicad_projects", json!({}));
    let project = &projects["projects"][0];
    let root = project["root_schematic"]
        .as_str()
        .expect("a root schematic")
        .to_string();
    let board = project["board"].as_str().expect("a board").to_string();

    let schematic = call(bridge, "kicad_schematic", json!({"file": root}));
    let parts = schematic["parts"].as_array().unwrap();
    assert!(!parts.is_empty(), "the demo has parts");
    let part = parts
        .iter()
        .find(|p| p["reference"].as_str().is_some_and(|r| r.starts_with('R')))
        .unwrap_or(&parts[0])
        .clone();

    let pcb = call(bridge, "kicad_board", json!({"file": board}));
    assert!(!pcb["footprints"].as_array().unwrap().is_empty());
    assert!(pcb["outline"].is_array(), "the demo board has an outline");

    let netlist = call(bridge, "kicad_netlist", json!({"file": root}));
    assert!(netlist["nets"].as_array().unwrap().len() > 5);

    let erc = call(bridge, "kicad_erc", json!({"file": root}));
    assert!(erc["total"].is_u64(), "{erc}");
    let drc = call(
        bridge,
        "kicad_drc",
        json!({"file": board, "schematic_parity": true}),
    );
    assert!(drc["total"].is_u64(), "{drc}");

    // Edit a real part, and KiCad must still read the sheet and see the new value.
    let reference = part["reference"].as_str().unwrap();
    let edited = call(
        bridge,
        "kicad_set_fields",
        json!({"file": root, "reference": reference, "fields": {"Value": "4k7", "MPN": "ARCHIE-TEST-1"}}),
    );
    assert!(edited["symbols_edited"].as_u64().unwrap() >= 1);
    let bom_dir = call(
        bridge,
        "kicad_export",
        json!({"schematic": root, "kinds": ["bom"]}),
    );
    let bom_file = bom_dir["files"]["bom"][0].as_str().unwrap();
    let bom = std::fs::read_to_string(workspace.join(bom_file)).unwrap();
    assert!(
        bom.contains("4k7"),
        "KiCad's own BOM shows the edit:\n{bom}"
    );
    call(bridge, "kicad_erc", json!({"file": root}));

    let fab = call(
        bridge,
        "kicad_export",
        json!({"board": board, "schematic": root, "kinds": ["gerbers", "drill", "positions", "schematic_pdf"]}),
    );
    let gerbers = fab["files"]["gerbers"].as_array().unwrap();
    assert!(
        gerbers.len() >= 5,
        "copper, mask, silk, outline: {gerbers:?}"
    );
    assert!(!fab["files"]["drill"].as_array().unwrap().is_empty());

    let arguments: Map<String, Value> =
        json!({"file": board, "side": "top", "width": 800, "height": 600})
            .as_object()
            .unwrap()
            .clone();
    let render = bridge.call("kicad_render", &arguments);
    assert!(!render.is_error, "{}", render.text);
    let answer: Value = serde_json::from_str(&render.text).unwrap();
    let png = std::fs::read(workspace.join(answer["image"].as_str().unwrap())).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    println!(
        "render: {} bytes, returned inline: {}",
        png.len(),
        !render.images.is_empty()
    );

    if std::env::var_os("ARCHIE_KEEP_E2E").is_none() {
        let _ = std::fs::remove_dir_all(&workspace);
    } else {
        println!("kept {}", workspace.display());
    }
}
