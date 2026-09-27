//! The gimp bridge against the real GIMP 3: inspect, retouch, export a PSD.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=gimp cargo test -p archie-bridges --test gimp_e2e -- --nocapture
//! ```

mod common;
use archie_bridges::preview::{render_png, View};
use common::*;
use serde_json::json;

#[test]
fn a_picture_is_retouched_and_exported_as_psd() {
    if !enabled("gimp") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name gimp");
        return;
    }
    let dir = workspace("gimp");
    let triangle = [[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 5.0]]];
    std::fs::write(
        dir.join("photo.png"),
        render_png(&triangle, View::Iso, 800, 600).unwrap(),
    )
    .unwrap();
    let bridge = bridge("gimp", &dir);
    let bridge = bridge.as_ref();

    let inspected = call(bridge, "gimp_inspect", json!({"file": "photo.png"}));
    assert_eq!(inspected["size"], json!([800, 600]));

    let edited = call(
        bridge,
        "gimp_edit",
        json!({"file": "photo.png", "format": "png", "operations": [
            "auto_levels",
            {"sharpen": {"radius": 1.5, "amount": 0.5}},
            {"text": {"text": "Archie", "x": 20, "y": 20, "size": 48, "color": "#ffffff"}},
            {"resize": {"width": 400}}
        ]}),
    );
    assert_eq!(edited["image"]["size"], json!([400, 300]));

    let exported = call(
        bridge,
        "gimp_export",
        json!({"file": edited["file"], "formats": ["psd", "jpg"]}),
    );
    let psd = std::fs::read(dir.join(exported["files"][0].as_str().unwrap())).unwrap();
    assert!(psd.starts_with(b"8BPS"), "a real Photoshop file");
    finish(dir);
}
