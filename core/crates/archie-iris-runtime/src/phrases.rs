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
use std::collections::HashMap;
use std::path::Path;
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
}

const CACHE_LIMIT: usize = 300_000;

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
        })
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
                self.cache.insert(
                    text.clone(),
                    Arc::new(PhraseFeatures::new(pooled[row].clone(), rows)?),
                );
            }
        }
        Ok(phrases
            .iter()
            .map(|phrase| self.cache[phrase].clone())
            .collect())
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({"kind": "siglip2-text-tower", "weights_sha256": self.weights_sha256,
                           "attention_mask": self.attention_mask, "cached": self.cache.len()})
    }
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
