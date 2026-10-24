//! The microphone and the speaker, through PulseAudio (feature `pulse`), and what every
//! source of sound shares: the capture rate, loudness, and telling a voice from the room.
//!
//! PulseAudio is what a Linux desktop — and WSLg, which forwards Windows' microphone and
//! speakers — offers every program. Capture is mono float at 16 kHz, what the recogniser
//! wants; playback is mono PCM16 at whatever rate the voice makes.
//!
//! Playback has one property the conversation depends on: it can be stopped *now*. When a
//! person starts talking over Archie, the audio already queued in the server is dropped,
//! not played out, because a voice that keeps going for another second after being
//! interrupted is a voice that was not listening.

#[cfg(feature = "pulse")]
use libpulse_binding::def::BufferAttr;
#[cfg(feature = "pulse")]
use libpulse_binding::sample::{Format, Spec};
#[cfg(feature = "pulse")]
use libpulse_binding::stream::Direction;
#[cfg(feature = "pulse")]
use libpulse_simple_binding::Simple;

/// The rate the ears hear at.
pub const CAPTURE_RATE: u32 = 16_000;
/// Samples in one captured frame: 20 ms.
pub const FRAME: usize = (CAPTURE_RATE as usize) / 50;

#[derive(Debug, thiserror::Error)]
#[error("{what}: {message}")]
pub struct AudioError {
    what: &'static str,
    message: String,
}

impl AudioError {
    pub fn new(what: &'static str, message: impl Into<String>) -> Self {
        Self {
            what,
            message: message.into(),
        }
    }
}

#[cfg(feature = "pulse")]
fn failed(what: &'static str) -> impl FnOnce(libpulse_binding::error::PAErr) -> AudioError {
    move |error| AudioError {
        what,
        message: error.to_string().unwrap_or_else(|| format!("{error:?}")),
    }
}

/// The microphone.
#[cfg(feature = "pulse")]
pub struct Microphone {
    simple: Simple,
}

#[cfg(feature = "pulse")]
impl Microphone {
    pub fn open(app: &str) -> Result<Self, AudioError> {
        let spec = Spec {
            format: Format::FLOAT32NE,
            channels: 1,
            rate: CAPTURE_RATE,
        };
        // Small fragments: the ears should get sound 20 ms after it is made, not 2 s.
        let fragment = (FRAME * 4) as u32;
        let attr = BufferAttr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: fragment,
        };
        let simple = Simple::new(
            None,
            app,
            Direction::Record,
            None,
            "escuchando",
            &spec,
            None,
            Some(&attr),
        )
        .map_err(failed("opening the microphone"))?;
        Ok(Self { simple })
    }

    /// Reads the next 20 ms of sound.
    pub fn frame(&mut self) -> Result<[f32; FRAME], AudioError> {
        let mut bytes = [0u8; FRAME * 4];
        self.simple
            .read(&mut bytes)
            .map_err(failed("hearing the microphone"))?;
        let mut samples = [0f32; FRAME];
        for (sample, raw) in samples.iter_mut().zip(bytes.chunks_exact(4)) {
            *sample = f32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
        }
        Ok(samples)
    }
}

/// The speaker.
#[cfg(feature = "pulse")]
pub struct Speaker {
    simple: Simple,
    rate: u32,
}

#[cfg(feature = "pulse")]
impl Speaker {
    pub fn open(app: &str, rate: u32) -> Result<Self, AudioError> {
        let spec = Spec {
            format: Format::S16NE,
            channels: 1,
            rate,
        };
        // A short target buffer, so stopping loses at most ~100 ms of queued voice.
        let bytes_per_second = rate * 2;
        let attr = BufferAttr {
            maxlength: u32::MAX,
            tlength: bytes_per_second / 10,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        let simple = Simple::new(
            None,
            app,
            Direction::Playback,
            None,
            "voz de Archie",
            &spec,
            None,
            Some(&attr),
        )
        .map_err(failed("opening the speaker"))?;
        Ok(Self { simple, rate })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Queues samples to be played. Blocks while the server's buffer is full, which is
    /// what keeps synthesis and playback in step.
    pub fn play(&mut self, samples: &[i16]) -> Result<(), AudioError> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_ne_bytes());
        }
        self.simple
            .write(&bytes)
            .map_err(failed("playing the voice"))
    }

    /// Waits until everything queued has been heard.
    pub fn drain(&mut self) -> Result<(), AudioError> {
        self.simple.drain().map_err(failed("finishing the voice"))
    }

    /// Drops everything queued, unheard.
    pub fn stop(&mut self) -> Result<(), AudioError> {
        self.simple.flush().map_err(failed("stopping the voice"))
    }
}

/// How loud a frame is, as root-mean-square amplitude in [0, 1].
pub fn loudness(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Tells speech from the room's own noise by loudness, against a noise floor it keeps
/// learning. The recogniser decides what was said; this only decides whether someone is
/// talking, which is what stopping the voice needs to know fast.
#[derive(Debug, Clone)]
pub struct VoiceActivity {
    floor: f32,
    /// How many times louder than the floor counts as a voice.
    ratio: f32,
    /// Consecutive loud frames needed before it counts: a cough is one or two.
    needed_ms: u32,
    loud_us: u64,
    quiet_us: u64,
}

impl Default for VoiceActivity {
    fn default() -> Self {
        Self {
            floor: 0.003,
            ratio: 4.0,
            needed_ms: 120,
            loud_us: 0,
            quiet_us: 0,
        }
    }
}

impl VoiceActivity {
    /// Takes one 20 ms frame's loudness; says whether a voice is present.
    pub fn hear(&mut self, level: f32) -> bool {
        self.hear_for(level, 20_000)
    }

    /// Measures audio time, independent of network packet size. Inspect short windows
    /// so a packet containing both speech and silence does not smear their boundary.
    /// Missing/invalid audio supplies no evidence that the room became quiet.
    pub fn hear_audio(&mut self, samples: &[f32], rate: u32) -> bool {
        if samples.is_empty() {
            return false;
        }
        if !(8_000..=192_000).contains(&rate)
            || samples.iter().any(|s| !s.is_finite() || s.abs() > 1.0)
        {
            self.loud_us = 0;
            self.quiet_us = 0;
            return false;
        }
        let mut voiced = false;
        for frame in samples.chunks((rate / 50) as usize) {
            let micros = frame.len() as u64 * 1_000_000 / u64::from(rate);
            voiced = self.hear_for(loudness(frame), micros);
        }
        voiced
    }

    fn hear_for(&mut self, level: f32, micros: u64) -> bool {
        if !level.is_finite() || !(0.0..=1.0).contains(&level) {
            self.loud_us = 0;
            self.quiet_us = 0;
            return false;
        }
        let voiced = level > (self.floor * self.ratio).max(0.01);
        if voiced {
            self.loud_us = self.loud_us.saturating_add(micros);
            self.quiet_us = 0;
        } else {
            self.loud_us = 0;
            self.quiet_us = self.quiet_us.saturating_add(micros);
            // The floor follows quiet sound slowly, and never learns from speech.
            let retain = 0.98_f32.powf(micros as f32 / 20_000.0);
            self.floor = self.floor * retain + level * (1.0 - retain);
        }
        self.loud_us >= u64::from(self.needed_ms) * 1000
    }

    /// Milliseconds since the last loud frame.
    pub fn quiet_ms(&self) -> u32 {
        (self.quiet_us / 1000).min(u64::from(u32::MAX)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_packet_size_does_not_change_voice_or_silence_time() {
        for chunk in [320, 1600, 2560] {
            let mut activity = VoiceActivity::default();
            for frame in vec![0.1; 16_000].chunks(chunk) {
                activity.hear_audio(frame, 16_000);
            }
            assert!(activity.hear_audio(&[0.1; 320], 16_000));
            for frame in vec![0.0; 16_000].chunks(chunk) {
                activity.hear_audio(frame, 16_000);
            }
            assert_eq!(activity.quiet_ms(), 1000, "packet size {chunk}");
        }
    }

    #[test]
    fn empty_or_invalid_audio_is_not_evidence_of_a_quiet_room() {
        let mut activity = VoiceActivity::default();
        activity.hear_audio(&[0.0; 1600], 16_000);
        assert_eq!(activity.quiet_ms(), 100);
        activity.hear_audio(&[], 16_000);
        assert_eq!(activity.quiet_ms(), 100);
        activity.hear_audio(&[f32::NAN; 320], 16_000);
        assert_eq!(activity.quiet_ms(), 0);
        assert!(!activity.hear_audio(&[0.1; 320], 0));
        assert_eq!(activity.quiet_ms(), 0);
    }

    #[test]
    fn a_voice_is_a_run_of_loud_frames_not_a_click() {
        let mut activity = VoiceActivity::default();
        for _ in 0..50 {
            assert!(!activity.hear(0.002), "room noise is not a voice");
        }
        assert!(!activity.hear(0.2), "one loud frame is a click");
        assert!(!activity.hear(0.0), "and then quiet again");
        let mut heard = false;
        for _ in 0..8 {
            heard = activity.hear(0.1);
        }
        assert!(heard, "120 ms of speech is a voice");
        assert_eq!(activity.quiet_ms(), 0);
        for _ in 0..10 {
            activity.hear(0.001);
        }
        assert_eq!(activity.quiet_ms(), 200);
    }

    #[test]
    fn loudness_is_rms() {
        assert_eq!(loudness(&[]), 0.0);
        assert!((loudness(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
    }
}
