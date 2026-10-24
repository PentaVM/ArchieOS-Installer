//! Building blocks shared by the tokenizer and the generator.
//!
//! Every linear layer can carry a low-rank adapter. An adapter is attached after the base is
//! built, from its own variables, and starts at exactly zero (its `B` matrix is zero), so a
//! freshly attached adapter changes nothing until it is trained. The base weights are never
//! touched by adapter training: they come from a frozen checkpoint as plain tensors and no
//! gradient is ever computed for them.

use crate::config::{LoraConfig, LoraTarget};
use candle_core::{DType, Device, Result, Tensor, D};
use candle_nn::{Init, VarBuilder};

/// Large enough that `exp` of it is exactly zero in f32, small enough to never overflow
/// when two masks are added.
pub const MASKED: f32 = -1.0e9;

/// Whether a forward pass is for training (dropout on) or for use.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub train: bool,
    pub dropout: f32,
}

impl Ctx {
    pub const EVAL: Ctx = Ctx {
        train: false,
        dropout: 0.0,
    };
    pub fn train(dropout: f32) -> Self {
        Self {
            train: true,
            dropout,
        }
    }
    pub fn drop(&self, x: &Tensor) -> Result<Tensor> {
        if self.train && self.dropout > 0.0 {
            candle_nn::ops::dropout(x, self.dropout)
        } else {
            Ok(x.clone())
        }
    }
}

pub fn normal(stdev: f64) -> Init {
    Init::Randn { mean: 0.0, stdev }
}

#[derive(Clone)]
struct Lora {
    a: Tensor,
    b: Tensor,
    scale: f64,
}

/// `y = x Wᵀ + b (+ s · x Aᵀ Bᵀ)`.
#[derive(Clone)]
pub struct Lin {
    w: Tensor,
    b: Option<Tensor>,
    lora: Option<Lora>,
    path: String,
    target: Option<LoraTarget>,
    in_dim: usize,
    out_dim: usize,
}

impl Lin {
    pub fn new(
        vb: &VarBuilder,
        name: &str,
        in_dim: usize,
        out_dim: usize,
        bias: bool,
        stdev: f64,
        target: Option<LoraTarget>,
    ) -> Result<Self> {
        let vb = vb.pp(name);
        let w = vb.get_with_hints((out_dim, in_dim), "weight", normal(stdev))?;
        let b = if bias {
            Some(vb.get_with_hints(out_dim, "bias", Init::Const(0.0))?)
        } else {
            None
        };
        Ok(Self {
            w,
            b,
            lora: None,
            path: vb.prefix(),
            target,
            in_dim,
            out_dim,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let flat = x.reshape(((), self.in_dim))?;
        let mut y = flat.matmul(&self.w.t()?)?;
        if let Some(b) = &self.b {
            y = y.broadcast_add(b)?;
        }
        if let Some(lora) = &self.lora {
            let delta = flat.matmul(&lora.a.t()?)?.matmul(&lora.b.t()?)?;
            y = (y + (delta * lora.scale)?)?;
        }
        let mut out = dims;
        *out.last_mut()
            .expect("a linear input has at least one dimension") = self.out_dim;
        y.reshape(out)
    }

    /// Attaches an adapter if this layer is one the config targets. `vb` is the adapter's
    /// own builder: a `VarMap` to train one, or loaded tensors to use one.
    pub fn attach_lora(&mut self, vb: &VarBuilder, config: &LoraConfig) -> Result<()> {
        let Some(target) = self.target else {
            return Ok(());
        };
        if !config.applies(target) {
            return Ok(());
        }
        let vb = vb.pp(&self.path);
        let a = vb.get_with_hints(
            (config.rank, self.in_dim),
            "lora_a",
            normal(1.0 / (self.in_dim as f64).sqrt()),
        )?;
        let b = vb.get_with_hints((self.out_dim, config.rank), "lora_b", Init::Const(0.0))?;
        self.lora = Some(Lora {
            a,
            b,
            scale: config.scale(),
        });
        Ok(())
    }

    pub fn detach_lora(&mut self) {
        self.lora = None;
    }
}

/// Whether a fused last-dimension kernel (RMS norm, softmax) beats the composed ops: always on
/// a GPU, where each op is a launch; on a CPU only for large tensors, below which spreading a
/// few rows over threads costs more than it saves (measured on the gate's short prompt:
/// 24 ms composed, 33 ms fused; on a full update prompt: 818 ms composed, 506 ms fused).
fn fused_pays(x: &Tensor) -> Result<bool> {
    Ok(!x.device().is_cpu() || x.elem_count() >= 32_768)
}

#[derive(Clone)]
pub struct RmsNorm {
    w: Tensor,
    eps: f64,
}

impl RmsNorm {
    pub fn new(vb: &VarBuilder, name: &str, dim: usize) -> Result<Self> {
        Ok(Self {
            w: vb
                .pp(name)
                .get_with_hints(dim, "weight", Init::Const(1.0))?,
            eps: 1e-6,
        })
    }
    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        // In use (no gradient), one fused kernel instead of five ops and four temporaries;
        // the fused op has no backward, so training keeps the composed form.
        if !x.track_op() && !self.w.track_op() && x.is_contiguous() && fused_pays(x)? {
            return candle_nn::ops::rms_norm(x, &self.w, self.eps as f32);
        }
        let mean_square = x.sqr()?.mean_keepdim(D::Minus1)?;
        let normed = x.broadcast_div(&(mean_square + self.eps)?.sqrt()?)?;
        normed.broadcast_mul(&self.w)
    }
}

#[derive(Clone)]
pub struct Attention {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    heads: usize,
    head_dim: usize,
}

impl Attention {
    pub fn new(vb: &VarBuilder, d: usize, heads: usize, out_stdev: f64) -> Result<Self> {
        Ok(Self {
            q: Lin::new(vb, "q", d, d, false, 0.02, Some(LoraTarget::Q))?,
            k: Lin::new(vb, "k", d, d, false, 0.02, Some(LoraTarget::K))?,
            v: Lin::new(vb, "v", d, d, false, 0.02, Some(LoraTarget::V))?,
            o: Lin::new(vb, "o", d, d, false, out_stdev, Some(LoraTarget::O))?,
            heads,
            head_dim: d / heads,
        })
    }

    fn split(&self, x: &Tensor) -> Result<Tensor> {
        let (b, t, _) = x.dims3()?;
        x.reshape((b, t, self.heads, self.head_dim))?
            .transpose(1, 2)?
            .contiguous()
    }

    fn merge(&self, x: &Tensor) -> Result<Tensor> {
        let (b, _, t, _) = x.dims4()?;
        x.transpose(1, 2)?
            .reshape((b, t, self.heads * self.head_dim))
    }

    fn attend(&self, q: &Tensor, k: &Tensor, v: &Tensor, bias: Option<&Tensor>) -> Result<Tensor> {
        let scale = 1.0 / (self.head_dim as f64).sqrt();
        // The matmul reads K transposed in place; copying it contiguous was one more
        // (B, H, T, d) buffer per layer and call.
        let mut scores = (q.matmul(&k.t()?)? * scale)?;
        if let Some(bias) = bias {
            scores = scores.broadcast_add(bias)?;
        }
        let weights = if scores.track_op() || !fused_pays(&scores)? {
            candle_nn::ops::softmax(&scores, D::Minus1)?
        } else {
            // One fused kernel (max, exp, sum, divide) in use.
            candle_nn::ops::softmax_last_dim(&scores)?
        };
        self.o.forward(&self.merge(&weights.matmul(v)?)?)
    }

    /// Attention over the sequence itself. `bias` is added to the scores and must broadcast
    /// to `(batch, heads, T, T)`.
    pub fn forward(&self, x: &Tensor, bias: Option<&Tensor>) -> Result<Tensor> {
        let q = self.split(&self.q.forward(x)?)?;
        let k = self.split(&self.k.forward(x)?)?;
        let v = self.split(&self.v.forward(x)?)?;
        self.attend(&q, &k, &v, bias)
    }

    /// Unmasked attention that also returns its keys and values, for a prompt that later
    /// queries will read without it being computed again.
    pub fn forward_kv(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let q = self.split(&self.q.forward(x)?)?;
        let k = self.split(&self.k.forward(x)?)?;
        let v = self.split(&self.v.forward(x)?)?;
        let out = self.attend(&q, &k, &v, None)?;
        Ok((out, k, v))
    }

    /// Queries from `x` read the cached prefix and `x` itself; nothing is masked.
    pub fn forward_with_prefix(
        &self,
        x: &Tensor,
        k_prefix: &Tensor,
        v_prefix: &Tensor,
    ) -> Result<Tensor> {
        let q = self.split(&self.q.forward(x)?)?;
        let k = Tensor::cat(&[k_prefix, &self.split(&self.k.forward(x)?)?], 2)?;
        let v = Tensor::cat(&[v_prefix, &self.split(&self.v.forward(x)?)?], 2)?;
        self.attend(&q, &k, &v, None)
    }

    fn layers_mut(&mut self) -> [&mut Lin; 4] {
        [&mut self.q, &mut self.k, &mut self.v, &mut self.o]
    }
}

/// Pre-norm transformer block: attention then a GELU feed-forward, each residual.
#[derive(Clone)]
pub struct Block {
    norm1: RmsNorm,
    attn: Attention,
    norm2: RmsNorm,
    ff_in: Lin,
    ff_out: Lin,
}

impl Block {
    pub fn new(vb: &VarBuilder, d: usize, heads: usize, d_ff: usize, depth: usize) -> Result<Self> {
        // Residual projections start smaller the deeper the stack, so its output variance does
        // not grow with depth (GPT-2's scheme).
        let out_stdev = 0.02 / (2.0 * depth.max(1) as f64).sqrt();
        Ok(Self {
            norm1: RmsNorm::new(vb, "norm1", d)?,
            attn: Attention::new(&vb.pp("attn"), d, heads, out_stdev)?,
            norm2: RmsNorm::new(vb, "norm2", d)?,
            ff_in: Lin::new(vb, "ff_in", d, d_ff, true, 0.02, Some(LoraTarget::FfIn))?,
            ff_out: Lin::new(
                vb,
                "ff_out",
                d_ff,
                d,
                true,
                out_stdev,
                Some(LoraTarget::FfOut),
            )?,
        })
    }

    fn feed_forward(&self, x: &Tensor, ctx: Ctx) -> Result<Tensor> {
        let h = self.ff_in.forward(&self.norm2.forward(x)?)?.gelu()?;
        ctx.drop(&self.ff_out.forward(&h)?)
    }

    pub fn forward(&self, x: &Tensor, bias: Option<&Tensor>, ctx: Ctx) -> Result<Tensor> {
        let h = (x + ctx.drop(&self.attn.forward(&self.norm1.forward(x)?, bias)?)?)?;
        &h + self.feed_forward(&h, ctx)?
    }

    pub fn forward_kv(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let (a, k, v) = self.attn.forward_kv(&self.norm1.forward(x)?)?;
        let h = (x + a)?;
        let y = (&h + self.feed_forward(&h, Ctx::EVAL)?)?;
        Ok((y, k, v))
    }

    pub fn forward_with_prefix(&self, x: &Tensor, k: &Tensor, v: &Tensor) -> Result<Tensor> {
        let a = self
            .attn
            .forward_with_prefix(&self.norm1.forward(x)?, k, v)?;
        let h = (x + a)?;
        &h + self.feed_forward(&h, Ctx::EVAL)?
    }

    pub fn attach_lora(&mut self, vb: &VarBuilder, config: &LoraConfig) -> Result<()> {
        for layer in self.attn.layers_mut() {
            layer.attach_lora(vb, config)?;
        }
        self.ff_in.attach_lora(vb, config)?;
        self.ff_out.attach_lora(vb, config)
    }

    pub fn detach_lora(&mut self) {
        for layer in self.attn.layers_mut() {
            layer.detach_lora();
        }
        self.ff_in.detach_lora();
        self.ff_out.detach_lora();
    }
}

/// `(B, L, C) → (B, l, C)` by averaging consecutive groups of `L / l` positions.
pub fn downsample(x: &Tensor, l: usize) -> Result<Tensor> {
    let (b, full, c) = x.dims3()?;
    if l == full {
        return Ok(x.clone());
    }
    x.reshape((b, l, full / l, c))?.mean(2)
}

/// `(B, l, C) → (B, L, C)` by repeating each position `L / l` times.
pub fn upsample(x: &Tensor, full: usize) -> Result<Tensor> {
    let (b, l, c) = x.dims3()?;
    if l == full {
        return Ok(x.clone());
    }
    x.unsqueeze(2)?
        .broadcast_as((b, l, full / l, c))?
        .contiguous()?
        .reshape((b, full, c))
}

/// Additive key mask `(B, 1, 1, T)`: for each row of `valid`, the listed key ranges are
/// visible and every other key is masked.
pub fn key_mask(valid: &[Vec<(usize, usize)>], total: usize, device: &Device) -> Result<Tensor> {
    let mut data = vec![MASKED; valid.len() * total];
    for (row, ranges) in valid.iter().enumerate() {
        for &(start, end) in ranges {
            for slot in &mut data[row * total + start.min(total)..row * total + end.min(total)] {
                *slot = 0.0;
            }
        }
    }
    Tensor::from_vec(data, (valid.len(), 1, 1, total), device)
}

/// Additive structural mask `(1, 1, T, T)` for a prompt of `prompt` positions followed by
/// blocks of the given lengths: the prompt reads only the prompt; each block reads the prompt
/// and itself.
pub fn prefix_block_mask(prompt: usize, blocks: &[usize], device: &Device) -> Result<Tensor> {
    let total = prompt + blocks.iter().sum::<usize>();
    let mut data = vec![MASKED; total * total];
    for row in 0..prompt {
        data[row * total..row * total + prompt].fill(0.0);
    }
    let mut start = prompt;
    for &len in blocks {
        for row in start..start + len {
            data[row * total..row * total + prompt].fill(0.0);
            data[row * total + start..row * total + start + len].fill(0.0);
        }
        start += len;
    }
    Tensor::from_vec(data, (1, 1, total, total), device)
}

/// Mean of `h` over each row's span: `weights` is `(B, S, T)` with rows summing to one.
pub fn span_pool(h: &Tensor, weights: &Tensor) -> Result<Tensor> {
    weights.matmul(h)
}

/// Builds the `(B, S, T)` averaging weights for spans, zero rows where a span is absent.
pub fn span_weights(
    spans: &[Vec<(usize, usize)>],
    slots: usize,
    total: usize,
    device: &Device,
) -> Result<Tensor> {
    let mut data = vec![0f32; spans.len() * slots * total];
    for (row, row_spans) in spans.iter().enumerate() {
        for (slot, &(start, end)) in row_spans.iter().take(slots).enumerate() {
            let end = end.min(total);
            if end <= start {
                continue;
            }
            let w = 1.0 / (end - start) as f32;
            let base = (row * slots + slot) * total;
            data[base + start..base + end].fill(w);
        }
    }
    Tensor::from_vec(data, (spans.len(), slots, total), device)
}

/// A learned table of `rows × dim` rows, initialised small.
pub fn table(vb: &VarBuilder, name: &str, rows: usize, dim: usize) -> Result<Tensor> {
    vb.get_with_hints((rows, dim), name, normal(0.02))
}

/// Gathers `rows` of `table` for each id of `ids` `(B, T)` → `(B, T, dim)`.
pub fn gather(table: &Tensor, ids: &Tensor) -> Result<Tensor> {
    let (b, t) = ids.dims2()?;
    let dim = table.dim(1)?;
    table
        .index_select(&ids.flatten_all()?, 0)?
        .reshape((b, t, dim))
}

pub fn ids_tensor(rows: &[Vec<u32>], device: &Device) -> Result<Tensor> {
    let width = rows.first().map_or(0, Vec::len);
    let flat: Vec<u32> = rows.iter().flat_map(|r| r.iter().copied()).collect();
    Tensor::from_vec(flat, (rows.len(), width), device)
}

pub fn scalar(x: &Tensor) -> Result<f32> {
    x.to_dtype(DType::F32)?.to_scalar::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_nn::VarMap;

    fn builder(map: &VarMap) -> VarBuilder<'_> {
        VarBuilder::from_varmap(map, DType::F32, &Device::Cpu)
    }

    #[test]
    fn a_fresh_adapter_changes_nothing() {
        let base = VarMap::new();
        let mut lin = Lin::new(
            &builder(&base),
            "layer",
            8,
            6,
            true,
            0.02,
            Some(LoraTarget::Q),
        )
        .unwrap();
        let x = Tensor::randn(0f32, 1.0, (2, 3, 8), &Device::Cpu).unwrap();
        let before = lin.forward(&x).unwrap();
        let adapters = VarMap::new();
        let config = LoraConfig {
            rank: 2,
            alpha: 4.0,
            targets: vec![LoraTarget::Q],
        };
        lin.attach_lora(&builder(&adapters), &config).unwrap();
        assert_eq!(adapters.all_vars().len(), 2);
        let after = lin.forward(&x).unwrap();
        let diff = (before - after).unwrap().abs().unwrap().sum_all().unwrap();
        assert_eq!(scalar(&diff).unwrap(), 0.0);
    }

    #[test]
    fn untargeted_layers_take_no_adapter() {
        let base = VarMap::new();
        let mut lin = Lin::new(
            &builder(&base),
            "layer",
            4,
            4,
            false,
            0.02,
            Some(LoraTarget::K),
        )
        .unwrap();
        let adapters = VarMap::new();
        let config = LoraConfig {
            rank: 2,
            alpha: 4.0,
            targets: vec![LoraTarget::Q],
        };
        lin.attach_lora(&builder(&adapters), &config).unwrap();
        assert!(adapters.all_vars().is_empty());
    }

    #[test]
    fn prompt_never_reads_the_blocks() {
        let map = VarMap::new();
        let block = Block::new(&builder(&map), 16, 2, 32, 1).unwrap();
        let mask = prefix_block_mask(3, &[2, 4], &Device::Cpu).unwrap();
        let x = Tensor::randn(0f32, 1.0, (1, 9, 16), &Device::Cpu).unwrap();
        let mut changed = x.to_vec3::<f32>().unwrap();
        for row in &mut changed[0][3..] {
            for v in row.iter_mut() {
                *v += 5.0;
            }
        }
        let changed = Tensor::new(changed, &Device::Cpu).unwrap();
        let a = block.forward(&x, Some(&mask), Ctx::EVAL).unwrap();
        let b = block.forward(&changed, Some(&mask), Ctx::EVAL).unwrap();
        let prompt_diff = (a.narrow(1, 0, 3).unwrap() - b.narrow(1, 0, 3).unwrap())
            .unwrap()
            .abs()
            .unwrap()
            .max_all()
            .unwrap();
        assert!(scalar(&prompt_diff).unwrap() < 1e-6);
    }

    #[test]
    fn cached_prefix_matches_the_masked_pass() {
        let map = VarMap::new();
        let block = Block::new(&builder(&map), 16, 2, 32, 1).unwrap();
        let x = Tensor::randn(0f32, 1.0, (1, 7, 16), &Device::Cpu).unwrap();
        let mask = prefix_block_mask(4, &[3], &Device::Cpu).unwrap();
        let full = block.forward(&x, Some(&mask), Ctx::EVAL).unwrap();
        let (prompt_out, k, v) = block.forward_kv(&x.narrow(1, 0, 4).unwrap()).unwrap();
        let tail = block
            .forward_with_prefix(&x.narrow(1, 4, 3).unwrap(), &k, &v)
            .unwrap();
        let joined = Tensor::cat(&[prompt_out, tail], 1).unwrap();
        let diff = (full - joined).unwrap().abs().unwrap().max_all().unwrap();
        assert!(scalar(&diff).unwrap() < 1e-5);
    }

    #[test]
    fn resize_round_trip_and_span_pooling() {
        let x = Tensor::arange(0f32, 8.0, &Device::Cpu)
            .unwrap()
            .reshape((1, 4, 2))
            .unwrap();
        let down = downsample(&x, 2).unwrap();
        assert_eq!(
            down.to_vec3::<f32>().unwrap(),
            vec![vec![vec![1.0, 2.0], vec![5.0, 6.0]]]
        );
        let up = upsample(&down, 4).unwrap();
        assert_eq!(up.dims(), &[1, 4, 2]);
        let w = span_weights(&[vec![(1, 3)]], 2, 4, &Device::Cpu).unwrap();
        let pooled = span_pool(&x, &w).unwrap().to_vec3::<f32>().unwrap();
        assert_eq!(pooled[0][0], vec![3.0, 4.0]);
        assert_eq!(pooled[0][1], vec![0.0, 0.0]);
    }

    #[test]
    fn key_mask_opens_only_listed_ranges() {
        let m = key_mask(&[vec![(0, 2), (3, 4)]], 5, &Device::Cpu).unwrap();
        let v = m.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(v, vec![0.0, 0.0, MASKED, 0.0, MASKED]);
    }
}
