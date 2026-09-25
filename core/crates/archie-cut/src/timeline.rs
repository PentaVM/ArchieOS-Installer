//! A timeline, as an agent may ask for one, and the checks it passes before anything runs.
//!
//! The shape is deliberately small: clips one after another, each cut from a source by an
//! in and out point; titles laid over them at a time; one music bed; a fade at the start
//! and the end. That covers a demo video, a highlight reel and a tutorial, and every field
//! of it can be checked.
//!
//! Every file named here is resolved inside one media folder. A timeline that points
//! outside it — `../`, an absolute path elsewhere, a symbolic link out — is refused, so an
//! agent editing video cannot use the renderer to read the rest of the disk.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const MAX_CLIPS: usize = 200;
pub const MAX_TITLES: usize = 50;
pub const MAX_TITLE_CHARS: usize = 200;
/// Four hours: longer than any video this is for, short enough to bound a render.
pub const MAX_SECONDS: f64 = 4.0 * 3600.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub titles: Vec<Title>,
    #[serde(default)]
    pub music: Option<Music>,
    /// Seconds of fade from and to black at the start and the end. Zero for none.
    #[serde(default)]
    pub fade_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// A file in the media folder, relative to it.
    pub source: String,
    /// Where in the source the clip starts, in seconds.
    #[serde(default)]
    pub in_s: f64,
    /// Where it ends, in seconds. Always given: the renderer is told how long each clip is,
    /// never left to guess from the file.
    pub out_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Title {
    pub text: String,
    /// When it appears, in seconds from the start of the video.
    pub start_s: f64,
    pub duration_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Music {
    pub source: String,
    /// Linear gain, 0 to 1: under speech, 0.2 is about right.
    #[serde(default = "default_volume")]
    pub volume: f64,
}

fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}
fn default_fps() -> u32 {
    30
}
fn default_volume() -> f64 {
    0.3
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TimelineError {
    #[error("a video needs at least one clip, and at most {MAX_CLIPS}")]
    Clips,
    #[error("at most {MAX_TITLES} titles, one at a time, each 1..={MAX_TITLE_CHARS} characters of plain text")]
    Titles,
    #[error("the frame is 16..=7680 wide, 16..=4320 high, at 1..=120 frames a second")]
    Format,
    #[error("clip {0}: in and out must be seconds with in < out, within {MAX_SECONDS} s")]
    ClipTimes(usize),
    #[error("title {0}: it must start and last within the video")]
    TitleTimes(usize),
    #[error("the fade is 0 to 10 seconds, and shorter than half the video")]
    Fade,
    #[error("music volume is 0 to 1")]
    Volume,
    #[error("{0} is not a file inside the media folder")]
    Outside(String),
}

impl Timeline {
    /// Seconds the finished video lasts.
    pub fn duration_s(&self) -> f64 {
        self.clips.iter().map(|clip| clip.out_s - clip.in_s).sum()
    }

    /// Checks everything that can be checked without the files, and resolves every source
    /// inside `media`. Returns the resolved paths, clips first and then the music.
    pub fn validate(&self, media: &Path) -> Result<Resolved, TimelineError> {
        if !(16..=7680).contains(&self.width)
            || !(16..=4320).contains(&self.height)
            || !(1..=120).contains(&self.fps)
        {
            return Err(TimelineError::Format);
        }
        if self.clips.is_empty() || self.clips.len() > MAX_CLIPS {
            return Err(TimelineError::Clips);
        }
        let finite = |value: f64| value.is_finite() && (0.0..=MAX_SECONDS).contains(&value);
        for (at, clip) in self.clips.iter().enumerate() {
            if !finite(clip.in_s) || !finite(clip.out_s) || clip.in_s >= clip.out_s {
                return Err(TimelineError::ClipTimes(at));
            }
        }
        let total = self.duration_s();
        if total > MAX_SECONDS {
            return Err(TimelineError::ClipTimes(self.clips.len() - 1));
        }
        if self.titles.len() > MAX_TITLES
            || self.titles.iter().any(|title| {
                let chars = title.text.chars().count();
                chars == 0 || chars > MAX_TITLE_CHARS || title.text.chars().any(char::is_control)
            })
        {
            return Err(TimelineError::Titles);
        }
        let mut starts: Vec<(f64, f64)> = self
            .titles
            .iter()
            .map(|title| (title.start_s, title.start_s + title.duration_s))
            .collect();
        starts.sort_by(|a, b| a.0.total_cmp(&b.0));
        if starts.windows(2).any(|pair| pair[1].0 < pair[0].1) {
            return Err(TimelineError::Titles);
        }
        for (at, title) in self.titles.iter().enumerate() {
            if !finite(title.start_s)
                || !finite(title.duration_s)
                || title.duration_s <= 0.0
                || title.start_s + title.duration_s > total + 1e-6
            {
                return Err(TimelineError::TitleTimes(at));
            }
        }
        if !self.fade_s.is_finite()
            || !(0.0..=10.0).contains(&self.fade_s)
            || self.fade_s * 2.0 >= total
        {
            return Err(TimelineError::Fade);
        }
        if let Some(music) = &self.music {
            if !music.volume.is_finite() || !(0.0..=1.0).contains(&music.volume) {
                return Err(TimelineError::Volume);
            }
        }
        let clips = self
            .clips
            .iter()
            .map(|clip| inside(media, &clip.source))
            .collect::<Result<Vec<_>, _>>()?;
        let music = self
            .music
            .as_ref()
            .map(|music| inside(media, &music.source))
            .transpose()?;
        Ok(Resolved { clips, music })
    }
}

/// The files a checked timeline uses, as real paths inside the media folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub clips: Vec<PathBuf>,
    pub music: Option<PathBuf>,
}

/// `name` as a file inside `media`, or a refusal. Checked twice: by its components, so that
/// no `..` or root ever takes part, and by the real path after links are followed, so that
/// a link inside the folder cannot point out of it.
pub fn inside(media: &Path, name: &str) -> Result<PathBuf, TimelineError> {
    let outside = || TimelineError::Outside(name.to_string());
    let relative = Path::new(name);
    if name.is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(outside());
    }
    let root = media.canonicalize().map_err(|_| outside())?;
    let real = root.join(relative).canonicalize().map_err(|_| outside())?;
    if !real.starts_with(&root) || !real.is_file() {
        return Err(outside());
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archie-cut-media-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        std::fs::write(dir.join("sub/b.mp4"), b"x").unwrap();
        std::fs::write(dir.join("song.mp3"), b"x").unwrap();
        dir
    }

    fn timeline() -> Timeline {
        serde_json::from_value(serde_json::json!({
            "clips": [{"source": "a.mp4", "in_s": 1.0, "out_s": 4.0}, {"source": "sub/b.mp4", "out_s": 2.0}],
            "titles": [{"text": "ArchieOS", "start_s": 0.5, "duration_s": 2.0}],
            "music": {"source": "song.mp3"},
            "fade_s": 0.5
        }))
        .unwrap()
    }

    #[test]
    fn a_plain_timeline_is_accepted_with_its_files_resolved() {
        let media = media();
        let resolved = timeline().validate(&media).unwrap();
        assert_eq!(resolved.clips.len(), 2);
        assert!(resolved.clips[1].ends_with("sub/b.mp4"));
        assert!(resolved.music.is_some());
        assert!((timeline().duration_s() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_outside_the_media_folder_can_be_named() {
        let media = media();
        for name in [
            "../etc/passwd",
            "/etc/passwd",
            "sub/../../x",
            "",
            "missing.mp4",
            "sub",
        ] {
            assert!(inside(&media, name).is_err(), "{name:?} was let through");
        }
        #[cfg(unix)]
        {
            let link = media.join("escape.mp4");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink("/etc/hostname", &link).unwrap();
            assert!(
                inside(&media, "escape.mp4").is_err(),
                "a link out is still out"
            );
        }
    }

    #[test]
    fn times_titles_and_fades_have_to_make_sense() {
        let media = media();
        let mut backwards = timeline();
        backwards.clips[0].out_s = 0.5;
        assert_eq!(backwards.validate(&media), Err(TimelineError::ClipTimes(0)));

        let mut late = timeline();
        late.titles[0].start_s = 4.5;
        assert_eq!(late.validate(&media), Err(TimelineError::TitleTimes(0)));

        let mut control = timeline();
        control.titles[0].text = "a\u{7}b".into();
        assert_eq!(control.validate(&media), Err(TimelineError::Titles));

        let mut long_fade = timeline();
        long_fade.fade_s = 3.0;
        assert_eq!(long_fade.validate(&media), Err(TimelineError::Fade));

        let mut nan = timeline();
        nan.clips[1].out_s = f64::NAN;
        assert_eq!(nan.validate(&media), Err(TimelineError::ClipTimes(1)));

        let unknown = serde_json::from_value::<Timeline>(serde_json::json!({
            "clips": [{"source": "a.mp4", "out_s": 1.0, "shell": "rm -rf /"}]
        }));
        assert!(
            unknown.is_err(),
            "a field nobody defined is refused, not ignored"
        );
    }
}
