use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

const MAX_AUDIO_SAMPLES: usize = 16_000 * 30;
const MAX_TRANSCRIPT_BYTES: usize = 16 * 1024;

/// Ephemeral audio window. It cannot be serialized and is overwritten on drop.
#[derive(Debug)]
pub struct EphemeralAudio {
    samples: Vec<f32>,
}

impl EphemeralAudio {
    pub fn new(mut samples: Vec<f32>) -> Self {
        if samples.len() > MAX_AUDIO_SAMPLES {
            samples.drain(..samples.len() - MAX_AUDIO_SAMPLES);
        }
        for sample in &mut samples {
            if !sample.is_finite() {
                *sample = 0.0;
            }
            *sample = sample.clamp(-1.0, 1.0);
        }
        Self { samples }
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

impl Drop for EphemeralAudio {
    fn drop(&mut self) {
        self.samples.fill(0.0);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEvent {
    pub stream_id: String,
    pub segment_id: u64,
    pub text: String,
    pub is_final: bool,
    /// Upstream confidence when available. `None` means unknown; it is never invented.
    pub speech_probability: Option<f32>,
    pub timestamp_ms: u64,
}

impl TranscriptEvent {
    pub fn validate(&self) -> bool {
        !self.stream_id.is_empty()
            && self.stream_id.len() <= 128
            && self.text.len() <= MAX_TRANSCRIPT_BYTES
            && self
                .speech_probability
                .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    }
}

/// Parses the live CLI prefixes emitted by NeMo-Speech.cpp. Diagnostic lines are ignored.
pub fn parse_live_event(stream_id: &str, segment_id: u64, line: &str) -> Option<TranscriptEvent> {
    let (prefix, is_final) = if line.starts_with("[live partial @ ") {
        ("[live partial @ ", false)
    } else if line.starts_with("[live final @ ") {
        ("[live final @ ", true)
    } else {
        return None;
    };
    let tail = line.strip_prefix(prefix)?;
    let (seconds, text) = tail.split_once("s] ")?;
    let timestamp_ms = (seconds.parse::<f64>().ok()? * 1000.0).round();
    if !timestamp_ms.is_finite() || timestamp_ms < 0.0 || timestamp_ms > u64::MAX as f64 {
        return None;
    }
    let event = TranscriptEvent {
        stream_id: stream_id.into(),
        segment_id,
        text: text.into(),
        is_final,
        speech_probability: None,
        timestamp_ms: timestamp_ms as u64,
    };
    event.validate().then_some(event)
}

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error("speech runtime input must be an existing local file")]
    InvalidInput,
    #[error("speech runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("speech runtime returned invalid output")]
    InvalidOutput,
    #[error("speech runtime failed")]
    RuntimeFailed,
}

/// Local process adapter. It has no network client and never copies the input audio.
pub struct LocalSpeechRuntime {
    command: PathBuf,
    model: PathBuf,
}

impl LocalSpeechRuntime {
    pub fn new(command: impl Into<PathBuf>, model: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            model: model.into(),
        }
    }

    pub fn transcribe(&self, audio: &Path) -> Result<Vec<TranscriptEvent>, SpeechError> {
        if !audio.is_file() || !self.model.is_file() {
            return Err(SpeechError::InvalidInput);
        }
        let mut child = Command::new(&self.command)
            .arg("--json")
            .arg("transcribe")
            .arg(audio)
            .arg("--model")
            .arg(&self.model)
            .arg("--stream")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().ok_or(SpeechError::InvalidOutput)?;
        let mut events = Vec::new();
        let native: serde_json::Value = serde_json::from_reader(BufReader::new(stdout))
            .map_err(|_| SpeechError::InvalidOutput)?;
        if !child.wait()?.success() {
            return Err(SpeechError::RuntimeFailed);
        }
        let text = native
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or(SpeechError::InvalidOutput)?;
        let confidence = native
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .map(|value| value as f32)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
        events.push(TranscriptEvent {
            stream_id: "local-file".into(),
            segment_id: 0,
            text: text.into(),
            is_final: true,
            speech_probability: confidence,
            timestamp_ms: 0,
        });
        if !events.last().is_some_and(TranscriptEvent::validate) {
            return Err(SpeechError::InvalidOutput);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_is_bounded_and_sanitized() {
        let audio = EphemeralAudio::new(vec![f32::NAN, f32::INFINITY, -3.0, 3.0]);
        assert_eq!(audio.samples(), &[0.0, 0.0, -1.0, 1.0]);
        let long = EphemeralAudio::new(vec![0.0; MAX_AUDIO_SAMPLES + 10]);
        assert_eq!(long.samples().len(), MAX_AUDIO_SAMPLES);
    }

    #[test]
    fn invalid_probabilities_are_rejected() {
        for probability in [f32::NAN, -0.1, 1.1] {
            let event = TranscriptEvent {
                stream_id: "s".into(),
                segment_id: 1,
                text: "x".into(),
                is_final: false,
                speech_probability: Some(probability),
                timestamp_ms: 0,
            };
            assert!(!event.validate());
        }
    }

    #[test]
    fn live_partial_and_final_contract_ignores_diagnostics() {
        let partial = parse_live_event("s", 1, "[live partial @ 0.32s] open the").unwrap();
        let final_event =
            parse_live_event("s", 1, "[live final @ 0.80s] open the project").unwrap();
        assert!(!partial.is_final);
        assert!(final_event.is_final);
        assert_eq!(partial.timestamp_ms, 320);
        assert!(parse_live_event("s", 1, "[nemo-speech] model loaded").is_none());
    }
}
