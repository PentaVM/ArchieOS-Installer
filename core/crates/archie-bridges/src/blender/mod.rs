//! Blender: scenes read, assembled, rendered and exported for Maya, 3ds Max, Unreal,
//! Unity or a web viewer, with a studio set up for a bare model.
//!
//! `blender --background --factory-startup` runs the one script in [`script`] with its
//! instructions as JSON. Stills render in the call; animations and turntables render as a
//! [`crate::jobs`] job, a few frames per Blender run so progress shows and a cancel lands
//! between chunks, then FFmpeg joins the frames.

pub mod script;

use crate::jobs::{self, Runner, Spec};
use crate::manifest::Manifest;
use crate::run::Run;
use crate::script::Script;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/blender/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/blender/SKILL.md");

const JOB: Script = Script {
    name: "blender",
    extension: "py",
    text: script::JOB,
};
const READ_LIMIT: Duration = Duration::from_secs(300);
const RENDER_LIMIT: Duration = Duration::from_secs(3 * 3600);
const READS: [&str; 12] = [
    "blend", "fbx", "glb", "gltf", "obj", "stl", "ply", "usd", "usda", "usdc", "usdz", "abc",
];
const EXPORTS: [&str; 10] = [
    "fbx", "glb", "gltf", "obj", "stl", "ply", "usdc", "usdz", "abc", "blend",
];
const ENGINES: [&str; 3] = ["cycles", "eevee", "workbench"];
const CHUNK: u64 = 12;
const MAX_FRAMES: u64 = 2400;
const MAX_IMAGE: usize = 700 * 1024;

pub struct Blender {
    manifest: Manifest,
    workspace: Workspace,
    programs: Programs,
}

impl Blender {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            programs: programs.clone(),
        }
    }

    fn run(&self, arguments: Value, limit: Duration) -> Result<Value, String> {
        let answer = JOB
            .run(
                &self.workspace,
                self.programs.get("blender"),
                |script| {
                    vec![
                        "--background".into(),
                        "--factory-startup".into(),
                        "--python-exit-code".into(),
                        "1".into(),
                        "--python".into(),
                        script.as_os_str().to_os_string(),
                    ]
                },
                &arguments,
                limit,
            )
            .map_err(|e| e.to_string())?;
        if let Some(error) = answer.value.get("error").and_then(Value::as_str) {
            return Err(format!("Blender: {error}"));
        }
        Ok(answer.value)
    }

    fn inputs(&self, arguments: &Map<String, Value>) -> Result<Vec<PathBuf>, String> {
        let names: Vec<&str> = match (arguments.get("file"), arguments.get("files")) {
            (Some(Value::String(file)), _) => vec![file.as_str()],
            (_, Some(Value::Array(files))) if !files.is_empty() => {
                files.iter().filter_map(Value::as_str).collect()
            }
            _ => return Err("file (or files) names scenes or models in the workspace".into()),
        };
        names
            .into_iter()
            .map(|name| {
                let path = self.workspace.file(name).map_err(|e| e.to_string())?;
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                if READS.contains(&ext.as_str()) {
                    Ok(path)
                } else {
                    Err(format!("{name} is not a kind Blender opens here: {READS:?}; convert CAD with freecad_export to stl, obj or glb first"))
                }
            })
            .collect()
    }

    fn stem(path: &Path) -> String {
        crate::delivery::to_name(path.file_stem().and_then(|s| s.to_str()).unwrap_or("scene"))
    }

    fn settings(arguments: &Map<String, Value>) -> Result<Value, String> {
        let engine = arguments
            .get("engine")
            .and_then(Value::as_str)
            .unwrap_or("cycles");
        if !ENGINES.contains(&engine) {
            return Err(format!("engine is one of {ENGINES:?}"));
        }
        let number = |key: &str, default: u64, lo: u64, hi: u64| {
            arguments
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or(default)
                .clamp(lo, hi)
        };
        Ok(json!({
            "engine": engine,
            "samples": number("samples", 64, 1, 4096),
            "width": number("width", 1600, 64, 7680),
            "height": number("height", 1000, 64, 4320),
            "studio": arguments.get("studio").and_then(Value::as_bool).unwrap_or(true),
        }))
    }

    fn image(&self, answer: Value, png: &Path) -> Outcome {
        let outcome = Outcome::json(&answer);
        match std::fs::read(png) {
            Ok(bytes) if bytes.len() <= MAX_IMAGE => outcome.with_image(Image {
                mime: "image/png",
                data: bytes,
            }),
            _ => outcome,
        }
    }

    fn inspect(&self, arguments: &Map<String, Value>) -> Outcome {
        let files = match self.inputs(arguments) {
            Ok(files) => files,
            Err(why) => return Outcome::error(why),
        };
        match self.run(json!({"action": "inspect", "files": files}), READ_LIMIT) {
            Ok(report) => Outcome::json(&report),
            Err(why) => Outcome::error(why),
        }
    }

    fn assemble(&self, arguments: &Map<String, Value>) -> Outcome {
        let files = match self.inputs(arguments) {
            Ok(files) => files,
            Err(why) => return Outcome::error(why),
        };
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("scene");
        let output =
            match self
                .workspace
                .fresh_file("scenes", &crate::delivery::to_name(name), "blend")
            {
                Ok(output) => output,
                Err(e) => return Outcome::error(e.to_string()),
            };
        match self.run(
            json!({"action": "save", "files": files, "output": output}),
            READ_LIMIT,
        ) {
            Ok(mut report) => {
                report["file"] = json!(self.workspace.relative(&output));
                Outcome::json(&report)
            }
            Err(why) => Outcome::error(why),
        }
    }

    fn render(&self, arguments: &Map<String, Value>) -> Outcome {
        let files = match self.inputs(arguments) {
            Ok(files) => files,
            Err(why) => return Outcome::error(why),
        };
        let mut settings = match Self::settings(arguments) {
            Ok(settings) => settings,
            Err(why) => return Outcome::error(why),
        };
        let png = match self
            .workspace
            .fresh_file("renders", &Self::stem(&files[0]), "png")
        {
            Ok(png) => png,
            Err(e) => return Outcome::error(e.to_string()),
        };
        settings["action"] = json!("render");
        settings["files"] = json!(files);
        settings["output"] = json!(png);
        if let Some(frame) = arguments.get("frame").and_then(Value::as_i64) {
            settings["frame"] = json!(frame);
        }
        match self.run(settings, RENDER_LIMIT) {
            Ok(report) => self.image(
                json!({"image": self.workspace.relative(&png), "engine": report["engine"], "studio_added": report["added"]}),
                &png,
            ),
            Err(why) => Outcome::error(why),
        }
    }

    fn export(&self, arguments: &Map<String, Value>) -> Outcome {
        let files = match self.inputs(arguments) {
            Ok(files) => files,
            Err(why) => return Outcome::error(why),
        };
        let formats: Vec<String> = match arguments.get("formats").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => list
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => vec!["glb".into()],
        };
        if let Some(bad) = formats.iter().find(|f| !EXPORTS.contains(&f.as_str())) {
            return Outcome::error(format!("{bad} is not one of {EXPORTS:?}"));
        }
        let stem = Self::stem(&files[0]);
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.run(json!({"action": "export", "files": files, "formats": formats, "folder": folder, "stem": stem}), READ_LIMIT) {
            Ok(report) => {
                let written: Vec<String> = report["exported"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|p| self.workspace.relative(Path::new(p)))
                    .collect();
                Outcome::json(&json!({"folder": self.workspace.relative(&folder), "files": written}))
            }
            Err(why) => Outcome::error(why),
        }
    }

    fn animation(&self, arguments: &Map<String, Value>) -> Outcome {
        let files = match self.inputs(arguments) {
            Ok(files) => files,
            Err(why) => return Outcome::error(why),
        };
        let settings = match Self::settings(arguments) {
            Ok(settings) => settings,
            Err(why) => return Outcome::error(why),
        };
        let turntable = arguments
            .get("turntable")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let frames = arguments
            .get("frames")
            .and_then(Value::as_u64)
            .unwrap_or(96)
            .clamp(2, MAX_FRAMES);
        let fps = arguments
            .get("fps")
            .and_then(Value::as_u64)
            .unwrap_or(24)
            .clamp(1, 120);
        let deliver = arguments
            .get("deliver_to_project")
            .and_then(Value::as_str)
            .map(str::to_string);
        if deliver.is_some() && self.programs.delivery.is_none() {
            return Outcome::error(
                "deliver_to_project needs a paired desk: start the server with --deliver-key",
            );
        }
        let spec = Spec {
            bridge: "blender".into(),
            kind: "animation".into(),
            arguments: json!({
                "files": files.iter().map(|f| self.workspace.relative(f)).collect::<Vec<_>>(),
                "settings": settings,
                "turntable": turntable,
                "frames": frames,
                "fps": fps,
                "deliver_to_project": deliver,
            }),
        };
        let chunks = frames.div_ceil(CHUNK);
        match jobs::start(&self.workspace, &self.programs, &spec, chunks) {
            Ok(id) => Outcome::json(&json!({
                "job": id,
                "frames": frames,
                "chunks": chunks,
                "next": "archie_job_status {bridge: blender, job} shows progress; the video lands in animations/.",
            })),
            Err(e) => Outcome::error(e.to_string()),
        }
    }

    fn run_animation(&self, arguments: &Value, runner: &mut Runner) -> Result<(), String> {
        let files: Vec<PathBuf> = arguments["files"]
            .as_array()
            .ok_or("the job names no files")?
            .iter()
            .filter_map(Value::as_str)
            .map(|f| self.workspace.file(f).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let turntable = arguments["turntable"].as_bool().unwrap_or(true);
        let fps = arguments["fps"].as_u64().unwrap_or(24);
        // A scene's own animation runs over its own frame range.
        let (first, last) = if turntable {
            (1, arguments["frames"].as_u64().unwrap_or(96))
        } else {
            let report = self.run(json!({"action": "inspect", "files": files}), READ_LIMIT)?;
            let range = &report["frames"];
            let first = range[0].as_u64().unwrap_or(1);
            (
                first,
                range[1]
                    .as_u64()
                    .unwrap_or(first)
                    .min(first + MAX_FRAMES - 1),
            )
        };
        let stem = Self::stem(&files[0]);
        let folder = self
            .workspace
            .fresh_folder("animations", &stem)
            .map_err(|e| e.to_string())?;
        let frames_dir = folder.join("frames");
        std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
        let mut start = first;
        while start <= last {
            if runner.cancelled() {
                return Ok(());
            }
            let end = (start + CHUNK - 1).min(last);
            let mut settings = arguments["settings"].clone();
            settings["action"] = json!("frames");
            settings["files"] = json!(files);
            settings["turntable"] = json!(turntable);
            settings["total"] = json!(last);
            settings["start"] = json!(start);
            settings["end"] = json!(end);
            settings["folder"] = json!(frames_dir);
            self.run(settings, RENDER_LIMIT)?;
            runner
                .record(json!({"frames": [start, end], "of": last}))
                .map_err(|e| e.to_string())?;
            start = end + 1;
        }
        let video = folder.join(format!("{stem}.mp4"));
        let ran = Run::new(self.programs.get("ffmpeg"))
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-framerate",
                &fps.to_string(),
                "-start_number",
                &first.to_string(),
                "-i",
            ])
            .arg(frames_dir.join("frame_%04d.png"))
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "18",
                "-movflags",
                "+faststart",
            ])
            .arg(&video)
            .limit(RENDER_LIMIT)
            .run()
            .map_err(|e| e.to_string())?;
        if !ran.success() {
            return Err(format!("FFmpeg could not join the frames: {}", ran.said()));
        }
        let poster = frames_dir.join(format!("frame_{:04}.png", (first + last) / 2));
        let poster_copy = folder.join(format!("{stem}-poster.png"));
        let _ = std::fs::copy(&poster, &poster_copy);
        if let (Some(project), Some(Ok(deliverer))) = (
            arguments["deliver_to_project"].as_str(),
            self.programs.deliverer(),
        ) {
            let files = vec![
                self.workspace.relative(&video),
                self.workspace.relative(&poster_copy),
            ];
            if let Err(e) = deliverer.deliver(
                &self.workspace,
                &files,
                project,
                &format!("{stem}-animation"),
                "rendered animation",
                false,
            ) {
                eprintln!("delivery: {e}");
            }
        }
        Ok(())
    }
}

impl Bridge for Blender {
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
        let files = json!({"type": "string", "description": "A .blend, or a model: FBX, glTF/GLB, OBJ, STL, PLY, USD, Alembic."});
        let render_settings = json!({
            "file": files,
            "engine": {"type": "string", "enum": ENGINES},
            "samples": {"type": "integer", "minimum": 1, "maximum": 4096},
            "width": {"type": "integer"}, "height": {"type": "integer"},
            "studio": {"type": "boolean", "description": "Add a camera, lights and a clay material where the scene has none. Default true."}
        });
        let mut still = render_settings.clone();
        still["frame"] = json!({"type": "integer"});
        let mut moving = render_settings;
        moving["turntable"] = json!({"type": "boolean", "description": "Orbit the camera once around the model (default), or render the scene's own animation."});
        moving["frames"] = json!({"type": "integer", "minimum": 2, "maximum": MAX_FRAMES});
        moving["fps"] = json!({"type": "integer"});
        moving["deliver_to_project"] = json!({"type": "string"});
        vec![
            BridgeTool {
                name: "blender_scenes",
                title: "Scenes and models",
                description: "The .blend scenes and 3D models in the workspace.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "blender_inspect",
                title: "What a scene holds",
                description: "Objects with type, location, rotation, size, materials and mesh counts; the frame range, \
                              fps, engine, resolution, camera and lights; the models' centre and radius.",
                input_schema: json!({"type": "object", "properties": {"file": files}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "blender_assemble",
                title: "Build a scene from models",
                description: "Imports models (and objects of .blend files) into one new scenes/<name>-<n>.blend.",
                input_schema: json!({"type": "object", "properties": {"files": {"type": "array", "items": {"type": "string"}}, "name": {"type": "string"}},
                                     "required": ["files"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "blender_render",
                title: "Render a picture",
                description: "Renders a still into renders/ and returns it: cycles (default; path-traced on the CPU, \
                              denoised), eevee, or workbench (fastest, flat). A bare model gets a studio: camera \
                              framed on it, three soft lights, clay material, light backdrop.",
                input_schema: json!({"type": "object", "properties": still, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "blender_export",
                title: "Export for other 3D software",
                description: "Exports into exports/<name>-<n>/: fbx (Maya, 3ds Max, Unity, Unreal), glb or gltf (web, \
                              three.js, most viewers), usdc or usdz (Omniverse, Apple AR, Houdini), obj, stl (printing), \
                              ply, abc (animation caches), blend. Default glb.",
                input_schema: json!({"type": "object", "properties": {"file": files, "formats": {"type": "array", "items": {"type": "string", "enum": EXPORTS}}},
                                     "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "blender_animation",
                title: "Render a turntable or animation",
                description: "Starts a job that renders a turntable (the camera orbits the model once) or the scene's \
                              own animation, a few frames at a time, then joins them into an MP4 with a poster frame \
                              in animations/<name>-<n>/. With deliver_to_project the video goes to the person's desk.",
                input_schema: json!({"type": "object", "properties": moving, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "blender_scenes" => {
                let (files, truncated) = self.workspace.find(&READS, 5, 2000);
                Outcome::json(&json!({"files": files, "truncated": truncated}))
            }
            "blender_inspect" => self.inspect(arguments),
            "blender_assemble" => self.assemble(arguments),
            "blender_render" => self.render(arguments),
            "blender_export" => self.export(arguments),
            "blender_animation" => self.animation(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }

    fn run_job(&self, spec: &Spec, runner: &mut Runner) -> Result<(), String> {
        match spec.kind.as_str() {
            "animation" => self.run_animation(&spec.arguments, runner),
            other => Err(format!("blender runs no {other} jobs")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_checked_before_blender_runs() {
        let dir = std::env::temp_dir().join(format!("archie-blender-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("part.step"), b"x").unwrap();
        std::fs::write(dir.join("part.stl"), b"x").unwrap();
        let blender = Blender::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &Programs::default(),
        );
        let args = |v: Value| v.as_object().unwrap().clone();
        assert!(blender
            .call("blender_render", &args(json!({"file": "part.step"})))
            .text
            .contains("freecad_export"));
        assert!(
            blender
                .call(
                    "blender_render",
                    &args(json!({"file": "part.stl", "engine": "octane"}))
                )
                .is_error
        );
        assert!(
            blender
                .call(
                    "blender_export",
                    &args(json!({"file": "part.stl", "formats": ["max"]}))
                )
                .is_error
        );
        assert!(!dir.join("renders").exists() && !dir.join("exports").exists());
        assert!(!script::JOB.contains("eval(") && !script::JOB.contains("exec("));
        let _ = std::fs::remove_dir_all(dir);
    }
}
