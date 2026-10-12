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
}

impl SparseMoe {
    fn load(weights: &Weights, prefix: &str, config: &ModelConfig) -> Result<Self> {
        Ok(Self {
            router: Linear::load(weights, &format!("{prefix}.router"))?,
            shared: Mlp::load(
                weights,
                &format!("{prefix}.shared.0"),
                &format!("{prefix}.shared.2"),
                true,
            )?,
            w_in: weights.get(&format!("{prefix}.w_in"))?,
            b_in: weights.get(&format!("{prefix}.b_in"))?,
            w_out: weights.get(&format!("{prefix}.w_out"))?,
            b_out: weights.get(&format!("{prefix}.b_out"))?,
            experts: config.experts,
            top_k: config.expert_top_k,
            capacity_factor: config.expert_capacity_factor,
        })
    }

    /// Every token of the batch competes for the same capacity, in batch-major order: the
    /// same rows `SparseMoeFeedForward` flattens. A token over an expert's capacity keeps
    /// only its shared-expert path, as in training.
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let shape = x.shape().clone();
        let width = x.dim(D::Minus1)?;
        let flat = x.reshape(((), width))?;
        let tokens = flat.dim(0)?;
        let probabilities: Vec<f32> =
            candle_nn::ops::softmax_last_dim(&self.router.forward(&flat)?.to_dtype(DType::F32)?)?
                .flatten_all()?
                .to_vec1()?;
        let capacity = ((self.capacity_factor * (tokens * self.top_k) as f64 / self.experts as f64)
            .ceil() as usize)
            .max(4);
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
        let mut output = self.shared.forward(&flat)?;
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
        Ok(output.reshape(shape)?)
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
