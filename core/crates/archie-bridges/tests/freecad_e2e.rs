//! The freecad bridge against the real FreeCAD: build, vary, export, picture.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=freecad cargo test -p archie-bridges --test freecad_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;

#[test]
fn a_plate_is_built_varied_exported_and_pictured() {
    if !enabled("freecad") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name freecad");
        return;
    }
    let dir = workspace("freecad");
    let bridge = bridge("freecad", &dir);
    let bridge = bridge.as_ref();

    let built = call(
        bridge,
        "freecad_build",
        json!({"name": "plate", "parameters": {"width": 80, "depth": 40, "thickness": 5, "hole": 8},
               "parts": [
                   {"name": "Plate", "kind": "box", "length": "width", "width": "depth", "height": "thickness"},
                   {"name": "Hole", "kind": "cylinder", "radius": "hole/2", "height": "thickness*3", "at": ["width/2", "depth/2", "-thickness"]},
                   {"name": "Bracket", "kind": "cut", "base": "Plate", "tools": ["Hole"]},
                   {"name": "Soft", "kind": "fillet", "base": "Bracket", "radius": 1, "edges": [1, 2, 3, 4]}]}),
    );
    assert_eq!(built["valid"], true, "{built}");
    let volume = built["volume_mm3"].as_f64().unwrap();
    let expected = 80.0 * 40.0 * 5.0 - std::f64::consts::PI * 16.0 * 5.0;
    assert!(
        (volume - expected).abs() / expected < 0.02,
        "{volume} vs {expected}"
    );
    let file = built["file"].as_str().unwrap().to_string();
    assert!(dir.join(built["preview"].as_str().unwrap()).is_file());

    let inspected = call(bridge, "freecad_inspect", json!({"file": file}));
    assert_eq!(inspected["parameters"]["thickness"]["value"], 5.0);

    let thicker = call(
        bridge,
        "freecad_set_parameters",
        json!({"file": file, "parameters": {"thickness": 8}}),
    );
    assert!(thicker["volume_mm3"].as_f64().unwrap() > volume * 1.5);

    let exported = call(
        bridge,
        "freecad_export",
        json!({"file": file, "formats": ["step", "stl", "3mf"]}),
    );
    let files = exported["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    let step = std::fs::read_to_string(dir.join(files[0].as_str().unwrap())).unwrap();
    assert!(step.starts_with("ISO-10303-21"), "a real STEP file");

    let picture = outcome(
        bridge,
        "freecad_render",
        json!({"file": file, "view": "top"}),
    );
    assert!(!picture.is_error && !picture.images.is_empty());
    finish(dir);
}
