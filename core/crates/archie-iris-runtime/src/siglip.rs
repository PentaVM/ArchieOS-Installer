//! SigLIP2 ViT-B/16, both towers, as Hugging Face's `SiglipModel` runs them.
//!
//! The vision tower is fine-tuned and ships in the IRIS release (`siglip.model.*`). Its
//! 24 x 24 position table is resized for larger canvases exactly as `interpolate_pos_encoding`
//! does: bicubic (A = -0.75), `align_corners=False`, edges clamped.
//!
//! The text tower is frozen and is *not* in the release: `beta_model.PhraseEncoder` fetches
//! it from `google/siglip2-base-patch16-384` at a pinned revision. Here it is read from that
//! repository's `model.safetensors` on disk (`text_model.*`), with its `tokenizer.json`.

use crate::nn::{attend, merge_heads, split_heads, LayerNorm, Linear, Mlp, Result};
use crate::vjepa::patchify;
use crate::weights::Weights;
use crate::Error;
use candle_core::{DType, Device, Tensor};

const WIDTH: usize = 768;
const HEADS: usize = 12;
const LAYERS: usize = 12;
const PATCH: usize = 16;
const EPS: f64 = 1e-6;

struct EncoderLayer {
    norm1: LayerNorm,
    q: Linear,
    k: Linear,
    v: Linear,
    out: Linear,
    norm2: LayerNorm,
    mlp: Mlp,
}

impl EncoderLayer {
    fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        Ok(Self {
            norm1: LayerNorm::load(weights, &format!("{prefix}.layer_norm1"), EPS)?,
            q: Linear::load(weights, &format!("{prefix}.self_attn.q_proj"))?,
            k: Linear::load(weights, &format!("{prefix}.self_attn.k_proj"))?,
            v: Linear::load(weights, &format!("{prefix}.self_attn.v_proj"))?,
            out: Linear::load(weights, &format!("{prefix}.self_attn.out_proj"))?,
            norm2: LayerNorm::load(weights, &format!("{prefix}.layer_norm2"), EPS)?,
            // `gelu_pytorch_tanh`.
            mlp: Mlp::load(
                weights,
                &format!("{prefix}.mlp.fc1"),
                &format!("{prefix}.mlp.fc2"),
                false,
            )?,
        })
    }

    /// `allowed`: `[B, 1, 1, S]`, 1 where a key may be attended.
    fn forward(&self, x: &Tensor, allowed: Option<&Tensor>) -> Result<Tensor> {
        let h = self.norm1.forward(x)?;
        let q = split_heads(&self.q.forward(&h)?, HEADS)?;
        let k = split_heads(&self.k.forward(&h)?, HEADS)?;
        let v = split_heads(&self.v.forward(&h)?, HEADS)?;
        let attended = self
            .out
            .forward(&merge_heads(&attend(&q, &k, &v, allowed)?)?)?;
        let x = (x + attended)?;
        Ok((&x + self.mlp.forward(&self.norm2.forward(&x)?)?)?)
    }
}

pub struct SiglipVision {
    patch_weight: Tensor,
    patch_bias: Tensor,
    positions: Tensor,
    layers: Vec<EncoderLayer>,
    post: LayerNorm,
}

impl SiglipVision {
    /// `grid` is the canvas in patches per side (32 for a 512 canvas).
    pub fn load(weights: &Weights, grid: usize) -> Result<Self> {
        let prefix = "siglip.model";
        let patch = weights.get(&format!("{prefix}.embeddings.patch_embedding.weight"))?;
        let table = weights.get(&format!("{prefix}.embeddings.position_embedding.weight"))?;
        let positions = interpolate_positions(&table, grid)?;
        let layers = (0..LAYERS)
            .map(|index| EncoderLayer::load(weights, &format!("{prefix}.encoder.layers.{index}")))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            patch_weight: patch.reshape((WIDTH, 3 * PATCH * PATCH))?,
            patch_bias: weights.get(&format!("{prefix}.embeddings.patch_embedding.bias"))?,
            positions,
            layers,
            post: LayerNorm::load(weights, &format!("{prefix}.post_layernorm"), EPS)?,
        })
    }

    /// `images`: `[B, 3, H, W]` in [0, 1]; returns `last_hidden_state`, `[B, N, 768]`.
    pub fn forward(&self, images: &Tensor) -> Result<Tensor> {
        let dtype = self.patch_weight.dtype();
        let normalised = ((images.to_dtype(DType::F32)? - 0.5)? / 0.5)?.to_dtype(dtype)?;
        let mut x = patchify(&normalised, PATCH)?
            .broadcast_matmul(&self.patch_weight.t()?)?
            .broadcast_add(&self.patch_bias)?
            .broadcast_add(&self.positions)?;
        for layer in &self.layers {
            x = layer.forward(&x, None)?;
        }
        self.post.forward(&x)
    }
}

/// `torch.nn.functional.interpolate(mode="bicubic", align_corners=False)` of a square
/// `[side*side, D]` table to `[1, grid*grid, D]`. Computed once, in f64, on the host.
fn interpolate_positions(table: &Tensor, grid: usize) -> Result<Tensor> {
    let (count, width) = table.dims2()?;
    let side = (count as f64).sqrt() as usize;
    if side * side != count {
        return Err(Error::Weights("SigLIP position table is not square".into()));
    }
    let device = table.device().clone();
    let dtype = table.dtype();
    if side == grid {
        return Ok(table.unsqueeze(0)?);
    }
    let values: Vec<f32> = table.to_dtype(DType::F32)?.flatten_all()?.to_vec1()?;
    let taps = |out: usize| -> Vec<([usize; 4], [f64; 4])> {
        let scale = side as f64 / grid as f64;
        (0..out)
            .map(|index| {
                let real = scale * (index as f64 + 0.5) - 0.5;
                let floor = real.floor();
                let t = real - floor;
                let a = -0.75;
                let near = |x: f64| ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0;
                let far = |x: f64| ((a * x - 5.0 * a) * x + 8.0 * a) * x - 4.0 * a;
                let weights = [far(t + 1.0), near(t), near(1.0 - t), far(2.0 - t)];
                let clamp =
                    |offset: i64| (floor as i64 - 1 + offset).clamp(0, side as i64 - 1) as usize;
                ([clamp(0), clamp(1), clamp(2), clamp(3)], weights)
            })
            .collect()
    };
    let rows = taps(grid);
    let cols = taps(grid);
    let mut out = vec![0f32; grid * grid * width];
    for (y, (row_index, row_weight)) in rows.iter().enumerate() {
        for (x, (col_index, col_weight)) in cols.iter().enumerate() {
            for channel in 0..width {
                let mut total = 0.0f64;
                for i in 0..4 {
                    let mut line = 0.0f64;
                    for j in 0..4 {
                        line += col_weight[j]
                            * f64::from(
                                values[(row_index[i] * side + col_index[j]) * width + channel],
                            );
                    }
                    total += row_weight[i] * line;
                }
                out[(y * grid + x) * width + channel] = total as f32;
            }
        }
    }
    Ok(Tensor::from_vec(out, (1, grid * grid, width), &device)?.to_dtype(dtype)?)
}

/// The frozen text tower: token and position embeddings, twelve layers, the final norm and
/// the head that makes the pooled vector from the last position.
pub struct SiglipText {
    tokens: Tensor,
    positions: Tensor,
    layers: Vec<EncoderLayer>,
    norm: LayerNorm,
    head: Linear,
}

/// What the tower returns for a batch: the pooled vector `[N, 768]` and every position's
/// state `[N, T, 768]`.
pub struct TextOutput {
    pub pooled: Tensor,
    pub states: Tensor,
}

impl SiglipText {
    /// `weights` holds the tower under `prefix` (`text_model` in the Hugging Face file).
    pub fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        let layers = (0..LAYERS)
            .map(|index| EncoderLayer::load(weights, &format!("{prefix}.encoder.layers.{index}")))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tokens: weights.get(&format!("{prefix}.embeddings.token_embedding.weight"))?,
            positions: weights.get(&format!("{prefix}.embeddings.position_embedding.weight"))?,
            layers,
            norm: LayerNorm::load(weights, &format!("{prefix}.final_layer_norm"), EPS)?,
            head: Linear::load(weights, &format!("{prefix}.head"))?,
        })
    }

    /// `ids`: `[N, T]` u32. `attention`: `[N, T]` u8, 1 for a real token, or `None` when the
    /// tokenizer does not produce a mask (SigLIP attends to padding then).
    pub fn forward(&self, ids: &Tensor, attention: Option<&Tensor>) -> Result<TextOutput> {
        let (count, length) = ids.dims2()?;
        let embedded = self
            .tokens
            .index_select(&ids.flatten_all()?, 0)?
            .reshape((count, length, WIDTH))?;
        let mut x = embedded.broadcast_add(&self.positions.narrow(0, 0, length)?.unsqueeze(0)?)?;
        let allowed = match attention {
            None => None,
            Some(mask) => Some(mask.reshape((count, 1, 1, length))?),
        };
        for layer in &self.layers {
            x = layer.forward(&x, allowed.as_ref())?;
        }
        let states = self.norm.forward(&x)?;
        // The last position is a strided view; cuBLAS refuses it for two or more phrases.
        let last = states.narrow(1, length - 1, 1)?.squeeze(1)?.contiguous()?;
        Ok(TextOutput {
            pooled: self.head.forward(&last)?,
            states,
        })
    }

    pub fn device(&self) -> &Device {
        self.tokens.device()
    }
}

/// Reads a safetensors file into a [`Weights`] holding only tensors under `prefix`.
pub fn load_text_weights(
    path: &std::path::Path,
    prefix: &str,
    device: &Device,
    dtype: DType,
) -> Result<std::collections::HashMap<String, Tensor>> {
    let all = candle_core::safetensors::load(path, device)?;
    let mut kept = std::collections::HashMap::new();
    for (name, tensor) in all {
        if name.starts_with(prefix) {
            let tensor = if tensor.dtype().is_float() {
                tensor.to_dtype(dtype)?
            } else {
                tensor
            };
            kept.insert(name, tensor);
        }
    }
    if kept.is_empty() {
        return Err(Error::Weights(format!(
            "{} holds no {prefix}.* tensors",
            path.display()
        )));
    }
    Ok(kept)
}
