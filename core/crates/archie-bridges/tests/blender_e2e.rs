//! The blender bridge against the real Blender: a bare STL gets a studio, a render and
//! exports.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=blender cargo test -p archie-bridges --test blender_e2e -- --nocapture
//! ```

mod common;
use archie_bridges::preview::write_stl;
use common::*;
use serde_json::json;

#[test]
fn a_bare_model_is_staged_rendered_and_exported() {
    if !enabled("blender") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name blender");
        return;
    }
    let dir = workspace("blender");
    let (x, y, z) = (20.0, 10.0, 5.0);
    let p = [
        [0.0, 0.0, 0.0],
        [x, 0.0, 0.0],
        [x, y, 0.0],
        [0.0, y, 0.0],
        [0.0, 0.0, z],
        [x, 0.0, z],
        [x, y, z],
        [0.0, y, z],
    ];
    let faces = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let triangles: Vec<_> = faces.iter().map(|f| [p[f[0]], p[f[1]], p[f[2]]]).collect();
    std::fs::write(dir.join("block.stl"), write_stl(&triangles)).unwrap();
    let bridge = bridge("blender", &dir);
    let bridge = bridge.as_ref();

    let inspected = call(bridge, "blender_inspect", json!({"file": "block.stl"}));
    assert_eq!(inspected["objects"][0]["faces"], 12);

    let still = outcome(
        bridge,
        "blender_render",
        json!({"file": "block.stl", "engine": "cycles", "samples": 8, "width": 640, "height": 400}),
    );
    assert!(!still.is_error, "{}", still.text);
    assert!(!still.images.is_empty());
    let answer: serde_json::Value = serde_json::from_str(&still.text).unwrap();
    assert!(answer["studio_added"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "camera"));

    let exported = call(
        bridge,
        "blender_export",
        json!({"file": "block.stl", "formats": ["glb", "fbx", "usdc"]}),
    );
    assert_eq!(exported["files"].as_array().unwrap().len(), 3);
    let glb = std::fs::read(dir.join(exported["files"][0].as_str().unwrap())).unwrap();
    assert!(glb.starts_with(b"glTF"));
    finish(dir);
}
