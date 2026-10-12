//! A character for the voice: Magpie's words, said slower, lower and softer.
//!
//! Nelson asked for Baymax's voice. That voice is an actor's (Scott Adsit's), and the
//! copies of it passed around online are clones made without him, so Archie does not
//! wear one. What Baymax sounds like can be had honestly from any voice: unhurried, a
//! little deeper and larger than a person, soft at the top, with the faint hollow of a
//! body that is mostly air. That is [`Style::Gentle`], applied to each sentence after
//! Magpie has said it — latency is not what matters here.
//!
//! Each sentence goes through, in order:
//!
//! 1. **Lower and larger**: resampled as a slowed tape is, which lowers the pitch and the
//!    formants together, so the speaker sounds bigger, not just deeper.
//! 2. **Unhurried**: stretched in time with WSOLA — overlapping windows of the sound,
//!    each placed where it best continues the last — which slows it without changing
//!    the pitch again.
//! 3. **Soft**: a low-pass takes the edge off sibilants.
//! 4. **Hollow**: a short, gentle comb of echoes, a few milliseconds long, like a voice
//!    inside a balloon.

use crate::voice::{Said, Speaks};
use std::time::Instant;

/// How the voice is shaped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    /// Magpie as it is.
    Plain,
    /// Slow, low, soft and a little hollow: a gentle robot in the manner of Baymax.
    Gentle,
}

impl Style {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "plain" => Ok(Self::Plain),
            "gentle" | "baymax" => Ok(Self::Gentle),
            other => Err(format!("unknown voice style {other:?}: plain or gentle")),
        }
    }
}

/// The shaping of [`Style::Gentle`]; the numbers are ears' choices, not physics.
struct Gentle {
    /// Tape speed: 0.94 is about one semitone lower, and 6 % slower.
    tape: f32,
    /// Further slowing without a change of pitch.
    tempo: f32,
    low_pass_hz: f32,
    /// Echo delays in milliseconds and their gains.
    body: [(f32, f32); 2],
}

const GENTLE: Gentle = Gentle {
    tape: 0.94,
    tempo: 0.92,
    low_pass_hz: 5200.0,
    body: [(6.0, 0.22), (11.0, 0.12)],
};

/// Resamples as a tape played at `speed` would sound: slower and lower below 1.
pub fn tape(samples: &[f32], speed: f32) -> Vec<f32> {
    if samples.is_empty() || speed <= 0.0 {
        return Vec::new();
    }
    let length = ((samples.len() as f32) / speed) as usize;
    (0..length)
        .map(|i| {
            let t = i as f32 * speed;
            let k = t as usize;
            let f = t - k as f32;
            let a = samples[k.min(samples.len() - 1)];
            let b = samples[(k + 1).min(samples.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

/// Stretches speech in time by 1 / `tempo` without changing its pitch (WSOLA).
pub fn stretch(samples: &[f32], tempo: f32, rate: u32) -> Vec<f32> {
    let window = (rate as usize * 30 / 1000).max(16); // 30 ms
    let hop = window / 2;
    let search = rate as usize * 8 / 1000; // ±8 ms
    if samples.len() < window * 2 || (tempo - 1.0).abs() < 1e-3 {
        return samples.to_vec();
    }
    let hann: Vec<f32> = (0..window)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / window as f32).cos())
        .collect();
    let frames = ((samples.len() - window) as f32 / (hop as f32 * tempo)) as usize;
    let mut out = vec![0.0f32; frames * hop + window];
    let mut weight = vec![0.0f32; out.len()];
    // Where the previous window was actually taken from.
    let mut previous: usize = 0;
    for frame in 0..frames {
        let nominal = (frame as f32 * hop as f32 * tempo) as usize;
        let start = if frame == 0 {
            0
        } else {
            // The window that best continues the sound the last one ended on.
            let natural = previous + hop;
            let lo = nominal.saturating_sub(search);
            let hi = (nominal + search).min(samples.len() - window);
            let mut best = (nominal.min(samples.len() - window), f32::MIN);
            for candidate in lo..=hi {
                let mut score = 0.0f32;
                for i in (0..hop).step_by(2) {
                    let a = samples.get(natural + i).copied().unwrap_or(0.0);
                    score += a * samples[candidate + i];
                }
                if score > best.1 {
                    best = (candidate, score);
                }
            }
            best.0
        };
        previous = start;
        let at = frame * hop;
        for i in 0..window {
            out[at + i] += samples[start + i] * hann[i];
            weight[at + i] += hann[i];
        }
    }
    for (value, w) in out.iter_mut().zip(&weight) {
        if *w > 1e-3 {
            *value /= w;
        }
    }
    out
}

/// A two-pole low-pass (a biquad, Butterworth).
fn low_pass(samples: &mut [f32], cutoff: f32, rate: u32) {
    let w = std::f32::consts::TAU * cutoff / rate as f32;
    let alpha = w.sin() / std::f32::consts::SQRT_2;
    let cos = w.cos();
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = (
        (1.0 - cos) / 2.0 / a0,
        (1.0 - cos) / a0,
        (1.0 - cos) / 2.0 / a0,
    );
    let (a1, a2) = (-2.0 * cos / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for sample in samples.iter_mut() {
        let x = *sample;
        let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
        *sample = y;
    }
}

/// Short echoes, summed in: the hollow of a large, soft body.
fn body(samples: &[f32], echoes: &[(f32, f32)], rate: u32) -> Vec<f32> {
    let mut out = samples.to_vec();
    for (ms, gain) in echoes {
        let delay = (ms * rate as f32 / 1000.0) as usize;
        for i in delay..samples.len() {
            out[i] += gain * samples[i - delay];
        }
    }
    out
}

/// Shapes one sentence of PCM16 at `rate`.
pub fn shape(pcm: &[i16], style: Style, rate: u32) -> Vec<i16> {
    if style == Style::Plain || pcm.is_empty() {
        return pcm.to_vec();
    }
    let g = &GENTLE;
    let peak_in = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0;
    let samples: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
    let lowered = tape(&samples, g.tape);
    let mut slowed = stretch(&lowered, g.tempo, rate);
    low_pass(&mut slowed, g.low_pass_hz, rate);
    let shaped = body(&slowed, &g.body, rate);
    // As loud as it came in, never clipped.
    let peak_out = shaped.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let gain = (peak_in.max(0.05) / peak_out).min(4.0);
    shaped
        .iter()
        .map(|s| (s * gain * 32767.0).clamp(-32768.0, 32767.0) as i16)
        .collect()
}

/// A voice with a style: each sentence is said in full by the voice underneath, shaped,
/// then handed on in pieces of a tenth of a second, so being spoken over still stops it.
pub struct StyledVoice<V: Speaks> {
    voice: V,
    style: Style,
}

impl<V: Speaks> StyledVoice<V> {
    pub fn new(voice: V, style: Style) -> Self {
        Self { voice, style }
    }
}

impl<V: Speaks> Speaks for StyledVoice<V> {
    fn sample_rate(&self) -> u32 {
        self.voice.sample_rate()
    }

    fn say(&mut self, text: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String> {
        if self.style == Style::Plain {
            return self.voice.say(text, hear);
        }
        let started = Instant::now();
        let mut sentence = Vec::new();
        let inner = self.voice.say(text, &mut |pcm| {
            sentence.extend_from_slice(pcm);
            true
        })?;
        let rate = self.voice.sample_rate();
        let shaped = shape(&sentence, self.style, rate);
        let first_audio_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut stopped = false;
        for piece in shaped.chunks((rate / 10).max(1) as usize) {
            if !hear(piece) {
                stopped = true;
                break;
            }
        }
        Ok(Said {
            sample_rate: rate,
            audio_s: shaped.len() as f64 / rate as f64,
            elapsed_s: started.elapsed().as_secs_f64(),
            first_audio_ms,
            stopped: stopped || inner.stopped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, seconds: f32, rate: u32) -> Vec<f32> {
        (0..(seconds * rate as f32) as usize)
            .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    /// The strongest frequency, by counting upward zero crossings.
    fn pitch(samples: &[f32], rate: u32) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f32 * rate as f32 / samples.len() as f32
    }

    #[test]
    fn stretching_slows_speech_without_changing_its_pitch() {
        let rate = 22050;
        let voice = tone(150.0, 2.0, rate);
        let slowed = stretch(&voice, 0.8, rate);
        let ratio = slowed.len() as f32 / voice.len() as f32;
        assert!((ratio - 1.25).abs() < 0.03, "{ratio}");
        let middle = &slowed[slowed.len() / 4..slowed.len() * 3 / 4];
        assert!(
            (pitch(middle, rate) - 150.0).abs() < 4.0,
            "{}",
            pitch(middle, rate)
        );
    }

    #[test]
    fn the_tape_lowers_pitch_and_slows_together() {
        let rate = 22050;
        let lowered = tape(&tone(200.0, 1.0, rate), 0.94);
        assert!(
            (pitch(&lowered, rate) - 188.0).abs() < 3.0,
            "{}",
            pitch(&lowered, rate)
        );
        assert!((lowered.len() as f32 / rate as f32 - 1.0 / 0.94).abs() < 0.01);
    }

    #[test]
    fn a_gentle_sentence_is_longer_lower_and_as_loud() {
        let rate = 22050;
        let pcm: Vec<i16> = tone(150.0, 1.0, rate)
            .iter()
            .map(|s| (s * 32767.0) as i16)
            .collect();
        let shaped = shape(&pcm, Style::Gentle, rate);
        assert!(shaped.len() as f32 > pcm.len() as f32 * 1.12);
        let peak = shaped.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((14_000..=16_500).contains(&peak), "{peak}");
        assert_eq!(shape(&pcm, Style::Plain, rate), pcm);
        assert_eq!(Style::parse("baymax"), Ok(Style::Gentle));
    }
}
