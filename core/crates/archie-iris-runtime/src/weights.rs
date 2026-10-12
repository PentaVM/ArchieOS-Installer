//! The released weights, read straight from the `.pt` PyTorch wrote.
//!
//! A release file is a zip holding one pickle (`data.pkl`) and one raw storage per tensor.
//! The pickle carries the format tag, the step, the score, the model configuration and the
//! state dict under `model`. Nothing here runs pickled code: the pickle is walked as data
//! by candle's reader, and anything that is not a dict, a number, a string or a tensor
//! reference is refused.

use crate::Error;
use candle_core::pickle::{Object, Stack};
use candle_core::{DType, Device, Tensor};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// The formats `iris_infer.from_weights` accepts.
const FORMATS: [&str; 2] = [
    "archie-iris-beta1-weights-v1",
    "archie-iris-beta1-checkpoint-v1",
];

/// `Beta1ModelConfig`, the fields inference reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelConfig {
    pub image_size: usize,
    pub dimension: usize,
    pub heads: usize,
    pub feedforward: usize,
    pub fusion_layers: usize,
    pub decoder_layers: usize,
    pub experts: usize,
    pub expert_top_k: usize,
    pub expert_capacity_factor: f64,
    pub queries: usize,
    pub max_phrases: usize,
    pub text_length: usize,
    pub text_tokens_per_phrase: usize,
    pub act_head: bool,
    pub act_layers: usize,
    pub act_slots: usize,
    pub act_chunk: usize,
    pub siglip_repository: String,
    pub siglip_revision: String,
}

/// What the file says about itself, beside the tensors.
#[derive(Debug, Clone)]
pub struct Release {
    pub path: PathBuf,
    pub sha256: String,
    pub format: String,
    pub step: Option<i64>,
    pub score: Option<f64>,
    pub config: ModelConfig,
}

/// Every tensor of the state dict, converted to the dtype inference runs in.
pub struct Weights {
    tensors: HashMap<String, Tensor>,
    pub release: Release,
}

impl Weights {
    /// Read a release. `expected_sha256`, when given, must match the file before anything
    /// in it is parsed.
    pub fn load(
        path: &Path,
        expected_sha256: Option<&str>,
        device: &Device,
        dtype: DType,
    ) -> Result<Self, Error> {
        let sha256 = file_sha256(path)?;
        if let Some(expected) = expected_sha256 {
            if !expected.eq_ignore_ascii_case(&sha256) {
                return Err(Error::Weights(format!(
                    "sha256 {sha256} does not match the expected {expected}"
                )));
            }
        }
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(BufReader::new(file))
            .map_err(|error| Error::Weights(format!("not a PyTorch zip: {error}")))?;
        let pickle_name = (0..zip.len())
            .filter_map(|index| {
                zip.by_index(index)
                    .ok()
                    .map(|entry| entry.name().to_owned())
            })
            .find(|name| name.ends_with("/data.pkl") || name == "data.pkl")
            .ok_or_else(|| Error::Weights("no data.pkl".into()))?;
        let root = pickle_name.trim_end_matches("data.pkl").to_owned();
        let object = {
            let entry = zip
                .by_name(&pickle_name)
                .map_err(|error| Error::Weights(error.to_string()))?;
            let mut reader = BufReader::new(entry);
            let mut stack = Stack::empty();
            stack.read_loop(&mut reader)?;
            stack.finalize()?
        };
        let Object::Dict(top) = object else {
            return Err(Error::Weights("the pickle is not a dict".into()));
        };
        let field = |name: &str| {
            top.iter()
                .find(|(key, _)| matches!(key, Object::Unicode(text) if text == name))
                .map(|(_, value)| value.clone())
        };
        let format = match field("format") {
            Some(Object::Unicode(text)) => text,
            _ => return Err(Error::Weights("no format tag".into())),
        };
        if !FORMATS.contains(&format.as_str()) {
            return Err(Error::Weights(format!("unknown format {format}")));
        }
        let step = field("step")
            .or_else(|| field("global_step"))
            .and_then(|value| as_i64(&value));
        let score = field("score").and_then(|value| as_f64(&value));
        let config = match field("model_config") {
            Some(Object::Dict(entries)) => parse_config(&entries)?,
            _ => return Err(Error::Weights("no model_config".into())),
        };
        let state = match field("model") {
            Some(Object::Dict(entries)) => entries,
            _ => return Err(Error::Weights("no model state dict".into())),
        };

        let dir = PathBuf::from(format!("{root}data"));
        let mut tensors = HashMap::with_capacity(state.len());
        for (key, value) in state {
            let Object::Unicode(name) = key.clone() else {
                continue;
            };
            let Some(info) = value.into_tensor_info(key, &dir)? else {
                return Err(Error::Weights(format!("{name} is not a tensor")));
            };
            if !info.layout.is_contiguous() {
                return Err(Error::Weights(format!("{name} is not contiguous")));
            }
            let mut entry = zip
                .by_name(&info.path)
                .map_err(|error| Error::Weights(format!("{name}: {error}")))?;
            let skip = (info.layout.start_offset() * info.dtype.size_in_bytes()) as u64;
            if skip > 0 {
                std::io::copy(&mut entry.by_ref().take(skip), &mut std::io::sink())?;
            }
            let mut bytes =
                vec![0u8; info.layout.shape().elem_count() * info.dtype.size_in_bytes()];
            entry.read_exact(&mut bytes)?;
            let tensor =
                Tensor::from_raw_buffer(&bytes, info.dtype, info.layout.dims(), &Device::Cpu)?;
            let tensor = if tensor.dtype().is_float() {
                tensor.to_dtype(dtype)?
            } else {
                tensor
            };
            tensors.insert(name, tensor.to_device(device)?);
        }
        Ok(Self {
            tensors,
            release: Release {
                path: path.to_owned(),
                sha256,
                format,
                step,
                score,
                config,
            },
        })
    }

    /// Weights from tensors already in memory: tests, and the text tower's own file.
    pub fn from_tensors(tensors: HashMap<String, Tensor>, release: Release) -> Self {
        Self { tensors, release }
    }

    pub fn get(&self, name: &str) -> Result<Tensor, Error> {
        self.tensors
            .get(name)
            .cloned()
            .ok_or_else(|| Error::MissingTensor(name.to_owned()))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tensors.contains_key(name)
    }

    pub fn parameter_count(&self) -> usize {
        self.tensors.values().map(Tensor::elem_count).sum()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.tensors.keys()
    }
}

pub(crate) fn file_sha256(path: &Path) -> Result<String, Error> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 16 * 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn as_i64(value: &Object) -> Option<i64> {
    match value {
        Object::Int(value) => Some(i64::from(*value)),
        Object::Long(value) => Some(*value),
        _ => None,
    }
}

fn as_f64(value: &Object) -> Option<f64> {
    match value {
        Object::Float(value) => Some(*value),
        other => as_i64(other).map(|value| value as f64),
    }
}

fn parse_config(entries: &[(Object, Object)]) -> Result<ModelConfig, Error> {
    let get = |name: &str| {
        entries
            .iter()
            .find(|(key, _)| matches!(key, Object::Unicode(text) if text == name))
            .map(|(_, value)| value)
    };
    let int = |name: &str, default: usize| -> Result<usize, Error> {
        match get(name) {
            None => Ok(default),
            Some(value) => as_i64(value)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| Error::Weights(format!("model_config.{name} is not a count"))),
        }
    };
    let flag = |name: &str| matches!(get(name), Some(Object::Bool(true)));
    let text = |name: &str| match get(name) {
        Some(Object::Unicode(value)) => value.clone(),
        _ => String::new(),
    };
    if text("backbone") != "dual" {
        return Err(Error::Weights(
            "only the dual V-JEPA + SigLIP2 backbone is served".into(),
        ));
    }
    let config = ModelConfig {
        image_size: int("image_size", 384)?,
        dimension: int("dimension", 512)?,
        heads: int("heads", 8)?,
        feedforward: int("feedforward", 2048)?,
        fusion_layers: int("fusion_layers", 6)?,
        decoder_layers: int("decoder_layers", 6)?,
        experts: int("experts", 8)?,
        expert_top_k: int("expert_top_k", 2)?,
        expert_capacity_factor: get("expert_capacity_factor")
            .and_then(as_f64)
            .unwrap_or(1.5),
        queries: int("queries", 100)?,
        max_phrases: int("max_phrases", 16)?,
        text_length: int("text_length", 64)?,
        text_tokens_per_phrase: int("text_tokens_per_phrase", 1)?,
        act_head: flag("act_head"),
        act_layers: int("act_layers", 4)?,
        act_slots: int("act_slots", 8)?,
        act_chunk: int("act_chunk", 1)?,
        siglip_repository: text("siglip_repository"),
        siglip_revision: text("siglip_revision"),
    };
    if !config.image_size.is_multiple_of(16)
        || !config.dimension.is_multiple_of(config.heads)
        || config.expert_top_k == 0
        || config.expert_top_k > config.experts
        || config.text_tokens_per_phrase == 0
    {
        return Err(Error::Weights("model_config is not valid".into()));
    }
    Ok(config)
}
