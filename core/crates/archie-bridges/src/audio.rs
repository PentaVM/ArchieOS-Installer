//! Audio: clean, level, cut, mix and convert recordings with FFmpeg's filters, the
//! engine Audacity and most editors lean on, and picture them as waveforms.
//!
//! Every filter chain is built here from checked numbers; nothing a planner writes
//! reaches FFmpeg's filter parser as text.

use crate::manifest::Manifest;
use crate::run::Run;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../bridges/audio/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../bridges/audio/SKILL.md");

const LIMIT: Duration = Duration::from_secs(1800);
const READS: [&str; 12] = [
    "wav", "flac", "mp3", "m4a", "aac", "ogg", "opus", "aiff", "aif", "wma", "mp4", "mkv",
];
const WRITES: [&str; 7] = ["wav", "flac", "mp3", "m4a", "ogg", "opus", "aiff"];
const MAX_IMAGE: usize = 700 * 1024;

pub struct Audio {
    manifest: Manifest,
    workspace: Workspace,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

fn number(value: &Value, key: &str, range: std::ops::RangeInclusive<f64>) -> Result<f64, String> {
    let n = value
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{key} is a number"))?;
    if !n.is_finite() || !range.contains(&n) {
        return Err(format!(
            "{key} is between {} and {}",
            range.start(),
            range.end()
        ));
    }
    Ok(n)
}

fn optional(
    value: &Value,
    key: &str,
    default: f64,
    range: std::ops::RangeInclusive<f64>,
) -> Result<f64, String> {
    if value.get(key).is_none() {
        Ok(default)
    } else {
        number(value, key, range)
    }
}

/// One described step as an FFmpeg audio filter. `duration` is the input's length after
/// the steps before it, which fades out need.
pub fn step_filter(step: &Value, duration: f64) -> Result<(String, f64), String> {
    let (name, args) = match step {
        Value::String(name) => (name.as_str(), json!({})),
        Value::Object(map) if map.len() == 1 => {
            let (name, args) = map.iter().next().expect("one entry");
            (name.as_str(), args.clone())
        }
        _ => return Err(format!("a step is a name or {{name: settings}}: {step}")),
    };
    Ok(match name {
        "trim" => {
            let start = optional(&args, "start_s", 0.0, 0.0..=86_400.0)?;
            let end = optional(&args, "end_s", duration, 0.0..=86_400.0)?;
            if end <= start {
                return Err("trim needs end_s after start_s".into());
            }
            (
                format!("atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS"),
                end.min(duration) - start,
            )
        }
        "fade_in" => {
            let s = optional(&args, "seconds", 1.0, 0.01..=600.0)?;
            (format!("afade=t=in:st=0:d={s:.3}"), duration)
        }
        "fade_out" => {
            let s = optional(&args, "seconds", 1.0, 0.01..=600.0)?;
            (
                format!("afade=t=out:st={:.3}:d={s:.3}", (duration - s).max(0.0)),
                duration,
            )
        }
        "gain" => (
            format!("volume={:.2}dB", number(&args, "db", -60.0..=40.0)?),
            duration,
        ),
        "normalize" => {
            let lufs = optional(&args, "lufs", -16.0, -40.0..=-5.0)?;
            let peak = optional(&args, "true_peak", -1.5, -9.0..=0.0)?;
            (
                format!("loudnorm=I={lufs:.1}:TP={peak:.1}:LRA=11"),
                duration,
            )
        }
        "denoise" => {
            let strength = optional(&args, "db", 12.0, 1.0..=60.0)?;
            (format!("afftdn=nr={strength:.1}:nf=-40"), duration)
        }
        "highpass" => (
            format!(
                "highpass=f={:.1}",
                optional(&args, "hz", 80.0, 10.0..=5000.0)?
            ),
            duration,
        ),
        "lowpass" => (
            format!(
                "lowpass=f={:.1}",
                optional(&args, "hz", 12000.0, 200.0..=22000.0)?
            ),
            duration,
        ),
        "compress" => {
            let ratio = optional(&args, "ratio", 3.0, 1.0..=20.0)?;
            let threshold = optional(&args, "threshold_db", -18.0, -60.0..=0.0)?;
            (
                format!(
                    "acompressor=threshold={threshold:.1}dB:ratio={ratio:.2}:attack=10:release=200"
                ),
                duration,
            )
        }
        "trim_silence" => {
            let threshold = optional(&args, "threshold_db", -50.0, -90.0..=-10.0)?;
            (
                format!("silenceremove=start_periods=1:start_threshold={threshold:.0}dB:start_silence=0.2,areverse,silenceremove=start_periods=1:start_threshold={threshold:.0}dB:start_silence=0.2,areverse"),
                duration,
            )
        }
        "speed" => {
            let factor = number(&args, "factor", 0.25..=4.0)?;
            // atempo takes 0.5 to 2 per stage.
            let mut stages = Vec::new();
            let mut rest = factor;
            while rest > 2.0 {
                stages.push("atempo=2.0".to_string());
                rest /= 2.0;
            }
            while rest < 0.5 {
                stages.push("atempo=0.5".to_string());
                rest /= 0.5;
            }
            stages.push(format!("atempo={rest:.4}"));
            (stages.join(","), duration / factor)
        }
        "mono" => ("pan=mono|c0=0.5*c0+0.5*c1".into(), duration),
        "sample_rate" => {
            let hz = number(&args, "hz", 8000.0..=192000.0)?;
            (format!("aresample={}", hz as u32), duration)
        }
        other => return Err(format!("{other} is not a step this bridge runs")),
    })
}

impl Audio {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            ffmpeg: programs.get("ffmpeg"),
            ffprobe: programs.get("ffprobe"),
        }
    }

    fn input(&self, name: Option<&str>) -> Result<PathBuf, String> {
        let file = name.ok_or("file is a recording in the workspace")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !READS.contains(&ext.as_str()) {
            return Err(format!("{file} is not audio this bridge reads: {READS:?}"));
        }
        Ok(path)
    }

    fn format(arguments: &Map<String, Value>, default: &str) -> Result<String, String> {
        let format = arguments
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string();
        if WRITES.contains(&format.as_str()) {
            Ok(format)
        } else {
            Err(format!("format is one of {WRITES:?}"))
        }
    }

    fn codec(format: &str) -> Vec<&'static str> {
        match format {
            "wav" => vec!["-c:a", "pcm_s24le"],
            "aiff" => vec!["-c:a", "pcm_s24be"],
            "flac" => vec!["-c:a", "flac"],
            "mp3" => vec!["-c:a", "libmp3lame", "-q:a", "2"],
            "m4a" => vec!["-c:a", "aac", "-b:a", "256k"],
            "ogg" => vec!["-c:a", "libvorbis", "-q:a", "6"],
            _ => vec!["-c:a", "libopus", "-b:a", "160k"],
        }
    }

    fn probe(&self, path: &Path) -> Result<Value, String> {
        let ran = Run::new(&self.ffprobe)
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "-select_streams",
                "a",
            ])
            .arg(path)
            .limit(Duration::from_secs(60))
            .run()
            .map_err(|e| e.to_string())?;
        let info: Value =
            serde_json::from_str(&ran.stdout).map_err(|_| "ffprobe gave no answer".to_string())?;
        let stream = info["streams"]
            .get(0)
            .cloned()
            .ok_or("the file has no audio")?;
        Ok(json!({
            "seconds": info["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()),
            "codec": stream["codec_name"],
            "sample_rate": stream["sample_rate"].as_str().and_then(|r| r.parse::<u32>().ok()),
            "channels": stream["channels"],
            "bit_rate": info["format"]["bit_rate"].as_str().and_then(|r| r.parse::<u64>().ok()),
        }))
    }

    /// Integrated loudness and true peak, as EBU R128 measures them.
    fn loudness(&self, path: &Path) -> Result<Value, String> {
        let ran = Run::new(&self.ffmpeg)
            .args(["-hide_banner", "-nostats", "-i"])
            .arg(path)
            .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
            .limit(LIMIT)
            .run()
            .map_err(|e| e.to_string())?;
        let summary = ran.stderr.rsplit("Summary:").next().unwrap_or("");
        let find = |label: &str| {
            summary
                .lines()
                .find(|l| l.trim_start().starts_with(label))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
        };
        Ok(
            json!({"integrated_lufs": find("I:"), "loudness_range_lu": find("LRA:"), "true_peak_dbtp": find("Peak:")}),
        )
    }

    fn ffmpeg(&self, args: Vec<std::ffi::OsString>) -> Result<(), String> {
        let ran = Run::new(&self.ffmpeg)
            .args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error"])
            .args(args)
            .cwd(self.workspace.root())
            .limit(LIMIT)
            .run()
            .map_err(|e| e.to_string())?;
        if ran.success() {
            Ok(())
        } else {
            Err(format!("FFmpeg failed: {}", ran.said()))
        }
    }

    fn waveform(&self, path: &Path, png: &Path) -> Result<Vec<u8>, String> {
        self.ffmpeg(vec![
            "-i".into(),
            path.as_os_str().to_os_string(),
            "-filter_complex".into(),
            "[0:a]aformat=channel_layouts=mono,showwavespic=s=1600x360:colors=#2f6fde:scale=sqrt,pad=1640:400:20:20:color=#f6f6f3".into(),
            "-frames:v".into(),
            "1".into(),
            png.as_os_str().to_os_string(),
        ])?;
        std::fs::read(png).map_err(|e| e.to_string())
    }

    fn with_waveform(&self, answer: Value, audio: &Path) -> Outcome {
        let png = audio.with_extension("png");
        match self.waveform(audio, &png) {
            Ok(bytes) => {
                let mut answer = answer;
                answer["waveform"] = json!(self.workspace.relative(&png));
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
            Err(_) => Outcome::json(&answer),
        }
    }

    fn stem(path: &Path) -> String {
        crate::delivery::to_name(path.file_stem().and_then(|s| s.to_str()).unwrap_or("audio"))
    }

    fn inspect(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments.get("file").and_then(Value::as_str)) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let mut info = match self.probe(&path) {
            Ok(info) => info,
            Err(why) => return Outcome::error(why),
        };
        if let Ok(loudness) = self.loudness(&path) {
            info["loudness"] = loudness;
        }
        Outcome::json(&info)
    }

    fn process(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments.get("file").and_then(Value::as_str)) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let Some(steps) = arguments
            .get("steps")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
        else {
            return Outcome::error(
                "steps is a list, such as [\"denoise\", {\"normalize\": {\"lufs\": -16}}]",
            );
        };
        let format = match Self::format(arguments, "wav") {
            Ok(format) => format,
            Err(why) => return Outcome::error(why),
        };
        let mut duration = match self.probe(&path) {
            Ok(info) => info["seconds"].as_f64().unwrap_or(0.0),
            Err(why) => return Outcome::error(why),
        };
        let mut filters = Vec::new();
        for step in steps {
            match step_filter(step, duration) {
                Ok((filter, after)) => {
                    filters.push(filter);
                    duration = after;
                }
                Err(why) => return Outcome::error(why),
            }
        }
        let output = match self
            .workspace
            .fresh_file("edits", &Self::stem(&path), &format)
        {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let mut args: Vec<std::ffi::OsString> = vec![
            "-i".into(),
            path.into_os_string(),
            "-vn".into(),
            "-af".into(),
            filters.join(",").into(),
        ];
        args.extend(Self::codec(&format).into_iter().map(Into::into));
        args.push(output.as_os_str().to_os_string());
        if let Err(why) = self.ffmpeg(args) {
            let _ = std::fs::remove_file(&output);
            return Outcome::error(why);
        }
        let loudness = self.loudness(&output).unwrap_or(Value::Null);
        self.with_waveform(json!({"file": self.workspace.relative(&output), "seconds": duration, "loudness": loudness}), &output)
    }

    fn mix(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(tracks) = arguments
            .get("tracks")
            .and_then(Value::as_array)
            .filter(|t| !t.is_empty() && t.len() <= 32)
        else {
            return Outcome::error("tracks is a list of 1 to 32 {file, start_s, gain_db}");
        };
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("mix");
        let format = match Self::format(arguments, "wav") {
            Ok(format) => format,
            Err(why) => return Outcome::error(why),
        };
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        let mut chains = Vec::new();
        for (i, track) in tracks.iter().enumerate() {
            let path = match self.input(track.get("file").and_then(Value::as_str)) {
                Ok(path) => path,
                Err(why) => return Outcome::error(why),
            };
            let start = match optional(track, "start_s", 0.0, 0.0..=86_400.0) {
                Ok(start) => start,
                Err(why) => return Outcome::error(why),
            };
            let gain = match optional(track, "gain_db", 0.0, -60.0..=20.0) {
                Ok(gain) => gain,
                Err(why) => return Outcome::error(why),
            };
            args.push("-i".into());
            args.push(path.into_os_string());
            let delay = (start * 1000.0).round() as u64;
            chains.push(format!(
                "[{i}:a]adelay={delay}:all=1,volume={gain:.2}dB[t{i}]"
            ));
        }
        let inputs: String = (0..tracks.len()).map(|i| format!("[t{i}]")).collect();
        let normalize = arguments
            .get("normalize")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let tail = if normalize {
            ",loudnorm=I=-16:TP=-1.5:LRA=11"
        } else {
            ""
        };
        let graph = format!(
            "{};{inputs}amix=inputs={}:duration=longest:normalize=0{tail}[out]",
            chains.join(";"),
            tracks.len()
        );
        let output =
            match self
                .workspace
                .fresh_file("mixes", &crate::delivery::to_name(name), &format)
            {
                Ok(output) => output,
                Err(e) => return Outcome::error(e.to_string()),
            };
        args.extend([
            "-filter_complex".into(),
            graph.into(),
            "-map".into(),
            "[out]".into(),
        ]);
        args.extend(Self::codec(&format).into_iter().map(Into::into));
        args.push(output.as_os_str().to_os_string());
        if let Err(why) = self.ffmpeg(args) {
            let _ = std::fs::remove_file(&output);
            return Outcome::error(why);
        }
        let seconds = self.probe(&output).ok().and_then(|i| i["seconds"].as_f64());
        self.with_waveform(
            json!({"file": self.workspace.relative(&output), "seconds": seconds}),
            &output,
        )
    }

    fn convert(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments.get("file").and_then(Value::as_str)) {
            Ok(path) => path,
            Err(why) => return Outcome::error(why),
        };
        let format = match arguments.get("to").and_then(Value::as_str) {
            Some(to) if WRITES.contains(&to) => to.to_string(),
            _ => return Outcome::error(format!("to is one of {WRITES:?}")),
        };
        let output = match self
            .workspace
            .fresh_file("exports", &Self::stem(&path), &format)
        {
            Ok(output) => output,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let mut args: Vec<std::ffi::OsString> =
            vec!["-i".into(), path.into_os_string(), "-vn".into()];
        args.extend(Self::codec(&format).into_iter().map(Into::into));
        args.push(output.as_os_str().to_os_string());
        match self.ffmpeg(args) {
            Ok(()) => Outcome::json(&json!({"file": self.workspace.relative(&output)})),
            Err(why) => {
                let _ = std::fs::remove_file(&output);
                Outcome::error(why)
            }
        }
    }

    fn picture(&self, arguments: &Map<String, Value>) -> Outcome {
        let path = match self.input(arguments.get("file").and_then(Value::as_str)) {
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
        match self.waveform(&path, &png) {
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

impl Bridge for Audio {
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
        let file = json!({"type": "string", "description": "A recording in the workspace."});
        let formats = json!({"type": "string", "enum": WRITES});
        vec![
            BridgeTool {
                name: "audio_files",
                title: "Recordings",
                description: "The audio files in the workspace, and videos whose sound can be used.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "audio_inspect",
                title: "What a recording is",
                description: "Duration, codec, sample rate, channels, bit rate, and loudness as EBU R128 measures \
                              it: integrated LUFS, loudness range and true peak.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "audio_process",
                title: "Clean and level a recording",
                description: "Runs steps in order and writes edits/<name>-<n>.<format> with a waveform picture: \
                              trim {start_s, end_s}, fade_in {seconds}, fade_out {seconds}, gain {db}, normalize \
                              {lufs (-16 podcasts, -14 streaming, -23 broadcast), true_peak}, denoise {db}, highpass \
                              {hz}, lowpass {hz}, compress {ratio, threshold_db}, trim_silence {threshold_db}, speed \
                              {factor}, mono, sample_rate {hz}. Default format wav (24-bit).",
                input_schema: json!({
                    "type": "object",
                    "properties": {"file": file, "steps": {"type": "array"}, "format": formats},
                    "required": ["file", "steps"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "audio_mix",
                title: "Mix tracks",
                description: "Mixes tracks, each placed at start_s with gain_db, into mixes/<name>-<n>.<format>, \
                              levelled to -16 LUFS unless normalize is false.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "tracks": {"type": "array", "items": {"type": "object", "properties": {
                            "file": {"type": "string"}, "start_s": {"type": "number"}, "gain_db": {"type": "number"}},
                            "required": ["file"]}},
                        "normalize": {"type": "boolean"},
                        "format": formats
                    },
                    "required": ["tracks"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "audio_convert",
                title: "Convert a recording",
                description: "Converts to wav (24-bit), aiff, flac, mp3, m4a, ogg or opus in exports/.",
                input_schema: json!({"type": "object", "properties": {"file": file, "to": formats}, "required": ["file", "to"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "audio_waveform",
                title: "Picture of a recording",
                description: "The recording's waveform as a picture in renders/, returned as an image.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "audio_files" => {
                let (files, truncated) = self.workspace.find(&READS, 5, 2000);
                Outcome::json(&json!({"files": files, "truncated": truncated}))
            }
            "audio_inspect" => self.inspect(arguments),
            "audio_process" => self.process(arguments),
            "audio_mix" => self.mix(arguments),
            "audio_convert" => self.convert(arguments),
            "audio_waveform" => self.picture(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_become_filters_from_numbers_only() {
        assert_eq!(
            step_filter(&json!("denoise"), 10.0).unwrap().0,
            "afftdn=nr=12.0:nf=-40"
        );
        let (fade, _) = step_filter(&json!({"fade_out": {"seconds": 2}}), 10.0).unwrap();
        assert_eq!(fade, "afade=t=out:st=8.000:d=2.000");
        let (trim, left) = step_filter(&json!({"trim": {"start_s": 1, "end_s": 4}}), 10.0).unwrap();
        assert!(trim.starts_with("atrim=start=1.000:end=4.000"));
        assert!((left - 3.0).abs() < 1e-9);
        let (speed, after) = step_filter(&json!({"speed": {"factor": 3}}), 9.0).unwrap();
        assert_eq!(speed, "atempo=2.0,atempo=1.5000");
        assert!((after - 3.0).abs() < 1e-9);
        for bad in [
            json!({"gain": {"db": "3;anullsink"}}),
            json!({"gain": {"db": 900}}),
            json!({"normalize": {"lufs": 0}}),
            json!("reverse"),
            json!({"trim": {"start_s": 5, "end_s": 2}}),
        ] {
            assert!(step_filter(&bad, 10.0).is_err(), "{bad}");
        }
    }
}
