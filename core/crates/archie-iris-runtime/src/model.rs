//! IRIS Beta: two encoders, phrase conditioning, sparse-MoE fusion and a DETR decoder.
//!
//! A line-for-line port of `archie_iris_foundry.beta_model.IrisBeta1` for inference. Names
//! follow the PyTorch module tree so a tensor's key says where it is used. Training-only
//! parts (the balance loss, checkpointing, the JEPA target) are absent; so are the mask,
//! keypoint and reading heads, which nothing served reads.

use crate::nn::{l2_normalize, Attention, LayerNorm, Linear, Mlp, Result};
use crate::phrases::PhraseFeatures;
use crate::preprocess::{canvas_tensor, Frame};
use crate::roi::{roi_align, roi_align_matrix};
use crate::siglip::SiglipVision;
use crate::vjepa::{Rope, VJepa};
use crate::weights::{ModelConfig, Weights};
use candle_core::{DType, Device, Tensor, D};
use std::sync::{Arc, Mutex};

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

/// `value` in the shape, dtype and on the device of `like`, set there by one fill kernel
/// rather than copied from the host.
fn filled(like: &Tensor, value: f64) -> Result<Tensor> {
    let full = Tensor::full(value as f32, like.shape(), like.device())?;
    Ok(if like.dtype() == DType::F32 {
        full
    } else {
        full.to_dtype(like.dtype())?
    })
}

/// Token-routed top-k experts with a fixed capacity, plus an always-on shared expert.
struct SparseMoe {
    router: Linear,
    shared: Mlp,
    /// Expert weights on the device, one row per resident expert (all of them unless a
    /// residency plan says otherwise): `[R, D, H]`, `[R, 1, H]`, `[R, H, D]`, `[R, 1, D]`.
    w_in: Tensor,
    b_in: Tensor,
    w_out: Tensor,
    b_out: Tensor,
    /// Every expert's weights on the host, held only while some are not resident.
    host: Option<[Tensor; 4]>,
    /// Each expert's row in the device weights; `None` when it is not resident.
    row_of: Vec<Option<usize>>,
    /// `row_of` on the device as f32, -1 for an expert that is not resident.
    row_of_device: Tensor,
    experts: usize,
    top_k: usize,
    capacity_factor: f64,
    /// `0..experts` as f32, on the weights' device.
    expert_ids: Tensor,
    /// Lower-triangular ones, `[SCAN_BLOCK, SCAN_BLOCK]` f32, for [`inclusive_counts`].
    triangle: Tensor,
    /// Choices per expert while recording, `[E]` f32, accumulated where the route runs.
    usage: Mutex<Option<Tensor>>,
}

impl SparseMoe {
    fn load(weights: &Weights, prefix: &str, config: &ModelConfig) -> Result<Self> {
        let w_in = weights.get(&format!("{prefix}.w_in"))?;
        let device = w_in.device().clone();
        let experts = config.experts;
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
            host: None,
            row_of: (0..experts).map(Some).collect(),
            row_of_device: Tensor::arange(0u32, experts as u32, &device)?.to_dtype(DType::F32)?,
            experts,
            top_k: config.expert_top_k,
            capacity_factor: config.expert_capacity_factor,
            expert_ids: Tensor::arange(0u32, experts as u32, &device)?.to_dtype(DType::F32)?,
            triangle: Tensor::tril2(SCAN_BLOCK, DType::F32, &device)?,
            usage: Mutex::new(None),
        })
    }

    fn capacity(&self, tokens: usize) -> usize {
        ((self.capacity_factor * (tokens * self.top_k) as f64 / self.experts as f64).ceil()
            as usize)
            .max(4)
    }

    /// Bytes one expert takes on the device.
    fn expert_bytes(&self) -> usize {
        let per = |t: &Tensor| t.elem_count() / t.dim(0).unwrap_or(1).max(1);
        (per(&self.w_in) + per(&self.b_in) + per(&self.w_out) + per(&self.b_out))
            * self.w_in.dtype().size_in_bytes()
    }

    /// Keeps only `resident` experts on the device (`None`: all of them). The others act as
    /// experts at capacity 0: their tokens keep the shared expert, the overflow path of
    /// training. The full weights wait on the host while any expert is away.
    fn set_residency(&mut self, resident: Option<&[bool]>) -> Result<()> {
        let device = self.w_in.device().clone();
        if self.host.is_none() && self.row_of.iter().all(Option::is_some) {
            if resident.is_none() {
                return Ok(());
            }
            self.host = Some([
                self.w_in.to_device(&Device::Cpu)?,
                self.b_in.to_device(&Device::Cpu)?,
                self.w_out.to_device(&Device::Cpu)?,
                self.b_out.to_device(&Device::Cpu)?,
            ]);
        }
        let host = self
            .host
            .clone()
            .expect("the host holds the experts while some are away");
        let ids: Vec<u32> = match resident {
            None => (0..self.experts as u32).collect(),
            Some(mask) => {
                if mask.len() != self.experts {
                    return Err(crate::Error::Refused(format!(
                        "residency for {} experts, the layer has {}",
                        mask.len(),
                        self.experts
                    )));
                }
                (0..self.experts as u32)
                    .filter(|&e| mask[e as usize])
                    .collect()
            }
        };
        let index = Tensor::from_vec(ids.clone(), ids.len(), &Device::Cpu)?;
        let pick = |t: &Tensor| -> Result<Tensor> {
            Ok(t.index_select(&index, 0)?
                .contiguous()?
                .to_device(&device)?)
        };
        self.w_in = pick(&host[0])?;
        self.b_in = pick(&host[1])?;
        self.w_out = pick(&host[2])?;
        self.b_out = pick(&host[3])?;
        self.row_of = vec![None; self.experts];
        for (row, &expert) in ids.iter().enumerate() {
            self.row_of[expert as usize] = Some(row);
        }
        let rows: Vec<f32> = self
            .row_of
            .iter()
            .map(|r| r.map_or(-1.0, |r| r as f32))
            .collect();
        self.row_of_device = Tensor::from_vec(rows, self.experts, &device)?;
        if resident.is_none() {
            self.host = None;
        }
        Ok(())
    }

    fn record_usage(&self, on: bool) {
        if let Ok(mut usage) = self.usage.lock() {
            *usage = if on {
                Tensor::zeros(self.experts, DType::F32, self.expert_ids.device()).ok()
            } else {
                None
            };
        }
    }

    /// Choices per expert since recording started (reads the device).
    fn usage(&self) -> Result<Option<Vec<u64>>> {
        let usage = self
            .usage
            .lock()
            .map_err(|_| crate::Error::Refused("usage lock poisoned".into()))?;
        Ok(match usage.as_ref() {
            None => None,
            Some(t) => Some(t.to_vec1::<f32>()?.into_iter().map(|v| v as u64).collect()),
        })
    }

    fn add_usage(&self, counts: &Tensor) -> Result<()> {
        if let Ok(mut usage) = self.usage.lock() {
            if let Some(total) = usage.as_mut() {
                *total = (&*total + counts.to_device(total.device())?)?;
            }
        }
        Ok(())
    }

    /// The tokens of each of `groups` equal, consecutive parts of the batch compete for
    /// their own capacity, in batch-major order: one group is the batch, as
    /// `SparseMoeFeedForward` flattens it (open mode's rows of one image); a group per
    /// image runs each image as if alone (several foci in one batch). A token over an
    /// expert's capacity, or routed to an expert that is not resident, keeps only its
    /// shared-expert path, as in training.
    ///
    /// Both routes give the same numbers. On the CPU reading the router back costs nothing
    /// and only the assigned rows are multiplied; on a GPU every read back drains it.
    fn forward(&self, x: &Tensor, groups: usize) -> Result<Tensor> {
        let shape = x.shape().clone();
        let width = x.dim(D::Minus1)?;
        let flat = x.reshape(((), width))?;
        let groups = groups.max(1);
        if flat.dim(0)? % groups != 0 {
            return Err(crate::Error::Refused(format!(
                "{} tokens do not split into {groups} groups",
                flat.dim(0)?
            )));
        }
        let output = if flat.device().is_cpu() {
            self.route_on_host(&flat, groups)?
        } else {
            self.route_on_device(&flat, groups)?
        };
        Ok(output.reshape(shape)?)
    }

    /// Probabilities read back, a stable sort per token, one product per expert, added in
    /// expert order.
    fn route_on_host(&self, flat: &Tensor, groups: usize) -> Result<Tensor> {
        let tokens = flat.dim(0)?;
        let per_group = tokens / groups;
        let probabilities: Vec<f32> =
            candle_nn::ops::softmax_last_dim(&self.router.forward(flat)?.to_dtype(DType::F32)?)?
                .flatten_all()?
                .to_vec1()?;
        let capacity = self.capacity(per_group);
        let mut assigned: Vec<(Vec<u32>, Vec<f32>)> = vec![(Vec::new(), Vec::new()); self.experts];
        let mut queued = vec![0usize; groups * self.experts];
        let mut chosen_count = vec![0f32; self.experts];
        let mut order: Vec<usize> = (0..self.experts).collect();
        for token in 0..tokens {
            let group = token / per_group;
            let row = &probabilities[token * self.experts..(token + 1) * self.experts];
            order.sort_by(|&a, &b| row[b].total_cmp(&row[a]).then(a.cmp(&b)));
            let chosen = &order[..self.top_k];
            let total: f32 = chosen.iter().map(|&e| row[e]).sum();
            for &expert in chosen {
                chosen_count[expert] += 1.0;
                if self.row_of[expert].is_none() {
                    continue;
                }
                let queue = &mut queued[group * self.experts + expert];
                if *queue < capacity {
                    *queue += 1;
                    let (ids, weights) = &mut assigned[expert];
                    ids.push(token as u32);
                    weights.push(row[expert] / total);
                }
            }
        }
        self.add_usage(&Tensor::from_vec(chosen_count, self.experts, &Device::Cpu)?)?;
        let mut output = self.shared.forward(flat)?;
        let dtype = flat.dtype();
        for (expert, (ids, weights)) in assigned.into_iter().enumerate() {
            let Some(row) = self.row_of[expert] else {
                continue;
            };
            if ids.is_empty() {
                continue;
            }
            let count = ids.len();
            let ids = Tensor::from_vec(ids, count, flat.device())?;
            let gathered = flat.index_select(&ids, 0)?;
            let hidden = gathered
                .matmul(&self.w_in.get(row)?)?
                .broadcast_add(&self.b_in.get(row)?)?;
            let hidden = crate::fast::gelu_erf(&hidden)?;
            let routed = hidden
                .matmul(&self.w_out.get(row)?)?
                .broadcast_add(&self.b_out.get(row)?)?;
            let scale = Tensor::from_vec(weights, (count, 1), flat.device())?.to_dtype(dtype)?;
            output = output.index_add(&ids, &routed.broadcast_mul(&scale)?, 0)?;
        }
        Ok(output)
    }

    /// Nothing read back, as in `SparseMoeFeedForward`: top-k by repeated maxima (ties to
    /// the lower expert, as the stable sort), queue positions by running counts over the
    /// `(token, k)` order within each group, one batched product per group of resident
    /// experts. Each token adds its experts' outputs in ascending expert order, the order the
    /// host route adds them, so the sums round the same way.
    fn route_on_device(&self, flat: &Tensor, groups: usize) -> Result<Tensor> {
        let (tokens, width) = flat.dims2()?;
        let device = flat.device();
        let dtype = flat.dtype();
        let experts = self.experts;
        let k = self.top_k;
        let per_group = tokens / groups;
        let capacity = self.capacity(per_group);
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

        // Queue position of each (token, k) at its expert, in arrival order, per group.
        let assignments = tokens * k;
        let per_group_assignments = per_group * k;
        let chosen_flat = chosen.reshape((assignments, 1))?;
        let one_hot = self
            .expert_ids
            .unsqueeze(0)?
            .broadcast_eq(&chosen_flat)?
            .to_dtype(DType::F32)?;
        self.add_usage(&one_hot.sum(0)?)?;
        let counts = if groups == 1 {
            inclusive_counts(&one_hot, &self.triangle)?
        } else {
            let parts = (0..groups)
                .map(|g| {
                    inclusive_counts(
                        &one_hot.narrow(0, g * per_group_assignments, per_group_assignments)?,
                        &self.triangle,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            Tensor::cat(&parts, 0)?
        };
        let position = ((counts * &one_hot)?.sum_keepdim(1)? - 1.0)?;
        // The device row of each choice's expert; -1 when it is not resident.
        let row = self
            .row_of_device
            .index_select(&chosen_flat.flatten_all()?.to_dtype(DType::U32)?, 0)?
            .reshape((assignments, 1))?;
        let resident_rows = self.w_in.dim(0)?;
        let kept = (position
            .lt(&filled(&position, capacity as f64)?)?
            .to_dtype(DType::F32)?
            * row.ge(&filled(&row, 0.0)?)?.to_dtype(DType::F32)?)?
        .gt(&filled(&position, 0.5)?)?;
        // Slots: resident expert's rows together, each group's queue inside them.
        let group_slots = groups * capacity;
        let slots = resident_rows * group_slots;
        let group_start = if groups == 1 {
            filled(&position, 0.0)?
        } else {
            Tensor::arange(0u32, groups as u32, device)?
                .to_dtype(DType::F32)?
                .unsqueeze(1)?
                .broadcast_as((groups, per_group_assignments))?
                .reshape((assignments, 1))?
                .affine(capacity as f64, 0.0)?
        };
        let slot = kept.where_cond(
            &(((&row * group_slots as f64)? + group_start)? + &position)?,
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

        let mut routed = Vec::with_capacity(resident_rows.div_ceil(EXPERT_GROUP) + 1);
        for first in (0..resident_rows).step_by(EXPERT_GROUP) {
            let count = EXPERT_GROUP.min(resident_rows - first);
            let input = dispatched
                .narrow(0, first * group_slots, count * group_slots)?
                .reshape((count, group_slots, width))?;
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
            routed.push(out.reshape((count * group_slots, width))?);
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

    /// `groups`: how the MoE splits the batch's capacity ([`SparseMoe::forward`]).
    fn forward(
        &self,
        image: &Tensor,
        position: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
        groups: usize,
    ) -> Result<(Tensor, Tensor)> {
        let image = self.image_self_block(image, position)?;
        self.after_image_self(&image, position, text, text_padding, groups)
    }

    /// The layer on `batch` rows that all hold the same image `[1, N, D]`: the image
    /// self-attention depends on nothing else, so it runs once and the rows part at the
    /// cross-attention with their own text.
    fn forward_shared_image(
        &self,
        image: &Tensor,
        batch: usize,
        position: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        let image = self.image_self_block(image, position)?;
        let (_, tokens, width) = image.dims3()?;
        let image = image.broadcast_as((batch, tokens, width))?.contiguous()?;
        self.after_image_self(&image, position, text, text_padding, 1)
    }

    fn image_self_block(&self, image: &Tensor, position: &Tensor) -> Result<Tensor> {
        let batch = image.dim(0)?;
        let normed = self.image_norm1.forward(image)?;
        let query = normed.broadcast_add(position)?;
        if crate::nn::attention_is_fused(image.device(), image.dtype()) {
            let attended = self.image_self.forward(&query, &query, &normed, None)?;
            return Ok((image + attended)?);
        }
        let attended = in_slices(batch, |start, len| {
            let q = query.narrow(0, start, len)?;
            self.image_self
                .forward(&q, &q, &normed.narrow(0, start, len)?, None)
        })?;
        Ok((image + attended)?)
    }

    fn after_image_self(
        &self,
        image: &Tensor,
        position: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
        groups: usize,
    ) -> Result<(Tensor, Tensor)> {
        let query = self.image_norm2.forward(image)?.broadcast_add(position)?;
        let image = (image
            + self
                .image_text
                .forward(&query, text, text, Some(text_padding))?)?;
        let keys = image.broadcast_add(position)?;
        let text = (text
            + self
                .text_image
                .forward(&self.text_norm1.forward(text)?, &keys, &image, None)?)?;
        let image = (&image
            + self
                .moe
                .forward(&self.image_norm3.forward(&image)?, groups)?)?;
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

/// The `count` tokens of each row with the highest confidence, ties to the lower index,
/// as `[B, count]` u32 on the confidence's device. The host route reads the row back and
/// sorts it stably.
fn top_tokens_on_host(confidence: &Tensor, count: usize) -> Result<Tensor> {
    let rows: Vec<Vec<f32>> = confidence.to_vec2()?;
    let mut selected = Vec::with_capacity(rows.len() * count);
    for row in &rows {
        let mut order: Vec<u32> = (0..row.len() as u32).collect();
        order.sort_by(|&a, &b| row[b as usize].total_cmp(&row[a as usize]).then(a.cmp(&b)));
        selected.extend_from_slice(&order[..count]);
    }
    Ok(Tensor::from_vec(
        selected,
        (rows.len(), count),
        confidence.device(),
    )?)
}

/// [`top_tokens_on_host`] without leaving the device. The sort there is not stable, so
/// every key is made distinct in f64 without changing the order of distinct confidences:
/// token `i` loses `|c| * i * 2^-40 + i * 2^-1000`. Two different f32 values differ by at
/// least 2^-24 of their size (or, against zero, by far more than 2^-1000 * N), while the
/// largest loss is under 2^-30 of it, and each step is above f64's 2^-52 resolution, so
/// equal confidences come out in index order.
fn top_tokens_on_device(confidence: &Tensor, count: usize) -> Result<Tensor> {
    let (batch, tokens) = confidence.dims2()?;
    let value = confidence.to_dtype(DType::F64)?;
    let index = Tensor::arange(0u32, tokens as u32, confidence.device())?
        .to_dtype(DType::F64)?
        .unsqueeze(0)?
        .broadcast_as((batch, tokens))?;
    let relative = (value.abs()? * &index)? * 2f64.powi(-40);
    let absolute = (&index * 2f64.powi(-1000))?;
    let key = ((value - relative?)? - absolute)?;
    Ok(key
        .contiguous()?
        .arg_sort_last_dim(false)?
        .narrow(1, 0, count)?
        .contiguous()?)
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
    /// The last frame encoded, by canvas digest: `/act` and `/detect` on the same frame,
    /// or a screen that did not change, encode once.
    last_encoded: Mutex<Option<([u8; 32], Tensor)>>,
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
            last_encoded: Mutex::new(None),
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

    /// `encode_image` of a frame's canvas, reusing the last result when the canvas digest
    /// is the same. The encoders carry no skill adapter, so the reuse is exact whatever
    /// adapter is active.
    pub fn encode_frame(&self, frame: &Frame) -> Result<Tensor> {
        if let Ok(last) = self.last_encoded.lock() {
            if let Some((digest, tokens)) = last.as_ref() {
                if *digest == frame.digest {
                    return Ok(tokens.clone());
                }
            }
        }
        let tokens = self.encode_image(&canvas_tensor(&frame.canvas, &self.device)?)?;
        if let Ok(mut last) = self.last_encoded.lock() {
            *last = Some((frame.digest, tokens.clone()));
        }
        Ok(tokens)
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

    /// The fusion stack. Returns the normed image tokens and the fused text. The batch's
    /// rows share the MoE capacity, as in Python: open mode's rows are one image.
    pub fn fuse(
        &self,
        tokens: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        self.fuse_grouped(tokens, text, text_padding, 1)
    }

    /// [`IrisModel::fuse`] for a batch of different images (several foci in one pass):
    /// each row has its own MoE capacity, so each comes out as it would alone.
    pub fn fuse_per_image(
        &self,
        tokens: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        self.fuse_grouped(tokens, text, text_padding, tokens.dim(0)?)
    }

    fn fuse_grouped(
        &self,
        tokens: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
        groups: usize,
    ) -> Result<(Tensor, Tensor)> {
        let mut image = tokens.clone();
        let mut text = text.clone();
        for layer in &self.fusion {
            (image, text) = layer.forward(&image, &self.position, &text, text_padding, groups)?;
        }
        Ok((self.fused_norm.forward(&image)?, text))
    }

    /// Bytes one MoE expert takes on the device.
    pub fn expert_bytes(&self) -> usize {
        self.fusion
            .first()
            .map_or(0, |layer| layer.moe.expert_bytes())
    }

    /// Keeps only the experts `plan[layer][expert]` marks on the device (`None`: all).
    /// Results change for tokens routed to an absent expert; see `archie_accel::memory`.
    pub fn set_expert_residency(&mut self, plan: Option<&[Vec<bool>]>) -> Result<()> {
        if let Some(plan) = plan {
            if plan.len() != self.fusion.len() {
                return Err(crate::Error::Refused(format!(
                    "residency for {} layers, the model has {}",
                    plan.len(),
                    self.fusion.len()
                )));
            }
        }
        for (index, layer) in self.fusion.iter_mut().enumerate() {
            layer.moe.set_residency(plan.map(|p| p[index].as_slice()))?;
        }
        Ok(())
    }

    /// Starts (or stops) counting how often each expert is chosen, per layer.
    pub fn record_expert_usage(&self, on: bool) {
        for layer in &self.fusion {
            layer.moe.record_usage(on);
        }
    }

    /// Choices per expert per layer since recording started; empty when not recording.
    pub fn expert_usage(&self) -> Result<Vec<Vec<u64>>> {
        let mut out = Vec::with_capacity(self.fusion.len());
        for layer in &self.fusion {
            match layer.moe.usage()? {
                Some(counts) => out.push(counts),
                None => return Ok(Vec::new()),
            }
        }
        Ok(out)
    }

    /// [`IrisModel::fuse`] for one image `[1, N, D]` read against every row of `text`
    /// (open mode's chunks, `/act`'s instruction and null rows). The same numbers as
    /// fusing the image repeated per row, without repeating the first self-attention.
    pub fn fuse_shared_image(
        &self,
        tokens: &Tensor,
        text: &Tensor,
        text_padding: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        let batch = text.dim(0)?;
        let Some((first, rest)) = self.fusion.split_first() else {
            let (_, n, d) = tokens.dims3()?;
            let image = tokens.broadcast_as((batch, n, d))?.contiguous()?;
            return Ok((self.fused_norm.forward(&image)?, text.clone()));
        };
        let (mut image, mut text) =
            first.forward_shared_image(tokens, batch, &self.position, text, text_padding)?;
        for layer in rest {
            (image, text) = layer.forward(&image, &self.position, &text, text_padding, 1)?;
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
        let confidence = token_logits.max(D::Minus1)?;
        let selected = if self.device.is_cpu() {
            top_tokens_on_host(&confidence, count)?
        } else {
            top_tokens_on_device(&confidence, count)?
        };
        let width = fused.dim(2)?;
        let gather = |source: &Tensor, columns: usize| -> Result<Tensor> {
            let ids = selected
                .unsqueeze(2)?
                .broadcast_as((batch, count, columns))?
                .contiguous()?;
            Ok(source.contiguous()?.gather(&ids, 1)?)
        };
        let mut queries = self.query_projection_norm.forward(
            &self
                .query_projection_linear
                .forward(&gather(fused, width)?)?,
        )?;
        let mut reference = gather(&token_boxes, 4)?;
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
        let scaled: Vec<[f32; 4]> = boxes.iter().map(|b| b.map(|v| v * g as f32)).collect();
        let pooled = if self.device.is_cpu() {
            // [N, D] -> [D, G, G], channel-major for roi_align.
            let map: Vec<f32> = fused
                .to_dtype(DType::F32)?
                .t()?
                .contiguous()?
                .flatten_all()?
                .to_vec1()?;
            let mut pooled = Vec::with_capacity(boxes.len() * d * 9);
            for b in &scaled {
                pooled.extend(roi_align(&map, d, g, g, *b, 3));
            }
            Tensor::from_vec(pooled, (boxes.len(), d * 9), &self.device)?
        } else {
            // The map stays on the device: a small matrix of bilinear weights goes up and
            // one product pools every box, `[boxes * 9, N] x [N, D]`.
            let weights = Tensor::from_vec(
                roi_align_matrix(g, g, &scaled, 3),
                (boxes.len() * 9, g * g),
                &self.device,
            )?;
            weights
                .matmul(&fused.to_dtype(DType::F32)?)?
                .reshape((boxes.len(), 9, d))?
                .transpose(1, 2)?
                .contiguous()?
                .reshape((boxes.len(), d * 9))?
        };
        let pooled = pooled.to_dtype(self.dtype)?;
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
        let config = config(experts, top_k);
        let weights = Weights::from_tensors(
            moe_tensors(experts, favourites, "moe"),
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

    /// The MoE's tensors under `prefix`, in the release's shapes.
    fn moe_tensors(experts: usize, favourites: usize, prefix: &str) -> HashMap<String, Tensor> {
        let device = Device::Cpu;
        let randn = |shape: &[usize]| Tensor::randn(0f32, 0.5, shape, &device).unwrap();
        let mut router = randn(&[experts, WIDTH]).to_vec2::<f32>().unwrap();
        router[..favourites]
            .iter_mut()
            .for_each(|row| row.iter_mut().for_each(|v| *v = 1.0));
        let router: Vec<f32> = router.into_iter().flatten().collect();
        let mut map = HashMap::new();
        let mut put = |name: &str, tensor: Tensor| {
            map.insert(format!("{prefix}.{name}"), tensor);
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
        map
    }

    fn grouped_on_host(moe: &SparseMoe, x: &Tensor, groups: usize) -> Tensor {
        moe.route_on_host(&x.reshape(((), WIDTH)).unwrap(), groups)
            .unwrap()
    }

    fn grouped_on_device(moe: &SparseMoe, x: &Tensor, groups: usize) -> Tensor {
        moe.route_on_device(&x.reshape(((), WIDTH)).unwrap(), groups)
            .unwrap()
    }

    fn on_host(moe: &SparseMoe, x: &Tensor) -> Tensor {
        grouped_on_host(moe, x, 1)
    }

    fn on_device(moe: &SparseMoe, x: &Tensor) -> Tensor {
        grouped_on_device(moe, x, 1)
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

    /// One fusion layer of width 16, 4 heads, on random weights, with the MoE above.
    fn fusion_layer() -> FusionLayer {
        let device = Device::Cpu;
        let randn = |shape: &[usize]| Tensor::randn(0f32, 0.3, shape, &device).unwrap();
        let mut map = moe_tensors(8, 1, "f.moe");
        for norm in [
            "image_norm1",
            "image_norm2",
            "image_norm3",
            "text_norm1",
            "text_norm2",
        ] {
            map.insert(format!("f.{norm}.weight"), (randn(&[WIDTH]) + 1.0).unwrap());
            map.insert(format!("f.{norm}.bias"), randn(&[WIDTH]));
        }
        for block in ["image_self", "image_text", "text_image"] {
            for proj in ["query", "key", "value", "output"] {
                map.insert(format!("f.{block}.{proj}.weight"), randn(&[WIDTH, WIDTH]));
                map.insert(format!("f.{block}.{proj}.bias"), randn(&[WIDTH]));
            }
        }
        map.insert("f.text_ffn.0.weight".into(), randn(&[HIDDEN, WIDTH]));
        map.insert("f.text_ffn.0.bias".into(), randn(&[HIDDEN]));
        map.insert("f.text_ffn.2.weight".into(), randn(&[WIDTH, HIDDEN]));
        map.insert("f.text_ffn.2.bias".into(), randn(&[WIDTH]));
        let config = config(8, 2);
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
        FusionLayer::load(&weights, "f", &config).unwrap()
    }

    #[test]
    fn a_shared_image_fuses_like_the_image_repeated() {
        let layer = fusion_layer();
        let (batch, tokens, words) = (5, 24, 6);
        let image = Tensor::randn(0f32, 1.0, (1, tokens, WIDTH), &Device::Cpu).unwrap();
        let position = Tensor::randn(0f32, 0.1, (tokens, WIDTH), &Device::Cpu).unwrap();
        let text = Tensor::randn(0f32, 1.0, (batch, words, WIDTH), &Device::Cpu).unwrap();
        let padding: Vec<u8> = (0..batch * words)
            .map(|i| u8::from(i % words >= 2 + i / words % 4))
            .collect();
        let padding = Tensor::from_vec(padding, (batch, words), &Device::Cpu).unwrap();
        let repeated = image
            .broadcast_as((batch, tokens, WIDTH))
            .unwrap()
            .contiguous()
            .unwrap();
        let (image_a, text_a) = layer
            .forward(&repeated, &position, &text, &padding, 1)
            .unwrap();
        let (image_b, text_b) = layer
            .forward_shared_image(&image, batch, &position, &text, &padding)
            .unwrap();
        assert_eq!(max_difference(&image_a, &image_b), 0.0);
        assert_eq!(max_difference(&text_a, &text_b), 0.0);
    }

    #[test]
    fn queries_are_chosen_alike_on_both_routes() {
        let rows: Vec<Vec<f32>> = vec![
            // ties at a positive value, at zero, at a negative value, and a blocked row
            vec![
                0.5, 2.0, 0.5, 2.0, -1.0, 0.0, 0.0, -1.0, 1e-45, -1e-45, 3.0, 0.5,
            ],
            vec![-1e4; 12],
            (0..12).map(|i| ((i * 7) % 5) as f32 * 0.25 - 0.5).collect(),
            vec![
                1.0,
                1.0 + f32::EPSILON,
                1.0,
                1.0 - f32::EPSILON / 2.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
                1.0,
            ],
        ];
        let confidence = Tensor::from_vec(
            rows.iter().flatten().copied().collect::<Vec<f32>>(),
            (rows.len(), 12),
            &Device::Cpu,
        )
        .unwrap();
        for count in [1, 5, 12] {
            let host: Vec<Vec<u32>> = top_tokens_on_host(&confidence, count)
                .unwrap()
                .to_vec2()
                .unwrap();
            let device: Vec<Vec<u32>> = top_tokens_on_device(&confidence, count)
                .unwrap()
                .to_vec2()
                .unwrap();
            assert_eq!(host, device, "count {count}");
        }
        let all: Vec<Vec<u32>> = top_tokens_on_host(&confidence, 12)
            .unwrap()
            .to_vec2()
            .unwrap();
        assert_eq!(all[0][..4], [10, 1, 3, 0]);
        assert_eq!(all[1], (0..12).collect::<Vec<u32>>());
    }

    #[test]
    fn a_group_per_image_routes_each_image_as_if_alone() {
        let moe = moe(8, 2, 1);
        let images: Vec<Tensor> = (0..3).map(|_| tokens(1, 120)).collect();
        let batch = Tensor::cat(&images, 0).unwrap();
        let alone: Vec<Tensor> = images.iter().map(|x| on_host(&moe, x)).collect();
        let alone = Tensor::cat(&alone, 0).unwrap();
        let host = grouped_on_host(&moe, &batch, 3);
        let device = grouped_on_device(&moe, &batch, 3);
        assert_eq!(max_difference(&host, &alone), 0.0);
        assert!(max_difference(&device, &alone) <= 1e-5);
        // One group for the whole batch is a different (shared) capacity.
        assert!(max_difference(&on_host(&moe, &batch), &alone) > 0.0);
        assert!(moe.forward(&batch, 7).is_err());
    }

    #[test]
    fn absent_experts_act_at_capacity_zero_on_both_routes() {
        let mut moe = moe(8, 2, 1);
        let x = tokens(2, 100);
        let full = on_host(&moe, &x);
        let resident = [true, false, true, true, false, true, true, false];
        moe.set_residency(Some(&resident)).unwrap();
        assert_eq!(moe.w_in.dim(0).unwrap(), 5);
        let host = on_host(&moe, &x);
        let device = on_device(&moe, &x);
        assert!(max_difference(&host, &device) <= 1e-5);
        assert!(
            max_difference(&host, &full) > 0.0,
            "expert 1 was chosen and is gone"
        );
        // Every expert back: the original numbers.
        moe.set_residency(None).unwrap();
        assert_eq!(moe.w_in.dim(0).unwrap(), 8);
        assert!(moe.host.is_none());
        assert_eq!(max_difference(&on_host(&moe, &x), &full), 0.0);
    }

    #[test]
    fn usage_counts_every_choice_on_both_routes() {
        let moe = moe(8, 2, 1);
        let x = tokens(1, 50);
        assert!(moe.usage().unwrap().is_none());
        moe.record_usage(true);
        on_host(&moe, &x);
        let host = moe.usage().unwrap().unwrap();
        assert_eq!(host.iter().sum::<u64>(), 100);
        moe.record_usage(true);
        on_device(&moe, &x);
        assert_eq!(moe.usage().unwrap().unwrap(), host);
    }
}
