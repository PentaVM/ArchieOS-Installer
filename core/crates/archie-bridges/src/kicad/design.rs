//! Reading KiCad's own files: what a schematic sheet and a board hold, without KiCad.

use crate::sexpr::Node;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The most symbols or footprints one answer lists.
pub const MAX_PARTS: usize = 2000;
/// The most labels one answer lists.
const MAX_LABELS: usize = 500;

/// Fields every symbol has, reported on their own rather than among `fields`.
const COMMON: [&str; 4] = ["Reference", "Value", "Footprint", "Datasheet"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Part {
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub lib_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datasheet: Option<String>,
    /// The units of a multi-unit part placed in this sheet, such as `[1, 2]`.
    pub units: Vec<u32>,
    pub dnp: bool,
    pub in_bom: bool,
    /// Where its first unit sits, in mm.
    pub at: [f64; 2],
    /// Every other field: MPN, manufacturer, description, …
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Schematic {
    pub version: Option<String>,
    pub title: Option<String>,
    pub revision: Option<String>,
    pub parts: Vec<Part>,
    /// Power symbols (GND, +3V3), which are not parts.
    pub power: Vec<String>,
    pub sheets: Vec<Sheet>,
    pub labels: BTreeMap<&'static str, Vec<String>>,
    pub wires: usize,
    pub no_connects: usize,
    pub truncated: bool,
}

fn property<'a>(symbol: &'a Node, name: &str) -> Option<&'a Node> {
    symbol.children("property").find(|p| p.arg(0) == Some(name))
}

fn property_value(symbol: &Node, name: &str) -> Option<String> {
    property(symbol, name)
        .and_then(|p| p.arg(1))
        .map(str::to_string)
}

fn yes(node: &Node, name: &str) -> Option<bool> {
    node.value_of(name).map(|v| v == "yes")
}

fn number(text: Option<&str>) -> f64 {
    text.and_then(|t| t.parse().ok()).unwrap_or(0.0)
}

/// Every reference a placed symbol answers to: its Reference field, and the references its
/// instances give it in a sheet used more than once.
pub fn references(symbol: &Node) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    if let Some(reference) = property_value(symbol, "Reference") {
        found.insert(reference);
    }
    let mut instances = Vec::new();
    symbol.descendants("reference", &mut instances);
    for reference in instances {
        if let Some(name) = reference.arg(0) {
            found.insert(name.to_string());
        }
    }
    found.retain(|r| !r.ends_with('?'));
    found
}

/// The symbols placed in a sheet: the direct `symbol` children of `kicad_sch`. The ones
/// under `lib_symbols` are library definitions, not placements.
pub fn placed(root: &Node) -> impl Iterator<Item = &Node> {
    root.children("symbol")
}

pub fn schematic(root: &Node) -> Schematic {
    let mut parts: BTreeMap<String, Part> = BTreeMap::new();
    let mut power = BTreeSet::new();
    let mut truncated = false;
    for symbol in placed(root) {
        let lib_id = symbol.value_of("lib_id").unwrap_or("").to_string();
        let reference = references(symbol)
            .into_iter()
            .next()
            .unwrap_or_else(|| property_value(symbol, "Reference").unwrap_or_default());
        let value = property_value(symbol, "Value").unwrap_or_default();
        if lib_id.starts_with("power:") || reference.starts_with('#') {
            power.insert(value);
            continue;
        }
        let unit = symbol
            .value_of("unit")
            .and_then(|u| u.parse().ok())
            .unwrap_or(1);
        if let Some(part) = parts.get_mut(&reference) {
            part.units.push(unit);
            continue;
        }
        if parts.len() == MAX_PARTS {
            truncated = true;
            continue;
        }
        let at = symbol.child("at");
        let fields = symbol
            .children("property")
            .filter_map(|p| Some((p.arg(0)?.to_string(), p.arg(1)?.to_string())))
            .filter(|(name, value)| {
                !COMMON.contains(&name.as_str()) && !value.is_empty() && value != "~"
            })
            .collect();
        parts.insert(
            reference.clone(),
            Part {
                reference,
                value,
                footprint: property_value(symbol, "Footprint").unwrap_or_default(),
                lib_id,
                datasheet: property_value(symbol, "Datasheet")
                    .filter(|d| !d.is_empty() && d != "~"),
                units: vec![unit],
                dnp: yes(symbol, "dnp").unwrap_or(false),
                in_bom: yes(symbol, "in_bom").unwrap_or(true),
                at: [
                    number(at.and_then(|a| a.arg(0))),
                    number(at.and_then(|a| a.arg(1))),
                ],
                fields,
            },
        );
    }
    let mut parts: Vec<Part> = parts.into_values().collect();
    parts.sort_by_key(|part| natural(&part.reference));
    for part in &mut parts {
        part.units.sort_unstable();
    }

    let sheets = root
        .children("sheet")
        .map(|sheet| Sheet {
            name: property_value(sheet, "Sheetname")
                .or_else(|| property_value(sheet, "Sheet name"))
                .unwrap_or_default(),
            file: property_value(sheet, "Sheetfile")
                .or_else(|| property_value(sheet, "Sheet file"))
                .unwrap_or_default(),
        })
        .collect();

    let mut labels = BTreeMap::new();
    for kind in ["label", "global_label", "hierarchical_label"] {
        let mut names: Vec<String> = root
            .children(kind)
            .filter_map(|l| l.arg(0).map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if names.len() > MAX_LABELS {
            names.truncate(MAX_LABELS);
            truncated = true;
        }
        if !names.is_empty() {
            labels.insert(kind, names);
        }
    }
    let title = root.child("title_block");
    Schematic {
        version: root.value_of("version").map(str::to_string),
        title: title.and_then(|t| t.value_of("title")).map(str::to_string),
        revision: title.and_then(|t| t.value_of("rev")).map(str::to_string),
        parts,
        power: power.into_iter().collect(),
        sheets,
        labels,
        wires: root.children("wire").count(),
        no_connects: root.children("no_connect").count(),
        truncated,
    }
}

/// `R10` after `R9`: letters compare as text, the number after them as a number.
fn natural(reference: &str) -> (String, u64, String) {
    let letters: String = reference
        .chars()
        .take_while(|c| !c.is_ascii_digit())
        .collect();
    let rest = &reference[letters.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (
        letters,
        digits.parse().unwrap_or(0),
        rest[digits.len()..].to_string(),
    )
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Footprint {
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub layer: String,
    /// x, y in mm and rotation in degrees.
    pub at: [f64; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct Board {
    pub version: Option<String>,
    pub footprints: Vec<Footprint>,
    pub copper_layers: Vec<String>,
    /// The other layers the board defines, canonical names.
    pub layers: Vec<String>,
    pub nets: usize,
    pub tracks: usize,
    pub vias: usize,
    pub zones: usize,
    /// The Edge.Cuts outline's bounding box: [min x, min y, max x, max y] in mm.
    pub outline: Option<[f64; 4]>,
    pub truncated: bool,
}

pub fn board(root: &Node) -> Board {
    let mut footprints = Vec::new();
    let mut truncated = false;
    for footprint in root.children("footprint") {
        if footprints.len() == MAX_PARTS {
            truncated = true;
            break;
        }
        // KiCad 8 and later keep the reference as a property; 7 as fp_text.
        let text = |name: &str| {
            property_value(footprint, name).or_else(|| {
                footprint
                    .children("fp_text")
                    .find(|t| t.arg(0) == Some(&name.to_ascii_lowercase()))
                    .and_then(|t| t.arg(1))
                    .map(str::to_string)
            })
        };
        let at = footprint.child("at");
        footprints.push(Footprint {
            reference: text("Reference").unwrap_or_default(),
            value: text("Value").unwrap_or_default(),
            footprint: footprint.arg(0).unwrap_or("").to_string(),
            layer: footprint.value_of("layer").unwrap_or("").to_string(),
            at: [
                number(at.and_then(|a| a.arg(0))),
                number(at.and_then(|a| a.arg(1))),
                number(at.and_then(|a| a.arg(2))),
            ],
        });
    }
    footprints.sort_by_key(|footprint| natural(&footprint.reference));

    let defined: Vec<String> = root
        .child("layers")
        .map(|layers| {
            layers.items()[1..]
                .iter()
                .filter_map(|layer| layer.arg(0).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let (copper_layers, layers) = defined.into_iter().partition(|l| l.ends_with(".Cu"));

    // Net names: the board's net table where there is one, else every net a pad names.
    let mut nets: BTreeSet<String> = root
        .children("net")
        .filter_map(|n| n.items().last().and_then(Node::atom).map(str::to_string))
        .collect();
    if nets.is_empty() {
        let mut pads = Vec::new();
        root.descendants("pad", &mut pads);
        for pad in pads {
            if let Some(name) = pad
                .child("net")
                .and_then(|n| n.items().last())
                .and_then(Node::atom)
            {
                nets.insert(name.to_string());
            }
        }
    }
    nets.remove("");

    Board {
        version: root.value_of("version").map(str::to_string),
        footprints,
        copper_layers,
        layers,
        nets: nets.len(),
        tracks: root.children("segment").count() + root.children("arc").count(),
        vias: root.children("via").count(),
        zones: root.children("zone").count(),
        outline: outline(root),
        truncated,
    }
}

fn outline(root: &Node) -> Option<[f64; 4]> {
    let mut points = Vec::new();
    for item in root.items() {
        let shape = item.head().unwrap_or("");
        if !shape.starts_with("gr_") || item.value_of("layer") != Some("Edge.Cuts") {
            continue;
        }
        let mut xy = Vec::new();
        for key in ["start", "end", "mid", "center"] {
            if let Some(point) = item.child(key) {
                xy.push(point);
            }
        }
        if let Some(pts) = item.child("pts") {
            xy.extend(pts.children("xy"));
        }
        for point in xy {
            points.push((number(point.arg(0)), number(point.arg(1))));
        }
    }
    let first = *points.first()?;
    Some(points.iter().fold(
        [first.0, first.1, first.0, first.1],
        |[x0, y0, x1, y1], &(x, y)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sexpr::parse;

    const SHEET: &str = r##"(kicad_sch (version 20250114) (generator "eeschema")
	(title_block (title "Blinky") (rev "B"))
	(lib_symbols (symbol "Device:R" (property "Reference" "R")))
	(wire (pts (xy 0 0) (xy 1 0)))
	(label "LED" (at 1 1 0))
	(global_label "VBUS" (at 1 1 0))
	(symbol (lib_id "Device:R") (at 100 50 0) (unit 1) (in_bom yes) (on_board yes) (dnp no)
		(property "Reference" "R10" (at 0 0 0))
		(property "Value" "330" (at 0 0 0))
		(property "Footprint" "Resistor_SMD:R_0603_1608Metric" (at 0 0 0))
		(property "Datasheet" "~" (at 0 0 0))
		(property "MPN" "RC0603FR-07330RL" (at 0 0 0))
	)
	(symbol (lib_id "Device:R") (at 90 50 0) (unit 1) (dnp yes)
		(property "Reference" "R9" (at 0 0 0))
		(property "Value" "10k" (at 0 0 0))
		(property "Footprint" "" (at 0 0 0))
	)
	(symbol (lib_id "Amplifier_Operational:LM358") (at 10 10 0) (unit 2)
		(property "Reference" "U1" (at 0 0 0))
		(property "Value" "LM358" (at 0 0 0))
	)
	(symbol (lib_id "Amplifier_Operational:LM358") (at 20 10 0) (unit 1)
		(property "Reference" "U1" (at 0 0 0))
		(property "Value" "LM358" (at 0 0 0))
	)
	(symbol (lib_id "power:GND") (at 0 0 0) (unit 1)
		(property "Reference" "#PWR01" (at 0 0 0))
		(property "Value" "GND" (at 0 0 0))
	)
	(sheet (at 0 0) (size 10 10)
		(property "Sheetname" "Power" (at 0 0 0))
		(property "Sheetfile" "power.kicad_sch" (at 0 0 0))
	)
)"##;

    #[test]
    fn a_sheet_lists_its_parts_power_and_hierarchy() {
        let sch = schematic(&parse(SHEET).unwrap());
        assert_eq!(sch.title.as_deref(), Some("Blinky"));
        let refs: Vec<_> = sch.parts.iter().map(|p| p.reference.as_str()).collect();
        assert_eq!(
            refs,
            ["R9", "R10", "U1"],
            "natural order, library symbols skipped"
        );
        assert_eq!(sch.parts[1].fields["MPN"], "RC0603FR-07330RL");
        assert_eq!(sch.parts[1].datasheet, None, "~ is no datasheet");
        assert!(sch.parts[0].dnp);
        assert_eq!(sch.parts[2].units, [1, 2]);
        assert_eq!(sch.power, ["GND"]);
        assert_eq!(
            sch.sheets,
            [Sheet {
                name: "Power".into(),
                file: "power.kicad_sch".into()
            }]
        );
        assert_eq!(sch.labels["global_label"], ["VBUS"]);
        assert_eq!(sch.wires, 1);
    }

    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
	(layers (0 "F.Cu" signal) (2 "B.Cu" signal) (5 "F.SilkS" user "F.Silkscreen") (25 "Edge.Cuts" user))
	(net 0 "") (net 1 "GND") (net 2 "LED")
	(footprint "Resistor_SMD:R_0603_1608Metric" (layer "F.Cu") (at 120 80 90)
		(property "Reference" "R10" (at 0 0 0) (layer "F.SilkS"))
		(property "Value" "330" (at 0 0 0) (layer "F.Fab"))
		(pad "1" smd rect (at 0 0) (net 1 "GND"))
	)
	(footprint "LED_SMD:LED_0603" (layer "B.Cu") (at 110 80)
		(fp_text reference "D1" (at 0 0) (layer "B.SilkS"))
		(fp_text value "RED" (at 0 0) (layer "B.Fab"))
	)
	(segment (start 0 0) (end 1 1) (width 0.25) (layer "F.Cu") (net 1))
	(via (at 1 1) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
	(gr_rect (start 100 70) (end 140 95) (layer "Edge.Cuts"))
	(gr_line (start 0 0) (end 500 500) (layer "F.SilkS"))
)"#;

    #[test]
    fn a_board_lists_footprints_layers_and_outline() {
        let pcb = board(&parse(BOARD).unwrap());
        let refs: Vec<_> = pcb
            .footprints
            .iter()
            .map(|f| f.reference.as_str())
            .collect();
        assert_eq!(
            refs,
            ["D1", "R10"],
            "fp_text (KiCad 7) and property (KiCad 8+)"
        );
        assert_eq!(pcb.footprints[0].value, "RED");
        assert_eq!(pcb.footprints[1].at, [120.0, 80.0, 90.0]);
        assert_eq!(pcb.copper_layers, ["F.Cu", "B.Cu"]);
        assert_eq!(pcb.layers, ["F.SilkS", "Edge.Cuts"]);
        assert_eq!(pcb.nets, 2);
        assert_eq!((pcb.tracks, pcb.vias), (1, 1));
        assert_eq!(
            pcb.outline,
            Some([100.0, 70.0, 140.0, 95.0]),
            "silkscreen is not outline"
        );
    }
}
