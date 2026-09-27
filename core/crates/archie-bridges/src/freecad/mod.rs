//! FreeCAD: parametric solids, built, varied, checked and exported for any CAD package.
//!
//! FreeCAD is driven through `FreeCADCmd` with the one script in [`script`], which takes
//! its instructions as JSON. Pictures are drawn by [`crate::preview`] from FreeCAD's own
//! tessellation, so no display is needed. A sweep of parameters runs as a [`crate::jobs`]
//! job that outlives the conversation and delivers every iteration to the person's desk.

pub mod script;

use crate::jobs::{self, Runner, Spec};
use crate::manifest::Manifest;
use crate::preview::{read_stl, render_png, View};
use crate::script::Script;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/freecad/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/freecad/SKILL.md");

const JOB: Script = Script {
    name: "freecad",
    extension: "py",
    text: script::JOB,
};
const READ_LIMIT: Duration = Duration::from_secs(180);
const WORK_LIMIT: Duration = Duration::from_secs(900);
const FORMATS: [&str; 8] = ["step", "iges", "brep", "stl", "obj", "3mf", "ply", "fcstd"];
const READS: [&str; 10] = ["fcstd", "step", "stp", "iges", "igs", "brep", "stl", "obj", "3mf", "ply"];
/// The most variants one sweep may make.
const MAX_SWEEP: u64 = 100_000;
const MAX_IMAGE: usize = 700 * 1024;

pub struct Freecad {
    manifest: Manifest,
    workspace: Workspace,
    programs: Programs,
}

impl Freecad {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            programs: programs.clone(),
        }
    }

    /// Runs the script; an `error` in its answer is an error here.
    fn run(&self, arguments: Value, limit: Duration) -> Result<Value, String> {
        let answer = JOB
            .run(
                &self.workspace,
                self.programs.get("freecadcmd"),
                |script| vec![script.as_os_str().to_os_string()],
                &arguments,
                limit,
            )
            .map_err(|e| e.to_string())?;
        if let Some(error) = answer.value.get("error").and_then(Value::as_str) {
            let trace = answer.value["trace"].as_str().unwrap_or("");
            let last = trace.lines().rev().take(4).collect::<Vec<_>>();
            return Err(format!("FreeCAD: {error}\n{}", last.into_iter().rev().collect::<Vec<_>>().join("\n")));
        }
        Ok(self.relative_paths(answer.value))
    }

    /// Absolute paths FreeCAD reports become workspace-relative ones.
    fn relative_paths(&self, value: Value) -> Value {
        match value {
            Value::String(text) if Path::new(&text).is_absolute() => {
                Value::String(self.workspace.relative(Path::new(&text)))
            }
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.relative_paths(v)).collect()),
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, self.relative_paths(v))).collect()),
            other => other,
        }
    }

    fn input(&self, arguments: &Map<String, Value>) -> Result<PathBuf, String> {
        let file = arguments
            .get("file")
            .and_then(Value::as_str)
            .ok_or("file is a design in the workspace: .FCStd, .step, .iges, .brep or a mesh")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !READS.contains(&extension.as_str()) {
            return Err(format!("{file} is not a kind FreeCAD reads here: {READS:?}"));
        }
        Ok(path)
    }

    fn stem(path: &Path) -> String {
        crate::delivery::to_name(path.file_stem().and_then(|s| s.to_str()).unwrap_or("part"))
    }

    /// A shaded picture of `file` from `view`, written to `png`. Returns its bytes.
    fn picture(&self, file: &Path, view: View, png: &Path, size: (u32, u32)) -> Result<Vec<u8>, String> {
        let scratch = self.workspace.scratch("freecad").map_err(|e| e.to_string())?;
        let stem = "preview";
        let mesh = scratch.join(format!("{stem}.stl"));
        let _ = std::fs::remove_file(&mesh);
        self.run(
            json!({"action": "export", "file": file, "formats": ["stl"], "folder": scratch, "stem": stem}),
            WORK_LIMIT,
        )?;
        let triangles = read_stl(&std::fs::read(&mesh).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let bytes = render_png(&triangles, view, size.0, size.1).map_err(|e| e.to_string())?;
        std::fs::write(png, &bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Adds a picture of `file` to an answer, beside `folder` or in renders/.
    fn with_picture(&self, answer: Value, file: &Path, png: PathBuf) -> Outcome {
        match self.picture(file, View::Iso, &png, (1200, 900)) {
            Ok(bytes) => {
                let mut answer = answer;
                answer["preview"] = json!(self.workspace.relative(&png));
                let outcome = Outcome::json(&answer);
                if bytes.len() <= MAX_IMAGE {
                    outcome.with_image(Image { mime: "image/png", data: bytes })
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

    fn documents(&self) -> Outcome {
        let (files, truncated) = self.workspace.find(&READS, 5, 2000);
        Outcome::json(&json!({"files": files, "truncated": truncated}))
    }

    fn inspect(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        match self.run(json!({"action": "inspect", "file": path}), READ_LIMIT) {
            Ok(report) => Outcome::json(&report),
            Err(why) => Outcome::error(why),
        }
    }

    fn build(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return Outcome::error("freecad_build needs name, which names the new .FCStd");
        };
        let Some(parts) = arguments.get("parts").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
            return Outcome::error("freecad_build needs parts");
        };
        let stem = crate::delivery::to_name(name);
        let output = match self.workspace.fresh_file("designs", &stem, "FCStd") {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let spec = json!({"parameters": arguments.get("parameters").cloned().unwrap_or(json!({})), "parts": parts});
        match self.run(json!({"action": "build", "spec": spec, "output": output, "document": "Part"}), WORK_LIMIT) {
            Ok(mut report) => {
                report["file"] = json!(self.workspace.relative(&output));
                let png = output.with_extension("png");
                self.with_picture(report, &output, png)
            }
            Err(why) => {
                let _ = std::fs::remove_file(&output);
                Outcome::error(why)
            }
        }
    }

    fn set_parameters(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let Some(parameters) = arguments.get("parameters").and_then(Value::as_object).filter(|p| !p.is_empty()) else {
            return Outcome::error("parameters is an object of spreadsheet alias to value, such as {\"wall\": \"3 mm\"}");
        };
        let output = match self.workspace.fresh_file("variants", &Self::stem(&path), "FCStd") {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.run(
            json!({"action": "set", "file": path, "parameters": parameters, "output": output}),
            WORK_LIMIT,
        ) {
            Ok(mut report) => {
                report["file"] = json!(self.workspace.relative(&output));
                let png = output.with_extension("png");
                self.with_picture(report, &output, png)
            }
            Err(why) => {
                let _ = std::fs::remove_file(&output);
                Outcome::error(why)
            }
        }
    }

    fn formats(arguments: &Map<String, Value>) -> Result<Vec<String>, String> {
        let formats: Vec<String> = match arguments.get("formats") {
            None => vec!["step".into()],
            Some(Value::Array(list)) => list
                .iter()
                .map(|f| f.as_str().map(str::to_ascii_lowercase))
                .collect::<Option<_>>()
                .ok_or("formats are names")?,
            Some(_) => return Err("formats is a list".into()),
        };
        for format in &formats {
            if !FORMATS.contains(&format.as_str()) {
                return Err(format!("{format} is not one of {FORMATS:?}"));
            }
        }
        Ok(formats)
    }

    fn export(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let formats = match Self::formats(arguments) {
            Ok(formats) => formats,
            Err(why) => return Outcome::error(why),
        };
        let stem = Self::stem(&path);
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.run(
            json!({"action": "export", "file": path, "formats": formats, "folder": folder, "stem": stem}),
            WORK_LIMIT,
        ) {
            Ok(report) => {
                let answer = json!({"folder": self.workspace.relative(&folder), "files": report["exported"], "summary": report["summary"]});
                let png = folder.join(format!("{stem}-iso.png"));
                self.with_picture(answer, &path, png)
            }
            Err(why) => Outcome::error(why),
        }
    }

    fn render(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let view_name = arguments.get("view").and_then(Value::as_str).unwrap_or("iso");
        let Some(view) = View::parse(view_name) else {
            return Outcome::error(format!("view is one of {:?}", View::NAMES));
        };
        let size = |key: &str, default: u64| {
            arguments.get(key).and_then(Value::as_u64).unwrap_or(default).clamp(200, 2400) as u32
        };
        let png = match self
            .workspace
            .fresh_file("renders", &format!("{}-{view_name}", Self::stem(&path)), "png")
        {
            Ok(png) => png,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.picture(&path, view, &png, (size("width", 1200), size("height", 900))) {
            Ok(bytes) => {
                let outcome = Outcome::json(&json!({"image": self.workspace.relative(&png), "view": view_name}));
                if bytes.len() <= MAX_IMAGE {
                    outcome.with_image(Image { mime: "image/png", data: bytes })
                } else {
                    outcome
                }
            }
            Err(why) => Outcome::error(why),
        }
    }

    fn sweep(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let Some(grid) = arguments.get("parameters").and_then(Value::as_object).filter(|g| !g.is_empty()) else {
            return Outcome::error("parameters maps each spreadsheet alias to the list of values to try");
        };
        let mut total: u64 = 1;
        for (alias, values) in grid {
            let count = values.as_array().map(Vec::len).unwrap_or(0) as u64;
            if count == 0 {
                return Outcome::error(format!("{alias} needs a non-empty list of values"));
            }
            total = total.saturating_mul(count);
        }
        if total > MAX_SWEEP {
            return Outcome::error(format!("{total} variants is more than {MAX_SWEEP}; narrow the lists"));
        }
        let formats = match Self::formats(arguments) {
            Ok(formats) => formats,
            Err(why) => return Outcome::error(why),
        };
        let deliver = arguments.get("deliver_to_project").and_then(Value::as_str).map(str::to_string);
        if deliver.is_some() && self.programs.delivery.is_none() {
            return Outcome::error("deliver_to_project needs a paired desk: start the server with --deliver-key");
        }
        let spec = Spec {
            bridge: "freecad".into(),
            kind: "sweep".into(),
            arguments: json!({
                "file": self.workspace.relative(&path),
                "grid": grid,
                "formats": formats,
                "deliver_to_project": deliver,
            }),
        };
        match jobs::start(&self.workspace, &self.programs, &spec, total) {
            Ok(id) => Outcome::json(&json!({
                "job": id,
                "variants": total,
                "next": "archie_job_status {bridge: freecad, job} shows progress; archie_job_cancel stops it after the current variant.",
            })),
            Err(e) => Outcome::error(e.to_string()),
        }
    }

    /// The sweep itself, inside `archie-bridges run-job`.
    fn run_sweep(&self, arguments: &Value, runner: &mut Runner) -> Result<(), String> {
        let file = arguments["file"].as_str().ok_or("the job names no file")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let grid: Vec<(String, Vec<Value>)> = arguments["grid"]
            .as_object()
            .ok_or("the job has no grid")?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_array().cloned().unwrap_or_default()))
            .collect();
        let formats: Vec<Value> = arguments["formats"].as_array().cloned().unwrap_or_default();
        let project = arguments["deliver_to_project"].as_str().map(str::to_string);
        let deliverer = match (&project, self.programs.deliverer()) {
            (Some(_), Some(Ok(deliverer))) => Some(deliverer),
            (Some(_), Some(Err(e))) => return Err(e.to_string()),
            _ => None,
        };
        let stem = Self::stem(&path);
        let folder = self
            .workspace
            .fresh_folder("sweeps", &stem)
            .map_err(|e| e.to_string())?;
        let total: usize = grid.iter().map(|(_, v)| v.len()).product();
        let mut summary = String::from("variant,");
        summary.push_str(&grid.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(","));
        summary.push_str(",valid,volume_mm3,size_x_mm,size_y_mm,size_z_mm,error\n");
        for index in 0..total {
            if runner.cancelled() {
                break;
            }
            // The index in mixed radix: the last parameter changes fastest.
            let mut rest = index;
            let mut chosen = Map::new();
            for (alias, values) in grid.iter().rev() {
                chosen.insert(alias.clone(), values[rest % values.len()].clone());
                rest /= values.len();
            }
            let label = format!("{:04}", index + 1);
            let variant = folder.join(format!("{stem}-{label}"));
            std::fs::create_dir_all(&variant).map_err(|e| e.to_string())?;
            let design = variant.join(format!("{stem}-{label}.FCStd"));
            let mut all_formats = formats.clone();
            all_formats.push(json!("stl"));
            let outcome = self.run(
                json!({"action": "set", "file": path, "parameters": chosen, "output": design,
                       "formats": all_formats, "folder": variant, "stem": format!("{stem}-{label}")}),
                WORK_LIMIT,
            );
            let values = grid
                .iter()
                .map(|(k, _)| chosen[k].to_string().trim_matches('"').to_string())
                .collect::<Vec<_>>()
                .join(",");
            match outcome {
                Ok(report) => {
                    let mesh = variant.join(format!("{stem}-{label}.stl"));
                    let png = variant.join(format!("{stem}-{label}-iso.png"));
                    if let Ok(triangles) = std::fs::read(&mesh).map_err(|e| e.to_string()).and_then(|b| read_stl(&b).map_err(|e| e.to_string())) {
                        if let Ok(bytes) = render_png(&triangles, View::Iso, 1200, 900) {
                            let _ = std::fs::write(&png, bytes);
                        }
                    }
                    let _ = std::fs::remove_file(&mesh);
                    let size = &report["results"][0]["size_mm"];
                    summary.push_str(&format!(
                        "{label},{values},{},{},{},{},{},\n",
                        report["valid"], report["volume_mm3"], size[0], size[1], size[2]
                    ));
                    let result = json!({"variant": label, "parameters": chosen, "valid": report["valid"],
                                        "volume_mm3": report["volume_mm3"], "size_mm": size,
                                        "folder": self.workspace.relative(&variant)});
                    if let (Some(deliverer), Some(project)) = (&deliverer, &project) {
                        let mut files: Vec<String> = std::fs::read_dir(&variant)
                            .map_err(|e| e.to_string())?
                            .flatten()
                            .map(|e| self.workspace.relative(&e.path()))
                            .collect();
                        files.sort();
                        let note = format!(
                            "{} · volume {} mm³ · {}",
                            chosen.iter().map(|(k, v)| format!("{k}={}", v.to_string().trim_matches('"'))).collect::<Vec<_>>().join(", "),
                            report["volume_mm3"],
                            if report["valid"] == json!(true) { "valid" } else { "INVALID" }
                        );
                        if let Err(e) = deliverer.deliver(&self.workspace, &files, project, &format!("variant-{label}"), &note, false) {
                            eprintln!("delivery: {e}");
                        }
                    }
                    runner.record(result).map_err(|e| e.to_string())?;
                }
                Err(why) => {
                    summary.push_str(&format!("{label},{values},false,,,,,\"{}\"\n", why.lines().next().unwrap_or("").replace('"', "'")));
                    runner
                        .record(json!({"variant": label, "parameters": chosen, "error": why}))
                        .map_err(|e| e.to_string())?;
                }
            }
            let _ = std::fs::write(folder.join("summary.csv"), &summary);
        }
        if let (Some(deliverer), Some(project)) = (&deliverer, &project) {
            let summary = self.workspace.relative(&folder.join("summary.csv"));
            if let Err(e) = deliverer.deliver(&self.workspace, &[summary], project, "summary", "every variant, one row each", false) {
                eprintln!("delivery: {e}");
            }
        }
        Ok(())
    }
}

impl Bridge for Freecad {
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
        let file = |description: &str| json!({"type": "string", "description": description});
        vec![
            BridgeTool {
                name: "freecad_documents",
                title: "CAD files",
                description: "The designs in the workspace FreeCAD can read: .FCStd, STEP, IGES, BREP and meshes.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "freecad_inspect",
                title: "What a design holds",
                description: "Opens a design and reports each resulting solid (volume mm³, area, bounding box and \
                              size in mm, validity, number of solids) and every named spreadsheet parameter with \
                              its value and unit. Read this before changing parameters.",
                input_schema: json!({"type": "object", "properties": {"file": file("A design in the workspace.")},
                                     "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "freecad_build",
                title: "Build a part",
                description: "Builds a new parametric part into designs/<name>-<n>.FCStd from named parameters \
                              (a spreadsheet called Params) and parts: primitives (box length/width/height, \
                              cylinder radius/height, sphere, cone, torus, wedge) placed with at [x,y,z] and \
                              rotate {axis, angle}; booleans (cut base minus tools, fuse, common); fillet or \
                              chamfer of all or listed edges. A dimension is a number in mm or an expression \
                              over the parameters, such as \"wall*2\". Returns the solid's report and a picture.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "parameters": {"type": "object", "additionalProperties": {"type": ["number", "string"]},
                                       "description": "Such as {\"width\": 80, \"wall\": \"3 mm\"}."},
                        "parts": {"type": "array", "items": {"type": "object", "properties": {
                            "name": {"type": "string"},
                            "kind": {"type": "string", "enum": ["box", "cylinder", "sphere", "cone", "torus", "wedge", "cut", "fuse", "common", "fillet", "chamfer"]},
                            "base": {"type": "string"}, "tools": {"type": "array", "items": {"type": "string"}},
                            "at": {"type": "array", "items": {"type": ["number", "string"]}, "minItems": 3, "maxItems": 3},
                            "rotate": {"type": "object", "properties": {"axis": {"type": "array", "items": {"type": "number"}}, "angle": {"type": "number"}}}
                        }, "required": ["name", "kind"]}}
                    },
                    "required": ["name", "parts"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "freecad_set_parameters",
                title: "Make a variant",
                description: "Sets spreadsheet parameters of a design, recomputes it and saves the result as a new \
                              variants/<design>-<n>.FCStd, leaving the original as it was. Returns the new \
                              solid's report, validity and a picture.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": file("A .FCStd with spreadsheet aliases."),
                        "parameters": {"type": "object", "additionalProperties": {"type": ["number", "string"]}}
                    },
                    "required": ["file", "parameters"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "freecad_export",
                title: "Export for other CAD software",
                description: "Exports a design's solids into exports/<design>-<n>/ with a picture: step (Fusion 360, \
                              SolidWorks, Onshape, Inventor, COMSOL, ANSYS), iges, brep, stl or 3mf (printing), \
                              obj or ply (rendering), fcstd. Default step.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": file("A design in the workspace."),
                        "formats": {"type": "array", "items": {"type": "string", "enum": FORMATS}}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "freecad_render",
                title: "Picture of a design",
                description: "A shaded picture of a design from iso, front, back, top, bottom, left or right, into \
                              renders/, returned as an image. Drawn without a display.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": file("A design in the workspace."),
                        "view": {"type": "string", "enum": View::NAMES},
                        "width": {"type": "integer", "minimum": 200, "maximum": 2400},
                        "height": {"type": "integer", "minimum": 200, "maximum": 2400}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "freecad_sweep",
                title: "Try many variants, for hours",
                description: "Starts a job that makes every combination of the listed parameter values, each \
                              recomputed, checked, exported and pictured under sweeps/<design>-<n>/, with a \
                              summary.csv. It runs on after this conversation ends; with deliver_to_project each \
                              variant is delivered to the person's desk as it finishes. Returns the job id.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": file("A .FCStd with spreadsheet aliases."),
                        "parameters": {"type": "object", "additionalProperties": {"type": "array"},
                                       "description": "Alias to the values to try, such as {\"wall\": [2, 2.5, 3]}."},
                        "formats": {"type": "array", "items": {"type": "string", "enum": FORMATS}},
                        "deliver_to_project": {"type": "string"}
                    },
                    "required": ["file", "parameters"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "freecad_documents" => self.documents(),
            "freecad_inspect" => self.inspect(arguments),
            "freecad_build" => self.build(arguments),
            "freecad_set_parameters" => self.set_parameters(arguments),
            "freecad_export" => self.export(arguments),
            "freecad_render" => self.render(arguments),
            "freecad_sweep" => self.sweep(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }

    fn run_job(&self, spec: &Spec, runner: &mut Runner) -> Result<(), String> {
        match spec.kind.as_str() {
            "sweep" => self.run_sweep(&spec.arguments, runner),
            other => Err(format!("freecad runs no {other} jobs")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn freecad(name: &str) -> (Freecad, PathBuf) {
        let dir = std::env::temp_dir().join(format!("archie-freecad-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bracket.FCStd"), b"PK").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let bridge = Freecad::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &Programs::default(),
        );
        (bridge, dir)
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn inputs_and_formats_are_checked_before_freecad_runs() {
        let (bridge, dir) = freecad("inputs");
        assert!(bridge.call("freecad_inspect", &args(json!({"file": "notes.txt"}))).text.contains("not a kind"));
        assert!(bridge.call("freecad_inspect", &args(json!({"file": "../x.FCStd"}))).is_error);
        let bad = bridge.call("freecad_export", &args(json!({"file": "bracket.FCStd", "formats": ["dwg"]})));
        assert!(bad.text.contains("not one of"), "{}", bad.text);
        let huge = bridge.call(
            "freecad_sweep",
            &args(json!({"file": "bracket.FCStd", "parameters": {"a": (0..400).collect::<Vec<_>>(), "b": (0..400).collect::<Vec<_>>()}})),
        );
        assert!(huge.text.contains("narrow"), "{}", huge.text);
        let undelivered = bridge.call(
            "freecad_sweep",
            &args(json!({"file": "bracket.FCStd", "parameters": {"a": [1, 2]}, "deliver_to_project": "p"})),
        );
        assert!(undelivered.text.contains("--deliver-key"), "{}", undelivered.text);
        assert!(!dir.join("exports").exists() && !dir.join("sweeps").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_script_takes_everything_as_data() {
        // The script reads its instructions from the JSON file and has no place where a
        // value could be spliced into code.
        assert!(script::JOB.contains("json.load(open(os.environ[\"ARCHIE_JOB_ARGS\"]"));
        for forbidden in ["eval(", "exec(", "__import__", "subprocess", "os.system"] {
            assert!(!script::JOB.contains(forbidden), "{forbidden}");
        }
        let paths = bridge_paths();
        assert_eq!(paths, json!({"a": "sub/x.step", "n": 1}));
    }

    fn bridge_paths() -> Value {
        let (bridge, dir) = freecad("paths");
        let root = bridge.workspace.root().to_path_buf();
        let value = bridge.relative_paths(json!({"a": root.join("sub/x.step"), "n": 1}));
        let _ = std::fs::remove_dir_all(dir);
        value
    }
}
