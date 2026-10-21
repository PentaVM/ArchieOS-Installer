//! IRIS Beta: two encoders, phrase conditioning, sparse-MoE fusion and a DETR decoder.
//!
//! A line-for-line port of `archie_iris_foundry.beta_model.IrisBeta1` for inference. Names
//! follow the PyTorch module tree so a tensor's key says where it is used. Training-only
//! parts (the balance loss, checkpointing, the JEPA target) are absent; so are the mask,
//! keypoint and reading heads, which nothing served reads.

use crate::nn::{l2_normalize, Attention, LayerNorm, Linear, Mlp, Result};
use crate::phrases::PhraseFeatures;
use crate::roi::roi_align;
use crate::siglip::SiglipVision;
use crate::vjepa::{Rope, VJepa};
use crate::weights::{ModelConfig, Weights};
use candle_core::{DType, Device, Tensor, D};
use std::sync::Arc;

const EPS: f64 = 1e-5;

/// `nn.Sequential(nn.LayerNorm, nn.Linear)`.
struct NormLinear {
    norm: LayerNorm,
    linear: Linear,
}

impl NormLinear {
    fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        Ok(Self {
            norm: LayerNorm::load(weights, &format!("{prefix}.0"), EPS)?,
            linear: Linear::load(weights, &format!("{prefix}.1"))?,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        self.linear.forward(&self.norm.forward(x)?)
    }
}

/// Rows per block of [`inclusive_counts`]: one `[SCAN_BLOCK, SCAN_BLOCK]` triangle per pass.
const SCAN_BLOCK: usize = 128;

/// Experts per batched matrix product, so open mode's hidden layer stays near 50 MiB.
const EXPERT_GROUP: usize = 8;

/// Running column sums of a `[N, E]` f32 count matrix, row `i` included, without leaving
/// the device. candle's `cumsum` multiplies by an `N x N` triangle (4.8 GB at open mode's
/// 34 816 assignments); this does it by blocks of [`SCAN_BLOCK`] rows and carries the block
/// totals the same way. Counts stay integers below 2^24, so every sum is exact in f32.
fn inclusive_counts(x: &Tensor, triangle: &Tensor) -> Result<Tensor> {
    let (rows, columns) = x.dims2()?;
    if rows <= SCAN_BLOCK {
        let lower = triangle
            .narrow(0, 0, rows)?
            .narrow(1, 0, rows)?
            .contiguous()?;
        return Ok(lower.matmul(x)?);
    }
    let blocks = rows.div_ceil(SCAN_BLOCK);
    let padded = blocks * SCAN_BLOCK;
    let x = if padded == rows {
        x.clone()
    } else {
        Tensor::cat(
            &[
                x,
                &Tensor::zeros((padded - rows, columns), x.dtype(), x.device())?,
            ],
            0,
        )?
    };
    let x = x.reshape((blocks, SCAN_BLOCK, columns))?;
    let within = triangle.unsqueeze(0)?.broadcast_matmul(&x)?;
    let totals = x.sum(1)?;
    let before = (inclusive_counts(&totals, triangle)? - &totals)?;
    Ok(within
        .broadcast_add(&before.unsqueeze(1)?)?
        .reshape((padded, columns))?
        .narrow(0, 0, rows)?)
}

/// `value` in the shape and on the device of `like`, made there rather than copied from
/// the host.
fn filled(like: &Tensor, value: f64) -> Result<Tensor> {
    Ok((like.zeros_like()? + value)?)
}

/// Token-routed top-k experts with a fixed capacity, plus an always-on shared expert.
struct SparseMoe {
    router: Linear,
    shared: Mlp,
    w_in: Tensor,
    b_in: Tensor,
    w_out: Tensor,
    b_out: Tensor,
    experts: usize,
    top_k: usize,
    capacity_factor: f64,
    /// `0..experts` as f32, on the weights' device.
    expert_ids: Tensor,
    /// Lower-triangular ones, `[SCAN_BLOCK, SCAN_BLOCK]` f32, for [`inclusive_counts`].
    triangle: Tensor,
}

impl SparseMoe {
    fn load(weights: &Weights, prefix: &str, config: &ModelConfig) -> Result<Self> {
        let w_in = weights.get(&format!("{prefix}.w_in"))?;
        let device = w_in.device().clone();
        Ok(Self {
            router: Linear::load(weights, &format!("{prefix}.router"))?,
            shared: Mlp::load(
                weights,
                &format!("{prefix}.shared.0"),
                &format!("{prefix}.shared.2"),
                true,
            )?,
            w_in,
            b_in: weights.get(&format!("{prefix}.b_in"))?,
            w_out: weights.get(&format!("{prefix}.w_out"))?,
            b_out: weights.get(&format!("{prefix}.b_out"))?,
            experts: config.experts,
            top_k: config.expert_top_k,
            capacity_factor: config.expert_capacity_factor,
            expert_ids: Tensor::arange(0u32, config.experts as u32, &device)?
                .to_dtype(DType::F32)?,
            triangle: Tensor::tril2(SCAN_BLOCK, DType::F32, &device)?,
        })
    }

    fn capacity(&self, tokens: usize) -> usize {
        ((self.capacity_factor * (tokens * self.top_k) as f64 / self.experts as f64).ceil()
            as usize)
            .max(4)
    }

    /// Every token of the batch competes for the same capacity, in batch-major order: the
    /// same rows `SparseMoeFeedForward` flattens. A token over an expert's capacity keeps
    /// only its shared-expert path, as in training.
    ///
    /// Both routes give the same numbers. On the CPU reading the router back costs nothing
    /// and only the assigned rows are multiplied; on a GPU every read back drains it.
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let shape = x.shape().clone();
        let width = x.dim(D::Minus1)?;
        let flat = x.reshape(((), width))?;
        let output = if flat.device().is_cpu() {
            self.route_on_host(&flat)?
        } else {
            self.route_on_device(&flat)?
        };
        Ok(output.reshape(shape)?)
    }

    /// Probabilities read back, a stable sort per token, one product per expert, added in
    /// expert order.
    fn route_on_host(&self, flat: &Tensor) -> Result<Tensor> {
        let tokens = flat.dim(0)?;
        let probabilities: Vec<f32> =
            candle_nn::ops::softmax_last_dim(&self.router.forward(flat)?.to_dtype(DType::F32)?)?
                .flatten_all()?
                .to_vec1()?;
        let capacity = self.capacity(tokens);
        let mut assigned: Vec<(Vec<u32>, Vec<f32>)> = vec![(Vec::new(), Vec::new()); self.experts];
        let mut order: Vec<usize> = (0..self.experts).collect();
        for token in 0..tokens {
            let row = &probabilities[token * self.experts..(token + 1) * self.experts];
            order.sort_by(|&a, &b| row[b].total_cmp(&row[a]).then(a.cmp(&b)));
            let chosen = &order[..self.top_k];
            let total: f32 = chosen.iter().map(|&e| row[e]).sum();
            for &expert in chosen {
                let (ids, weights) = &mut assigned[expert];
                if ids.len() < capacity {
                    ids.push(token as u32);
                    weights.push(row[expert] / total);
                }
            }
        }
        let mut output = self.shared.forward(flat)?;
        let dtype = flat.dtype();
        for (expert, (ids, weights)) in assigned.into_iter().enumerate() {
            if ids.is_empty() {
                continue;
            }
            let count = ids.len();
            let ids = Tensor::from_vec(ids, count, flat.device())?;
            let gathered = flat.index_select(&ids, 0)?;
            let hidden = gathered
                .matmul(&self.w_in.get(expert)?)?
                .broadcast_add(&self.b_in.get(expert)?)?;
            let hidden = crate::fast::gelu_erf(&hidden)?;
            let routed = hidden
                .matmul(&self.w_out.get(expert)?)?
                .broadcast_add(&self.b_out.get(expert)?)?;
            let scale = Tensor::from_vec(weights, (count, 1), flat.device())?.to_dtype(dtype)?;
            output = output.index_add(&ids, &routed.broadcast_mul(&scale)?, 0)?;
        }
        Ok(output)
    }

    /// Nothing read back, as in `SparseMoeFeedForward`: top-k by repeated maxima (ties to
    /// the lower expert, as the stable sort), queue positions by running counts over the
    /// `(token, k)` order, one batched product per group of experts. Each token adds its
    /// experts' outputs in ascending expert order, the order the host route adds them, so
    /// the sums round the same way.
    fn route_on_device(&self, flat: &Tensor) -> Result<Tensor> {
        let (tokens, width) = flat.dims2()?;
        let device = flat.device();
        let dtype = flat.dtype();
        let experts = self.experts;
        let k = self.top_k;
        let capacity = self.capacity(tokens);
        let probabilities =
            candle_nn::ops::softmax_last_dim(&self.router.forward(flat)?.to_dtype(DType::F32)?)?;

        // Top-k: the largest probability, the lowest expert holding it, then mask it out.
        // Probabilities are never negative, so -1 sits below every remaining one.
        let ids = self
            .expert_ids
            .unsqueeze(0)?
            .broadcast_as((tokens, experts))?;
        let above = filled(&probabilities, experts as f64)?;
        let below = filled(&probabilities, -1.0)?;
        let mut remaining = probabilities.clone();
        let mut chosen = Vec::with_capacity(k);
        let mut chosen_probability = Vec::with_capacity(k);
        for _ in 0..k {
            let best = remaining.max_keepdim(1)?;
            let holds = remaining.broadcast_eq(&best)?;
            let expert = holds.where_cond(&ids, &above)?.min_keepdim(1)?;
            let one_hot = ids.broadcast_eq(&expert)?;
            remaining = one_hot.where_cond(&below, &remaining)?;
            chosen.push(expert);
            chosen_probability.push(best);
        }
        let chosen = Tensor::cat(&chosen, 1)?; // [T, k], expert ids as f32
        let chosen_probability = Tensor::cat(&chosen_probability, 1)?;
        let total = chosen_probability.sum_keepdim(1)?;
        let gate = chosen_probability.broadcast_div(&total)?.to_dtype(dtype)?;

        // Queue position of each (token, k) at its expert, in the order the tokens arrive.
        let assignments = tokens * k;
        let chosen_flat = chosen.reshape((assignments, 1))?;
        let one_hot = self
            .expert_ids
            .unsqueeze(0)?
            .broadcast_eq(&chosen_flat)?
            .to_dtype(DType::F32)?;
        let position =
            ((inclusive_counts(&one_hot, &self.triangle)? * &one_hot)?.sum_keepdim(1)? - 1.0)?;
        let kept = position.lt(&filled(&position, capacity as f64)?)?;
        let slots = experts * capacity;
        // Dropped assignments all land in one spare row past the last slot.
        let slot = kept.where_cond(
            &((&chosen_flat * capacity as f64)? + &position)?,
            &filled(&position, slots as f64)?,
        )?;
        let slot = slot.flatten_all()?.to_dtype(DType::U32)?;

        // Each token once per choice, in the same (token, k) order as `slot`.
        let repeated = flat
            .unsqueeze(1)?
            .broadcast_as((tokens, k, width))?
            .reshape((assignments, width))?;
        let dispatched =
            Tensor::zeros((slots + 1, width), dtype, device)?.index_add(&slot, &repeated, 0)?;

        let mut routed = Vec::with_capacity(experts.div_ceil(EXPERT_GROUP) + 1);
        for first in (0..experts).step_by(EXPERT_GROUP) {
            let count = EXPERT_GROUP.min(experts - first);
            let input = dispatched
                .narrow(0, first * capacity, count * capacity)?
                .reshape((count, capacity, width))?;
            let hidden = input
                .matmul(&self.w_in.narrow(0, first, count)?)?
                .broadcast_add(&self.b_in.narrow(0, first, count)?.reshape((count, 1, ()))?)?;
            let hidden = crate::fast::gelu_erf(&hidden)?;
            let out = hidden
                .matmul(&self.w_out.narrow(0, first, count)?)?
                .broadcast_add(
                    &self
                        .b_out
                        .narrow(0, first, count)?
                        .reshape((count, 1, ()))?,
                )?;
            routed.push(out.reshape((count * capacity, width))?);
        }
        routed.push(Tensor::zeros((1, width), dtype, device)?);
        let routed = Tensor::cat(&routed, 0)?;
        let contribution = routed
            .index_select(&slot, 0)?
            .broadcast_mul(&gate.reshape((assignments, 1))?)?
            .reshape((tokens, k, width))?;

        // Rank of each choice among the token's k experts, lowest expert first.
        let rank = chosen
            .unsqueeze(1)?
            .broadcast_lt(&chosen.unsqueeze(2)?)?
            .to_dtype(DType::F32)?
            .sum(2)?; // [T, k]
        let mut output = self.shared.forward(flat)?;
        for r in 0..k {
            let pick = rank
                .eq(&filled(&rank, r as f64)?)?
                .to_dtype(dtype)?
                .unsqueeze(2)?;
            output = (output + contribution.broadcast_mul(&pick)?.sum(1)?)?;
        }
        Ok(output)
    }
}

/// Runs `f` over slices of the batch so a wide batch does not hold every attention matrix
/// at once (seventeen 1024 x 1024 x 8 maps in "everything" mode).
fn in_slices(batch: usize, f: impl Fn(usize, usize) -> Result<Tensor>) -> Result<Tensor> {
    const SLICE: usize = 4;
    if batch <= SLICE {
        return f(0, batch);
    }
    let parts = (0..batch)
        .step_by(SLICE)
        .map(|start| f(start, SLICE.min(batch - start)))
        .collect::<Result<Vec<_>>>()?;
    Ok(Tensor::cat(&parts, 0)?)
}

struct FusionLayer {
    image_norm1: LayerNorm,
    image_self: Attention,
    image_norm2: LayerNorm,
    image_text: Attention,
    text_norm1: LayerNorm,
    text_image: Attention,
    image_norm3: LayerNorm,
    moe: SparseMoe,
    text_norm2: LayerNorm,
    text_ffn: Mlp,
}

impl FusionLayer {
    fn load(weights: &Weights, prefix: &str, config: &ModelConfig) -> Result<Self> {
        let h = config.heads;
        Ok(Self {
            image_norm1: LayerNorm::load(weights, &format!("{prefix}.image_norm1"), EPS)?,
            image_self: Attention::load(weights, &format!("{prefix}.image_self"), h)?,
            image_norm2: LayerNorm::load(weights, &format!("{prefix}.image_norm2"), EPS)?,
            image_text: Attention::load(weights, &format!("{prefix}.image_text"), h)?,
            text_norm1: LayerNorm::load(weights, &format!("{prefix}.text_norm1"), EPS)?,
            text_image: Attention::load(weights, &format!("{prefix}.text_image"), h)?,
            image_norm3: LayerNorm::load(weights, &format!("{prefix}.image_norm3"), EPS)?,
            moe: SparseMoe::load(weights, &format!("{prefix}.moe"), config)?,
            text_norm2: LayerNorm::load(weights, &format!("{prefix}.text_norm2"), EPS)?,
            text_ffn: Mlp::load(
                weights,
                &format!("{prefix}.text_ffn.0"),
                &format!("{prefix}.text_ffn.2"),
                true,
            )?,
        })
    }

    fn forward(
        &self,
        image: &Tensor,
        position: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        let batch = image.dim(0)?;
        let normed = self.image_norm1.forward(image)?;
        let query = normed.broadcast_add(position)?;
        let attended = in_slices(batch, |start, len| {
            let q = query.narrow(0, start, len)?;
            self.image_self
                .forward(&q, &q, &normed.narrow(0, start, len)?, None)
        })?;
        let image = (image + attended)?;
        let query = self.image_norm2.forward(&image)?.broadcast_add(position)?;
        let image = (&image
            + self
                .image_text
                .forward(&query, text, text, Some(text_padding))?)?;
        let keys = image.broadcast_add(position)?;
        let text = (text
            + self
                .text_image
                .forward(&self.text_norm1.forward(text)?, &keys, &image, None)?)?;
        let image = (&image + self.moe.forward(&self.image_norm3.forward(&image)?)?)?;
        let text = (&text + self.text_ffn.forward(&self.text_norm2.forward(&text)?)?)?;
        Ok((image, text))
    }
}

struct DecoderLayer {
    norm1: LayerNorm,
    self_attention: Attention,
    norm2: LayerNorm,
    image_attention: Attention,
    norm3: LayerNorm,
    text_attention: Attention,
    norm4: LayerNorm,
    ffn: Mlp,
    box_position: Mlp,
}

impl DecoderLayer {
    fn load(weights: &Weights, prefix: &str, heads: usize) -> Result<Self> {
        Ok(Self {
            norm1: LayerNorm::load(weights, &format!("{prefix}.norm1"), EPS)?,
            self_attention: Attention::load(weights, &format!("{prefix}.self_attention"), heads)?,
            norm2: LayerNorm::load(weights, &format!("{prefix}.norm2"), EPS)?,
            image_attention: Attention::load(weights, &format!("{prefix}.image_attention"), heads)?,
            norm3: LayerNorm::load(weights, &format!("{prefix}.norm3"), EPS)?,
            text_attention: Attention::load(weights, &format!("{prefix}.text_attention"), heads)?,
            norm4: LayerNorm::load(weights, &format!("{prefix}.norm4"), EPS)?,
            ffn: Mlp::load(
                weights,
                &format!("{prefix}.ffn.0"),
                &format!("{prefix}.ffn.2"),
                true,
            )?,
            box_position: Mlp::load(
                weights,
                &format!("{prefix}.box_position.0"),
                &format!("{prefix}.box_position.2"),
                true,
            )?,
        })
    }

    fn forward(
        &self,
        queries: &Tensor,
        reference: &Tensor,
        image_keys: &Tensor,
        image: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<Tensor> {
        let position = self.box_position.forward(reference)?;
        let normed = self.norm1.forward(queries)?;
        let q = (&normed + &position)?;
        let queries = (queries + self.self_attention.forward(&q, &q, &normed, None)?)?;
        let q = (self.norm2.forward(&queries)? + &position)?;
        let queries = (&queries + self.image_attention.forward(&q, image_keys, image, None)?)?;
        let queries = (&queries
            + self.text_attention.forward(
                &self.norm3.forward(&queries)?,
                text,
                text,
                Some(text_padding),
            )?)?;
        Ok((&queries + self.ffn.forward(&self.norm4.forward(&queries)?)?)?)
    }
}

/// Scaled cosine between regions and phrases.
struct PhraseScorer {
    region: Linear,
    phrase: Linear,
    scale: f64,
    bias: f64,
}

impl PhraseScorer {
    fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        let log_scale: f32 = weights
            .get(&format!("{prefix}.log_scale"))?
            .to_dtype(DType::F32)?
            .to_scalar()?;
        let bias: f32 = weights
            .get(&format!("{prefix}.bias"))?
            .to_dtype(DType::F32)?
            .to_scalar()?;
        Ok(Self {
            region: Linear::load(weights, &format!("{prefix}.region"))?,
            phrase: Linear::load(weights, &format!("{prefix}.phrase"))?,
            scale: f64::from(log_scale.exp().min(100.0)),
            bias: f64::from(bias),
        })
    }

    /// `[B, Q, P]` in f32; padded phrases score -1e4.
    fn forward(&self, regions: &Tensor, phrases: &Tensor, padding: &Tensor) -> Result<Tensor> {
        let region = l2_normalize(&self.region.forward(regions)?.to_dtype(DType::F32)?)?;
        let phrase = l2_normalize(&self.phrase.forward(phrases)?.to_dtype(DType::F32)?)?;
        let logits =
            ((region.matmul(&phrase.transpose(1, 2)?.contiguous()?)? * self.scale)? + self.bias)?;
        let (batch, queries, width) = logits.dims3()?;
        let blocked = Tensor::full(-1e4f32, (batch, queries, width), logits.device())?;
        let mask = padding
            .unsqueeze(1)?
            .broadcast_as((batch, queries, width))?;
        Ok(mask.where_cond(&blocked, &logits)?)
    }
}

/// `sigmoid(inverse_sigmoid(reference) + mlp(features))`, in f32.
struct BoxHead {
    first: Linear,
    second: Linear,
    third: Linear,
}

impl BoxHead {
    fn load(weights: &Weights, prefix: &str) -> Result<Self> {
        Ok(Self {
            first: Linear::load(weights, &format!("{prefix}.mlp.0"))?,
            second: Linear::load(weights, &format!("{prefix}.mlp.2"))?,
            third: Linear::load(weights, &format!("{prefix}.mlp.4"))?,
        })
    }

    fn forward(&self, features: &Tensor, reference: &Tensor) -> Result<Tensor> {
        let hidden = crate::fast::gelu_erf(&self.first.forward(features)?)?;
        let hidden = crate::fast::gelu_erf(&self.second.forward(&hidden)?)?;
        let delta = self.third.forward(&hidden)?.to_dtype(DType::F32)?;
        let reference = reference.to_dtype(DType::F32)?.clamp(1e-4, 1.0 - 1e-4)?;
        let logit = (&reference / (1.0 - &reference)?)?.log()?;
        Ok(candle_nn::ops::sigmoid(&(logit + delta)?)?)
    }
}

/// Phrases ready for fusion: one row per image of the batch.
#[derive(Clone)]
pub struct EncodedPhrases {
    /// `[B, P, D]`.
    pub vectors: Tensor,
    /// `[B, P]` u8, 1 for an empty phrase slot.
    pub phrase_padding: Tensor,
    /// `[B, L, D]`.
    pub text: Tensor,
    /// `[B, L]` u8, 1 for padding.
    pub text_padding: Tensor,
    /// `[B, L, P]` f32: which phrase each text token belongs to.
    pub membership: Tensor,
    pub widths: Vec<usize>,
}

/// What decoding returns, every tensor on the model's device.
pub struct Decoded {
    /// `[B, Q, P]` f32 logits.
    pub logits: Tensor,
    /// `[B, Q, 4]` cx, cy, w, h on the canvas, in [0, 1].
    pub boxes: Tensor,
}

pub struct IrisModel {
    pub config: ModelConfig,
    pub device: Device,
    pub dtype: DType,
    pub grid: usize,
    vjepa: VJepa,
    rope: Rope,
    siglip: SiglipVision,
    vjepa_projection: NormLinear,
    siglip_projection: NormLinear,
    text_projection: NormLinear,
    text_token_projection: NormLinear,
    phrase_slot: Tensor,
    pub position: Tensor,
    pub null_phrase: Tensor,
    fusion: Vec<FusionLayer>,
    fused_norm: LayerNorm,
    token_scorer: PhraseScorer,
    token_box: BoxHead,
    query_projection_linear: Linear,
    query_projection_norm: LayerNorm,
    decoder: Vec<DecoderLayer>,
    decoder_scorers: Vec<PhraseScorer>,
    decoder_boxes: Vec<BoxHead>,
    instance_norm: LayerNorm,
    instance_first: Linear,
    instance_second: Linear,
    /// `[N, 4]` f32 anchors: each token's cell centre and a two-cell box.
    pub anchors: Tensor,
    anchors_host: Vec<[f32; 4]>,
}

impl IrisModel {
    pub fn load(weights: &Weights) -> Result<Self> {
        let config = weights.release.config.clone();
        let sample = weights.get("fused_norm.weight")?;
        let device = sample.device().clone();
        let dtype = sample.dtype();
        let grid = config.image_size / 16;
        let fusion = (0..config.fusion_layers)
            .map(|i| FusionLayer::load(weights, &format!("fusion.{i}"), &config))
            .collect::<Result<Vec<_>>>()?;
        let decoder = (0..config.decoder_layers)
            .map(|i| DecoderLayer::load(weights, &format!("decoder.{i}"), config.heads))
            .collect::<Result<Vec<_>>>()?;
        let decoder_scorers = (0..config.decoder_layers)
            .map(|i| PhraseScorer::load(weights, &format!("decoder_scorers.{i}")))
            .collect::<Result<Vec<_>>>()?;
        let decoder_boxes = (0..config.decoder_layers)
            .map(|i| BoxHead::load(weights, &format!("decoder_boxes.{i}")))
            .collect::<Result<Vec<_>>>()?;
        let mut anchors_host = Vec::with_capacity(grid * grid);
        for y in 0..grid {
            for x in 0..grid {
                let g = grid as f32;
                anchors_host.push([(x as f32 + 0.5) / g, (y as f32 + 0.5) / g, 2.0 / g, 2.0 / g]);
            }
        }
        let anchors = Tensor::from_vec(anchors_host.concat(), (grid * grid, 4), &device)?;
        Ok(Self {
            rope: Rope::new(grid, grid, &device, dtype)?,
            vjepa: VJepa::load(weights)?,
            siglip: SiglipVision::load(weights, grid)?,
            vjepa_projection: NormLinear::load(weights, "vjepa_projection")?,
            siglip_projection: NormLinear::load(weights, "siglip_projection")?,
            text_projection: NormLinear::load(weights, "text_projection")?,
            text_token_projection: NormLinear::load(weights, "text_token_projection")?,
            phrase_slot: weights.get("phrase_slot.weight")?,
            position: weights.get("position")?,
            null_phrase: weights.get("null_phrase")?,
            fusion,
            fused_norm: LayerNorm::load(weights, "fused_norm", EPS)?,
            token_scorer: PhraseScorer::load(weights, "token_scorer")?,
            token_box: BoxHead::load(weights, "token_box")?,
            query_projection_linear: Linear::load(weights, "query_projection.0")?,
            query_projection_norm: LayerNorm::load(weights, "query_projection.1", EPS)?,
            decoder,
            decoder_scorers,
            decoder_boxes,
            instance_norm: LayerNorm::load(weights, "instance_head.0", EPS)?,
            instance_first: Linear::load(weights, "instance_head.1")?,
            instance_second: Linear::load(weights, "instance_head.3")?,
            anchors,
            anchors_host,
            config,
            device,
            dtype,
            grid,
        })
    }

    /// `images`: `[B, 3, S, S]` in [0, 1]. Returns `[B, N, D]` tokens ready for fusion.
    pub fn encode_image(&self, images: &Tensor) -> Result<Tensor> {
        let images = images.to_device(&self.device)?;
        let vjepa = self.vjepa.forward(&images, &self.rope)?;
        let siglip = self.siglip.forward(&images)?;
        Ok(
            (self.vjepa_projection.forward(&vjepa)? + self.siglip_projection.forward(&siglip)?)?
                .broadcast_add(&self.position)?,
        )
    }

    /// The two encoders' own outputs, for the parity test.
    pub fn encode_parts(&self, images: &Tensor) -> Result<(Tensor, Tensor)> {
        let images = images.to_device(&self.device)?;
        Ok((
            self.vjepa.forward(&images, &self.rope)?,
            self.siglip.forward(&images)?,
        ))
    }

    /// `encode_phrases`: phrase vectors for scoring, phrase tokens for fusion. `features[b]`
    /// holds the text tower's output for each phrase of image `b`.
    pub fn encode_phrases(&self, features: &[Vec<Arc<PhraseFeatures>>]) -> Result<EncodedPhrases> {
        let batch = features.len();
        let d = self.config.dimension;
        let width = features.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let null = self.null_phrase.reshape((1, d))?;
        let all: Vec<&Arc<PhraseFeatures>> = features.iter().flatten().collect();
        if all.is_empty() {
            return Ok(EncodedPhrases {
                vectors: null
                    .unsqueeze(0)?
                    .broadcast_as((batch, width, d))?
                    .contiguous()?,
                phrase_padding: Tensor::ones((batch, width), DType::U8, &self.device)?,
                text: null
                    .unsqueeze(0)?
                    .broadcast_as((batch, 1, d))?
                    .contiguous()?,
                text_padding: Tensor::zeros((batch, 1), DType::U8, &self.device)?,
                membership: Tensor::zeros((batch, 1, width), DType::F32, &self.device)?,
                widths: vec![0; batch],
            });
        }
        let text_width = all[0].pooled.len();
        let pooled: Vec<f32> = all.iter().flat_map(|f| f.pooled.iter().copied()).collect();
        let pooled = Tensor::from_vec(pooled, (all.len(), text_width), &self.device)?
            .to_dtype(self.dtype)?;
        let projected = self.text_projection.forward(&pooled)?;
        let states: Vec<f32> = all
            .iter()
            .flat_map(|f| f.states.iter().flatten().copied())
            .collect();
        let total_tokens = all.iter().map(|f| f.states.len()).sum::<usize>();
        let states = Tensor::from_vec(states, (total_tokens, text_width), &self.device)?
            .to_dtype(self.dtype)?;
        let token_values = self.text_token_projection.forward(&states)?;

        let lengths: Vec<usize> = features
            .iter()
            .map(|items| items.iter().map(|f| f.states.len()).sum())
            .collect();
        let length = lengths.iter().copied().max().unwrap_or(0).max(1);
        let mut vector_rows = Vec::with_capacity(batch);
        let mut text_rows = Vec::with_capacity(batch);
        let mut phrase_padding = vec![1u8; batch * width];
        let mut text_padding = vec![1u8; batch * length];
        let mut membership = vec![0f32; batch * length * width];
        let (mut phrase_cursor, mut token_cursor) = (0usize, 0usize);
        for (row, items) in features.iter().enumerate() {
            let mut vector_parts = Vec::new();
            if !items.is_empty() {
                vector_parts.push(projected.narrow(0, phrase_cursor, items.len())?);
            }
            if width > items.len() {
                vector_parts.push(null.broadcast_as((width - items.len(), d))?.contiguous()?);
            }
            vector_rows.push(Tensor::cat(&vector_parts, 0)?);
            let mut text_parts = Vec::new();
            let mut position = 0;
            for (local, item) in items.iter().enumerate() {
                phrase_padding[row * width + local] = 0;
                let count = item.states.len();
                let slot = self
                    .phrase_slot
                    .narrow(0, local.min(self.config.max_phrases - 1), 1)?;
                text_parts.push(
                    token_values
                        .narrow(0, token_cursor, count)?
                        .broadcast_add(&slot)?,
                );
                for offset in 0..count {
                    text_padding[row * length + position + offset] = 0;
                    membership[(row * length + position + offset) * width + local] = 1.0;
                }
                position += count;
                token_cursor += count;
            }
            if length > position {
                text_parts.push(null.broadcast_as((length - position, d))?.contiguous()?);
            }
            if position == 0 {
                // A sample with no phrase attends to the null token rather than to nothing.
                text_padding[row * length] = 0;
            }
            text_rows.push(Tensor::cat(&text_parts, 0)?);
            phrase_cursor += items.len();
        }
        Ok(EncodedPhrases {
            vectors: Tensor::stack(&vector_rows, 0)?,
            phrase_padding: Tensor::from_vec(phrase_padding, (batch, width), &self.device)?,
            text: Tensor::stack(&text_rows, 0)?,
            text_padding: Tensor::from_vec(text_padding, (batch, length), &self.device)?,
            membership: Tensor::from_vec(membership, (batch, length, width), &self.device)?,
            widths: features.iter().map(Vec::len).collect(),
        })
    }

    /// The fusion stack. Returns the normed image tokens and the fused text.
    pub fn fuse(
        &self,
        tokens: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        let mut image = tokens.clone();
        let mut text = text.clone();
        for layer in &self.fusion {
            (image, text) = layer.forward(&image, &self.position, &text, text_padding)?;
        }
        Ok((self.fused_norm.forward(&image)?, text))
    }

    /// `decode_fused` without the extras: phrase scores and boxes for every query.
    pub fn decode(
        &self,
        fused: &Tensor,
        text: &Tensor,
        encoded: &EncodedPhrases,
    ) -> Result<Decoded> {
        let phrase_values = self.phrase_representations(encoded, text)?;
        let batch = fused.dim(0)?;
        let anchors = self
            .anchors
            .to_dtype(self.dtype)?
            .unsqueeze(0)?
            .broadcast_as((batch, self.grid * self.grid, 4))?;
        let token_logits =
            self.token_scorer
                .forward(fused, &phrase_values, &encoded.phrase_padding)?;
        let token_boxes = self.token_box.forward(fused, &anchors)?;
        let count = self.config.queries.min(fused.dim(1)?);
        let confidence: Vec<Vec<f32>> = token_logits.max(D::Minus1)?.to_vec2()?;
        let mut selected = Vec::with_capacity(batch * count);
        for row in &confidence {
            let mut order: Vec<u32> = (0..row.len() as u32).collect();
            order.sort_by(|&a, &b| row[b as usize].total_cmp(&row[a as usize]).then(a.cmp(&b)));
            selected.extend_from_slice(&order[..count]);
        }
        let mut queries = Vec::with_capacity(batch);
        let mut references = Vec::with_capacity(batch);
        for row in 0..batch {
            let ids = Tensor::from_vec(
                selected[row * count..(row + 1) * count].to_vec(),
                count,
                &self.device,
            )?;
            queries.push(fused.get(row)?.index_select(&ids, 0)?);
            references.push(token_boxes.get(row)?.index_select(&ids, 0)?);
        }
        let mut queries = self.query_projection_norm.forward(
            &self
                .query_projection_linear
                .forward(&Tensor::stack(&queries, 0)?)?,
        )?;
        let mut reference = Tensor::stack(&references, 0)?;
        let image_keys = fused.broadcast_add(&self.position)?;
        let mut last = None;
        for ((layer, scorer), box_head) in self
            .decoder
            .iter()
            .zip(&self.decoder_scorers)
            .zip(&self.decoder_boxes)
        {
            queries = layer.forward(
                &queries,
                &reference.to_dtype(self.dtype)?,
                &image_keys,
                fused,
                text,
                &encoded.text_padding,
            )?;
            let boxes = box_head.forward(&queries, &reference)?;
            last = Some((scorer, boxes.clone()));
            reference = boxes;
        }
        let (scorer, boxes) = last.expect("at least one decoder layer");
        Ok(Decoded {
            logits: scorer.forward(&queries, &phrase_values, &encoded.phrase_padding)?,
            boxes,
        })
    }

    /// Phrase vector plus the mean of that phrase's fused tokens.
    fn phrase_representations(
        &self,
        encoded: &EncodedPhrases,
        fused_text: &Tensor,
    ) -> Result<Tensor> {
        let membership = encoded.membership.to_dtype(self.dtype)?;
        let totals = membership
            .transpose(1, 2)?
            .contiguous()?
            .matmul(fused_text)?;
        let counts = membership
            .sum(1)?
            .clamp(1.0, f64::INFINITY)?
            .unsqueeze(D::Minus1)?;
        Ok((&encoded.vectors + totals.broadcast_div(&counts)?)?)
    }

    /// Re-identification embeddings for boxes (canvas-normalised xyxy) of one image's fused
    /// map `[N, D]`: `roi_align` 3 x 3, then the instance head, unit length.
    pub fn instance_embeddings(&self, fused: &Tensor, boxes: &[[f32; 4]]) -> Result<Vec<Vec<f32>>> {
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        let d = self.config.dimension;
        let g = self.grid;
        // [N, D] -> [D, G, G], channel-major for roi_align.
        let map: Vec<f32> = fused
            .to_dtype(DType::F32)?
            .t()?
            .contiguous()?
            .flatten_all()?
            .to_vec1()?;
        let mut pooled = Vec::with_capacity(boxes.len() * d * 9);
        for b in boxes {
            let scaled = [
                b[0] * g as f32,
                b[1] * g as f32,
                b[2] * g as f32,
                b[3] * g as f32,
            ];
            pooled.extend(roi_align(&map, d, g, g, scaled, 3));
        }
        let pooled =
            Tensor::from_vec(pooled, (boxes.len(), d * 9), &self.device)?.to_dtype(self.dtype)?;
        let hidden = crate::fast::gelu_erf(
            &self
                .instance_first
                .forward(&self.instance_norm.forward(&pooled)?)?,
        )?;
        let embedded = l2_normalize(
            &self
                .instance_second
                .forward(&hidden)?
                .to_dtype(DType::F32)?,
        )?;
        Ok(embedded.to_vec2()?)
    }

    pub fn anchor(&self, index: usize) -> [f32; 4] {
        self.anchors_host[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weights::Release;
    use std::collections::HashMap;
    use std::path::PathBuf;

    const WIDTH: usize = 16;
    const HIDDEN: usize = 32;

    fn config(experts: usize, top_k: usize) -> ModelConfig {
        ModelConfig {
            image_size: 64,
            dimension: WIDTH,
            heads: 4,
            feedforward: HIDDEN,
            fusion_layers: 1,
            decoder_layers: 1,
            experts,
            expert_top_k: top_k,
            expert_capacity_factor: 1.5,
            queries: 10,
            max_phrases: 4,
            text_length: 16,
            text_tokens_per_phrase: 1,
            act_head: false,
            act_layers: 1,
            act_slots: 4,
            act_chunk: 1,
            siglip_repository: String::new(),
            siglip_revision: String::new(),
        }
    }

    /// A MoE on random weights. The router leans on the first `favourites` experts so their
    /// queues overflow, and has no bias, so a zero token sees every expert tied.
    fn moe(experts: usize, top_k: usize, favourites: usize) -> SparseMoe {
        let device = Device::Cpu;
        let randn = |shape: &[usize]| Tensor::randn(0f32, 0.5, shape, &device).unwrap();
        let mut router = randn(&[experts, WIDTH]).to_vec2::<f32>().unwrap();
        router[..favourites]
            .iter_mut()
            .for_each(|row| row.iter_mut().for_each(|v| *v = 1.0));
        let router: Vec<f32> = router.into_iter().flatten().collect();
        let mut map = HashMap::new();
        let mut put = |name: &str, tensor: Tensor| {
            map.insert(format!("moe.{name}"), tensor);
        };
        put(
            "router.weight",
            Tensor::from_vec(router, (experts, WIDTH), &device).unwrap(),
        );
        put("shared.0.weight", randn(&[HIDDEN, WIDTH]));
        put("shared.0.bias", randn(&[HIDDEN]));
        put("shared.2.weight", randn(&[WIDTH, HIDDEN]));
        put("shared.2.bias", randn(&[WIDTH]));
        put("w_in", randn(&[experts, WIDTH, HIDDEN]));
        put("b_in", randn(&[experts, 1, HIDDEN]));
        put("w_out", randn(&[experts, HIDDEN, WIDTH]));
        put("b_out", randn(&[experts, 1, WIDTH]));
        let config = config(experts, top_k);
        let weights = Weights::from_tensors(
            map,
            Release {
                path: PathBuf::from("test.pt"),
                sha256: "a".repeat(64),
                format: "archie-iris-beta1-weights-v1".to_owned(),
                step: None,
                score: None,
                config: config.clone(),
            },
        );
        SparseMoe::load(&weights, "moe", &config).unwrap()
    }

    fn on_host(moe: &SparseMoe, x: &Tensor) -> Tensor {
        moe.route_on_host(&x.reshape(((), WIDTH)).unwrap()).unwrap()
    }

    fn on_device(moe: &SparseMoe, x: &Tensor) -> Tensor {
        moe.route_on_device(&x.reshape(((), WIDTH)).unwrap())
            .unwrap()
    }

    /// Batch of `rows x columns` random tokens; every seventh is zero, a full tie.
    fn tokens(rows: usize, columns: usize) -> Tensor {
        let x = Tensor::randn(0f32, 1.0, (rows, columns, WIDTH), &Device::Cpu).unwrap();
        let keep: Vec<f32> = (0..rows * columns)
            .map(|i| if i % 7 == 3 { 0.0 } else { 1.0 })
            .collect();
        let keep = Tensor::from_vec(keep, (rows, columns, 1), &Device::Cpu).unwrap();
        x.broadcast_mul(&keep).unwrap()
    }

    fn max_difference(a: &Tensor, b: &Tensor) -> f32 {
        (a - b)
            .unwrap()
            .abs()
            .unwrap()
            .flatten_all()
            .unwrap()
            .max(0)
            .unwrap()
            .to_scalar()
            .unwrap()
    }

    #[test]
    fn inclusive_counts_match_a_running_sum() {
        let triangle = Tensor::tril2(SCAN_BLOCK, DType::F32, &Device::Cpu).unwrap();
        for rows in [
            1,
            5,
            SCAN_BLOCK,
            SCAN_BLOCK + 1,
            700,
            SCAN_BLOCK * SCAN_BLOCK + 3,
        ] {
            let values: Vec<f32> = (0..rows * 3).map(|i| ((i * 7919) % 3) as f32).collect();
            let x = Tensor::from_vec(values.clone(), (rows, 3), &Device::Cpu).unwrap();
            let got = inclusive_counts(&x, &triangle)
                .unwrap()
                .flatten_all()
                .unwrap()
                .to_vec1::<f32>()
                .unwrap();
            let mut running = [0f32; 3];
            let want: Vec<f32> = values
                .chunks(3)
                .flat_map(|row| {
                    for (total, v) in running.iter_mut().zip(row) {
                        *total += v;
                    }
                    running
                })
                .collect();
            assert_eq!(got, want, "{rows} rows");
        }
    }

    #[test]
    fn device_routing_matches_host_routing() {
        // 2 x 150 tokens: 600 assignments over two scan levels. 2 x 4200: 16 800, three.
        for (experts, top_k, rows, columns) in [(8, 2, 2, 150), (8, 2, 2, 4200), (12, 3, 3, 50)] {
            let moe = moe(experts, top_k, 1);
            let x = tokens(rows, columns);
            let device = on_device(&moe, &x);
            let host = on_host(&moe, &x);
            assert_eq!(device.dims(), host.dims());
            let difference = max_difference(&device, &host);
            assert!(
                difference <= 1e-5,
                "{experts} experts, top-{top_k}, {rows}x{columns}: max |diff| {difference:e}"
            );
        }
    }

    #[test]
    fn tokens_past_capacity_keep_only_the_shared_expert() {
        // Non-negative tokens all pick experts 0 and 1, so both queues fill at the same
        // token and every later one comes out as the shared expert alone.
        let moe = moe(8, 2, 2);
        let x = tokens(1, 400).abs().unwrap();
        let capacity = moe.capacity(400);
        assert!(capacity < 400);
        let shared = moe
            .shared
            .forward(&x.reshape(((), WIDTH)).unwrap())
            .unwrap();
        for routed in [on_host(&moe, &x), on_device(&moe, &x)] {
            let difference: Vec<f32> = (&routed - &shared)
                .unwrap()
                .abs()
                .unwrap()
                .max(1)
                .unwrap()
                .to_vec1()
                .unwrap();
            for (token, d) in difference.iter().enumerate() {
                assert_eq!(*d == 0.0, token >= capacity, "token {token}: {d:e}");
            }
        }
    }
}
