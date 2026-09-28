//! Inkscape: vector drawings, edited with Inkscape's own actions and exported for
//! Illustrator, CorelDRAW, print shops and cutters.
//!
//! Inkscape 1.x runs headless from its command line: `--query-all` reads a drawing,
//! `--export-type` writes PDF, EPS, EMF, PNG and plain SVG, and `--actions` applies edits.
//! Only the actions listed here run, each with checked arguments, and the file the edit
//! writes is always a new one the bridge names.

use crate::manifest::Manifest;
use crate::run::Run;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../bridges/inkscape/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../bridges/inkscape/SKILL.md");

const LIMIT: Duration = Duration::from_secs(300);
const READS: [&str; 6] = ["svg", "svgz", "pdf", "eps", "ai", "emf"];
const EXPORTS: [&str; 8] = ["pdf", "eps", "ps", "emf", "wmf", "png", "svg", "dxf"];
const MAX_OBJECTS: usize = 2000;
const MAX_SVG: usize = 8 * 1024 * 1024;
const MAX_IMAGE: usize = 700 * 1024;

pub struct Inkscape {
    manifest: Manifest,
    workspace: Workspace,
    inkscape: PathBuf,
}

/// An SVG id or class as Inkscape's actions take them.
fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// An argument that cannot end an action or start another.
fn is_plain(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && !text.contains([';', '\n', '\r'])
}

/// Turns one described operation into an Inkscape action, or says why it cannot.
pub fn action(op: &Value) -> Result<String, String> {
    let (name, argument) = match op {
        Value::String(name) => (name.as_str(), &Value::Null),
        Value::Object(map) if map.len() == 1 => {
            let (name, argument) = map.iter().next().expect("one entry");
            (name.as_str(), argument)
        }
        _ => {
            return Err(format!(
                "an operation is a name or {{name: argument}}: {op}"
            ))
        }
    };
    let number = |what: &str| {
        argument
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{name} takes a number ({what})"))
    };
    Ok(match name {
        "select" => {
            let ids: Vec<&str> = match argument {
                Value::String(id) => vec![id.as_str()],
                Value::Array(ids) => ids.iter().filter_map(Value::as_str).collect(),
                _ => return Err("select takes an id or a list of ids".into()),
            };
            if ids.is_empty() || !ids.iter().all(|id| is_id(id)) {
                return Err("select takes ids of letters, digits, - _ . :".into());
            }
            format!("select-by-id:{}", ids.join(","))
        }
        "select_class" => match argument.as_str().filter(|c| is_id(c)) {
            Some(class) => format!("select-by-class:{class}"),
            None => return Err("select_class takes a class name".into()),
        },
        "select_all" => "select-all".into(),
        "deselect" => "select-clear".into(),
        "to_path" => "object-to-path".into(),
        "stroke_to_path" => "object-stroke-to-path".into(),
        "union" => "path-union".into(),
        "difference" => "path-difference".into(),
        "intersection" => "path-intersection".into(),
        "exclusion" => "path-exclusion".into(),
        "combine" => "path-combine".into(),
        "break_apart" => "path-break-apart".into(),
        "simplify" => "path-simplify".into(),
        "group" => "selection-group".into(),
        "ungroup" => "selection-ungroup".into(),
        "delete" => "delete-selection".into(),
        "duplicate" => "duplicate".into(),
        "raise_to_top" => "selection-top".into(),
        "lower_to_bottom" => "selection-bottom".into(),
        "flip" => match argument.as_str() {
            Some("horizontal") => "object-flip-horizontal".into(),
            Some("vertical") => "object-flip-vertical".into(),
            _ => return Err("flip is horizontal or vertical".into()),
        },
        "rotate" => format!("transform-rotate:{}", number("degrees")?),
        "scale" => format!("transform-scale:{}", number("factor")?),
        "translate" => match argument
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
        {
            Some(d) if d.len() == 2 && d.iter().all(|n| n.is_finite()) => {
                format!("transform-translate:{},{}", d[0], d[1])
            }
            _ => return Err("translate takes [dx, dy] in user units".into()),
        },
        "set_style" => {
            let property = argument
                .get("property")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value = argument.get("value").and_then(Value::as_str).unwrap_or("");
            if !is_id(property) || !is_plain(value) || value.contains(':') {
                return Err("set_style takes {property, value}, such as {\"property\": \"fill\", \"value\": \"#2f6fde\"}".into());
            }
            format!("object-set-property:{property},{value}")
        }
        "set_attribute" => {
            let attribute = argument.get("name").and_then(Value::as_str).unwrap_or("");
            let value = argument.get("value").and_then(Value::as_str).unwrap_or("");
            let lower = attribute.to_ascii_lowercase();
            // No event handlers, no links: an edited SVG must not gain a way to run code.
            if !is_id(attribute)
                || lower.starts_with("on")
                || lower.contains("href")
                || !is_plain(value)
            {
                return Err(
                    "set_attribute takes {name, value}; event handlers and links are not set here"
                        .into(),
                );
            }
            format!("object-set-attribute:{attribute},{value}")
        }
        "fit_canvas" => match argument.as_str() {
            Some("drawing") | None => "fit-canvas-to-drawing".into(),
            Some("selection") => "fit-canvas-to-selection".into(),
            _ => return Err("fit_canvas is drawing or selection".into()),
        },
        "vacuum" => "vacuum-defs".into(),
        other => return Err(format!("{other} is not an operation this bridge runs")),
    })
}

/// Refuses SVG that could run code or reach the network when a person opens it.
pub fn check_svg(svg: &str) -> Result<(), String> {
    if svg.len() > MAX_SVG {
        return Err(format!("the drawing is larger than {MAX_SVG} bytes"));
    }
    let lower = svg.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return Err("this is not SVG: there is no <svg> element".into());
    }
    for (needle, why) in [
        ("<script", "scripts"),
        ("javascript:", "javascript: links"),
        ("<foreignobject", "embedded HTML"),
        ("<!entity", "XML entities"),
        ("http://", "links to the network"),
        ("https://", "links to the network"),
    ] {
        // The SVG namespace declarations are the one expected http:// text.
        let hits = lower.matches(needle).count();
        let namespaces = if needle == "http://" {
            lower.matches("http://www.w3.org/").count()
                + lower.matches("http://www.inkscape.org/").count()
                + lower.matches("http://sodipodi.sourceforge.net/").count()
                + lower.matches("http://purl.org/dc/").count()
                + lower.matches("http://creativecommons.org/").count()
        } else {
            0
        };
        if hits > namespaces {
            return Err(format!(
                "the drawing contains {why}, which this bridge does not write"
            ));
        }
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find(" on") {
        let after = &rest[at + 3..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        if !name.is_empty() && after[name.len()..].trim_start().starts_with('=') {
            return Err(format!(
                "the drawing has an event handler (on{name}), which this bridge does not write"
            ));
        }
        rest = after;
    }
    Ok(())
}

impl Inkscape {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            inkscape: programs.get("inkscape"),
        }
    }

    fn drawing(&self, arguments: &Map<String, Value>) -> Result<PathBuf, String> {
        let file = arguments
            .get("file")
            .and_then(Value::as_str)
            .ok_or("file is a drawing in the workspace")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !READS.contains(&ext.as_str()) {
            return Err(format!(
                "{file} is not a drawing Inkscape reads here: {READS:?}"
            ));
        }
        Ok(path)
    }

    fn run(&self, args: Vec<std::ffi::OsString>) -> Result<String, String> {
        let ran = Run::new(&self.inkscape)
            .args(args)
            .cwd(self.workspace.root())
            .limit(LIMIT)
            .run()
            .map_err(|e| e.to_string())?;
        if ran.success() {
            Ok(ran.stdout)
        } else {
            Err(format!(
                "Inkscape failed (exit {:?}): {}",
                ran.code,
                ran.said()
            ))
        }
    }

    fn picture(&self, input: &Path, png: &Path) -> Result<Vec<u8>, String> {
        let mut out = std::ffi::OsString::from("--export-filename=");
        out.push(png);
        self.run(vec![
            "--export-type=png".into(),
            out,
            "--export-area-drawing".into(),
            "--export-background=#ffffff".into(),
            "--export-background-opacity=1".into(),
            "--export-width=1200".into(),
            input.as_os_str().to_os_string(),
        ])?;
        std::fs::read(png).map_err(|e| e.to_string())
    }

    fn with_picture(&self, answer: Value, input: &Path, png: PathBuf) -> Outcome {
        match self.picture(input, &png) {
            Ok(bytes) => {
                let mut answer = answer;
                answer["preview"] = json!(self.workspace.relative(&png));
                let outcome = Outcome::json(&answer);
                if bytes.len() <= MAX_IMAGE {
                    outcome.with_image(Image {
                        mime: "image/png",
                        data: bytes,
                    })
                } else {
                    outcome
                }
            }
            Err(why) => {
                let mut answer = answer;
                answer["preview_failed"] = json!(why);
                Outcome::json(&answer)
            }
        }
    }

    fn stem(path: &Path) -> String {
        crate::delivery::to_name(
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("drawing"),
        )
    }

    fn inspect(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.drawing(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let listed = match self.run(vec!["--query-all".into(), path.as_os_str().to_os_string()]) {
            Ok(listed) => listed,
            Err(why) => return Outcome::error(why),
        };
        let mut objects = Vec::new();
        let mut count = 0;
        for line in listed.lines() {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() != 5 {
                continue;
            }
            count += 1;
            if objects.len() < MAX_OBJECTS {
                let n = |i: usize| {
                    fields[i]
                        .trim()
                        .parse::<f64>()
                        .map(|v| (v * 100.0).round() / 100.0)
                        .unwrap_or(0.0)
                };
                objects.push(
                    json!({"id": fields[0], "x": n(1), "y": n(2), "width": n(3), "height": n(4)}),
                );
            }
        }
        // The first entry is the whole drawing.
        let size = objects.first().map(|o| json!([o["width"], o["height"]]));
        Outcome::json(
            &json!({"size": size, "objects": count, "listed": objects, "truncated": count > MAX_OBJECTS}),
        )
    }

    fn export(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.drawing(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let formats: Vec<String> = match arguments.get("formats") {
            None => vec!["pdf".into()],
            Some(Value::Array(list)) => match list
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
            {
                Some(list) if !list.is_empty() => list,
                _ => return Outcome::error("formats is a non-empty list of names"),
            },
            Some(_) => return Outcome::error("formats is a list"),
        };
        for format in &formats {
            if !EXPORTS.contains(&format.as_str()) {
                return Outcome::error(format!("{format} is not one of {EXPORTS:?}"));
            }
        }
        let area = match arguments
            .get("area")
            .and_then(Value::as_str)
            .unwrap_or("page")
        {
            "page" => "--export-area-page",
            "drawing" => "--export-area-drawing",
            _ => return Outcome::error("area is page or drawing"),
        };
        let dpi = arguments
            .get("dpi")
            .and_then(Value::as_u64)
            .unwrap_or(150)
            .clamp(30, 1200);
        let only: Option<String> = match arguments.get("id").and_then(Value::as_str) {
            Some(id) if is_id(id) => Some(id.to_string()),
            Some(_) => return Outcome::error("id is an object id from inkscape_inspect"),
            None => None,
        };
        let stem = Self::stem(&path);
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let mut files = Vec::new();
        for format in &formats {
            let (kind, extension, extra): (&str, &str, Vec<&str>) = match format.as_str() {
                "svg" => ("svg", "svg", vec!["--export-plain-svg"]),
                "pdf" => ("pdf", "pdf", vec!["--export-text-to-path"]),
                "eps" => ("eps", "eps", vec!["--export-text-to-path"]),
                "dxf" => ("dxf", "dxf", vec![]),
                other => (other, other, vec![]),
            };
            let target = folder.join(format!("{stem}.{extension}"));
            let mut out = std::ffi::OsString::from("--export-filename=");
            out.push(&target);
            let mut args: Vec<std::ffi::OsString> =
                vec![format!("--export-type={kind}").into(), out, area.into()];
            args.extend(extra.into_iter().map(Into::into));
            if kind == "png" {
                args.push(format!("--export-dpi={dpi}").into());
            }
            if let Some(id) = &only {
                args.push(format!("--export-id={id}").into());
                args.push("--export-id-only".into());
            }
            args.push(path.as_os_str().to_os_string());
            if let Err(why) = self.run(args) {
                return Outcome::error(format!("{format}: {why}"));
            }
            if !target.is_file() {
                return Outcome::error(format!("Inkscape wrote no {format}"));
            }
            files.push(self.workspace.relative(&target));
        }
        let png = folder.join(format!("{stem}-preview.png"));
        self.with_picture(
            json!({"folder": self.workspace.relative(&folder), "files": files}),
            &path,
            png,
        )
    }

    fn edit(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.drawing(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let Some(operations) = arguments
            .get("operations")
            .and_then(Value::as_array)
            .filter(|o| !o.is_empty())
        else {
            return Outcome::error("operations is a list, such as [{\"select\": \"logo\"}, \"to_path\", {\"set_style\": {\"property\": \"fill\", \"value\": \"#000\"}}]");
        };
        let mut actions = Vec::new();
        for op in operations {
            match action(op) {
                Ok(action) => actions.push(action),
                Err(why) => return Outcome::error(why),
            }
        }
        let output = match self
            .workspace
            .fresh_file("edits", &Self::stem(&path), "svg")
        {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let mut out = std::ffi::OsString::from("--export-filename=");
        out.push(&output);
        let args = vec![
            format!("--actions={}", actions.join(";")).into(),
            "--export-type=svg".into(),
            "--export-overwrite".into(),
            out,
            path.as_os_str().to_os_string(),
        ];
        match self.run(args) {
            Ok(_) if output.is_file() => {
                let png = output.with_extension("png");
                self.with_picture(
                    json!({"file": self.workspace.relative(&output), "actions": actions}),
                    &output,
                    png,
                )
            }
            Ok(_) => Outcome::error(
                "Inkscape wrote nothing; check that the ids exist with inkscape_inspect",
            ),
            Err(why) => Outcome::error(why),
        }
    }

    fn write_svg(&self, arguments: &Map<String, Value>) -> Outcome {
        let (Some(name), Some(svg)) = (
            arguments.get("name").and_then(Value::as_str),
            arguments.get("svg").and_then(Value::as_str),
        ) else {
            return Outcome::error("inkscape_write_svg needs name and svg");
        };
        if let Err(why) = check_svg(svg) {
            return Outcome::error(why);
        }
        let output =
            match self
                .workspace
                .fresh_file("drawings", &crate::delivery::to_name(name), "svg")
            {
                Ok(output) => output,
                Err(e) => return Outcome::error(e.to_string()),
            };
        if let Err(e) = std::fs::write(&output, svg) {
            return Outcome::error(e.to_string());
        }
        let png = output.with_extension("png");
        self.with_picture(
            json!({"file": self.workspace.relative(&output)}),
            &output,
            png,
        )
    }

    fn render(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.drawing(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let png = match self
            .workspace
            .fresh_file("renders", &Self::stem(&path), "png")
        {
            Ok(png) => png,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.picture(&path, &png) {
            Ok(bytes) => {
                let outcome = Outcome::json(&json!({"image": self.workspace.relative(&png)}));
                if bytes.len() <= MAX_IMAGE {
                    outcome.with_image(Image {
                        mime: "image/png",
                        data: bytes,
                    })
                } else {
                    outcome
                }
            }
            Err(why) => Outcome::error(why),
        }
    }
}

impl Bridge for Inkscape {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    fn guide(&self) -> &'static str {
        GUIDE
    }

    fn tools(&self) -> Vec<BridgeTool> {
        let file = json!({"type": "string", "description": "A drawing in the workspace (SVG, PDF, EPS, AI)."});
        vec![
            BridgeTool {
                name: "inkscape_drawings",
                title: "Drawings",
                description: "The vector drawings in the workspace: SVG, SVGZ, PDF, EPS, AI, EMF.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "inkscape_inspect",
                title: "What a drawing holds",
                description: "The drawing's size and every object's id and bounding box, as Inkscape measures them. \
                              Ids are what inkscape_edit selects and inkscape_export exports alone.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "inkscape_write_svg",
                title: "Write a drawing",
                description: "Saves SVG you wrote as drawings/<name>-<n>.svg and returns a picture of it. Scripts, \
                              event handlers, embedded HTML and network links are refused.",
                input_schema: json!({"type": "object", "properties": {"name": {"type": "string"}, "svg": {"type": "string"}},
                                     "required": ["name", "svg"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "inkscape_edit",
                title: "Edit a drawing",
                description: "Applies Inkscape operations in order and saves the result as edits/<name>-<n>.svg: \
                              select (ids), select_class, select_all, deselect, to_path, stroke_to_path, union, \
                              difference, intersection, exclusion, combine, break_apart, simplify, group, ungroup, \
                              delete, duplicate, raise_to_top, lower_to_bottom, flip (horizontal|vertical), rotate \
                              (degrees), scale (factor), translate ([dx, dy]), set_style ({property, value}), \
                              set_attribute ({name, value}), fit_canvas (drawing|selection), vacuum.",
                input_schema: json!({
                    "type": "object",
                    "properties": {"file": file, "operations": {"type": "array"}},
                    "required": ["file", "operations"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "inkscape_export",
                title: "Export for other software",
                description: "Exports a drawing into exports/<name>-<n>/: pdf (Illustrator, print; text as paths), \
                              eps, ps, emf and wmf (Office, CorelDRAW), png (at dpi), svg (plain, without \
                              Inkscape's extras), dxf (laser cutters, CAD). area page or drawing; id exports one \
                              object alone. Default pdf.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": file,
                        "formats": {"type": "array", "items": {"type": "string", "enum": EXPORTS}},
                        "area": {"type": "string", "enum": ["page", "drawing"]},
                        "dpi": {"type": "integer", "minimum": 30, "maximum": 1200},
                        "id": {"type": "string"}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "inkscape_render",
                title: "Picture of a drawing",
                description: "The drawing on white, 1200 px wide, in renders/, returned as an image.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "inkscape_drawings" => {
                let (files, truncated) = self.workspace.find(&READS, 5, 2000);
                Outcome::json(&json!({"files": files, "truncated": truncated}))
            }
            "inkscape_inspect" => self.inspect(arguments),
            "inkscape_write_svg" => self.write_svg(arguments),
            "inkscape_edit" => self.edit(arguments),
            "inkscape_export" => self.export(arguments),
            "inkscape_render" => self.render(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_become_actions_and_nothing_else() {
        assert_eq!(
            action(&json!({"select": ["a", "b"]})).unwrap(),
            "select-by-id:a,b"
        );
        assert_eq!(action(&json!("union")).unwrap(), "path-union");
        assert_eq!(
            action(&json!({"translate": [5, -2.5]})).unwrap(),
            "transform-translate:5,-2.5"
        );
        assert_eq!(
            action(&json!({"set_style": {"property": "fill", "value": "#2f6fde"}})).unwrap(),
            "object-set-property:fill,#2f6fde"
        );
        for bad in [
            json!({"select": "a;export-do"}),
            json!({"set_attribute": {"name": "onclick", "value": "x"}}),
            json!({"set_attribute": {"name": "xlink:href", "value": "x"}}),
            json!({"set_style": {"property": "fill", "value": "red;x:y"}}),
            json!("export-do"),
            json!({"rotate": "90"}),
        ] {
            assert!(action(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn written_svg_may_not_carry_code_or_links() {
        let ok = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5" fill="#2f6fde"/></svg>"##;
        assert!(check_svg(ok).is_ok());
        for bad in [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect onload = "x"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://evil.example/x.png"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><a href="javascript:x">t</a></svg>"#,
            "no svg here",
        ] {
            assert!(check_svg(bad).is_err(), "{bad}");
        }
    }
}
