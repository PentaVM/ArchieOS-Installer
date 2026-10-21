//! What the frozen text tower says about a phrase, and where that comes from.
//!
//! `beta_model.PhraseEncoder` lowercases a phrase, tokenizes it to 64 positions, runs the
//! SigLIP2 text tower, and keeps the pooled vector and the first `text_tokens_per_phrase`
//! token states, stored in float16. The same happens here, with the same float16 rounding,
//! so a phrase reaches fusion as the same numbers.
//!
//! The text tower is not part of the IRIS release. A node either has the tower on disk
//! (`TextTower`, the production path) or a table of phrases encoded elsewhere
//! (`PhraseTable`), and says which in `/health`.

use crate::siglip::{load_text_weights, SiglipText};
use crate::weights::{file_sha256, Release, Weights};
use crate::Error;
use candle_core::{DType, Device, Tensor};
use half::f16;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One phrase as fusion receives it, already rounded through float16.
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseFeatures {
    pub pooled: Vec<f32>,
    /// Real tokens only (at least one, at most `text_tokens_per_phrase`).
    pub states: Vec<Vec<f32>>,
}

impl PhraseFeatures {
    pub fn new(pooled: Vec<f32>, states: Vec<Vec<f32>>) -> Result<Self, Error> {
        if pooled.is_empty() || states.is_empty() || states.iter().any(|s| s.len() != pooled.len())
        {
            return Err(Error::Phrase(
                "phrase features have inconsistent widths".into(),
            ));
        }
        let round = |v: f32| f16::from_f32(v).to_f32();
        Ok(Self {
            pooled: pooled.into_iter().map(round).collect(),
            states: states
                .into_iter()
                .map(|row| row.into_iter().map(round).collect())
                .collect(),
        })
    }
}

pub trait PhraseSource: Send {
    fn encode(&mut self, phrases: &[String]) -> Result<Vec<Arc<PhraseFeatures>>, Error>;
    /// For `/health`: what produces phrase features on this node.
    fn describe(&self) -> serde_json::Value;
}

/// Phrases encoded elsewhere: `<dir>/phrases.json` (a list of phrases) and
/// `<dir>/phrases.safetensors` with `phrase.<i>.pooled` and `phrase.<i>.states`.
pub struct PhraseTable {
    entries: HashMap<String, Arc<PhraseFeatures>>,
    origin: String,
}

impl PhraseTable {
    pub fn load(list: &Path, tensors: &Path) -> Result<Self, Error> {
        let names: Vec<String> = serde_json::from_slice(&std::fs::read(list)?)
            .map_err(|error| Error::Phrase(format!("{}: {error}", list.display())))?;
        let stored = candle_core::safetensors::load(tensors, &Device::Cpu)?;
        let mut entries = HashMap::with_capacity(names.len());
        for (index, name) in names.into_iter().enumerate() {
            let get = |part: &str| {
                stored
                    .get(&format!("phrase.{index}.{part}"))
                    .cloned()
                    .ok_or_else(|| Error::Phrase(format!("phrase {index} has no {part}")))
            };
            let pooled: Vec<f32> = get("pooled")?.to_dtype(DType::F32)?.to_vec1()?;
            let states: Vec<Vec<f32>> = get("states")?.to_dtype(DType::F32)?.to_vec2()?;
            entries.insert(name, Arc::new(PhraseFeatures::new(pooled, states)?));
        }
        Ok(Self {
            entries,
            origin: tensors.display().to_string(),
        })
    }
}

impl PhraseSource for PhraseTable {
    fn encode(&mut self, phrases: &[String]) -> Result<Vec<Arc<PhraseFeatures>>, Error> {
        phrases
            .iter()
            .map(|phrase| {
                self.entries
                    .get(phrase)
                    .cloned()
                    .ok_or_else(|| Error::Phrase(format!("phrase not in the table: {phrase}")))
            })
            .collect()
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({"kind": "table", "phrases": self.entries.len(), "origin": self.origin})
    }
}

/// The SigLIP2 text tower and its tokenizer, from the Hugging Face repository's files.
pub struct TextTower {
    tower: SiglipText,
    tokenizer: tokenizers::Tokenizer,
    pad_id: u32,
    attention_mask: bool,
    length: usize,
    limit: usize,
    cache: HashMap<String, Arc<PhraseFeatures>>,
    weights_sha256: String,
    tokenizer_sha256: String,
    /// Phrase features kept across restarts, under a directory named for everything that
    /// decides them (tower, tokenizer, lengths, mask).
    disk: Option<PathBuf>,
}

const CACHE_LIMIT: usize = 300_000;
const DISK_MAGIC: &[u8; 8] = b"IRPF1\0\0\0";

impl TextTower {
    /// `dir` holds `model.safetensors`, `tokenizer.json` and, if the repository has one,
    /// `tokenizer_config.json`. `length` is `text_length`, `limit` `text_tokens_per_phrase`.
    pub fn load(
        dir: &Path,
        length: usize,
        limit: usize,
        device: &Device,
        dtype: DType,
    ) -> Result<Self, Error> {
        let model = dir.join("model.safetensors");
        let tensors = load_text_weights(&model, "text_model", device, dtype)?;
        let release = Release {
            path: model.clone(),
            sha256: String::new(),
            format: "safetensors".into(),
            step: None,
            score: None,
            config: placeholder_config(),
        };
        let weights = Weights::from_tensors(tensors, release);
        let tower = SiglipText::load(&weights, "text_model")?;
        let mut tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| Error::Phrase(format!("tokenizer.json: {error}")))?;
        let config: serde_json::Value = match std::fs::read(dir.join("tokenizer_config.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| Error::Phrase(format!("tokenizer_config.json: {error}")))?,
            Err(_) => serde_json::Value::Null,
        };
        let pad_token = config
            .get("pad_token")
            .and_then(|v| {
                v.as_str()
                    .or_else(|| v.get("content").and_then(|c| c.as_str()))
            })
            .unwrap_or("<pad>");
        let pad_id = tokenizer.token_to_id(pad_token).unwrap_or(0);
        // Gemma's fast tokenizer returns an attention mask unless the repository says
        // otherwise, and `PhraseEncoder` passes whatever it returns to the tower.
        let attention_mask = match config.get("model_input_names").and_then(|v| v.as_array()) {
            Some(names) => names.iter().any(|name| name == "attention_mask"),
            None => true,
        };
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: length,
                ..Default::default()
            }))
            .map_err(|error| Error::Phrase(format!("tokenizer truncation: {error}")))?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::Fixed(length),
            pad_id,
            pad_token: pad_token.to_owned(),
            ..Default::default()
        }));
        Ok(Self {
            tower,
            tokenizer,
            pad_id,
            attention_mask,
            length,
            limit,
            cache: HashMap::new(),
            weights_sha256: file_sha256(&model)?,
            tokenizer_sha256: file_sha256(&dir.join("tokenizer.json"))?,
            disk: None,
        })
    }

    /// Keep phrase features under `root` across restarts: one file per phrase, named by
    /// the digest of its lowercased text, owner-only. A file that does not read back whole
    /// is ignored and recomputed.
    pub fn with_disk_cache(mut self, root: &Path) -> Result<Self, Error> {
        let mut key = Sha256::new();
        for part in [
            self.weights_sha256.as_str(),
            self.tokenizer_sha256.as_str(),
            &self.length.to_string(),
            &self.limit.to_string(),
            if self.attention_mask {
                "mask"
            } else {
                "nomask"
            },
        ] {
            key.update(part.as_bytes());
            key.update([0]);
        }
        let dir = root.join(hex::encode(key.finalize()));
        private_dir(root)?;
        private_dir(&dir)?;
        self.disk = Some(dir);
        Ok(self)
    }

    fn disk_path(&self, phrase: &str) -> Option<PathBuf> {
        Some(phrase_file(self.disk.as_ref()?, phrase))
    }

    fn read_disk(&self, phrase: &str) -> Option<PhraseFeatures> {
        let bytes = std::fs::read(self.disk_path(phrase)?).ok()?;
        decode_features(&bytes)
    }

    fn write_disk(&self, phrase: &str, features: &PhraseFeatures) -> Result<(), Error> {
        let Some(path) = self.disk_path(phrase) else {
            return Ok(());
        };
        let partial = path.with_extension(format!("{}.part", std::process::id()));
        write_private(&partial, &encode_features(features))?;
        std::fs::rename(&partial, &path)?;
        Ok(())
    }

    /// The tower on explicit ids, for tests: pooled `[N, 768]` and states `[N, T, 768]`.
    pub fn run_ids(&self, ids: &[Vec<u32>]) -> Result<(Tensor, Tensor), Error> {
        let (count, length) = (ids.len(), ids.first().map_or(0, Vec::len));
        let device = self.tower.device();
        let flat: Vec<u32> = ids.iter().flatten().copied().collect();
        let tensor = Tensor::from_vec(flat.clone(), (count, length), device)?;
        let mask = if self.attention_mask {
            let mask: Vec<u8> = flat.iter().map(|&id| u8::from(id != self.pad_id)).collect();
            Some(Tensor::from_vec(mask, (count, length), device)?)
        } else {
            None
        };
        let output = self.tower.forward(&tensor, mask.as_ref())?;
        Ok((output.pooled, output.states))
    }
}

impl PhraseSource for TextTower {
    fn encode(&mut self, phrases: &[String]) -> Result<Vec<Arc<PhraseFeatures>>, Error> {
        let mut missing: Vec<String> = Vec::new();
        for phrase in phrases {
            if !self.cache.contains_key(phrase) && !missing.contains(phrase) {
                missing.push(phrase.clone());
            }
        }
        if !missing.is_empty() && self.cache.len() + missing.len() > CACHE_LIMIT {
            self.cache.clear();
            missing.clear();
            for phrase in phrases {
                if !missing.contains(phrase) {
                    missing.push(phrase.clone());
                }
            }
        }
        if self.disk.is_some() {
            let mut still = Vec::with_capacity(missing.len());
            for phrase in missing {
                match self.read_disk(&phrase) {
                    Some(features) => {
                        self.cache.insert(phrase, Arc::new(features));
                    }
                    None => still.push(phrase),
                }
            }
            missing = still;
        }
        for chunk in missing.chunks(512) {
            let lowered: Vec<String> = chunk.iter().map(|text| text.to_lowercase()).collect();
            let encodings = self
                .tokenizer
                .encode_batch(lowered, true)
                .map_err(|error| Error::Phrase(format!("tokenize: {error}")))?;
            let ids: Vec<Vec<u32>> = encodings
                .iter()
                .map(|e| e.get_ids()[..self.length].to_vec())
                .collect();
            let (pooled, states) = self.run_ids(&ids)?;
            let pooled: Vec<Vec<f32>> = pooled.to_dtype(DType::F32)?.to_vec2()?;
            for (row, text) in chunk.iter().enumerate() {
                let count = ids[row]
                    .iter()
                    .filter(|&&id| id != self.pad_id)
                    .count()
                    .clamp(1, self.limit);
                let rows: Vec<Vec<f32>> = states
                    .get(row)?
                    .narrow(0, 0, count)?
                    .to_dtype(DType::F32)?
                    .to_vec2()?;
                let features = PhraseFeatures::new(pooled[row].clone(), rows)?;
                self.write_disk(text, &features)?;
                self.cache.insert(text.clone(), Arc::new(features));
            }
        }
        Ok(phrases
            .iter()
            .map(|phrase| self.cache[phrase].clone())
            .collect())
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({"kind": "siglip2-text-tower", "weights_sha256": self.weights_sha256,
                           "attention_mask": self.attention_mask, "cached": self.cache.len(),
                           "device": format!("{:?}", self.tower.device()),
                           "disk_cache": self.disk.is_some()})
    }
}

/// The tower sees the lowercased phrase, so differently cased phrases share one file; the
/// name is a digest, so the directory does not list what was asked.
fn phrase_file(dir: &Path, phrase: &str) -> PathBuf {
    let digest = Sha256::digest(phrase.to_lowercase().as_bytes());
    dir.join(format!("{}.f16", hex::encode(digest)))
}

/// `IRPF1`, width and row count (u32 little-endian), the pooled vector, then the token
/// states, every value as float16. Features are already rounded through float16, so the
/// round trip is exact.
fn encode_features(features: &PhraseFeatures) -> Vec<u8> {
    let width = features.pooled.len();
    let mut out = Vec::with_capacity(16 + 2 * width * (1 + features.states.len()));
    out.extend_from_slice(DISK_MAGIC);
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(features.states.len() as u32).to_le_bytes());
    for value in features
        .pooled
        .iter()
        .chain(features.states.iter().flatten())
    {
        out.extend_from_slice(&f16::from_f32(*value).to_le_bytes());
    }
    out
}

fn decode_features(bytes: &[u8]) -> Option<PhraseFeatures> {
    let header = bytes.get(..16)?;
    if &header[..8] != DISK_MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
    let rows = u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
    let body = &bytes[16..];
    if width == 0 || rows == 0 || body.len() != 2 * width * (1 + rows) {
        return None;
    }
    let values: Vec<f32> = body
        .chunks_exact(2)
        .map(|pair| f16::from_le_bytes([pair[0], pair[1]]).to_f32())
        .collect();
    let pooled = values[..width].to_vec();
    let states = values[width..]
        .chunks_exact(width)
        .map(<[f32]>::to_vec)
        .collect();
    PhraseFeatures::new(pooled, states).ok()
}

fn private_dir(dir: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    Ok(())
}

fn placeholder_config() -> crate::weights::ModelConfig {
    crate::weights::ModelConfig {
        image_size: 384,
        dimension: 768,
        heads: 12,
        feedforward: 3072,
        fusion_layers: 0,
        decoder_layers: 0,
        experts: 1,
        expert_top_k: 1,
        expert_capacity_factor: 1.0,
        queries: 0,
        max_phrases: 0,
        text_length: 64,
        text_tokens_per_phrase: 1,
        act_head: false,
        act_layers: 0,
        act_slots: 0,
        act_chunk: 1,
        siglip_repository: String::new(),
        siglip_revision: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features() -> PhraseFeatures {
        PhraseFeatures::new(
            vec![0.1, -2.5, 3.0e-3],
            vec![vec![1.0, 2.0, 3.0], vec![-0.333, 0.5, 65504.0]],
        )
        .unwrap()
    }

    #[test]
    fn features_round_trip_exactly_through_the_disk_format() {
        let original = features();
        let bytes = encode_features(&original);
        assert_eq!(bytes.len(), 16 + 2 * 3 * 3);
        assert_eq!(decode_features(&bytes), Some(original));
    }

    #[test]
    fn a_damaged_file_reads_as_missing() {
        let bytes = encode_features(&features());
        assert_eq!(decode_features(&bytes[..bytes.len() - 1]), None);
        let mut wrong = bytes.clone();
        wrong[0] = b'X';
        assert_eq!(decode_features(&wrong), None);
        assert_eq!(decode_features(&[]), None);
    }

    #[test]
    fn phrase_files_are_private_and_named_by_the_lowercased_digest() {
        let dir = std::env::temp_dir().join(format!("iris-phrases-{}", std::process::id()));
        private_dir(&dir).unwrap();
        let path = phrase_file(&dir, "Taza Roja");
        assert_eq!(path, phrase_file(&dir, "taza roja"));
        assert!(!path.to_string_lossy().contains("taza"));
        write_private(&path, &encode_features(&features())).unwrap();
        assert_eq!(
            decode_features(&std::fs::read(&path).unwrap()),
            Some(features())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
