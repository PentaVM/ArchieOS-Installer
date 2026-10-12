//! The few layers IRIS is built from, read from the state dict by their PyTorch names.

use crate::weights::Weights;
use crate::Error;
use candle_core::{DType, Module, Tensor, D};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Linear {
    weight: Tensor,
    bias: Option<Tensor>,
}

impl Linear {
    pub fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        let bias = format!("{prefix}.bias");
        Ok(Self {
            weight: weights.get(&format!("{prefix}.weight"))?,
            bias: if weights.contains(&bias) {
                Some(weights.get(&bias)?)
            } else {
                None
            },
        })
    }

    pub fn from_parts(weight: Tensor, bias: Option<Tensor>) -> Self {
        Self { weight, bias }
    }

    pub fn out_features(&self) -> usize {
        self.weight.dims()[0]
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        Ok(candle_nn::Linear::new(self.weight.clone(), self.bias.clone()).forward(x)?)
    }
}

#[derive(Debug, Clone)]
pub struct LayerNorm {
    inner: candle_nn::LayerNorm,
}

impl LayerNorm {
    /// `eps` is the module's own: PyTorch's default 1e-5, 1e-6 inside V-JEPA and SigLIP.
    pub fn load(weights: &Weights, prefix: &str, eps: f64) -> Result<Self> {
        Ok(Self {
            inner: candle_nn::LayerNorm::new(
                weights.get(&format!("{prefix}.weight"))?,
                weights.get(&format!("{prefix}.bias"))?,
                eps,
            ),
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x = if x.is_contiguous() {
            x.clone()
        } else {
            x.contiguous()?
        };
        Ok(self.inner.forward(&x)?)
    }
}

/// `Linear -> GELU -> Linear`, the shape of every small MLP here (`nn.Sequential` indices
/// 0 and 2). `exact` picks erf GELU (PyTorch's default) over the tanh approximation.
#[derive(Debug, Clone)]
pub struct Mlp {
    first: Linear,
    second: Linear,
    exact: bool,
}

impl Mlp {
    pub fn load(weights: &Weights, first: &str, second: &str, exact: bool) -> Result<Self> {
        Ok(Self {
            first: Linear::load(weights, first)?,
            second: Linear::load(weights, second)?,
            exact,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let hidden = self.first.forward(x)?;
        let hidden = if self.exact {
            crate::fast::gelu_erf(&hidden)?
        } else {
            crate::fast::gelu_tanh(&hidden)?
        };
        self.second.forward(&hidden)
    }
}

/// Scaled dot-product attention over `[B, H, L, Dh]` queries and `[B, H, S, Dh]` keys and
/// values. `allowed` is `[B or 1, 1 or H, L or 1, S]`, 1 where a query may attend; positions
/// that may not are given minus infinity before the softmax, as PyTorch's boolean mask does.
pub fn attend(q: &Tensor, k: &Tensor, v: &Tensor, allowed: Option<&Tensor>) -> Result<Tensor> {
    let scale = 1.0 / (q.dim(D::Minus1)? as f64).sqrt();
    if matches!(q.device(), candle_core::Device::Cpu) && q.dtype() == DType::F32 {
        return attend_per_head(q, k, v, allowed, scale);
    }
    let q = q.contiguous()?;
    let kt = k.transpose(D::Minus2, D::Minus1)?.contiguous()?;
    let scores = (q.matmul(&kt)? * scale)?;
    let scores = match allowed {
        None => scores,
        Some(allowed) => {
            let blocked = Tensor::full(f32::NEG_INFINITY, (), scores.device())?
                .to_dtype(scores.dtype())?
                .broadcast_as(scores.shape())?;
            allowed
                .broadcast_as(scores.shape())?
                .where_cond(&scores, &blocked)?
        }
    };
    let dtype = scores.dtype();
    let weights = crate::fast::softmax_last_dim(&scores.to_dtype(DType::F32)?)?.to_dtype(dtype)?;
    Ok(weights.matmul(&v.contiguous()?)?)
}

/// The CPU path: one rayon task per (image, head). A batched product over twelve heads of
/// width 64 kept the matrix kernel a third as busy as twelve independent ones.
fn attend_per_head(
    q: &Tensor,
    k: &Tensor,
    v: &Tensor,
    allowed: Option<&Tensor>,
    scale: f64,
) -> Result<Tensor> {
    use rayon::prelude::*;
    let (batch, heads, length, width) = q.dims4()?;
    let source = k.dim(2)?;
    let q = (q * scale)?;
    let allowed = match allowed {
        Some(mask) => Some(mask.broadcast_as((batch, heads, length, source))?),
        None => None,
    };
    let outputs = (0..batch * heads)
        .into_par_iter()
        .map(|index| -> Result<Tensor> {
            let (b, h) = (index / heads, index % heads);
            let qh = q.get(b)?.get(h)?;
            let scores = qh.matmul(&k.get(b)?.get(h)?.t()?)?;
            let scores = match &allowed {
                None => scores,
                Some(mask) => {
                    let blocked =
                        Tensor::full(f32::NEG_INFINITY, (length, source), scores.device())?;
                    mask.get(b)?.get(h)?.where_cond(&scores, &blocked)?
                }
            };
            let weights = candle_nn::ops::softmax_last_dim(&scores)?;
            Ok(weights.matmul(&v.get(b)?.get(h)?)?)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Tensor::stack(&outputs, 0)?.reshape((batch, heads, length, width))?)
}

/// `[B, L, H*Dh]` to `[B, H, L, Dh]`.
pub fn split_heads(x: &Tensor, heads: usize) -> Result<Tensor> {
    let (batch, length, width) = x.dims3()?;
    Ok(x.reshape((batch, length, heads, width / heads))?
        .transpose(1, 2)?)
}

/// `[B, H, L, Dh]` back to `[B, L, H*Dh]`.
pub fn merge_heads(x: &Tensor) -> Result<Tensor> {
    let (batch, heads, length, width) = x.dims4()?;
    Ok(x.transpose(1, 2)?
        .contiguous()?
        .reshape((batch, length, heads * width))?)
}

/// Multi-head attention with separate `query`, `key`, `value` and `output` projections:
/// IRIS's own `Attention` module.
#[derive(Debug, Clone)]
pub struct Attention {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
    heads: usize,
}

impl Attention {
    pub fn load(weights: &Weights, prefix: &str, heads: usize) -> Result<Self> {
        Ok(Self {
            query: Linear::load(weights, &format!("{prefix}.query"))?,
            key: Linear::load(weights, &format!("{prefix}.key"))?,
            value: Linear::load(weights, &format!("{prefix}.value"))?,
            output: Linear::load(weights, &format!("{prefix}.output"))?,
            heads,
        })
    }

    /// `key_padding` is `[B, S]`, 1 where the key is padding (PyTorch's convention).
    pub fn forward(
        &self,
        query: &Tensor,
        key: &Tensor,
        value: &Tensor,
        key_padding: Option<&Tensor>,
    ) -> Result<Tensor> {
        let q = split_heads(&self.query.forward(query)?, self.heads)?;
        let k = split_heads(&self.key.forward(key)?, self.heads)?;
        let v = split_heads(&self.value.forward(value)?, self.heads)?;
        let allowed = match key_padding {
            None => None,
            Some(padding) => {
                let (batch, length) = padding.dims2()?;
                Some(padding.eq(0u8)?.reshape((batch, 1, 1, length))?)
            }
        };
        let attended = attend(&q, &k, &v, allowed.as_ref())?;
        self.output.forward(&merge_heads(&attended)?)
    }
}

/// `torch.nn.MultiheadAttention` (batch first), with its packed input projection.
#[derive(Debug, Clone)]
pub struct MultiheadAttention {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
    heads: usize,
}

impl MultiheadAttention {
    pub fn load(weights: &Weights, prefix: &str, heads: usize) -> Result<Self> {
        let packed = weights.get(&format!("{prefix}.in_proj_weight"))?;
        let bias = weights.get(&format!("{prefix}.in_proj_bias"))?;
        let width = packed.dim(1)?;
        let part = |index: usize| -> Result<Linear> {
            Ok(Linear::from_parts(
                packed.narrow(0, index * width, width)?.contiguous()?,
                Some(bias.narrow(0, index * width, width)?.contiguous()?),
            ))
        };
        Ok(Self {
            query: part(0)?,
            key: part(1)?,
            value: part(2)?,
            output: Linear::load(weights, &format!("{prefix}.out_proj"))?,
            heads,
        })
    }

    /// `allowed` as in [`attend`].
    pub fn forward(
        &self,
        query: &Tensor,
        key: &Tensor,
        value: &Tensor,
        allowed: Option<&Tensor>,
    ) -> Result<Tensor> {
        let q = split_heads(&self.query.forward(query)?, self.heads)?;
        let k = split_heads(&self.key.forward(key)?, self.heads)?;
        let v = split_heads(&self.value.forward(value)?, self.heads)?;
        let attended = attend(&q, &k, &v, allowed)?;
        self.output.forward(&merge_heads(&attended)?)
    }
}

/// `F.normalize(x, dim=-1)`: divided by the norm, never by less than 1e-12.
pub fn l2_normalize(x: &Tensor) -> Result<Tensor> {
    let norm = x
        .sqr()?
        .sum_keepdim(D::Minus1)?
        .sqrt()?
        .clamp(1e-12, f64::INFINITY)?;
    Ok(x.broadcast_div(&norm)?)
}
