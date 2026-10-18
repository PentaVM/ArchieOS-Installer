//! LoRA skill adapters (`archie-iris-skill-lora-v1`), read from the `.skill` zip PyTorch wrote.
//!
//! A skill package holds low-rank `(A, B)` pairs on the action head and the last fusion
//! layer's attention projections, plus the base fingerprint (`lora.base_fingerprint`) it was
//! trained against. Loading is refused unless the format tag, skill name, base fingerprint,
//! rank, alpha, target list and every tensor shape match the loaded model.

use crate::weights::Weights;
use crate::Error;
use candle_core::pickle::{Object, Stack};
use candle_core::{DType, Device, Module, Tensor};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

pub const SKILL_FORMAT: &str = "archie-iris-skill-lora-v1";
pub const DEFAULT_RANK: usize = 8;
pub const DEFAULT_ALPHA: f64 = 16.0;

#[derive(Debug, Clone)]
pub struct LoraPair {
    pub a: Tensor,
    pub b: Tensor,
}

#[derive(Debug, Clone)]
pub struct SkillAdapter {
    pub skill: String,
    pub scale: f64,
    pub pairs: HashMap<String, LoraPair>,
}

/// The loaded skill bank for an `Actor`: base fingerprint, expected `(in_features, out_features)`
/// per target layer, and loaded adapters keyed by skill name.
#[derive(Debug, Clone)]
pub struct SkillBank {
    pub base: String,
    pub targets: Vec<String>,
    pub shapes: HashMap<String, (usize, usize)>,
    pub device: Device,
    pub dtype: DType,
    pub adapters: HashMap<String, Arc<SkillAdapter>>,
}

thread_local! {
    static ACTIVE_ADAPTER: RefCell<Option<Arc<SkillAdapter>>> = const { RefCell::new(None) };
}

pub fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The ordered target layer names `LoRABank(model)` resolves for `DEFAULT_TARGETS`.
pub fn default_targets(fusion_layers: usize, act_layers: usize) -> Vec<String> {
    let mut out = Vec::new();
    if fusion_layers > 0 {
        let last = fusion_layers - 1;
        for block in ["image_self", "image_text", "text_image"] {
            for proj in ["query", "key", "value", "output"] {
                out.push(format!("fusion.{last}.{block}.{proj}"));
            }
        }
    }
    for layer in 0..act_layers {
        out.push(format!("act.memory.layers.{layer}.self_attn.out_proj"));
        out.push(format!("act.memory.layers.{layer}.linear1"));
        out.push(format!("act.memory.layers.{layer}.linear2"));
    }
    for head in [
        "step_type",
        "keys",
        "buttons",
        "mouse_x",
        "mouse_y",
        "scroll",
        "pad_axes",
        "pad_buttons",
        "pointer_query",
        "pointer_key",
        "pointer_refine",
    ] {
        out.push(format!("act.{head}"));
    }
    out
}

impl SkillBank {
    pub fn from_weights(weights: &Weights, device: Device, dtype: DType) -> Result<Self, Error> {
        let config = &weights.release.config;
        let targets = default_targets(config.fusion_layers, config.act_layers);
        let mut shapes = HashMap::with_capacity(targets.len());
        for name in &targets {
            let weight = weights.get(&format!("{name}.weight"))?;
            let (out_features, in_features) = weight.dims2()?;
            shapes.insert(name.clone(), (in_features, out_features));
        }
        Ok(Self {
            base: weights.base_fingerprint()?,
            targets,
            shapes,
            device,
            dtype,
            adapters: HashMap::new(),
        })
    }

    pub fn use_skill(&mut self, name: &str, directory: &Path) -> Result<(), Error> {
        if !valid_skill_name(name) {
            return Err(Error::Refused("skill_name_invalid".into()));
        }
        if self.adapters.contains_key(name) {
            return Ok(());
        }
        let direct = directory.join(format!("{name}.skill"));
        let pinned = directory.join(&self.base).join(format!("{name}.skill"));
        let path = if direct.is_file() {
            direct
        } else if pinned.is_file() {
            pinned
        } else {
            return Err(Error::Refused(format!("skill_not_found:{name}")));
        };
        let adapter = self.load_skill_file(&path, Some(name))?;
        self.adapters.insert(name.to_owned(), Arc::new(adapter));
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<SkillAdapter>> {
        self.adapters.get(name).cloned()
    }

    pub fn load_skill_file(
        &self,
        path: &Path,
        expected_skill: Option<&str>,
    ) -> Result<SkillAdapter, Error> {
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(BufReader::new(file))
            .map_err(|e| Error::Refused(format!("lora_skill_not_zip:{e}")))?;
        let pickle_name = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_owned()))
            .find(|n| n.ends_with("/data.pkl") || n == "data.pkl")
            .ok_or_else(|| Error::Refused("lora_skill_no_pickle".into()))?;
        let root = pickle_name.trim_end_matches("data.pkl").to_owned();
        let object = {
            let entry = zip
                .by_name(&pickle_name)
                .map_err(|e| Error::Refused(e.to_string()))?;
            let mut reader = BufReader::new(entry);
            let mut stack = Stack::empty();
            stack
                .read_loop(&mut reader)
                .map_err(|e| Error::Refused(e.to_string()))?;
            stack
                .finalize()
                .map_err(|e| Error::Refused(e.to_string()))?
        };
        let Object::Dict(top) = object else {
            return Err(Error::Refused("lora_skill_format_unknown".into()));
        };
        let field = |key: &str| {
            top.iter()
                .find(|(k, _)| matches!(k, Object::Unicode(t) if t == key))
                .map(|(_, v)| v.clone())
        };
        let format = match field("format") {
            Some(Object::Unicode(text)) => text,
            _ => return Err(Error::Refused("lora_skill_format_unknown".into())),
        };
        if format != SKILL_FORMAT {
            return Err(Error::Refused("lora_skill_format_unknown".into()));
        }
        let skill = match field("skill") {
            Some(Object::Unicode(text)) => text,
            _ => return Err(Error::Refused("lora_skill_name_mismatch".into())),
        };
        if let Some(expected) = expected_skill {
            if skill != expected {
                return Err(Error::Refused("lora_skill_name_mismatch".into()));
            }
        }
        let base = match field("base") {
            Some(Object::Unicode(text)) => text,
            _ => return Err(Error::Refused("lora_skill_trained_on_another_base".into())),
        };
        if base != self.base {
            return Err(Error::Refused("lora_skill_trained_on_another_base".into()));
        }
        let rank = match field("rank") {
            Some(Object::Int(v)) if v > 0 => v as usize,
            Some(Object::Long(v)) if v > 0 => v as usize,
            _ => return Err(Error::Refused("lora_skill_layout_differs".into())),
        };
        let alpha = match field("alpha") {
            Some(Object::Float(v)) => v,
            Some(Object::Int(v)) => f64::from(v),
            Some(Object::Long(v)) => v as f64,
            _ => return Err(Error::Refused("lora_skill_layout_differs".into())),
        };
        let targets: Vec<String> = match field("targets") {
            Some(Object::List(items) | Object::Tuple(items)) => items
                .into_iter()
                .map(|item| match item {
                    Object::Unicode(s) => Ok(s),
                    _ => Err(Error::Refused("lora_skill_layout_differs".into())),
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(Error::Refused("lora_skill_layout_differs".into())),
        };
        if rank != DEFAULT_RANK || (alpha - DEFAULT_ALPHA).abs() > 1e-6 || targets != self.targets {
            return Err(Error::Refused("lora_skill_layout_differs".into()));
        }
        let state = match field("state") {
            Some(Object::Dict(entries)) => entries,
            _ => return Err(Error::Refused("lora_skill_layers_do_not_match".into())),
        };
        let dir = std::path::PathBuf::from(format!("{root}data"));
        let mut raw_tensors = HashMap::with_capacity(state.len());
        for (k, v) in state {
            let Object::Unicode(name) = k.clone() else {
                return Err(Error::Refused("lora_skill_layers_do_not_match".into()));
            };
            let Some(info) = v
                .into_tensor_info(k, &dir)
                .map_err(|e| Error::Refused(e.to_string()))?
            else {
                return Err(Error::Refused("lora_skill_layers_do_not_match".into()));
            };
            if !info.layout.is_contiguous() {
                return Err(Error::Refused(format!("lora_skill_non_contiguous:{name}")));
            }
            let mut entry = zip
                .by_name(&info.path)
                .map_err(|e| Error::Refused(e.to_string()))?;
            let skip = (info.layout.start_offset() * info.dtype.size_in_bytes()) as u64;
            if skip > 0 {
                std::io::copy(&mut entry.by_ref().take(skip), &mut std::io::sink())?;
            }
            let mut bytes =
                vec![0u8; info.layout.shape().elem_count() * info.dtype.size_in_bytes()];
            entry.read_exact(&mut bytes)?;
            let tensor =
                Tensor::from_raw_buffer(&bytes, info.dtype, info.layout.dims(), &Device::Cpu)?
                    .to_dtype(self.dtype)?
                    .to_device(&self.device)?;
            raw_tensors.insert(name, tensor);
        }
        if raw_tensors.len() != self.targets.len() * 2 {
            return Err(Error::Refused("lora_skill_layers_do_not_match".into()));
        }
        let mut pairs = HashMap::with_capacity(self.targets.len());
        for target in &self.targets {
            let &(in_features, out_features) = self
                .shapes
                .get(target)
                .ok_or_else(|| Error::Refused("lora_skill_layers_do_not_match".into()))?;
            let key = target.replace('.', "__");
            let a_key = format!("{key}__a");
            let b_key = format!("{key}__b");
            let a = raw_tensors
                .remove(&a_key)
                .ok_or_else(|| Error::Refused("lora_skill_layers_do_not_match".into()))?;
            let b = raw_tensors
                .remove(&b_key)
                .ok_or_else(|| Error::Refused("lora_skill_layers_do_not_match".into()))?;
            if a.dims() != [rank, in_features] {
                return Err(Error::Refused(format!("lora_skill_shape_mismatch:{a_key}")));
            }
            if b.dims() != [out_features, rank] {
                return Err(Error::Refused(format!("lora_skill_shape_mismatch:{b_key}")));
            }
            pairs.insert(target.clone(), LoraPair { a, b });
        }
        Ok(SkillAdapter {
            skill,
            scale: alpha / rank as f64,
            pairs,
        })
    }
}

/// Runs `f` with `adapter` active on the current thread for `Linear::forward` hooks.
pub fn with_adapter<T>(
    adapter: Option<Arc<SkillAdapter>>,
    f: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    struct Reset(Option<Arc<SkillAdapter>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let prev = self.0.take();
            ACTIVE_ADAPTER.with(|slot| *slot.borrow_mut() = prev);
        }
    }
    let previous = ACTIVE_ADAPTER.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), adapter));
    let _guard = Reset(previous);
    f()
}

/// Computes `(x @ A.T) @ B.T * scale` when an adapter is active and targets `layer_name`.
pub fn delta_for(layer_name: &str, x: &Tensor, out_dtype: DType) -> Result<Option<Tensor>, Error> {
    let active = ACTIVE_ADAPTER.with(|slot| slot.borrow().clone());
    let Some(adapter) = active else {
        return Ok(None);
    };
    let Some(pair) = adapter.pairs.get(layer_name) else {
        return Ok(None);
    };
    let x_cast = x.to_dtype(pair.a.dtype())?;
    let low_rank = candle_nn::Linear::new(pair.a.clone(), None).forward(&x_cast)?;
    let projected = candle_nn::Linear::new(pair.b.clone(), None).forward(&low_rank)?;
    let scaled = (projected * adapter.scale)?.to_dtype(out_dtype)?;
    Ok(Some(scaled))
}
