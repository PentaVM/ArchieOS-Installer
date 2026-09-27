//! The qgis bridge against the real QGIS and GDAL: read, buffer, convert, draw.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=qgis cargo test -p archie-bridges --test qgis_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;

#[test]
fn points_are_read_buffered_converted_and_drawn() {
    if !enabled("qgis") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name qgis");
        return;
    }
    let dir = workspace("qgis");
    // Three places in Santiago, in a projected CRS (UTM 19S) so distances are metres.
    std::fs::write(
        dir.join("places.geojson"),
        r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32719"}},
"features":[
{"type":"Feature","properties":{"name":"a"},"geometry":{"type":"Point","coordinates":[346000,6297000]}},
{"type":"Feature","properties":{"name":"b"},"geometry":{"type":"Point","coordinates":[347500,6298200]}},
{"type":"Feature","properties":{"name":"c"},"geometry":{"type":"Point","coordinates":[345200,6299100]}}]}"#,
    )
    .unwrap();
    let bridge = bridge("qgis", &dir);
    let bridge = bridge.as_ref();

    let info = call(bridge, "qgis_info", json!({"file": "places.geojson"}));
    assert_eq!(info["layers"][0]["features"], 3);

    let found = call(bridge, "qgis_algorithms", json!({"search": "buffer"}));
    assert!(found["algorithms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "native:buffer"));

    let buffered = call(
        bridge,
        "qgis_run",
        json!({"algorithm": "native:buffer", "inputs": {"INPUT": "places.geojson", "DISTANCE": 250, "OUTPUT": "zones.gpkg"}}),
    );
    let zones = buffered["results"]["OUTPUT"].as_str().unwrap().to_string();
    assert!(dir.join(zones.split('|').next().unwrap()).is_file());

    let kml = call(
        bridge,
        "qgis_convert",
        json!({"file": zones.split('|').next().unwrap(), "to": "kml"}),
    );
    assert!(kml["results"]["OUTPUT"].as_str().unwrap().contains(".kml"));

    let map = outcome(
        bridge,
        "qgis_map",
        json!({"layers": [zones.split('|').next().unwrap(), "places.geojson"], "extent": "344500,348500,6296500,6299800 [EPSG:32719]", "width": 800}),
    );
    assert!(!map.is_error, "{}", map.text);
    finish(dir);
}
