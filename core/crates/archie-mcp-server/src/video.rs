//! Video tools: what is in the media folder, what a file holds, and a rendered timeline.
//!
//! Offered only when the operator names a media folder. Everything an agent names is a
//! path inside it; renders are written to its `renders/` folder and never replace a file.
//! Paths in answers are relative to the folder, so the agent learns nothing about the rest
//! of the disk.

use crate::toolbox::{Effect, ToolOutcome, ToolSpec};
use archie_cut::{probe, render, Timeline, RENDERS};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most files `video_sources` lists.
const MAX_LISTED: usize = 500;
/// How long a render may run before it is stopped.
const RENDER_LIMIT: Duration = Duration::from_secs(30 * 60);
const MEDIA: [&str; 12] = [
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "flac", "ogg", "m4a", "png",
];

/// The media folder and the engines that work in it.
#[derive(Debug, Clone)]
pub struct VideoTools {
    pub media: PathBuf,
    pub melt: PathBuf,
    pub ffprobe: PathBuf,
}

impl VideoTools {
    pub fn specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "video_sources",
                title: "Media files",
                description: "The video, audio and image files in the media folder that video_probe and \
                              video_render may use, with their sizes, as paths relative to that folder.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            ToolSpec {
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
            ToolSpec {
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
                effect: Effect::Acts,
            },
        ]
    }

    pub async fn call(&self, name: &str, arguments: Map<String, Value>) -> ToolOutcome {
        match name {
            "video_sources" => self.sources(),
            "video_probe" => {
                let Some(file) = arguments.get("file").and_then(Value::as_str) else {
                    return ToolOutcome::error("video_probe needs file");
                };
                match probe(&self.ffprobe, &self.media, file) {
                    Ok(found) => ToolOutcome::json(&json!(found)),
                    Err(error) => ToolOutcome::error(error.to_string()),
                }
            }
            "video_render" => {
                let Some(output) = arguments
                    .get("output")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    return ToolOutcome::error("video_render needs output");
                };
                let timeline: Timeline = match arguments
                    .get("timeline")
                    .cloned()
                    .map(serde_json::from_value)
                {
                    Some(Ok(timeline)) => timeline,
                    Some(Err(error)) => {
                        return ToolOutcome::error(format!("the timeline is not valid: {error}"))
                    }
                    None => return ToolOutcome::error("video_render needs timeline"),
                };
                let tools = self.clone();
                // A render takes minutes; it runs off the protocol's thread.
                let made = tokio::task::spawn_blocking(move || {
                    render(&timeline, &tools.media, &output, &tools.melt, RENDER_LIMIT)
                })
                .await;
                match made {
                    Ok(Ok(made)) => ToolOutcome::json(&json!({
                        "video": self.relative(&made.video),
                        "project": self.relative(&made.project),
                        "seconds": made.seconds,
                        "render_seconds": made.elapsed_s,
                    })),
                    Ok(Err(error)) => ToolOutcome::error(error.to_string()),
                    Err(_) => ToolOutcome::error("the render stopped unexpectedly"),
                }
            }
            _ => ToolOutcome::error("unknown tool"),
        }
    }

    fn relative(&self, path: &Path) -> String {
        let root = self
            .media
            .canonicalize()
            .unwrap_or_else(|_| self.media.clone());
        path.strip_prefix(&root)
            .or_else(|_| path.strip_prefix(&self.media))
            .map(|rest| rest.display().to_string())
            .unwrap_or_else(|_| format!("{RENDERS}/…"))
    }

    fn sources(&self) -> ToolOutcome {
        let mut found = Vec::new();
        let mut truncated = false;
        let mut pending = vec![(self.media.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                // Links are not followed: a link out of the folder is not a source.
                if kind.is_dir() && depth < 3 {
                    pending.push((path, depth + 1));
                } else if kind.is_file() {
                    let media = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| MEDIA.contains(&e.to_ascii_lowercase().as_str()));
                    if media {
                        if found.len() == MAX_LISTED {
                            truncated = true;
                            break;
                        }
                        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        found.push(json!({"file": self.relative(&path), "bytes": bytes}));
                    }
                }
            }
        }
        found.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
        ToolOutcome::json(&json!({"files": found, "truncated": truncated}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> (VideoTools, PathBuf) {
        let media = std::env::temp_dir().join(format!("archie-mcp-video-{}", std::process::id()));
        std::fs::create_dir_all(media.join("clips")).unwrap();
        std::fs::write(media.join("clips/a.mp4"), b"xx").unwrap();
        std::fs::write(media.join("notes.txt"), b"not media").unwrap();
        let tools = VideoTools {
            media: media.clone(),
            melt: PathBuf::from("melt"),
            ffprobe: PathBuf::from("ffprobe"),
        };
        (tools, media)
    }

    #[tokio::test]
    async fn sources_are_media_files_named_relative_to_the_folder() {
        let (tools, media) = tools();
        let outcome = tools.call("video_sources", Map::new()).await;
        assert!(!outcome.is_error, "{}", outcome.text);
        assert!(outcome.text.contains("\"clips/a.mp4\""), "{}", outcome.text);
        assert!(!outcome.text.contains("notes.txt"));
        assert!(
            !outcome.text.contains(&media.display().to_string()),
            "no absolute paths"
        );
        let _ = std::fs::remove_dir_all(media);
    }

    #[tokio::test]
    async fn a_render_that_names_a_file_outside_is_refused_before_anything_runs() {
        let (tools, media) = tools();
        let mut arguments = Map::new();
        arguments.insert("output".into(), json!("x.mp4"));
        arguments.insert(
            "timeline".into(),
            json!({"clips": [{"source": "../../etc/passwd", "out_s": 1.0}]}),
        );
        let outcome = tools.call("video_render", arguments).await;
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
