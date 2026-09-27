//! GIMP: raster images retouched and exported, layered PSD for Photoshop included.
//!
//! GIMP 3 runs in batch mode with the one script in [`script`]. Every operation a planner
//! asks for is checked here — its name against a fixed list, its settings against ranges —
//! before GIMP starts.

pub mod script;

use crate::manifest::Manifest;
use crate::script::Script;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/gimp/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/gimp/SKILL.md");

const JOB: Script = Script {
    name: "gimp",
    extension: "py",
    text: script::JOB,
};
const LIMIT: Duration = Duration::from_secs(900);
const READS: [&str; 11] = [
    "xcf", "psd", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "heic",
];
/// Formats that keep layers; the others are flattened copies.
const LAYERED: [&str; 3] = ["xcf", "psd", "tif"];
const EXPORTS: [&str; 8] = ["psd", "xcf", "png", "jpg", "tif", "webp", "pdf", "gif"];
const MAX_IMAGE: usize = 700 * 1024;

pub struct Gimp {
    manifest: Manifest,
    workspace: Workspace,
    gimp: PathBuf,
}

fn in_range(
    settings: &Value,
    key: &str,
    range: std::ops::RangeInclusive<f64>,
    required: bool,
) -> Result<(), String> {
    match settings.get(key) {
        None if !required => Ok(()),
        None => Err(format!("{key} is required")),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && range.contains(&n) => Ok(()),
            _ => Err(format!(
                "{key} is a number from {} to {}",
                range.start(),
                range.end()
            )),
        },
    }
}

/// Checks one described operation; GIMP never sees one that fails here.
pub fn check_operation(op: &Value) -> Result<(), String> {
    let (name, s) = match op {
        Value::String(name) => (name.as_str(), json!({})),
        Value::Object(map) if map.len() == 1 => {
            let (name, s) = map.iter().next().expect("one entry");
            (name.as_str(), s.clone())
        }
        _ => {
            return Err(format!(
                "an operation is a name or {{name: settings}}: {op}"
            ))
        }
    };
    match name {
        "resize" => {
            in_range(&s, "width", 1.0..=30000.0, false)?;
            in_range(&s, "height", 1.0..=30000.0, false)?;
            if s.get("width").is_none() && s.get("height").is_none() {
                return Err("resize needs width, height or both".into());
            }
        }
        "crop" => {
            in_range(&s, "width", 1.0..=30000.0, true)?;
            in_range(&s, "height", 1.0..=30000.0, true)?;
            in_range(&s, "x", 0.0..=30000.0, false)?;
            in_range(&s, "y", 0.0..=30000.0, false)?;
        }
        "rotate" => {
            if !matches!(
                s.get("degrees").and_then(Value::as_u64),
                Some(90 | 180 | 270)
            ) {
                return Err("rotate takes degrees 90, 180 or 270".into());
            }
        }
        "flip" => {
            if !matches!(
                s.get("direction").and_then(Value::as_str),
                None | Some("horizontal" | "vertical")
            ) {
                return Err("flip direction is horizontal or vertical".into());
            }
        }
        "brightness_contrast" => {
            in_range(&s, "brightness", -1.0..=1.0, false)?;
            in_range(&s, "contrast", -1.0..=1.0, false)?;
        }
        "hue_saturation" => {
            in_range(&s, "hue", -180.0..=180.0, false)?;
            in_range(&s, "lightness", -100.0..=100.0, false)?;
            in_range(&s, "saturation", -100.0..=100.0, false)?;
        }
        "blur" => in_range(&s, "radius", 0.1..=200.0, false)?,
        "sharpen" => {
            in_range(&s, "radius", 0.1..=50.0, false)?;
            in_range(&s, "amount", 0.0..=5.0, false)?;
        }
        "text" => {
            let text = s.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() || text.len() > 500 {
                return Err("text needs text, up to 500 characters".into());
            }
            in_range(&s, "size", 4.0..=2000.0, false)?;
            in_range(&s, "x", 0.0..=30000.0, false)?;
            in_range(&s, "y", 0.0..=30000.0, false)?;
            if let Some(colour) = s.get("color") {
                let ok = colour.as_str().is_some_and(|c| {
                    c.len() == 7
                        && c.starts_with('#')
                        && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
                });
                if !ok {
                    return Err("color is #rrggbb".into());
                }
            }
            if let Some(font) = s.get("font") {
                let ok = font.as_str().is_some_and(|f| {
                    !f.is_empty()
                        && f.len() <= 80
                        && f.chars().all(|c| c.is_alphanumeric() || " -_".contains(c))
                });
                if !ok {
                    return Err("font is a font's name".into());
                }
            }
        }
        "desaturate" | "invert" | "auto_levels" | "flatten" => {}
        other => return Err(format!("{other} is not an operation this bridge runs")),
    }
    Ok(())
}

impl Gimp {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            gimp: programs.get("gimp-console"),
        }
    }

    fn run(&self, arguments: Value) -> Result<Value, String> {
        let answer = JOB
            .run(
                &self.workspace,
                self.gimp.clone(),
                |_| {
                    vec![
                        "-i".into(),
                        "--batch-interpreter=python-fu-eval".into(),
                        "-b".into(),
                        script::BOOT.into(),
                        "--quit".into(),
                    ]
                },
                &arguments,
                LIMIT,
            )
            .map_err(|e| e.to_string())?;
        if let Some(error) = answer.value.get("error").and_then(Value::as_str) {
            return Err(format!("GIMP: {error}"));
        }
        Ok(answer.value)
    }

    fn input(&self, arguments: &Map<String, Value>) -> Result<PathBuf, String> {
        let file = arguments
            .get("file")
            .and_then(Value::as_str)
            .ok_or("file is an image in the workspace")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !READS.contains(&ext.as_str()) {
            return Err(format!("{file} is not an image GIMP reads here: {READS:?}"));
        }
        Ok(path)
    }

    fn stem(path: &Path) -> String {
        crate::delivery::to_name(path.file_stem().and_then(|s| s.to_str()).unwrap_or("image"))
    }

    fn with_picture(&self, answer: Value, png: &Path) -> Outcome {
        let outcome = Outcome::json(&answer);
        match std::fs::read(png) {
            Ok(bytes) if bytes.len() <= MAX_IMAGE => outcome.with_image(Image {
                mime: "image/png",
                data: bytes,
            }),
            _ => outcome,
        }
    }

    fn edit(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let Some(operations) = arguments
            .get("operations")
            .and_then(Value::as_array)
            .filter(|o| !o.is_empty())
        else {
            return Outcome::error(
                "operations is a list, such as [\"auto_levels\", {\"resize\": {\"width\": 1600}}]",
            );
        };
        for op in operations {
            if let Err(why) = check_operation(op) {
                return Outcome::error(why);
            }
        }
        let format = arguments
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("png");
        if !EXPORTS.contains(&format) {
            return Outcome::error(format!("format is one of {EXPORTS:?}"));
        }
        let output = match self
            .workspace
            .fresh_file("edits", &Self::stem(&path), format)
        {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let preview = output.with_extension("preview.png");
        match self.run(json!({
            "action": "edit", "file": path, "operations": operations, "output": output,
            "flatten": !LAYERED.contains(&format), "preview": preview,
        })) {
            Ok(report) => {
                let mut answer = json!({"file": self.workspace.relative(&output), "image": report});
                answer["preview"] = json!(self.workspace.relative(&preview));
                self.with_picture(answer, &preview)
            }
            Err(why) => {
                let _ = std::fs::remove_file(&output);
                Outcome::error(why)
            }
        }
    }

    fn export(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let formats: Vec<String> = match arguments.get("formats").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => vec!["psd".into()],
        };
        if let Some(bad) = formats.iter().find(|f| !EXPORTS.contains(&f.as_str())) {
            return Outcome::error(format!("{bad} is not one of {EXPORTS:?}"));
        }
        let stem = Self::stem(&path);
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let mut targets: Vec<(PathBuf, bool)> = formats
            .iter()
            .map(|f| {
                (
                    folder.join(format!("{stem}.{f}")),
                    !LAYERED.contains(&f.as_str()),
                )
            })
            .collect();
        let preview = folder.join(format!("{stem}-preview.png"));
        targets.push((preview.clone(), true));
        match self.run(json!({"action": "export", "file": path, "targets": targets})) {
            Ok(report) => {
                let files: Vec<String> = targets[..targets.len() - 1]
                    .iter()
                    .map(|(p, _)| self.workspace.relative(p))
                    .collect();
                self.with_picture(json!({"folder": self.workspace.relative(&folder), "files": files, "image": report["image"]}), &preview)
            }
            Err(why) => Outcome::error(why),
        }
    }
}

impl Bridge for Gimp {
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
        let file = json!({"type": "string", "description": "An image in the workspace: XCF, PSD, PNG, JPEG, TIFF, WebP…"});
        vec![
            BridgeTool {
                name: "gimp_images",
                title: "Images",
                description: "The raster images in the workspace.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "gimp_inspect",
                title: "What an image holds",
                description: "Size, colour mode, precision, and each layer's name, visibility, opacity, size and offset.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "gimp_edit",
                title: "Retouch an image",
                description: "Applies operations in order and saves edits/<name>-<n>.<format> with a picture: resize \
                              {width, height}, crop {x, y, width, height}, rotate {degrees}, flip {direction}, \
                              brightness_contrast {brightness, contrast: -1..1}, hue_saturation {hue, lightness, \
                              saturation}, desaturate, invert, auto_levels, blur {radius}, sharpen {radius, amount}, \
                              text {text, x, y, size, color, font}, flatten. Default format png; psd, xcf and tif keep \
                              layers.",
                input_schema: json!({
                    "type": "object",
                    "properties": {"file": file, "operations": {"type": "array"}, "format": {"type": "string", "enum": EXPORTS}},
                    "required": ["file", "operations"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "gimp_export",
                title: "Export for other software",
                description: "Exports into exports/<name>-<n>/: psd (Photoshop, layers kept), xcf, tif, png, jpg, webp, \
                              pdf, gif. Default psd.",
                input_schema: json!({
                    "type": "object",
                    "properties": {"file": file, "formats": {"type": "array", "items": {"type": "string", "enum": EXPORTS}}},
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "gimp_images" => {
                let (files, truncated) = self.workspace.find(&READS, 5, 2000);
                Outcome::json(&json!({"files": files, "truncated": truncated}))
            }
            "gimp_inspect" => match self.input(arguments) {
                Ok(path) => match self.run(json!({"action": "inspect", "file": path})) {
                    Ok(report) => Outcome::json(&report),
                    Err(why) => Outcome::error(why),
                },
                Err(why) => Outcome::error(why),
            },
            "gimp_edit" => self.edit(arguments),
            "gimp_export" => self.export(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_are_named_and_bounded() {
        for good in [
            json!("auto_levels"),
            json!({"resize": {"width": 1600}}),
            json!({"brightness_contrast": {"brightness": 0.1, "contrast": 0.2}}),
            json!({"text": {"text": "Hola", "size": 64, "color": "#ffffff", "font": "DejaVu Sans"}}),
        ] {
            assert!(check_operation(&good).is_ok(), "{good}");
        }
        for bad in [
            json!({"resize": {}}),
            json!({"rotate": {"degrees": 45}}),
            json!({"brightness_contrast": {"brightness": 5}}),
            json!({"text": {"text": "x", "font": "a'); import os #"}}),
            json!({"text": {"text": "x", "color": "red"}}),
            json!("python_fu_eval"),
        ] {
            assert!(check_operation(&bad).is_err(), "{bad}");
        }
    }
}
