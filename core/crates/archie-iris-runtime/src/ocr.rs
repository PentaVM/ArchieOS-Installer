//! IRIS's OCR expert: GOT-OCR 2.0 (`stepfun-ai/GOT-OCR-2.0-hf`, Apache-2.0), already trained
//! for reading, run in IRIS's process and memory instead of a separate server.
//!
//! IRIS's own reading head (`read`) is an 8.6 M character decoder trained on street crops; on
//! screens and paper it reads noise. Until it is distilled, this expert reads: a SAM-style
//! ViT-B (16-pixel patches on a 1024 canvas, 14 x 14 windows with relative positions, global
//! attention in layers 2, 5, 8 and 11), a convolutional neck and projector that leave 256
//! tokens, and a Qwen2 decoder (24 layers, width 1024) that writes the text. A port of
//! `transformers`' `GotOcr2ForConditionalGeneration` for inference, checked against it by
//! `tests/ocr.rs`.
//!
//! The image is encoded once per frame; each box is a separate prompt over the same image
//! (`[x1,y1,x2,y2] OCR:` on GOT's 0–1000 grid, the fine-grained reading it was trained on), so
//! reading four regions costs one encoder pass and four short decodes.

use crate::nn::{attend, merge_heads, split_heads, Result};
use crate::preprocess::Rgb;
use crate::Error;
use candle_core::{DType, Device, Tensor, D};
use std::collections::HashMap;
use std::path::Path;

const CANVAS: usize = 1024;
const PATCH: usize = 16;
const GRID: usize = CANVAS / PATCH;
const VISION_WIDTH: usize = 768;
const VISION_HEADS: usize = 12;
const VISION_LAYERS: usize = 12;
const WINDOW: usize = 14;
const GLOBAL_LAYERS: [usize; 4] = [2, 5, 8, 11];
const IMAGE_TOKENS: usize = 256;
const LN_EPS: f64 = 1e-6;
const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
// CLIP's published values, digit for digit as the processor has them.
#[allow(clippy::excessive_precision)]
const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];
const SYSTEM: &str = "system\nYou should follow the instructions carefully and explain your \
answers in detail.";
/// Query heads a global-attention layer scores at once: 4096 x 4096 scores per head.
const GLOBAL_HEAD_CHUNK: usize = 2;

fn linear(t: &HashMap<String, Tensor>, name: &str, bias: bool) -> Result<candle_nn::Linear> {
    let weight = get(t, &format!("{name}.weight"))?;
    let bias = if bias {
        Some(get(t, &format!("{name}.bias"))?)
    } else {
        None
    };
    Ok(candle_nn::Linear::new(weight, bias))
}

fn get(t: &HashMap<String, Tensor>, name: &str) -> Result<Tensor> {
    t.get(name)
        .cloned()
        .ok_or_else(|| Error::Weights(format!("GOT-OCR: missing {name}")))
}

fn layer_norm(t: &HashMap<String, Tensor>, name: &str) -> Result<candle_nn::LayerNorm> {
    Ok(candle_nn::LayerNorm::new(
        get(t, &format!("{name}.weight"))?,
        get(t, &format!("{name}.bias"))?,
        LN_EPS,
    ))
}

fn forward(module: &impl candle_core::Module, x: &Tensor) -> Result<Tensor> {
    Ok(module.forward(&x.contiguous()?)?)
}

/// `GotOcr2VisionAttention`'s relative-position table for one axis of `size` cells:
/// `table[q - k + size - 1]` for every query `q` and key `k`, `[size, size, head]`.
fn relative_table(rel_pos: &Tensor, size: usize) -> Result<Tensor> {
    if rel_pos.dim(0)? != 2 * size - 1 {
        return Err(Error::Weights(format!(
            "relative positions of {} for a {size}-cell axis",
            rel_pos.dim(0)?
        )));
    }
    let index: Vec<u32> = (0..size)
        .flat_map(|q| (0..size).map(move |k| (q + size - 1 - k) as u32))
        .collect();
    let index = Tensor::from_vec(index, size * size, rel_pos.device())?;
    Ok(rel_pos
        .index_select(&index, 0)?
        .reshape((size, size, rel_pos.dim(1)?))?)
}

struct VisionLayer {
    norm1: candle_nn::LayerNorm,
    qkv: candle_nn::Linear,
    proj: candle_nn::Linear,
    /// `[S, S, head]` for the height and width axes.
    rel_h: Tensor,
    rel_w: Tensor,
    norm2: candle_nn::LayerNorm,
    lin1: candle_nn::Linear,
    lin2: candle_nn::Linear,
    /// 0 for global attention over the whole 64 x 64 grid.
    window: usize,
}

impl VisionLayer {
    fn load(t: &HashMap<String, Tensor>, index: usize) -> Result<Self> {
        let p = format!("vision_tower.layers.{index}");
        let window = if GLOBAL_LAYERS.contains(&index) {
            0
        } else {
            WINDOW
        };
        let size = if window == 0 { GRID } else { window };
        Ok(Self {
            norm1: layer_norm(t, &format!("{p}.layer_norm1"))?,
            qkv: linear(t, &format!("{p}.attn.qkv"), true)?,
            proj: linear(t, &format!("{p}.attn.proj"), true)?,
            rel_h: relative_table(&get(t, &format!("{p}.attn.rel_pos_h"))?, size)?,
            rel_w: relative_table(&get(t, &format!("{p}.attn.rel_pos_w"))?, size)?,
            norm2: layer_norm(t, &format!("{p}.layer_norm2"))?,
            lin1: linear(t, &format!("{p}.mlp.lin1"), true)?,
            lin2: linear(t, &format!("{p}.mlp.lin2"), true)?,
            window,
        })
    }

    /// `x`: `[B, 64, 64, C]`.
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let (batch, height, width, channels) = x.dims4()?;
        let normed = forward(&self.norm1, x)?;
        let attended = if self.window == 0 {
            self.attention(&normed)?
        } else {
            let w = self.window;
            let (ph, pw) = (height.div_ceil(w) * w, width.div_ceil(w) * w);
            // Zero padding after the norm, as `F.pad` there: padded cells take part in their
            // window's attention like any token.
            let padded =
                normed
                    .pad_with_zeros(1, 0, ph - height)?
                    .pad_with_zeros(2, 0, pw - width)?;
            let windows = padded
                .reshape((batch, ph / w, w, pw / w, w, channels))?
                .permute((0, 1, 3, 2, 4, 5))?
                .contiguous()?
                .reshape((batch * (ph / w) * (pw / w), w, w, channels))?;
            let out = self.attention(&windows)?;
            out.reshape((batch, ph / w, pw / w, w, w, channels))?
                .permute((0, 1, 3, 2, 4, 5))?
                .contiguous()?
                .reshape((batch, ph, pw, channels))?
                .narrow(1, 0, height)?
                .narrow(2, 0, width)?
                .contiguous()?
        };
        let x = (x + attended)?;
        let hidden = crate::fast::gelu_erf(&forward(&self.lin1, &forward(&self.norm2, &x)?)?)?;
        Ok((&x + forward(&self.lin2, &hidden)?)?)
    }

    /// Multi-head attention over `[N, S, S, C]` with decomposed relative positions.
    fn attention(&self, x: &Tensor) -> Result<Tensor> {
        let (n, s, _, channels) = x.dims4()?;
        let head = channels / VISION_HEADS;
        let tokens = s * s;
        let qkv = forward(&self.qkv, x)?
            .reshape((n, tokens, 3, VISION_HEADS, head))?
            .permute((2, 0, 3, 1, 4))?
            .contiguous()?
            .reshape((3, n * VISION_HEADS, tokens, head))?;
        let (q, k, v) = (qkv.get(0)?, qkv.get(1)?, qkv.get(2)?);
        let nh = n * VISION_HEADS;
        // einsum("bhwc,hkc->bhwk") and einsum("bhwc,wkc->bhwk") as batched products.
        let q4 = q.reshape((nh, s, s, head))?;
        let rel_h = q4
            .permute((1, 0, 2, 3))?
            .contiguous()?
            .reshape((s, nh * s, head))?
            .matmul(&self.rel_h.transpose(1, 2)?.contiguous()?)?
            .reshape((s, nh, s, s))?
            .permute((1, 0, 2, 3))?;
        let rel_w = q4
            .permute((2, 0, 1, 3))?
            .contiguous()?
            .reshape((s, nh * s, head))?
            .matmul(&self.rel_w.transpose(1, 2)?.contiguous()?)?
            .reshape((s, nh, s, s))?
            .permute((1, 2, 0, 3))?;
        let scale = (head as f64).powf(-0.5);
        // Global layers hold 4096 x 4096 scores per head: a few heads at a time.
        let chunk = if self.window == 0 {
            GLOBAL_HEAD_CHUNK
        } else {
            nh
        };
        let mut parts = Vec::with_capacity(nh.div_ceil(chunk));
        for start in (0..nh).step_by(chunk) {
            let len = chunk.min(nh - start);
            let bias = (rel_h
                .narrow(0, start, len)?
                .unsqueeze(4)?
                .broadcast_add(&rel_w.narrow(0, start, len)?.unsqueeze(3)?))?
            .reshape((len, tokens, tokens))?;
            let scores = (q.narrow(0, start, len)? * scale)?
                .contiguous()?
                .matmul(&k.narrow(0, start, len)?.t()?.contiguous()?)?;
            let scores = (scores + bias)?;
            let weights = crate::fast::softmax_last_dim(&scores.to_dtype(DType::F32)?)?
                .to_dtype(q.dtype())?;
            parts.push(weights.matmul(&v.narrow(0, start, len)?.contiguous()?)?);
        }
        let out = Tensor::cat(&parts, 0)?
            .reshape((n, VISION_HEADS, s, s, head))?
            .permute((0, 2, 3, 1, 4))?
            .contiguous()?
            .reshape((n, s, s, channels))?;
        forward(&self.proj, &out)
    }
}

/// A channels-first LayerNorm (`GotOcr2LayerNorm`), `[B, C, H, W]`.
fn layer_norm_2d(norm: &candle_nn::LayerNorm, x: &Tensor) -> Result<Tensor> {
    Ok(forward(norm, &x.permute((0, 2, 3, 1))?)?.permute((0, 3, 1, 2))?)
}

struct Vision {
    patch_weight: Tensor,
    patch_bias: Tensor,
    position: Tensor,
    layers: Vec<VisionLayer>,
    neck_conv1: Tensor,
    neck_norm1: candle_nn::LayerNorm,
    neck_conv2: Tensor,
    neck_norm2: candle_nn::LayerNorm,
    up1: Tensor,
    up2: Tensor,
    projector: candle_nn::Linear,
}

impl Vision {
    fn load(t: &HashMap<String, Tensor>) -> Result<Self> {
        Ok(Self {
            patch_weight: get(t, "vision_tower.patch_embed.projection.weight")?
                .reshape((VISION_WIDTH, 3 * PATCH * PATCH))?,
            patch_bias: get(t, "vision_tower.patch_embed.projection.bias")?,
            position: get(t, "vision_tower.pos_embed")?,
            layers: (0..VISION_LAYERS)
                .map(|i| VisionLayer::load(t, i))
                .collect::<Result<_>>()?,
            neck_conv1: get(t, "vision_tower.neck.conv1.weight")?,
            neck_norm1: layer_norm(t, "vision_tower.neck.layer_norm1")?,
            neck_conv2: get(t, "vision_tower.neck.conv2.weight")?,
            neck_norm2: layer_norm(t, "vision_tower.neck.layer_norm2")?,
            up1: get(t, "multi_modal_projector.conv_upsampler1.weight")?,
            up2: get(t, "multi_modal_projector.conv_upsampler2.weight")?,
            projector: linear(t, "multi_modal_projector.multimodal_projector", true)?,
        })
    }

    /// `pixels`: `[B, 3, 1024, 1024]`, normalised. Returns `[B, 256, 1024]` image tokens.
    fn forward(&self, pixels: &Tensor) -> Result<Tensor> {
        let batch = pixels.dim(0)?;
        let mut x = crate::vjepa::patchify(pixels, PATCH)?
            .broadcast_matmul(&self.patch_weight.t()?)?
            .broadcast_add(&self.patch_bias)?
            .reshape((batch, GRID, GRID, VISION_WIDTH))?
            .broadcast_add(&self.position)?;
        for layer in &self.layers {
            x = layer.forward(&x)?;
        }
        let x = x.permute((0, 3, 1, 2))?.contiguous()?;
        let x = layer_norm_2d(&self.neck_norm1, &x.conv2d(&self.neck_conv1, 0, 1, 1, 1)?)?;
        let x = layer_norm_2d(
            &self.neck_norm2,
            &x.contiguous()?.conv2d(&self.neck_conv2, 1, 1, 1, 1)?,
        )?;
        let x = x.contiguous()?.conv2d(&self.up1, 1, 2, 1, 1)?;
        let x = x.conv2d(&self.up2, 1, 2, 1, 1)?;
        let (b, c, h, w) = x.dims4()?;
        let tokens = x.reshape((b, c, h * w))?.transpose(1, 2)?;
        forward(&self.projector, &tokens)
    }
}

struct DecoderLayer {
    input_norm: Tensor,
    q: candle_nn::Linear,
    k: candle_nn::Linear,
    v: candle_nn::Linear,
    o: candle_nn::Linear,
    post_norm: Tensor,
    gate: candle_nn::Linear,
    up: candle_nn::Linear,
    down: candle_nn::Linear,
}

/// The keys and values of every decoder layer so far.
pub struct Cache {
    layers: Vec<Option<(Tensor, Tensor)>>,
    len: usize,
}

struct Decoder {
    embed: Tensor,
    layers: Vec<DecoderLayer>,
    norm: Tensor,
    heads: usize,
    eps: f64,
    /// `[positions, head / 2]` rotary tables.
    cos: Tensor,
    sin: Tensor,
}

impl Decoder {
    fn load(t: &HashMap<String, Tensor>, config: &serde_json::Value) -> Result<Self> {
        let text = &config["text_config"];
        let layers = text["num_hidden_layers"].as_u64().unwrap_or(24) as usize;
        let heads = text["num_attention_heads"].as_u64().unwrap_or(16) as usize;
        let width = text["hidden_size"].as_u64().unwrap_or(1024) as usize;
        let theta = text["rope_theta"].as_f64().unwrap_or(1_000_000.0);
        let eps = text["rms_norm_eps"].as_f64().unwrap_or(1e-6);
        let head = width / heads;
        let p = "language_model.model";
        let embed = get(t, &format!("{p}.embed_tokens.weight"))?;
        let device = embed.device().clone();
        let dtype = embed.dtype();
        let positions = 4096usize;
        let inv: Vec<f32> = (0..head / 2)
            .map(|i| 1.0 / theta.powf(2.0 * i as f64 / head as f64) as f32)
            .collect();
        let angles: Vec<f32> = (0..positions)
            .flat_map(|p| inv.iter().map(move |f| p as f32 * f))
            .collect();
        let angles = Tensor::from_vec(angles, (positions, head / 2), &device)?;
        Ok(Self {
            layers: (0..layers)
                .map(|i| {
                    let l = format!("{p}.layers.{i}");
                    Ok(DecoderLayer {
                        input_norm: get(t, &format!("{l}.input_layernorm.weight"))?,
                        q: linear(t, &format!("{l}.self_attn.q_proj"), true)?,
                        k: linear(t, &format!("{l}.self_attn.k_proj"), true)?,
                        v: linear(t, &format!("{l}.self_attn.v_proj"), true)?,
                        o: linear(t, &format!("{l}.self_attn.o_proj"), false)?,
                        post_norm: get(t, &format!("{l}.post_attention_layernorm.weight"))?,
                        gate: linear(t, &format!("{l}.mlp.gate_proj"), false)?,
                        up: linear(t, &format!("{l}.mlp.up_proj"), false)?,
                        down: linear(t, &format!("{l}.mlp.down_proj"), false)?,
                    })
                })
                .collect::<Result<_>>()?,
            norm: get(t, &format!("{p}.norm.weight"))?,
            cos: angles.cos()?.to_dtype(dtype)?,
            sin: angles.sin()?.to_dtype(dtype)?,
            embed,
            heads,
            eps,
        })
    }

    fn rms(&self, x: &Tensor, weight: &Tensor) -> Result<Tensor> {
        Ok(candle_nn::ops::rms_norm(
            &x.contiguous()?,
            weight,
            self.eps as f32,
        )?)
    }

    fn new_cache(&self) -> Cache {
        Cache {
            layers: vec![None; self.layers.len()],
            len: 0,
        }
    }

    /// The new positions `x` `[1, T, D]` through the decoder, extending `cache`. Returns
    /// the last position's logits `[vocab]` (f32).
    fn step(&self, x: Tensor, cache: &mut Cache) -> Result<Tensor> {
        let length = x.dim(1)?;
        let start = cache.len;
        let cos = self.cos.narrow(0, start, length)?;
        let sin = self.sin.narrow(0, start, length)?;
        let mask = if length > 1 {
            let total = start + length;
            let allowed: Vec<u8> = (0..length)
                .flat_map(|row| (0..total).map(move |col| u8::from(col <= start + row)))
                .collect();
            Some(Tensor::from_vec(
                allowed,
                (1, 1, length, total),
                x.device(),
            )?)
        } else {
            None
        };
        let mut x = x;
        for (index, layer) in self.layers.iter().enumerate() {
            let h = self.rms(&x, &layer.input_norm)?;
            let q = split_heads(&forward(&layer.q, &h)?, self.heads)?.contiguous()?;
            let k = split_heads(&forward(&layer.k, &h)?, self.heads)?.contiguous()?;
            let v = split_heads(&forward(&layer.v, &h)?, self.heads)?.contiguous()?;
            let q = candle_nn::rotary_emb::rope(&q, &cos, &sin)?;
            let k = candle_nn::rotary_emb::rope(&k, &cos, &sin)?;
            let (k, v) = match &cache.layers[index] {
                Some((pk, pv)) => (Tensor::cat(&[pk, &k], 2)?, Tensor::cat(&[pv, &v], 2)?),
                None => (k, v),
            };
            let attended = attend(&q, &k, &v, mask.as_ref())?;
            cache.layers[index] = Some((k, v));
            x = (x + forward(&layer.o, &merge_heads(&attended)?)?)?;
            let h = self.rms(&x, &layer.post_norm)?;
            let gated =
                (candle_nn::ops::silu(&forward(&layer.gate, &h)?)? * forward(&layer.up, &h)?)?;
            x = (&x + forward(&layer.down, &gated)?)?;
        }
        cache.len += length;
        let last = self.rms(&x.narrow(1, length - 1, 1)?, &self.norm)?;
        Ok(last
            .reshape((1, last.dim(D::Minus1)?))?
            .matmul(&self.embed.t()?)?
            .squeeze(0)?
            .to_dtype(DType::F32)?)
    }
}

/// What the expert read in one region (or the whole frame).
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The prompt's box on GOT's 0–1000 grid, when one was asked.
    pub region: Option<[u32; 4]>,
    pub text: String,
    pub tokens: usize,
}

pub struct OcrExpert {
    vision: Vision,
    decoder: Decoder,
    tokenizer: tokenizers::Tokenizer,
    image_token: u32,
    stops: Vec<u32>,
    device: Device,
    dtype: DType,
}

impl OcrExpert {
    /// `dir` holds `config.json`, `model.safetensors` and `tokenizer.json` of
    /// `stepfun-ai/GOT-OCR-2.0-hf`.
    pub fn load(dir: &Path, device: &Device, dtype: DType) -> Result<Self> {
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)
                .map_err(|e| Error::Weights(format!("GOT-OCR config.json: {e}")))?;
        if config["model_type"].as_str() != Some("got_ocr2") {
            return Err(Error::Weights("not a GOT-OCR 2.0 checkpoint".into()));
        }
        let tensors: HashMap<String, Tensor> =
            candle_core::safetensors::load(dir.join("model.safetensors"), device)?
                .into_iter()
                .map(|(name, t)| Ok((name, t.to_dtype(dtype)?)))
                .collect::<Result<_>>()?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| Error::Weights(format!("GOT-OCR tokenizer.json: {e}")))?;
        let id = |token: &str| {
            tokenizer
                .token_to_id(token)
                .ok_or_else(|| Error::Weights(format!("GOT-OCR tokenizer has no {token}")))
        };
        let image_token = config["image_token_index"]
            .as_u64()
            .map(|v| v as u32)
            .map_or_else(|| id("<imgpad>"), Ok)?;
        let mut stops = vec![id("<|im_end|>")?];
        if let Some(end) = tokenizer.token_to_id("<|endoftext|>") {
            stops.push(end);
        }
        Ok(Self {
            vision: Vision::load(&tensors)?,
            decoder: Decoder::load(&tensors, &config)?,
            tokenizer,
            image_token,
            stops,
            device: device.clone(),
            dtype,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// `[B, 256, 1024]` image tokens for normalised `[B, 3, 1024, 1024]` pixels.
    pub fn encode(&self, pixels: &Tensor) -> Result<Tensor> {
        self.vision
            .forward(&pixels.to_device(&self.device)?.to_dtype(self.dtype)?)
    }

    /// The prompt GOT was trained on, with `region` (0–1000 grid) for fine-grained reading.
    pub fn prompt(region: Option<[u32; 4]>) -> String {
        let query = match region {
            Some([x0, y0, x1, y1]) => format!("[{x0}, {y0}, {x1}, {y1}] OCR: "),
            None => " OCR: ".to_owned(),
        };
        format!(
            "<|im_start|>{SYSTEM}<|im_end|><|im_start|>user\n<img>{}</img>\n{query}<|im_end|><|im_start|>assistant\n",
            "<imgpad>".repeat(IMAGE_TOKENS)
        )
    }

    pub fn prompt_ids(&self, region: Option<[u32; 4]>) -> Result<Vec<u32>> {
        let encoding = self
            .tokenizer
            .encode(Self::prompt(region), false)
            .map_err(|e| Error::Refused(format!("tokenize: {e}")))?;
        Ok(encoding.get_ids().to_vec())
    }

    /// Prompt embeddings with the image's tokens where `<imgpad>` stands.
    fn embed_prompt(&self, ids: &[u32], image: &Tensor) -> Result<Tensor> {
        let start = ids
            .iter()
            .position(|&t| t == self.image_token)
            .ok_or_else(|| Error::Refused("the prompt has no image".into()))?;
        let count = ids[start..]
            .iter()
            .take_while(|&&t| t == self.image_token)
            .count();
        if count != image.dim(1)? {
            return Err(Error::Refused(format!(
                "{count} image tokens for {} image features",
                image.dim(1)?
            )));
        }
        let text = |range: &[u32]| -> Result<Tensor> {
            let index = Tensor::from_vec(range.to_vec(), range.len(), &self.device)?;
            Ok(self.decoder.embed.index_select(&index, 0)?.unsqueeze(0)?)
        };
        Ok(Tensor::cat(
            &[
                &text(&ids[..start])?,
                &image.to_dtype(self.dtype)?,
                &text(&ids[start + count..])?,
            ],
            1,
        )?)
    }

    /// The logits after the prompt (for the parity test): `[vocab]` f32.
    pub fn prompt_logits(&self, ids: &[u32], image: &Tensor) -> Result<Tensor> {
        let mut cache = self.decoder.new_cache();
        self.decoder
            .step(self.embed_prompt(ids, image)?, &mut cache)
    }

    /// Greedy reading of one prompt over one encoded image `[1, 256, 1024]`.
    pub fn generate(&self, ids: &[u32], image: &Tensor, max_tokens: usize) -> Result<Vec<u32>> {
        let mut cache = self.decoder.new_cache();
        let mut logits = self
            .decoder
            .step(self.embed_prompt(ids, image)?, &mut cache)?;
        let mut out = Vec::new();
        for _ in 0..max_tokens {
            let next = logits.argmax(0)?.to_scalar::<u32>()?;
            if self.stops.contains(&next) {
                break;
            }
            out.push(next);
            let index = Tensor::new(&[next], &self.device)?;
            let embedded = self.decoder.embed.index_select(&index, 0)?.unsqueeze(0)?;
            logits = self.decoder.step(embedded, &mut cache)?;
        }
        Ok(out)
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        self.tokenizer
            .decode(ids, true)
            .map_err(|e| Error::Refused(format!("detokenize: {e}")))
    }

    /// Reads `image`: the whole of it, or each of `boxes` (pixels, x0 y0 x1 y1) over one
    /// encoding of it.
    pub fn read(&self, image: &Rgb, boxes: &[[f64; 4]], max_tokens: usize) -> Result<Vec<Reading>> {
        let pixels = canvas(image, &self.device)?;
        let encoded = self.encode(&pixels)?;
        let regions: Vec<Option<[u32; 4]>> = if boxes.is_empty() {
            vec![None]
        } else {
            boxes
                .iter()
                .map(|b| Some(grid_box(*b, image.width, image.height)))
                .collect()
        };
        regions
            .into_iter()
            .map(|region| {
                let ids = self.prompt_ids(region)?;
                let tokens = self.generate(&ids, &encoded, max_tokens)?;
                Ok(Reading {
                    region,
                    text: self.decode(&tokens)?.trim().to_owned(),
                    tokens: tokens.len(),
                })
            })
            .collect()
    }
}

/// A pixel box on GOT's 0–1000 grid (`preprocess_box_annotation`: truncated, as `int()`).
pub fn grid_box(b: [f64; 4], width: usize, height: usize) -> [u32; 4] {
    let scale = |v: f64, size: usize| ((v.max(0.0) / size as f64) * 1000.0).min(1000.0) as u32;
    [
        scale(b[0], width),
        scale(b[1], height),
        scale(b[2], width),
        scale(b[3], height),
    ]
}

/// Keys' cubic convolution weight (`a = -0.5`, as Pillow's BICUBIC).
fn cubic(x: f64) -> f64 {
    let a = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * a
    } else {
        0.0
    }
}

/// Antialiased separable bicubic weights from `input` to `output` samples (Pillow's scheme:
/// the kernel widens by the scale when shrinking).
fn taps(input: usize, output: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = input as f64 / output as f64;
    let support = 2.0 * scale.max(1.0);
    let stretch = scale.max(1.0);
    (0..output)
        .map(|o| {
            let centre = (o as f64 + 0.5) * scale;
            let first = (centre - support + 0.5).floor().max(0.0) as usize;
            let last = ((centre + support + 0.5).floor() as usize).min(input);
            let mut weights: Vec<f64> = (first..last)
                .map(|i| cubic((i as f64 - centre + 0.5) / stretch))
                .collect();
            let total: f64 = weights.iter().sum();
            if total != 0.0 {
                weights.iter_mut().for_each(|w| *w /= total);
            }
            (first, weights.into_iter().map(|w| w as f32).collect())
        })
        .collect()
}

/// The image stretched to 1024 x 1024 (bicubic, as GOT's processor; not bit-exact with
/// torchvision's), scaled to [0, 1] and normalised with CLIP's mean and deviation:
/// `[1, 3, 1024, 1024]`.
pub fn canvas(image: &Rgb, device: &Device) -> Result<Tensor> {
    let (w, h) = (image.width, image.height);
    let columns = taps(w, CANVAS);
    let rows = taps(h, CANVAS);
    // Horizontal pass, then vertical, per channel, in f32.
    let mut horizontal = vec![0f32; h * CANVAS * 3];
    for y in 0..h {
        for (x, (first, weights)) in columns.iter().enumerate() {
            for c in 0..3 {
                let mut sum = 0f32;
                for (i, weight) in weights.iter().enumerate() {
                    sum += weight * f32::from(image.pixels[(y * w + first + i) * 3 + c]);
                }
                horizontal[(y * CANVAS + x) * 3 + c] = sum;
            }
        }
    }
    let mut planes = vec![0f32; 3 * CANVAS * CANVAS];
    for (y, (first, weights)) in rows.iter().enumerate() {
        for x in 0..CANVAS {
            for c in 0..3 {
                let mut sum = 0f32;
                for (i, weight) in weights.iter().enumerate() {
                    sum += weight * horizontal[((first + i) * CANVAS + x) * 3 + c];
                }
                let value = (sum.round().clamp(0.0, 255.0)) / 255.0;
                planes[c * CANVAS * CANVAS + y * CANVAS + x] = (value - MEAN[c]) / STD[c];
            }
        }
    }
    Ok(Tensor::from_vec(planes, (1, 3, CANVAS, CANVAS), device)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_tables_index_q_minus_k() {
        let rel = Tensor::arange(0f32, 5.0, &Device::Cpu)
            .unwrap()
            .reshape((5, 1))
            .unwrap();
        let table: Vec<Vec<f32>> = relative_table(&rel, 3)
            .unwrap()
            .squeeze(2)
            .unwrap()
            .to_vec2()
            .unwrap();
        // table[q][k] = rel[q - k + 2].
        assert_eq!(
            table,
            vec![
                vec![2.0, 1.0, 0.0],
                vec![3.0, 2.0, 1.0],
                vec![4.0, 3.0, 2.0]
            ]
        );
        assert!(relative_table(&rel, 4).is_err());
    }

    #[test]
    fn boxes_land_on_the_grid_and_prompts_hold_the_image() {
        assert_eq!(
            grid_box([0.0, 360.0, 640.0, 720.0], 1280, 720),
            [0, 500, 500, 1000]
        );
        assert_eq!(
            grid_box([-5.0, 0.0, 2000.0, 10.0], 1280, 720),
            [0, 0, 1000, 13]
        );
        let prompt = OcrExpert::prompt(Some([1, 2, 3, 4]));
        assert_eq!(prompt.matches("<imgpad>").count(), IMAGE_TOKENS);
        assert!(prompt.contains("[1, 2, 3, 4] OCR: "));
        assert!(OcrExpert::prompt(None).contains("</img>\n OCR: <|im_end|>"));
    }

    #[test]
    fn the_canvas_keeps_a_flat_colour_flat() {
        let image = Rgb::new(40, 20, [200u8, 100, 50].repeat(40 * 20)).unwrap();
        let pixels = canvas(&image, &Device::Cpu).unwrap();
        assert_eq!(pixels.dims(), &[1, 3, CANVAS, CANVAS]);
        for (c, value) in [200f32, 100.0, 50.0].iter().enumerate() {
            let plane = pixels.get(0).unwrap().get(c).unwrap();
            let (lo, hi) = (
                plane.min_all().unwrap().to_scalar::<f32>().unwrap(),
                plane.max_all().unwrap().to_scalar::<f32>().unwrap(),
            );
            let want = (value / 255.0 - MEAN[c]) / STD[c];
            assert!(
                (lo - want).abs() < 1e-5 && (hi - want).abs() < 1e-5,
                "{lo} {hi} {want}"
            );
        }
    }
}
