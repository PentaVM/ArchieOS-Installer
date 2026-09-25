//! Piper, a voice that keeps up with speech on a CPU.
//!
//! Piper is C++ over ONNX Runtime. It runs here as one long-lived process, so the voice
//! model loads once: each piece of text goes in as a line of JSON naming a WAV file, and
//! Piper answers with that file's path when the audio is complete, which is how the end of
//! each piece is known without guessing from a stream of raw samples.
//!
//! Every file is read and deleted at once. Nothing Archie said stays on disk.

use crate::voice::{Said, Speaks};
use crate::wav::read_wav;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

/// Audio is handed on in pieces of this many milliseconds, so a person speaking over
/// Archie stops it within one piece.
const PIECE_MS: usize = 100;

pub struct Piper {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    rate: u32,
    dir: PathBuf,
    said: u64,
}

impl Piper {
    /// Starts Piper with a voice model (`.onnx`, with its `.onnx.json` beside it).
    pub fn start(binary: &Path, model: &Path, speaker: Option<u32>) -> Result<Self, String> {
        let config = PathBuf::from(format!("{}.json", model.display()));
        let rate = sample_rate_of(&config)?;
        let dir = std::env::temp_dir().join(format!("archie-piper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut command = Command::new(binary);
        command
            .arg("--model")
            .arg(model)
            .arg("--json-input")
            .arg("--quiet")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(speaker) = speaker {
            command.arg("--speaker").arg(speaker.to_string());
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("{}: {e}", binary.display()))?;
        let input = child.stdin.take().ok_or("piper has no input")?;
        let output = BufReader::new(child.stdout.take().ok_or("piper has no output")?);
        Ok(Self {
            child,
            input,
            output,
            rate,
            dir,
            said: 0,
        })
    }
}

/// The sample rate a Piper voice's config declares.
fn sample_rate_of(config: &Path) -> Result<u32, String> {
    let text = std::fs::read_to_string(config).map_err(|e| format!("{}: {e}", config.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", config.display()))?;
    value["audio"]["sample_rate"]
        .as_u64()
        .filter(|rate| (8_000..=48_000).contains(rate))
        .map(|rate| rate as u32)
        .ok_or_else(|| format!("{}: no audio.sample_rate", config.display()))
}

/// One request line: the text and where to put its audio.
fn request(text: &str, file: &Path) -> String {
    // Piper reads one JSON object per line, so the text must not break the line.
    let text = text.replace(['\n', '\r'], " ");
    serde_json::json!({"text": text, "output_file": file}).to_string()
}

impl Speaks for Piper {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn say(&mut self, text: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String> {
        let started = Instant::now();
        self.said += 1;
        let file = self.dir.join(format!("{}.wav", self.said));
        writeln!(self.input, "{}", request(text, &file))
            .and_then(|_| self.input.flush())
            .map_err(|e| format!("piper stopped listening: {e}"))?;
        let mut done = String::new();
        self.output
            .read_line(&mut done)
            .map_err(|e| format!("piper stopped answering: {e}"))?;
        if done.is_empty() {
            return Err("piper exited".into());
        }
        let first_audio_ms = started.elapsed().as_secs_f64() * 1000.0;
        let read = read_wav(&file);
        let _ = std::fs::remove_file(&file);
        let (samples, rate) = read?;
        let pcm: Vec<i16> = samples
            .iter()
            .map(|s| (s * 32768.0).clamp(-32768.0, 32767.0) as i16)
            .collect();
        let piece = (rate as usize * PIECE_MS / 1000).max(1);
        let mut stopped = false;
        for chunk in pcm.chunks(piece) {
            if !hear(chunk) {
                stopped = true;
                break;
            }
        }
        Ok(Said {
            sample_rate: rate,
            audio_s: pcm.len() as f64 / rate.max(1) as f64,
            elapsed_s: started.elapsed().as_secs_f64(),
            first_audio_ms,
            stopped,
        })
    }
}

impl Drop for Piper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_one_line_of_json_whatever_the_text() {
        let line = request("Hola.\nSoy \"Archie\".", Path::new("/tmp/a.wav"));
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["text"], "Hola. Soy \"Archie\".");
        assert_eq!(value["output_file"], "/tmp/a.wav");
    }

    #[test]
    fn the_rate_comes_from_the_voice_config_and_must_be_plausible() {
        let dir = std::env::temp_dir().join(format!("archie-piper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.onnx.json");
        std::fs::write(&good, r#"{"audio": {"sample_rate": 22050}}"#).unwrap();
        assert_eq!(sample_rate_of(&good), Ok(22_050));
        let bad = dir.join("bad.onnx.json");
        std::fs::write(&bad, r#"{"audio": {"sample_rate": 3}}"#).unwrap();
        assert!(sample_rate_of(&bad).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
