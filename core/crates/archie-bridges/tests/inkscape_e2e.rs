//! The inkscape bridge against the real Inkscape: write, inspect, edit, export.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=inkscape cargo test -p archie-bridges --test inkscape_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;

#[test]
fn a_label_is_drawn_edited_and_exported_for_a_cutter() {
    if !enabled("inkscape") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name inkscape");
        return;
    }
    let dir = workspace("inkscape");
    let bridge = bridge("inkscape", &dir);
    let bridge = bridge.as_ref();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
  <rect id="plate" x="2" y="2" width="96" height="46" rx="4" fill="none" stroke="#1d1d1b" stroke-width="0.5"/>
  <circle id="hole" cx="10" cy="25" r="3" fill="#2f6fde"/>
  <text id="label" x="20" y="30" font-size="12" font-family="sans-serif">ARCHIE</text>
</svg>"##;
    let written = call(
        bridge,
        "inkscape_write_svg",
        json!({"name": "label", "svg": svg}),
    );
    let file = written["file"].as_str().unwrap().to_string();
    let inspected = call(bridge, "inkscape_inspect", json!({"file": file}));
    let ids: Vec<&str> = inspected["listed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    assert!(ids.contains(&"plate") && ids.contains(&"label"), "{ids:?}");

    let edited = call(
        bridge,
        "inkscape_edit",
        json!({"file": file, "operations": [{"select": "label"}, "to_path", {"set_style": {"property": "fill", "value": "#000000"}}]}),
    );
    let edited_file = edited["file"].as_str().unwrap().to_string();
    let text = std::fs::read_to_string(dir.join(&edited_file)).unwrap();
    assert!(!text.contains("<text"), "the label became outlines");

    let exported = call(
        bridge,
        "inkscape_export",
        json!({"file": edited_file, "formats": ["pdf", "emf", "png"], "dpi": 300}),
    );
    assert_eq!(exported["files"].as_array().unwrap().len(), 3);
    let pdf = std::fs::read(dir.join(exported["files"][0].as_str().unwrap())).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    finish(dir);
}
