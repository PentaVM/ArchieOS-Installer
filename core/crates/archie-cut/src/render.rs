//! Running the engines: `melt` to render a timeline, `ffprobe` to read a source.
//!
//! Both are C programs run directly, never through a shell, with arguments this code
//! builds. A render has a time limit and is killed at it. Its output goes into `renders/`
//! in the media folder, under a name that is one plain file name, and never over a file
//! that is already there: an agent can make a video, not replace one.

use crate::mlt::document;
use crate::timeline::{inside, Timeline, TimelineError};
use serde::Serialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where renders go, inside the media folder.
pub const RENDERS: &str = "renders";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    #[error("the output is one file name ending in .mp4, not a path: {0}")]
    OutputName(String),
    #[error("{0} already exists; renders never replace a file")]
    Exists(String),
    #[error("{what}: {detail}")]
    Engine { what: &'static str, detail: String },
    #[error("the render took longer than {0:?} and was stopped")]
    TooLong(Duration),
}

fn engine(what: &'static str) -> impl FnOnce(std::io::Error) -> RenderError {
    move |error| RenderError::Engine {
        what,
        detail: error.to_string(),
    }
}

/// What a render made.
#[derive(Debug, Clone, Serialize)]
pub struct Rendered {
    /// The video, inside the media folder.
    pub video: PathBuf,
    /// The MLT project beside it, which Kdenlive or Shotcut can open to keep editing.
    pub project: PathBuf,
    pub seconds: f64,
    pub elapsed_s: f64,
}

/// The path a render named `name` goes to, or why it may not.
pub fn output_path(media: &Path, name: &str) -> Result<PathBuf, RenderError> {
    let plain = Path::new(name);
    let mut parts = plain.components();
    let one_part = matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none();
    let named = plain
        .file_stem()
        .is_some_and(|stem| !stem.is_empty() && !stem.to_string_lossy().starts_with('.'));
    if !one_part || !named || plain.extension().and_then(|e| e.to_str()) != Some("mp4") {
        return Err(RenderError::OutputName(name.to_string()));
    }
    let dir = media.join(RENDERS);
    std::fs::create_dir_all(&dir).map_err(engine("creating the renders folder"))?;
    let path = dir.join(plain);
    if path.exists() {
        return Err(RenderError::Exists(format!("{RENDERS}/{name}")));
    }
    Ok(path)
}

/// Renders `timeline` into `renders/<name>` in `media` with `melt`, within `limit`.
pub fn render(
    timeline: &Timeline,
    media: &Path,
    name: &str,
    melt: &Path,
    limit: Duration,
) -> Result<Rendered, RenderError> {
    let files = timeline.validate(media)?;
    let video = output_path(media, name)?;
    let project = video.with_extension("mlt");
    std::fs::write(&project, document(timeline, &files)).map_err(engine("writing the project"))?;

    let started = Instant::now();
    let mut child = Command::new(melt)
        .arg(&project)
        .arg("-quiet")
        .arg("-consumer")
        .arg(format!("avformat:{}", video.display()))
        .args([
            "vcodec=libx264",
            "acodec=aac",
            "preset=veryfast",
            "crf=23",
            "movflags=+faststart",
        ])
        // melt's titles are drawn by Qt, which looks for a display to open even though
        // nothing is shown. Without one (a service, a shell with no session) it would
        // not start at all; told to draw off screen, it never needs one.
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(engine("starting melt"))?;
    // Read what melt says as it goes, so a chatty render cannot fill the pipe and stall.
    let mut stderr = child.stderr.take();
    let said = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(64 * 1024).read_to_string(&mut text);
        }
        text
    });
    let status = loop {
        if let Some(status) = child.try_wait().map_err(engine("waiting for melt"))? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&video);
            return Err(RenderError::TooLong(limit));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let said = said.join().unwrap_or_default();
    let made = std::fs::metadata(&video)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if !status.success() || made == 0 {
        let _ = std::fs::remove_file(&video);
        return Err(RenderError::Engine {
            what: "melt",
            detail: last_lines(&said, 6),
        });
    }
    Ok(Rendered {
        video,
        project,
        seconds: timeline.duration_s(),
        elapsed_s: started.elapsed().as_secs_f64(),
    })
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// What a source file holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Probe {
    pub seconds: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video: bool,
    pub audio: bool,
}

/// Reads a source inside `media` with `ffprobe`.
pub fn probe(ffprobe: &Path, media: &Path, name: &str) -> Result<Probe, RenderError> {
    let path = inside(media, name)?;
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(engine("starting ffprobe"))?;
    if !output.status.success() {
        return Err(RenderError::Engine {
            what: "ffprobe",
            detail: format!("{name} is not a media file it can read"),
        });
    }
    parse_probe(&output.stdout).ok_or(RenderError::Engine {
        what: "ffprobe",
        detail: "an answer it could not read".into(),
    })
}

fn parse_probe(json: &[u8]) -> Option<Probe> {
    let value: serde_json::Value = serde_json::from_slice(json).ok()?;
    let seconds = value["format"]["duration"].as_str()?.parse::<f64>().ok()?;
    let streams = value["streams"].as_array()?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video");
    Some(Probe {
        seconds,
        width: video.and_then(|v| v["width"].as_u64()).map(|w| w as u32),
        height: video.and_then(|v| v["height"].as_u64()).map(|h| h as u32),
        video: video.is_some(),
        audio: streams.iter().any(|stream| stream["codec_type"] == "audio"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_is_named_by_one_plain_mp4_file_and_never_replaces_one() {
        let media = std::env::temp_dir().join(format!("archie-cut-out-{}", std::process::id()));
        std::fs::create_dir_all(&media).unwrap();
        for bad in [
            "../x.mp4",
            "/tmp/x.mp4",
            "sub/x.mp4",
            "x.mkv",
            ".mp4",
            "x",
            "",
        ] {
            assert!(output_path(&media, bad).is_err(), "{bad:?} was let through");
        }
        let fine = output_path(&media, "demo.mp4").unwrap();
        assert!(fine.ends_with("renders/demo.mp4"));
        std::fs::write(&fine, b"x").unwrap();
        assert!(matches!(
            output_path(&media, "demo.mp4"),
            Err(RenderError::Exists(_))
        ));
        let _ = std::fs::remove_dir_all(media);
    }

    #[test]
    fn a_probe_reads_duration_size_and_streams() {
        let json = br#"{"streams":[{"codec_type":"video","width":1920,"height":1080},{"codec_type":"audio"}],
                        "format":{"duration":"12.480000"}}"#;
        let probe = parse_probe(json).unwrap();
        assert_eq!(probe.seconds, 12.48);
        assert_eq!((probe.width, probe.height), (Some(1920), Some(1080)));
        assert!(probe.video && probe.audio);
        assert!(parse_probe(b"{}").is_none());
    }
}
