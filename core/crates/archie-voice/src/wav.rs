//! WAV files: reading what a recogniser should hear, writing what a voice said.

/// Reads a mono or stereo 16-bit PCM WAV as mono float samples and its rate.
pub fn read_wav(path: &std::path::Path) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{}: not a WAV file", path.display()));
    }
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if bits != 16 || channels == 0 {
                return Err(format!("{}: only 16-bit PCM is read", path.display()));
            }
            let samples = body
                .chunks_exact(2 * channels as usize)
                .map(|frame| {
                    let sum: i32 = frame
                        .chunks_exact(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]]) as i32)
                        .sum();
                    sum as f32 / channels as f32 / 32768.0
                })
                .collect();
            return Ok((samples, rate));
        }
        at += 8 + size + (size & 1);
    }
    Err(format!("{}: no audio in it", path.display()))
}

/// Writes mono PCM16 as a WAV file.
pub fn write_wav(path: &std::path::Path, samples: &[i16], rate: u32) -> std::io::Result<()> {
    let data = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_written_is_read_back_the_same() {
        let dir = std::env::temp_dir().join(format!("archie-voice-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let samples: Vec<i16> = (0..2205).map(|i| ((i % 100) as i16 - 50) * 300).collect();
        write_wav(&path, &samples, 22_050).unwrap();
        let (read, rate) = read_wav(&path).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(read.len(), samples.len());
        assert!((read[7] - samples[7] as f32 / 32768.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(dir);
    }
}
