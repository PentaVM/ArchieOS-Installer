//! The office bridge against the real LibreOffice: write, convert and read back.
//!
//! ```bash
//! ARCHIE_BRIDGE_E2E=office cargo test -p archie-bridges --test office_e2e -- --nocapture
//! ```

mod common;
use common::*;
use serde_json::json;

#[test]
fn documents_sheets_and_decks_are_written_and_read_back() {
    if !enabled("office") {
        eprintln!("skipped: ARCHIE_BRIDGE_E2E does not name office");
        return;
    }
    let dir = workspace("office");
    // A picture to embed: the bridge's own preview renderer makes one.
    let cube = archie_bridges::preview::read_stl(&archie_bridges::preview::write_stl(&[
        [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
        [[0.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]],
    ]))
    .unwrap();
    std::fs::write(
        dir.join("part.png"),
        archie_bridges::preview::render_png(&cube, archie_bridges::preview::View::Iso, 400, 300)
            .unwrap(),
    )
    .unwrap();
    let bridge = bridge("office", &dir);
    let bridge = bridge.as_ref();

    let report = call(
        bridge,
        "office_write_document",
        json!({"name": "report", "title": "Bracket study", "formats": ["docx", "pdf"], "blocks": [
            {"heading": {"text": "Findings", "level": 1}},
            {"paragraph": {"text": "A 3 mm wall carries the load with a margin of 2.1."}},
            {"bullets": {"items": ["mass −18 %", "one fewer machining step"]}},
            {"table": {"header": ["Wall", "Mass"], "rows": [["2 mm", "31 g"], ["3 mm", "41 g"]]}},
            {"image": {"file": "part.png", "width_cm": 10, "caption": "The chosen variant"}}
        ]}),
    );
    let files: Vec<String> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    let docx = files.iter().find(|f| f.ends_with(".docx")).expect("a docx");
    assert!(files.iter().any(|f| f.ends_with(".pdf")) && files.iter().any(|f| f.ends_with(".png")));
    let text = call(bridge, "office_text", json!({"file": docx}));
    let text = text["text"].as_str().unwrap();
    assert!(
        text.contains("Findings") && text.contains("margin of 2.1") && text.contains("31 g"),
        "{text}"
    );

    let sheet = call(
        bridge,
        "office_write_spreadsheet",
        json!({"name": "masses", "sheets": [{"name": "Parts", "rows": [["Part", "Mass"], ["A", 31], ["B", 41], ["Total", "=SUM(B2:B3)"]]}]}),
    );
    let xlsx = sheet["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .find(|f| f.ends_with(".xlsx"))
        .unwrap()
        .to_string();
    let read = call(bridge, "office_text", json!({"file": xlsx}));
    let csv = read["sheets"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        csv.contains("Total,72"),
        "the formula is computed by LibreOffice: {csv}"
    );

    let deck = call(
        bridge,
        "office_write_presentation",
        json!({"name": "deck", "slides": [
            {"title": "Bracket study", "subtitle": "200 variants"},
            {"title": "Result", "bullets": ["3 mm wall", "−18 % mass"], "image": "part.png", "notes": "lead with the saving"}
        ]}),
    );
    let pptx = deck["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .find(|f| f.ends_with(".pptx"))
        .unwrap()
        .to_string();
    let slides = call(bridge, "office_text", json!({"file": pptx}));
    assert_eq!(slides["slides"].as_array().unwrap().len(), 2);

    let pdf = call(bridge, "office_convert", json!({"file": docx, "to": "pdf"}));
    assert!(pdf["files"][0].as_str().unwrap().ends_with(".pdf"));
    finish(dir);
}
