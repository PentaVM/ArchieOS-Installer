//! V-JEPA 2.1 ViT-B/16 as IRIS runs it: one still frame through the image patch embedding.
//!
//! Ported from `facebookresearch/vjepa2` at `204698b` (`app/vjepa_2_1/models`), the revision
//! `beta_model.VJEPA21_REVISION` pins. The encoder has no position table: every block rotates
//! queries and keys by the token's depth, row and column (3-D RoPE, 20 of each head's 64
//! dimensions per axis, the last 4 untouched), and with `interpolate_rope` the row and column
//! are rescaled onto the 16-cell grid it was pretrained on. A single frame enters through
//! `patch_embed_img` (tubelet 1) with the image modality embedding added.

use crate::nn::{attend, merge_heads, LayerNorm, Linear, Mlp, Result};
use crate::weights::Weights;
use candle_core::{DType, Device, Tensor, D};

const PREFIX: &str = "vjepa.encoder";
const DEPTH: usize = 12;
const WIDTH: usize = 768;
const HEADS: usize = 12;
const PATCH: usize = 16;
const PRETRAINED_GRID: f32 = 16.0;
const EPS: f64 = 1e-6;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

struct Block {
    norm1: LayerNorm,
    qkv: Linear,
    proj: Linear,
    norm2: LayerNorm,
    mlp: Mlp,
}

pub struct VJepa {
    patch_weight: Tensor,
    patch_bias: Tensor,
    modality: Tensor,
    blocks: Vec<Block>,
    norm: LayerNorm,
}

/// Per-token cosine and sine for the whole head, `[N, 64]` each.
pub struct Rope {
    cos: Tensor,
    sin: Tensor,
}

impl Rope {
    /// `rotate_queries_or_keys` over the three axes, for a single frame of `rows` x `cols`
    /// patches.
    pub fn new(rows: usize, cols: usize, device: &Device, dtype: DType) -> Result<Self> {
        let head = WIDTH / HEADS;
        let axis = 2 * ((head / 3) / 2);
        let half = axis / 2;
        let omega: Vec<f32> = (0..half)
            .map(|j| 1.0 / 10000f32.powf(j as f32 / (axis as f32 / 2.0)))
            .collect();
        let count = rows * cols;
        let mut cos = vec![1f32; count * head];
        let mut sin = vec![0f32; count * head];
        for id in 0..count {
            let height = (id / cols) as f32;
            let width = (id % cols) as f32;
            // Depth is 0 for a still frame; rows and columns land on the pretrained grid.
            let positions = [
                0.0,
                if rows > 1 {
                    height * (PRETRAINED_GRID - 1.0) / (rows as f32 - 1.0)
                } else {
                    0.0
                },
                if cols > 1 {
                    width * (PRETRAINED_GRID - 1.0) / (cols as f32 - 1.0)
                } else {
                    0.0
                },
            ];
            for (block, &position) in positions.iter().enumerate() {
                for (j, &frequency) in omega.iter().enumerate() {
                    let angle = position * frequency;
                    for offset in 0..2 {
                        let index = id * head + block * axis + 2 * j + offset;
                        cos[index] = angle.cos();
                        sin[index] = angle.sin();
                    }
                }
            }
        }
        Ok(Self {
            cos: Tensor::from_vec(cos, (1, 1, count, head), device)?.to_dtype(dtype)?,
            sin: Tensor::from_vec(sin, (1, 1, count, head), device)?.to_dtype(dtype)?,
        })
    }

    /// `x * cos + rotate_half_pairs(x) * sin`, pairs `(a, b)` rotated to `(-b, a)`.
    pub fn apply(&self, x: &Tensor) -> Result<Tensor> {
        let (batch, heads, length, width) = x.dims4()?;
        let pairs = x.reshape((batch, heads, length, width / 2, 2))?;
        let first = pairs.narrow(D::Minus1, 0, 1)?;
        let second = pairs.narrow(D::Minus1, 1, 1)?;
        let rotated = Tensor::cat(&[&second.neg()?, &first], D::Minus1)?
            .reshape((batch, heads, length, width))?;
        Ok((x.broadcast_mul(&self.cos)? + rotated.broadcast_mul(&self.sin)?)?)
    }
}

impl VJepa {
    pub fn load(weights: &Weights) -> Result<Self> {
        let patch = weights.get(&format!("{PREFIX}.patch_embed_img.proj.weight"))?; // [768, 3, 1, 16, 16]
        let mut blocks = Vec::with_capacity(DEPTH);
        for index in 0..DEPTH {
            let block = format!("{PREFIX}.blocks.{index}");
            blocks.push(Block {
                norm1: LayerNorm::load(weights, &format!("{block}.norm1"), EPS)?,
                qkv: Linear::load(weights, &format!("{block}.attn.qkv"))?,
                proj: Linear::load(weights, &format!("{block}.attn.proj"))?,
                norm2: LayerNorm::load(weights, &format!("{block}.norm2"), EPS)?,
                mlp: Mlp::load(
                    weights,
                    &format!("{block}.mlp.fc1"),
                    &format!("{block}.mlp.fc2"),
                    true,
                )?,
            });
        }
        Ok(Self {
            patch_weight: patch.reshape((WIDTH, 3 * PATCH * PATCH))?,
            patch_bias: weights.get(&format!("{PREFIX}.patch_embed_img.proj.bias"))?,
            modality: weights
                .get(&format!("{PREFIX}.img_mod_embed"))?
                .reshape((1, 1, WIDTH))?,
            blocks,
            // Inference returns the last hierarchical layer through its own norm.
            norm: LayerNorm::load(weights, &format!("{PREFIX}.norms_block.3"), EPS)?,
        })
    }

    /// `images`: `[B, 3, H, W]` in [0, 1]. Returns `[B, (H/16)(W/16), 768]`.
    pub fn forward(&self, images: &Tensor, rope: &Rope) -> Result<Tensor> {
        let device = images.device();
        let dtype = self.patch_weight.dtype();
        let mean = Tensor::new(&MEAN, device)?.reshape((1, 3, 1, 1))?;
        let std = Tensor::new(&STD, device)?.reshape((1, 3, 1, 1))?;
        let normalised = images
            .to_dtype(DType::F32)?
            .broadcast_sub(&mean)?
            .broadcast_div(&std)?
            .to_dtype(dtype)?;
        let mut x = patchify(&normalised, PATCH)?
            .broadcast_matmul(&self.patch_weight.t()?)?
            .broadcast_add(&self.patch_bias)?
            .broadcast_add(&self.modality)?;
        for block in &self.blocks {
            let (batch, length, _) = x.dims3()?;
            let qkv = block
                .qkv
                .forward(&block.norm1.forward(&x)?)?
                .reshape((batch, length, 3, HEADS, WIDTH / HEADS))?
                .permute((2, 0, 3, 1, 4))?;
            let q = rope.apply(&qkv.get(0)?.contiguous()?)?;
            let k = rope.apply(&qkv.get(1)?.contiguous()?)?;
            let v = qkv.get(2)?.contiguous()?;
            let attended = merge_heads(&attend(&q, &k, &v, None)?)?;
            x = (x + block.proj.forward(&attended)?)?;
            x = (&x + block.mlp.forward(&block.norm2.forward(&x)?)?)?;
        }
        self.norm.forward(&x)
    }
}

/// `[B, C, H, W]` to `[B, (H/p)(W/p), C*p*p]`, each patch flattened channel, row, column:
/// the layout a `Conv2d(kernel=stride=p)` weight is reshaped to.
pub fn patchify(images: &Tensor, patch: usize) -> Result<Tensor> {
    let (batch, channels, height, width) = images.dims4()?;
    let (rows, cols) = (height / patch, width / patch);
    Ok(images
        .reshape((batch, channels, rows, patch, cols, patch))?
        .permute((0, 2, 4, 1, 3, 5))?
        .contiguous()?
        .reshape((batch, rows * cols, channels * patch * patch))?)
}
