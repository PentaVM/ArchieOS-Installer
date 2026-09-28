//! Video: what is in the media folder, what a file holds, and a rendered timeline.
//!
//! The engine is MLT's `melt`, under Kdenlive and Shotcut, with `ffprobe` to read sources
//! ([`archie_cut`]). Renders go to the folder's `renders/` and never replace a file, and
//! the MLT project stays beside each video so a person can open it in Kdenlive.

use crate::manifest::Manifest;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Outcome, Programs};
use archie_cut::{probe, render, Timeline};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../bridges/video/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../bridges/video/SKILL.md");

/// The most files `video_sources` lists.
const MAX_LISTED: usize = 500;
/// How long a render may run before it is stopped.
const RENDER_LIMIT: Duration = Duration::from_secs(30 * 60);
const MEDIA: [&str; 12] = [
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "flac", "ogg", "m4a", "png",
];

pub struct Video {
    manifest: Manifest,
    workspace: Workspace,
    melt: PathBuf,
    ffprobe: PathBuf,
}

impl Video {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            melt: programs.get("melt"),
            ffprobe: programs.get("ffprobe"),
        }
    }

    fn sources(&self) -> Outcome {
        let (files, truncated) = self.workspace.find(&MEDIA, 3, MAX_LISTED);
        let files: Vec<Value> = files
            .into_iter()
            .map(|file| {
                let bytes = std::fs::metadata(self.workspace.root().join(&file))
                    .map(|m| m.len())
                    .unwrap_or(0);
                json!({"file": file, "bytes": bytes})
            })
            .collect();
        Outcome::json(&json!({"files": files, "truncated": truncated}))
    }
}

impl Bridge for Video {
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
        vec![
            BridgeTool {
                name: "video_sources",
                title: "Media files",
                description: "The video, audio and image files in the media folder that video_probe and \
                              video_render may use, with their sizes, as paths relative to that folder.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "video_probe",
                title: "What a media file holds",
                description: "Duration in seconds, frame size, and whether a file in the media folder has \
                              video and audio. Read a source before cutting it: clips are cut by seconds.",
                input_schema: json!({
                    "type": "object",
                    "properties": {"file": {"type": "string", "description": "A path inside the media folder."}},
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "video_render",
                title: "Render a video",
                description: "Renders a timeline into renders/<output> in the media folder with MLT, the \
                              engine under Kdenlive: clips end to end, each cut from a source by in_s and \
                              out_s; titles over them at start_s for duration_s; optional music under them \
                              at a volume of 0 to 1; an optional fade from and to black. The MLT project is \
                              written beside the video so a person can open it in Kdenlive and keep editing. \
                              Never replaces an existing file.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "output": {"type": "string", "description": "One file name ending in .mp4."},
                        "timeline": {
                            "type": "object",
                            "properties": {
                                "width": {"type": "integer"}, "height": {"type": "integer"}, "fps": {"type": "integer"},
                                "clips": {"type": "array", "items": {"type": "object", "properties": {
                                    "source": {"type": "string"}, "in_s": {"type": "number"}, "out_s": {"type": "number"}},
                                    "required": ["source", "out_s"]}},
                                "titles": {"type": "array", "items": {"type": "object", "properties": {
                                    "text": {"type": "string"}, "start_s": {"type": "number"}, "duration_s": {"type": "number"}},
                                    "required": ["text", "start_s", "duration_s"]}},
                                "music": {"type": "object", "properties": {
                                    "source": {"type": "string"}, "volume": {"type": "number"}}, "required": ["source"]},
                                "fade_s": {"type": "number"}
                            },
                            "required": ["clips"]
                        }
                    },
                    "required": ["output", "timeline"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        let media = self.workspace.root();
        match tool {
            "video_sources" => self.sources(),
            "video_probe" => {
                let Some(file) = arguments.get("file").and_then(Value::as_str) else {
                    return Outcome::error("video_probe needs file");
                };
                match probe(&self.ffprobe, media, file) {
                    Ok(found) => Outcome::json(&json!(found)),
                    Err(error) => Outcome::error(error.to_string()),
                }
            }
            "video_render" => {
                let Some(output) = arguments.get("output").and_then(Value::as_str) else {
                    return Outcome::error("video_render needs output");
                };
                let timeline: Timeline = match arguments
                    .get("timeline")
                    .cloned()
                    .map(serde_json::from_value)
                {
                    Some(Ok(timeline)) => timeline,
                    Some(Err(error)) => {
                        return Outcome::error(format!("the timeline is not valid: {error}"))
                    }
                    None => return Outcome::error("video_render needs timeline"),
                };
                match render(&timeline, media, output, &self.melt, RENDER_LIMIT) {
                    Ok(made) => {
                        // The cut list beside the video, for Premiere, Resolve or Avid.
                        let edl = made.video.with_extension("edl");
                        let title = made
                            .video
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("archie");
                        let _ = std::fs::write(&edl, cut_list(title, &timeline));
                        Outcome::json(&json!({
                            "video": self.workspace.relative(&made.video),
                            "project": self.workspace.relative(&made.project),
                            "edl": self.workspace.relative(&edl),
                            "seconds": made.seconds,
                            "render_seconds": made.elapsed_s,
                        }))
                    }
                    Err(error) => Outcome::error(error.to_string()),
                }
            }
            _ => Outcome::error("unknown tool"),
        }
    }
}

/// `seconds` as SMPTE timecode at `fps`, non-drop.
fn timecode(seconds: f64, fps: u32) -> String {
    let fps = fps.max(1) as u64;
    let frames = (seconds.max(0.0) * fps as f64).round() as u64;
    let (s, f) = (frames / fps, frames % fps);
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        f
    )
}

/// A CMX 3600 edit decision list of the timeline's clips, which Premiere, DaVinci Resolve,
/// Avid and Final Cut import to rebuild the cut against the original sources.
pub fn cut_list(title: &str, timeline: &Timeline) -> String {
    let mut out = format!(
        "TITLE: {}\nFCM: NON-DROP FRAME\n\n",
        title.replace(['\n', '\r'], " ")
    );
    let mut record = 0.0;
    for (i, clip) in timeline.clips.iter().enumerate() {
        let length = clip.out_s - clip.in_s;
        let reel = format!("AX{:02}", (i + 1) % 100);
        out.push_str(&format!(
            "{:03}  {reel:<8} B     C        {} {} {} {}\n* FROM CLIP NAME: {}\n\n",
            i + 1,
            timecode(clip.in_s, timeline.fps),
            timecode(clip.out_s, timeline.fps),
            timecode(record, timeline.fps),
            timecode(record + length, timeline.fps),
            clip.source.replace(['\n', '\r'], " "),
        ));
        record += length;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_cut::RENDERS;

    #[test]
    fn the_cut_list_places_each_clip_after_the_last() {
        let timeline: Timeline = serde_json::from_value(json!({
            "fps": 25,
            "clips": [{"source": "a.mp4", "in_s": 1.0, "out_s": 3.4}, {"source": "b.mp4", "out_s": 2.0}]
        }))
        .unwrap();
        let edl = cut_list("cut", &timeline);
        assert!(edl.starts_with("TITLE: cut\nFCM: NON-DROP FRAME"));
        assert!(edl.contains(
            "001  AX01     B     C        00:00:01:00 00:00:03:10 00:00:00:00 00:00:02:10"
        ));
        assert!(edl.contains(
            "002  AX02     B     C        00:00:00:00 00:00:02:00 00:00:02:10 00:00:04:10"
        ));
        assert!(edl.contains("* FROM CLIP NAME: b.mp4"));
    }

    fn video(name: &str) -> (Video, PathBuf) {
        let media =
            std::env::temp_dir().join(format!("archie-bridge-video-{name}-{}", std::process::id()));
        std::fs::create_dir_all(media.join("clips")).unwrap();
        std::fs::write(media.join("clips/a.mp4"), b"xx").unwrap();
        std::fs::write(media.join("notes.txt"), b"not media").unwrap();
        let video = Video::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&media).unwrap(),
            &Programs::default(),
        );
        (video, media)
    }

    #[test]
    fn sources_are_media_files_named_relative_to_the_folder() {
        let (video, media) = video("sources");
        let outcome = video.call("video_sources", &Map::new());
        assert!(!outcome.is_error, "{}", outcome.text);
        assert!(outcome.text.contains("\"clips/a.mp4\""), "{}", outcome.text);
        assert!(!outcome.text.contains("notes.txt"));
        assert!(
            !outcome.text.contains(&media.display().to_string()),
            "no absolute paths"
        );
        let _ = std::fs::remove_dir_all(media);
    }

    #[test]
    fn a_render_that_names_a_file_outside_is_refused_before_anything_runs() {
        let (video, media) = video("outside");
        let mut arguments = Map::new();
        arguments.insert("output".into(), json!("x.mp4"));
        arguments.insert(
            "timeline".into(),
            json!({"clips": [{"source": "../../etc/passwd", "out_s": 1.0}]}),
        );
        let outcome = video.call("video_render", &arguments);
        assert!(outcome.is_error);
        assert!(
            outcome.text.contains("not a file inside the media folder"),
            "{}",
            outcome.text
        );
        assert!(
            !media.join(RENDERS).join("x.mlt").exists(),
            "nothing was written"
        );
        let _ = std::fs::remove_dir_all(media);
    }
}
