//! A voice converted into another: RVC (Retrieval-based Voice Conversion) v2, exported to
//! ONNX once (`tools/voice/export_rvc_onnx.py` in the IRIS Foundry repo) and run here by
//! ONNX Runtime, loaded from its official shared library.
//!
//! Magpie says the sentence, bilingual and in time; RVC then gives it another speaker's
//! timbre while keeping its words and its melody:
//!
//! ```text
//!  Magpie PCM ─► 16 kHz ─► high-pass ─► reflect pad 1 s ─┬─► ContentVec ─► index blend ─► ×2 frames ─┐
//!                                                        └─► RMVPE mel ─► RMVPE ─► pitch (Hz, coarse) ─┤
//!                                                    noise ─────────────────────────────────────────────┤
//!                                                                       synthesizer (the voice) ◄──────┘
//!                                                          trim the pad ─► the voice's rate ─► Magpie's rate
//! ```
//!
//! Every step follows RVC's own pipeline (`infer/vc/pipeline.py`), and the Python reference
//! beside the exporter runs the same steps over the same files.

use crate::voice::{Said, Speaks};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// What the exporter wrote beside the models.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub sample_rate: u32,
    pub hop: usize,
    pub input_rate: u32,
    pub window: usize,
    pub inter_channels: usize,
    pub f0_min: f32,
    pub f0_max: f32,
    pub rmvpe_threshold: f32,
    pub index_rows: usize,
}

/// How a voice is converted.
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Semitones to move the pitch by before conversion.
    pub key: f32,
    /// How much of each frame's features come from the voice's own training features
    /// (RVC's index rate): more sounds more like the voice's speaker, less keeps the
    /// pronunciation of what was said. A voice trained on English pulls Spanish toward
    /// English: with Baymax, Nemotron's error on Spanish sentences went from 1.8 % at 0 to
    /// 22 % at RVC's usual 0.75, which Nelson heard as "a gringo trying to speak Spanish".
    /// The timbre comes from the synthesizer either way.
    pub index_rate: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { key: 0.0, index_rate: 0.0 }
    }
}

/// Loads ONNX Runtime from its shared library. Once per process, before any voice.
pub fn init_runtime(library: &Path) -> Result<(), String> {
    let _ = ort::init_from(library)
        .map_err(|e| format!("loading ONNX Runtime from {}: {e}", library.display()))?
        .with_name("archie-voice")
        .commit();
    Ok(())
}

fn session(path: &Path, threads: usize) -> Result<Session, String> {
    let mut builder = Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(threads)
        .map_err(|e| e.to_string())?;
    builder
        .commit_from_file(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A converted voice: the five models and what the manifest says about them.
pub struct RvcVoice {
    manifest: Manifest,
    tuning: Tuning,
    content: Session,
    mel: Session,
    rmvpe: Session,
    synthesizer: Session,
    index: Option<Session>,
    noise: Noise,
}

impl RvcVoice {
    /// `manifest` is the exporter's `<name>.json`; the models sit beside it.
    pub fn load(manifest: &Path, tuning: Tuning, threads: usize) -> Result<Self, String> {
        let folder: PathBuf = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
        let described: Manifest = serde_json::from_str(
            &std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?,
        )
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let index_path = folder.join(format!("{}_index.onnx", described.name));
        Ok(Self {
            content: session(&folder.join("content.onnx"), threads)?,
            mel: session(&folder.join("rmvpe_mel.onnx"), threads)?,
            rmvpe: session(&folder.join("rmvpe.onnx"), threads)?,
            synthesizer: session(&folder.join(format!("{}.onnx", described.name)), threads)?,
            index: if index_path.exists() && tuning.index_rate > 0.0 {
                Some(session(&index_path, threads)?)
            } else {
                None
            },
            manifest: described,
            tuning,
            noise: Noise::new(0x5eed_a4c1),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.manifest.sample_rate
    }

    /// Converts mono audio at `rate` into the voice, at the voice's own rate.
    pub fn convert(&mut self, audio: &[f32], rate: u32) -> Result<Vec<f32>, String> {
        let m = self.manifest.clone();
        let mut audio = resample(audio, rate, m.input_rate);
        high_pass(&mut audio, 48.0, m.input_rate);
        let pad = m.input_rate as usize;
        let padded = reflect_pad(&audio, pad);
        let frames = padded.len() / m.window;
        if frames < 64 {
            return Err("too short to convert".into());
        }

        // Pitch: RMVPE's mel, padded to a multiple of 32 frames, then its salience.
        let mel = run1(&mut self.mel, "audio", vec![1, padded.len() as i64], padded.clone())?;
        let (mel_shape, mel_data) = mel;
        let (bins, mel_frames) = (mel_shape[1] as usize, mel_shape[2] as usize);
        let padded_frames = 32 * ((mel_frames - 1) / 32 + 1);
        let mut mel_padded = vec![0.0f32; bins * padded_frames];
        for bin in 0..bins {
            mel_padded[bin * padded_frames..bin * padded_frames + mel_frames]
                .copy_from_slice(&mel_data[bin * mel_frames..(bin + 1) * mel_frames]);
        }
        let rmvpe_input = self.rmvpe.inputs()[0].name().to_string();
        let (salience_shape, salience) = run1(
            &mut self.rmvpe,
            &rmvpe_input,
            vec![1, bins as i64, padded_frames as i64],
            mel_padded,
        )?;
        let columns = salience_shape[2] as usize;
        let mut f0: Vec<f32> = (0..mel_frames.min(frames))
            .map(|frame| decode_salience(&salience[frame * columns..(frame + 1) * columns], m.rmvpe_threshold))
            .collect();
        f0.resize(frames, 0.0);
        fill_unvoiced(&mut f0);
        let shift = 2f32.powf(self.tuning.key / 12.0);
        f0.iter_mut().for_each(|hz| *hz *= shift);
        let pitch: Vec<i64> = f0.iter().map(|hz| coarse_pitch(*hz, m.f0_min, m.f0_max)).collect();

        // Content: ContentVec at 50 frames a second, drawn toward the voice's own features.
        let (feature_shape, mut features) =
            run1(&mut self.content, "audio", vec![1, padded.len() as i64], padded)?;
        let (content_frames, width) = (feature_shape[1] as usize, feature_shape[2] as usize);
        if let Some(index) = &mut self.index {
            let (_, retrieved) =
                run1(index, "features", vec![content_frames as i64, width as i64], features.clone())?;
            let rate = self.tuning.index_rate;
            for (feature, near) in features.iter_mut().zip(retrieved) {
                *feature = rate * near + (1.0 - rate) * *feature;
            }
        }
        let length = frames.min(content_frames * 2);
        let mut phone = Vec::with_capacity(length * width);
        for frame in 0..length {
            let source = frame / 2;
            phone.extend_from_slice(&features[source * width..(source + 1) * width]);
        }

        let noise: Vec<f32> = (0..m.inter_channels * length).map(|_| self.noise.normal()).collect();
        let inputs = ort::inputs![
            "phone" => Tensor::from_array((vec![1i64, length as i64, width as i64], phone)).map_err(|e| e.to_string())?,
            "phone_lengths" => Tensor::from_array((vec![1i64], vec![length as i64])).map_err(|e| e.to_string())?,
            "pitch" => Tensor::from_array((vec![1i64, length as i64], pitch[..length].to_vec())).map_err(|e| e.to_string())?,
            "pitchf" => Tensor::from_array((vec![1i64, length as i64], f0[..length].to_vec())).map_err(|e| e.to_string())?,
            "sid" => Tensor::from_array((vec![1i64], vec![0i64])).map_err(|e| e.to_string())?,
            "noise" => Tensor::from_array((vec![1i64, m.inter_channels as i64, length as i64], noise)).map_err(|e| e.to_string())?,
        ];
        let outputs = self.synthesizer.run(inputs).map_err(|e| format!("synthesizer: {e}"))?;
        let (_, voice) = outputs["audio"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let trim = m.sample_rate as usize;
        let mut voice = if voice.len() > 2 * trim { voice[trim..voice.len() - trim].to_vec() } else { Vec::new() };
        let peak = voice.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        if peak > 0.99 {
            voice.iter_mut().for_each(|s| *s *= 0.99 / peak);
        }
        Ok(voice)
    }
}

/// Runs a model of one input and takes its first output.
fn run1(session: &mut Session, name: &str, shape: Vec<i64>, data: Vec<f32>) -> Result<(Vec<i64>, Vec<f32>), String> {
    let tensor = Tensor::from_array((shape, data)).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs![name => tensor]).map_err(|e| e.to_string())?;
    let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    Ok((shape.iter().copied().collect(), data.to_vec()))
}

/// Resamples with a windowed sinc, low-passed at the lower of the two Nyquist rates.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0) * 0.97;
    let half = 16.0 / cutoff; // taps either side, in input samples
    let length = (input.len() as f64 * ratio).round() as usize;
    (0..length)
        .map(|i| {
            let t = i as f64 / ratio;
            let first = (t - half).ceil().max(0.0) as usize;
            let last = ((t + half).floor() as usize).min(input.len() - 1);
            let mut sum = 0.0f64;
            let mut weight = 0.0f64;
            for (k, sample) in input.iter().enumerate().take(last + 1).skip(first) {
                // (`skip` on a slice's iterator jumps; it does not walk.)
                let x = k as f64 - t;
                let sinc = if x.abs() < 1e-9 {
                    cutoff
                } else {
                    (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * x / half).cos();
                let tap = sinc * window;
                sum += *sample as f64 * tap;
                weight += tap;
            }
            (if weight.abs() > 1e-9 { sum / weight } else { 0.0 }) as f32
        })
        .collect()
}

/// A fourth-order Butterworth high-pass run forward and backward (zero phase), in place of
/// RVC's fifth-order filtfilt at 48 Hz: it takes out the rumble ContentVec never heard.
pub fn high_pass(audio: &mut [f32], cutoff: f32, rate: u32) {
    // Two biquads with the Butterworth Qs of a fourth-order filter.
    for q in [0.541_196_1f32, 1.306_563] {
        let w = std::f32::consts::TAU * cutoff / rate as f32;
        let alpha = w.sin() / (2.0 * q);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let b = [(1.0 + cos) / 2.0 / a0, -(1.0 + cos) / a0, (1.0 + cos) / 2.0 / a0];
        let a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
        for pass in 0..2 {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut step = |sample: &mut f32| {
                let x = *sample;
                let y = b[0] * x + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                x2 = x1;
                x1 = x;
                y2 = y1;
                y1 = y;
                *sample = y;
            };
            if pass == 0 {
                audio.iter_mut().for_each(&mut step);
            } else {
                audio.iter_mut().rev().for_each(&mut step);
            }
        }
    }
}

/// `pad` samples of mirror image either side, as numpy's "reflect".
pub fn reflect_pad(audio: &[f32], pad: usize) -> Vec<f32> {
    let n = audio.len();
    if n < 2 {
        return [vec![0.0; pad], audio.to_vec(), vec![0.0; pad]].concat();
    }
    let at = |i: isize| -> f32 {
        let period = 2 * (n as isize - 1);
        let mut j = i.rem_euclid(period);
        if j >= n as isize {
            j = period - j;
        }
        audio[j as usize]
    };
    (-(pad as isize)..(n + pad) as isize).map(at).collect()
}

/// One frame of RMVPE salience (360 bins, 20 cents apart) to Hz: the weighted average of
/// the nine bins around the peak, or 0 when the peak is below `threshold`.
pub fn decode_salience(salience: &[f32], threshold: f32) -> f32 {
    let (peak, max) = salience
        .iter()
        .enumerate()
        .fold((0usize, f32::MIN), |best, (i, v)| if *v > best.1 { (i, *v) } else { best });
    if max <= threshold {
        return 0.0;
    }
    let cents = |bin: isize| 20.0 * bin as f32 + 1997.379_4;
    let (mut product, mut total) = (0.0f32, 0.0f32);
    for offset in -4isize..=4 {
        let bin = peak as isize + offset;
        if bin < 0 || bin >= salience.len() as isize {
            continue; // RVC pads the edges with zero weight
        }
        product += salience[bin as usize] * cents(bin);
        total += salience[bin as usize];
    }
    let hz = 10.0 * 2f32.powf(product / total / 1200.0);
    if (hz - 10.0).abs() < 1e-3 {
        0.0
    } else {
        hz
    }
}

/// Unvoiced frames take the pitch interpolated from their voiced neighbours, as RVC does
/// now, so the synthesizer's melody never drops to nothing between words.
pub fn fill_unvoiced(f0: &mut [f32]) {
    let voiced: Vec<usize> = (0..f0.len()).filter(|&i| f0[i] > 0.0).collect();
    if voiced.is_empty() || voiced.len() == f0.len() {
        return;
    }
    for i in 0..f0.len() {
        if f0[i] > 0.0 {
            continue;
        }
        let next = voiced.partition_point(|&v| v < i);
        f0[i] = match (next.checked_sub(1).map(|p| voiced[p]), voiced.get(next)) {
            (Some(a), Some(&b)) => f0[a] + (f0[b] - f0[a]) * (i - a) as f32 / (b - a) as f32,
            (Some(a), None) => f0[a],
            (None, Some(&b)) => f0[b],
            (None, None) => 0.0,
        };
    }
}

/// RVC's coarse pitch: mel-scaled between `min` and `max` Hz onto 1..=255.
pub fn coarse_pitch(hz: f32, min: f32, max: f32) -> i64 {
    let mel = |f: f32| 1127.0 * (1.0 + f / 700.0).ln();
    let (low, high) = (mel(min), mel(max));
    let mut value = mel(hz);
    if value > 0.0 {
        value = (value - low) * 254.0 / (high - low) + 1.0;
    }
    value.clamp(1.0, 255.0).round() as i64
}

/// Normal noise for the synthesizer's prior, from a small xorshift and Box-Muller.
struct Noise(u64);

impl Noise {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn normal(&mut self) -> f32 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// A voice whose every sentence is converted: the voice underneath says it in full, RVC
/// gives it the other timbre, and it goes on at the voice underneath's rate, in pieces of
/// a tenth of a second so being spoken over still stops it.
pub struct ConvertedVoice<V: Speaks> {
    voice: V,
    rvc: RvcVoice,
}

impl<V: Speaks> ConvertedVoice<V> {
    pub fn new(voice: V, rvc: RvcVoice) -> Self {
        Self { voice, rvc }
    }
}

impl<V: Speaks> Speaks for ConvertedVoice<V> {
    fn sample_rate(&self) -> u32 {
        self.voice.sample_rate()
    }

    fn say(&mut self, text: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String> {
        let started = Instant::now();
        let mut sentence = Vec::new();
        let inner = self.voice.say(text, &mut |pcm| {
            sentence.extend(pcm.iter().map(|s| *s as f32 / 32768.0));
            true
        })?;
        let rate = self.voice.sample_rate();
        let converted = match self.rvc.convert(&sentence, rate) {
            Ok(voice) => resample(&voice, self.rvc.sample_rate(), rate),
            // Too short, or a model failed: the sentence is said in the plain voice rather
            // than not at all.
            Err(_) => sentence,
        };
        let pcm: Vec<i16> = converted.iter().map(|s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16).collect();
        let first_audio_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut stopped = false;
        for piece in pcm.chunks((rate / 10).max(1) as usize) {
            if !hear(piece) {
                stopped = true;
                break;
            }
        }
        Ok(Said {
            sample_rate: rate,
            audio_s: pcm.len() as f64 / rate as f64,
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

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn resampling_keeps_what_fits_and_drops_what_would_fold_back() {
        let kept = resample(&tone(440.0, 1.0, 22050), 22050, 16000);
        assert_eq!(kept.len(), 16000);
        assert!((rms(&kept[1000..15000]) - 0.3536).abs() < 0.01, "{}", rms(&kept));
        let folded = resample(&tone(10_000.0, 1.0, 22050), 22050, 16000);
        assert!(rms(&folded[1000..15000]) < 0.02, "10 kHz must not alias to 6 kHz");
    }

    #[test]
    fn the_high_pass_takes_rumble_and_leaves_the_voice() {
        let mut rumble = tone(20.0, 2.0, 16000);
        high_pass(&mut rumble, 48.0, 16000);
        let mut voice = tone(200.0, 2.0, 16000);
        high_pass(&mut voice, 48.0, 16000);
        assert!(rms(&rumble[8000..24000]) < 0.03);
        assert!((rms(&voice[8000..24000]) - 0.3536).abs() < 0.01);
    }

    /// ARCHIE_RVC_VOICE (a `<name>.json`), ORT_DYLIB_PATH, ARCHIE_RVC_WAV in, ARCHIE_RVC_OUT out.
    #[test]
    #[ignore = "needs an exported voice and ONNX Runtime"]
    fn a_recording_is_converted_into_the_voice() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        init_runtime(Path::new(&env("ORT_DYLIB_PATH"))).unwrap();
        let mut voice = RvcVoice::load(Path::new(&env("ARCHIE_RVC_VOICE")), Tuning::default(), 4).unwrap();
        let (audio, rate) = crate::wav::read_wav(Path::new(&env("ARCHIE_RVC_WAV"))).unwrap();
        let started = Instant::now();
        let converted = voice.convert(&audio, rate).unwrap();
        let seconds_in = audio.len() as f32 / rate as f32;
        let seconds_out = converted.len() as f32 / voice.sample_rate() as f32;
        eprintln!("{seconds_in:.2} s in, {seconds_out:.2} s out, in {:.2} s", started.elapsed().as_secs_f32());
        assert!((seconds_in - seconds_out).abs() < 0.05);
        assert!(converted.iter().all(|s| s.is_finite()) && rms(&converted) > 0.01);
        let pcm: Vec<i16> = converted.iter().map(|s| (s * 32767.0) as i16).collect();
        crate::wav::write_wav(Path::new(&env("ARCHIE_RVC_OUT")), &pcm, voice.sample_rate()).unwrap();
    }

    #[test]
    fn reflect_padding_mirrors_like_numpy() {
        assert_eq!(reflect_pad(&[1.0, 2.0, 3.0], 2), vec![3.0, 2.0, 1.0, 2.0, 3.0, 2.0, 1.0]);
    }

    #[test]
    fn salience_decodes_to_the_peaks_pitch() {
        // A peak at bin 100: 20 * 100 + 1997.38 cents, 10 * 2^(3997.38 / 1200) Hz.
        let mut salience = vec![0.0f32; 360];
        salience[100] = 0.9;
        salience[99] = 0.3;
        salience[101] = 0.3;
        let expected = 10.0 * 2f32.powf(3997.3794 / 1200.0);
        assert!((decode_salience(&salience, 0.03) - expected).abs() < 0.01);
        assert_eq!(decode_salience(&vec![0.01; 360], 0.03), 0.0);
    }

    #[test]
    fn unvoiced_frames_are_filled_between_voiced_ones() {
        let mut f0 = vec![0.0, 100.0, 0.0, 0.0, 130.0, 0.0];
        fill_unvoiced(&mut f0);
        assert_eq!(f0, vec![100.0, 100.0, 110.0, 120.0, 130.0, 130.0]);
    }

    #[test]
    fn coarse_pitch_matches_rvcs_scale() {
        assert_eq!(coarse_pitch(0.0, 50.0, 1100.0), 1);
        assert_eq!(coarse_pitch(50.0, 50.0, 1100.0), 1);
        assert_eq!(coarse_pitch(1100.0, 50.0, 1100.0), 255);
        // 140 Hz, by RVC's formula: (1127 ln(1.2) - 1127 ln(1 + 50/700)) * 254 / range + 1.
        assert_eq!(coarse_pitch(140.0, 50.0, 1100.0), 34);
    }

    #[test]
    fn the_noise_is_standard_normal() {
        let mut noise = Noise::new(7);
        let draws: Vec<f32> = (0..20000).map(|_| noise.normal()).collect();
        let mean = draws.iter().sum::<f32>() / draws.len() as f32;
        let variance = draws.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / draws.len() as f32;
        assert!(mean.abs() < 0.03 && (variance - 1.0).abs() < 0.05, "{mean} {variance}");
    }
}
