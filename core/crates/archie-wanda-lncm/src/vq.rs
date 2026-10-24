//! The tokenizer of next-concept modelling: a multiscale residual VQ-VAE.
//!
//! A target of `L` tokens is encoded to `L` latent vectors. Level 1 averages them down to
//! `ℓ_1` positions and picks the nearest code for each; what level 1 cannot express — the
//! residual, once its codes are spread back over `L` — is what level 2 quantises at `ℓ_2`
//! positions, and so on to the last level, which has one position per token. The decoder reads
//! the sum of every level's contribution. Coarse levels hold the gist, fine levels the wording.
//!
//! While training, gradients pass the quantiser straight through (the decoder's gradient is
//! applied to the encoder as if quantising were the identity), the codebook is pulled towards
//! the residuals it was chosen for, and a commitment term keeps the encoder near its codes.
//! With nested dropout some steps decode from the first levels only, so the coarse levels must
//! carry meaning on their own.

use crate::config::LncmConfig;
use crate::nn::{self, Block, Ctx, Lin, RmsNorm};
use candle_core::{Result, Tensor, D};
use candle_nn::VarBuilder;

#[derive(Clone)]
pub struct VqVae {
    levels: Vec<usize>,
    target_len: usize,
    code_dim: usize,
    commitment: f64,
    embed: Tensor,
    pos: Tensor,
    encoder: Vec<Block>,
    enc_norm: RmsNorm,
    to_code: Lin,
    codebooks: Vec<Tensor>,
    phi: Vec<Lin>,
    from_code: Lin,
    dec_pos: Tensor,
    decoder: Vec<Block>,
    dec_norm: RmsNorm,
    to_vocab: Lin,
}

/// What quantising a batch produced.
pub struct Quantized {
    /// Per level, `(B, ℓ_k)` code ids.
    pub codes: Vec<Tensor>,
    /// Sum of every level's contribution, straight-through, `(B, L, C)`.
    pub full: Tensor,
    /// Sum of the first `keep` levels only, what the decoder reads under nested dropout.
    pub kept: Tensor,
    pub commitment: Tensor,
    pub codebook: Tensor,
    /// Per level, the residual vectors that were quantised `(B·ℓ_k, C)`, detached. Used to
    /// restart codes nothing chose.
    pub residuals: Vec<Tensor>,
}

pub struct VqLoss {
    pub total: Tensor,
    pub reconstruction: f32,
    pub commitment: f32,
    pub codebook: f32,
    /// Share of tokens reconstructed exactly.
    pub accuracy: f32,
    pub codes: Vec<Tensor>,
    pub residuals: Vec<Tensor>,
}

/// Name of the codebook variable of `level`, for code restarts.
pub fn codebook_name(level: usize) -> String {
    format!("vq.codebook.{level}")
}

impl VqVae {
    /// `vb` must be the root builder; every name is placed under `vq.`.
    pub fn new(vb: &VarBuilder, config: &LncmConfig) -> Result<Self> {
        let vb = vb.pp("vq");
        let v = &config.vq;
        let d = v.d_model;
        let c = config.code_dim;
        let encoder = (0..v.encoder_layers)
            .map(|i| {
                Block::new(
                    &vb.pp(format!("encoder.{i}")),
                    d,
                    v.heads,
                    v.d_ff,
                    v.encoder_layers,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let decoder = (0..v.decoder_layers)
            .map(|i| {
                Block::new(
                    &vb.pp(format!("decoder.{i}")),
                    d,
                    v.heads,
                    v.d_ff,
                    v.decoder_layers,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let codebooks = (0..config.levels.len())
            .map(|k| {
                vb.pp("codebook").get_with_hints(
                    (config.codebook_size, c),
                    &k.to_string(),
                    nn::normal(1.0 / (c as f64).sqrt()),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let phi = (0..config.levels.len())
            .map(|k| Lin::new(&vb, &format!("phi.{k}"), c, c, true, 0.01, None))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            levels: config.levels.clone(),
            target_len: config.target_len,
            code_dim: c,
            commitment: f64::from(v.commitment),
            embed: nn::table(&vb, "embed", config.vocab_size, d)?,
            pos: nn::table(&vb, "pos", config.target_len, d)?,
            encoder,
            enc_norm: RmsNorm::new(&vb, "enc_norm", d)?,
            to_code: Lin::new(&vb, "to_code", d, c, true, 0.02, None)?,
            codebooks,
            phi,
            from_code: Lin::new(&vb, "from_code", c, d, true, 0.02, None)?,
            dec_pos: nn::table(&vb, "dec_pos", config.target_len, d)?,
            decoder,
            dec_norm: RmsNorm::new(&vb, "dec_norm", d)?,
            to_vocab: Lin::new(&vb, "to_vocab", d, config.vocab_size, true, 0.02, None)?,
        })
    }

    pub fn levels(&self) -> &[usize] {
        &self.levels
    }

    /// Token embeddings `(B, T, d_vq)`. The generator reads prompts through this table, frozen,
    /// as the paper does.
    pub fn embed_tokens(&self, ids: &Tensor) -> Result<Tensor> {
        nn::gather(&self.embed, ids)
    }

    pub fn encode(&self, ids: &Tensor, ctx: Ctx) -> Result<Tensor> {
        let mut x = self.embed_tokens(ids)?.broadcast_add(&self.pos)?;
        for block in &self.encoder {
            x = block.forward(&x, None, ctx)?;
        }
        self.to_code.forward(&self.enc_norm.forward(&x)?)
    }

    /// One level's codes spread over the whole target: `φ_k(upsample(e_k[codes]))`.
    pub fn code_embedding(&self, level: usize, codes: &Tensor) -> Result<Tensor> {
        let picked = nn::gather(&self.codebooks[level], codes)?;
        self.contribution(level, &picked)
    }

    fn contribution(&self, level: usize, vectors: &Tensor) -> Result<Tensor> {
        let up = nn::upsample(vectors, self.target_len)?;
        &up + self.phi[level].forward(&up)?
    }

    /// Quantises encoder output `(B, L, C)`. `keep` (1..=levels) is how many levels the
    /// decoder input `kept` sums.
    pub fn quantize(&self, z: &Tensor, keep: usize) -> Result<Quantized> {
        let (b, full_len, c) = z.dims3()?;
        let mut full = Tensor::zeros((b, full_len, c), z.dtype(), z.device())?;
        let mut kept = full.clone();
        let mut commitment = Tensor::zeros((), z.dtype(), z.device())?;
        let mut codebook = commitment.clone();
        let mut codes = Vec::with_capacity(self.levels.len());
        let mut residuals = Vec::with_capacity(self.levels.len());
        for (k, &l) in self.levels.iter().enumerate() {
            let r = nn::downsample(&(z - &full)?, l)?;
            let flat = r.reshape((b * l, c))?.detach();
            let idx = nearest(&flat, &self.codebooks[k])?;
            let e = self.codebooks[k]
                .index_select(&idx, 0)?
                .reshape((b, l, c))?;
            codebook = (codebook + (&e - r.detach())?.sqr()?.mean_all()?)?;
            commitment = (commitment + (&r - e.detach())?.sqr()?.mean_all()?)?;
            let straight = (&r + (&e - &r)?.detach())?;
            let contribution = self.contribution(k, &straight)?;
            full = (full + &contribution)?;
            if k < keep {
                kept = (kept + &contribution)?;
            }
            codes.push(idx.reshape((b, l))?);
            residuals.push(flat);
        }
        Ok(Quantized {
            codes,
            full,
            kept,
            commitment,
            codebook,
            residuals,
        })
    }

    /// Token logits `(B, L, V)` from a latent sum.
    pub fn decode(&self, latent: &Tensor, ctx: Ctx) -> Result<Tensor> {
        let mut x = self
            .from_code
            .forward(latent)?
            .broadcast_add(&self.dec_pos)?;
        for block in &self.decoder {
            x = block.forward(&x, None, ctx)?;
        }
        self.to_vocab.forward(&self.dec_norm.forward(&x)?)
    }

    /// The most likely token at every position of a latent sum `(1, L, C)`.
    pub fn tokens(&self, latent: &Tensor) -> Result<Vec<u32>> {
        self.decode(latent, Ctx::EVAL)?
            .argmax(D::Minus1)?
            .flatten_all()?
            .to_vec1::<u32>()
    }

    /// Codes for targets, with no gradient: what the generator learns to predict.
    pub fn codes_for(&self, ids: &Tensor) -> Result<Vec<Tensor>> {
        let z = self.encode(ids, Ctx::EVAL)?.detach();
        Ok(self.quantize(&z, self.levels.len())?.codes)
    }

    pub fn loss(&self, ids: &Tensor, keep: usize, ctx: Ctx) -> Result<VqLoss> {
        let z = self.encode(ids, ctx)?;
        let q = self.quantize(&z, keep)?;
        let logits = self.decode(&q.kept, ctx)?;
        let (b, l, v) = logits.dims3()?;
        let flat_ids = ids.flatten_all()?;
        let reconstruction =
            candle_nn::loss::cross_entropy(&logits.reshape((b * l, v))?, &flat_ids)?;
        let accuracy = logits
            .argmax(D::Minus1)?
            .flatten_all()?
            .eq(&flat_ids)?
            .to_dtype(candle_core::DType::F32)?
            .mean_all()?;
        let total = ((&reconstruction + &q.codebook)? + (&q.commitment * self.commitment)?)?;
        Ok(VqLoss {
            reconstruction: nn::scalar(&reconstruction)?,
            commitment: nn::scalar(&q.commitment)?,
            codebook: nn::scalar(&q.codebook)?,
            accuracy: nn::scalar(&accuracy)?,
            total,
            codes: q.codes,
            residuals: q.residuals,
        })
    }

    pub fn code_dim(&self) -> usize {
        self.code_dim
    }
    pub fn target_len(&self) -> usize {
        self.target_len
    }
}

/// Index of the nearest codebook row for each row of `x` `(N, C)` → `(N,)`.
fn nearest(x: &Tensor, codebook: &Tensor) -> Result<Tensor> {
    let codebook = codebook.detach();
    let x_sq = x.sqr()?.sum_keepdim(1)?;
    let c_sq = codebook.sqr()?.sum_keepdim(1)?.t()?;
    let cross = x.matmul(&codebook.t()?)?;
    let distance = x_sq.broadcast_add(&c_sq)?.broadcast_sub(&(cross * 2.0)?)?;
    distance.argmin(D::Minus1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device};
    use candle_nn::{Optimizer, VarMap};

    #[test]
    fn shapes_and_identity_of_inference_codes() {
        let config = LncmConfig::tiny(272);
        let map = VarMap::new();
        let vb = VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu);
        let vq = VqVae::new(&vb, &config).unwrap();
        let ids = Tensor::zeros((2, 32), DType::U32, &Device::Cpu).unwrap();
        let z = vq.encode(&ids, Ctx::EVAL).unwrap();
        assert_eq!(z.dims(), &[2, 32, 16]);
        let q = vq.quantize(&z, 3).unwrap();
        assert_eq!(q.codes.len(), 3);
        assert_eq!(q.codes[0].dims(), &[2, 2]);
        assert_eq!(q.codes[2].dims(), &[2, 32]);
        // The straight-through sum equals the sum rebuilt from codes alone.
        let mut rebuilt = Tensor::zeros((2, 32, 16), DType::F32, &Device::Cpu).unwrap();
        for (k, codes) in q.codes.iter().enumerate() {
            rebuilt = (rebuilt + vq.code_embedding(k, codes).unwrap()).unwrap();
        }
        let diff = (rebuilt - &q.full)
            .unwrap()
            .abs()
            .unwrap()
            .max_all()
            .unwrap();
        assert!(nn::scalar(&diff).unwrap() < 1e-5);
    }

    #[test]
    fn learns_to_reconstruct_a_few_targets() {
        let config = LncmConfig::tiny(272);
        let map = VarMap::new();
        let vb = VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu);
        let vq = VqVae::new(&vb, &config).unwrap();
        let codec = crate::Codec::bytes();
        let rows: Vec<Vec<u32>> = ["sí", "no", "Pedro no lo sabe."]
            .iter()
            .map(|t| codec.encode_target(None, t, 32).unwrap())
            .collect();
        let ids = nn::ids_tensor(&rows, &Device::Cpu).unwrap();
        let mut opt = candle_nn::AdamW::new(
            map.all_vars(),
            candle_nn::ParamsAdamW {
                lr: 3e-3,
                ..Default::default()
            },
        )
        .unwrap();
        let first = vq.loss(&ids, 3, Ctx::train(0.0)).unwrap();
        let mut last_accuracy = 0.0;
        for _ in 0..150 {
            let loss = vq.loss(&ids, 3, Ctx::train(0.0)).unwrap();
            last_accuracy = loss.accuracy;
            opt.backward_step(&loss.total).unwrap();
        }
        let last = vq.loss(&ids, 3, Ctx::EVAL).unwrap();
        assert!(
            last.reconstruction < first.reconstruction * 0.3,
            "{} → {}",
            first.reconstruction,
            last.reconstruction
        );
        assert!(last_accuracy > 0.9, "accuracy {last_accuracy}");
    }
}
