//! IRIS reading: `beta_model.ReadDecoder`, the head that reads the text of a word or line
//! crop from the fused scene, for inference.
//!
//! A crop is letterboxed onto the canvas like any frame (training used TextOCR crops of at
//! most 30 characters, read the same way), encoded, fused with the null phrase
//! (`encode_fused_state`, the scene state `/act` and tracking also read), and decoded one
//! character at a time, greedily, as `ReadDecoder.generate` does. Python reruns the whole
//! decoder over the prefix at every step; here each layer keeps its keys and values, and the
//! memory's keys and values are projected once per crop. A step attends with one new query,
//! which is what the causal mask let the last position see, so the logits are the same.
//!
//! The decoder is `nn.TransformerDecoderLayer(norm_first=True)` with PyTorch's defaults:
//! ReLU, LayerNorm eps 1e-5, packed input projections. No final norm inside the stack; the
//! head's own norm and output layer follow it.

use crate::model::IrisModel;
use crate::nn::{attend, merge_heads, split_heads, LayerNorm, Linear, Result};
use crate::preprocess::{Frame, Rgb};
use crate::weights::Weights;
use crate::Error;
use candle_core::{DType, Tensor, D};

pub const READ_PAD: u32 = 0;
pub const READ_BOS: u32 = 1;
pub const READ_EOS: u32 = 2;
pub const READ_UNK: u32 = 3;
const EPS: f64 = 1e-5;
const PREFIX: &str = "read_decoder";
/// Most crops one call reads; each costs a pass of both encoders.
pub const MAX_CROPS: usize = 16;

/// `READ_CHARACTERS`: printable ASCII, then Spanish and a few symbols, tokens from 4.
pub fn characters() -> Vec<char> {
    (32u8..127)
        .map(char::from)
        .chain("áéíóúÁÉÍÓÚñÑüÜ¿¡°€£çÇ".chars())
        .collect()
}

/// `decode_text`: characters up to the first end token; unknowns read as `?`.
pub fn decode_text(tokens: &[u32], alphabet: &[char]) -> String {
    let mut text = String::new();
    for &token in tokens {
        if token == READ_EOS {
            break;
        }
        if token >= 4 {
            if let Some(&c) = alphabet.get((token - 4) as usize) {
                text.push(c);
            }
        } else if token == READ_UNK {
            text.push('?');
        }
    }
    text
}

/// `nn.MultiheadAttention` split into its projections, so keys and values can be kept.
struct Projected {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
    heads: usize,
}

impl Projected {
    fn load(weights: &Weights, prefix: &str, heads: usize) -> Result<Self> {
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

    /// Keys and values `[B, H, S, Dh]` of `x`.
    fn keys_values(&self, x: &Tensor) -> Result<(Tensor, Tensor)> {
        Ok((
            split_heads(&self.key.forward(x)?, self.heads)?.contiguous()?,
            split_heads(&self.value.forward(x)?, self.heads)?.contiguous()?,
        ))
    }

    fn attend(
        &self,
        x: &Tensor,
        keys: &Tensor,
        values: &Tensor,
        allowed: Option<&Tensor>,
    ) -> Result<Tensor> {
        let q = split_heads(&self.query.forward(x)?, self.heads)?;
        self.output
            .forward(&merge_heads(&attend(&q, keys, values, allowed)?)?)
    }
}

struct Layer {
    self_attention: Projected,
    cross_attention: Projected,
    norm1: LayerNorm,
    norm2: LayerNorm,
    norm3: LayerNorm,
    linear1: Linear,
    linear2: Linear,
}

impl Layer {
    fn feed_forward(&self, x: &Tensor) -> Result<Tensor> {
        self.linear2
            .forward(&self.linear1.forward(&self.norm3.forward(x)?)?.relu()?)
    }
}

/// One crop batch's state while decoding: per layer, the self-attention cache and the
/// memory's projections.
struct Cache {
    keys: Vec<Option<Tensor>>,
    values: Vec<Option<Tensor>>,
    memory: Vec<(Tensor, Tensor)>,
}

pub struct Reader {
    embedding: Tensor,
    position: Tensor,
    layers: Vec<Layer>,
    norm: LayerNorm,
    output: Linear,
    max_length: usize,
    alphabet: Vec<char>,
}

impl Reader {
    /// `None` when the release has no reading head.
    pub fn load(weights: &Weights) -> Result<Option<Self>> {
        if !weights.contains(&format!("{PREFIX}.embedding.weight")) {
            return Ok(None);
        }
        let heads = weights.release.config.heads;
        let mut layers = Vec::new();
        while weights.contains(&format!(
            "{PREFIX}.decoder.layers.{}.norm1.weight",
            layers.len()
        )) {
            let p = format!("{PREFIX}.decoder.layers.{}", layers.len());
            layers.push(Layer {
                self_attention: Projected::load(weights, &format!("{p}.self_attn"), heads)?,
                cross_attention: Projected::load(weights, &format!("{p}.multihead_attn"), heads)?,
                norm1: LayerNorm::load(weights, &format!("{p}.norm1"), EPS)?,
                norm2: LayerNorm::load(weights, &format!("{p}.norm2"), EPS)?,
                norm3: LayerNorm::load(weights, &format!("{p}.norm3"), EPS)?,
                linear1: Linear::load(weights, &format!("{p}.linear1"))?,
                linear2: Linear::load(weights, &format!("{p}.linear2"))?,
            });
        }
        if layers.is_empty() {
            return Err(Error::Weights(
                "the reading head has no decoder layers".into(),
            ));
        }
        let position = weights.get(&format!("{PREFIX}.position"))?;
        let max_length = position.dim(1)?;
        let embedding = weights.get(&format!("{PREFIX}.embedding.weight"))?;
        let alphabet = characters();
        if embedding.dim(0)? != 4 + alphabet.len() {
            return Err(Error::Weights(format!(
                "the reading head has {} tokens; this runtime knows {}",
                embedding.dim(0)?,
                4 + alphabet.len()
            )));
        }
        Ok(Some(Self {
            embedding,
            position,
            layers,
            norm: LayerNorm::load(weights, &format!("{PREFIX}.norm"), EPS)?,
            output: Linear::load(weights, &format!("{PREFIX}.output"))?,
            max_length,
            alphabet,
        }))
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }

    /// Token embeddings plus positions `start..start + T`, for `[B, T]` ids.
    fn embed(&self, tokens: &Tensor, start: usize) -> Result<Tensor> {
        let (batch, length) = tokens.dims2()?;
        let width = self.embedding.dim(1)?;
        Ok(self
            .embedding
            .index_select(&tokens.flatten_all()?, 0)?
            .reshape((batch, length, width))?
            .broadcast_add(&self.position.narrow(1, start, length)?)?)
    }

    fn cache(&self, memory: &Tensor) -> Result<Cache> {
        Ok(Cache {
            keys: vec![None; self.layers.len()],
            values: vec![None; self.layers.len()],
            memory: self
                .layers
                .iter()
                .map(|layer| layer.cross_attention.keys_values(memory))
                .collect::<Result<_>>()?,
        })
    }

    /// The new positions `x` `[B, T, D]` through every layer, extending the cache. `causal`
    /// masks within the new positions (teacher forcing); one position needs no mask.
    fn step(&self, mut x: Tensor, cache: &mut Cache, causal: Option<&Tensor>) -> Result<Tensor> {
        for (index, layer) in self.layers.iter().enumerate() {
            let normed = layer.norm1.forward(&x)?;
            let (k, v) = layer.self_attention.keys_values(&normed)?;
            let (k, v) = match (&cache.keys[index], &cache.values[index]) {
                (Some(pk), Some(pv)) => (
                    Tensor::cat(&[pk, &k], 2)?.contiguous()?,
                    Tensor::cat(&[pv, &v], 2)?.contiguous()?,
                ),
                _ => (k, v),
            };
            let attended = layer.self_attention.attend(&normed, &k, &v, causal)?;
            cache.keys[index] = Some(k);
            cache.values[index] = Some(v);
            x = (x + attended)?;
            let (mk, mv) = &cache.memory[index];
            let crossed = layer
                .cross_attention
                .attend(&layer.norm2.forward(&x)?, mk, mv, None)?;
            x = (&x + crossed)?;
            x = (&x + layer.feed_forward(&x)?)?;
        }
        self.output.forward(&self.norm.forward(&x)?)
    }

    /// `ReadDecoder.forward`: teacher-forced logits `[B, T, V]` (f32) for `[B, T]` tokens
    /// over `memory` `[B, N, D]`, every position causal. For the parity test.
    pub fn logits(&self, tokens: &Tensor, memory: &Tensor) -> Result<Tensor> {
        let length = tokens.dim(1)?;
        let mut cache = self.cache(memory)?;
        let mask: Vec<u8> = (0..length)
            .flat_map(|row| (0..length).map(move |column| u8::from(column <= row)))
            .collect();
        let mask = Tensor::from_vec(mask, (1, 1, length, length), memory.device())?;
        Ok(self
            .step(self.embed(tokens, 0)?, &mut cache, Some(&mask))?
            .to_dtype(DType::F32)?)
    }

    /// `ReadDecoder.generate`: greedy, until every row has ended or the length runs out.
    /// Returns each row's tokens after the start token, and its text.
    pub fn generate(&self, memory: &Tensor) -> Result<Vec<(Vec<u32>, String)>> {
        let batch = memory.dim(0)?;
        let device = memory.device();
        let mut cache = self.cache(memory)?;
        let mut rows: Vec<Vec<u32>> = vec![Vec::new(); batch];
        let mut finished = vec![false; batch];
        let mut current = Tensor::full(READ_BOS, (batch, 1), device)?;
        for position in 0..self.max_length - 1 {
            let logits = self.step(self.embed(&current, position)?, &mut cache, None)?;
            let next: Vec<u32> = logits
                .squeeze(1)?
                .to_dtype(DType::F32)?
                .argmax(D::Minus1)?
                .to_vec1()?;
            let mut chosen = Vec::with_capacity(batch);
            for (row, &token) in next.iter().enumerate() {
                let token = if finished[row] { READ_PAD } else { token };
                rows[row].push(token);
                finished[row] |= token == READ_EOS;
                chosen.push(token);
            }
            if finished.iter().all(|&f| f) {
                break;
            }
            current = Tensor::from_vec(chosen, (batch, 1), device)?;
        }
        Ok(rows
            .into_iter()
            .map(|tokens| {
                let text = decode_text(&tokens, &self.alphabet);
                (tokens, text)
            })
            .collect())
    }

    /// Reads each crop of `image` (pixel boxes x0 y0 x1 y1; the whole image when empty):
    /// letterboxed, encoded and fused with the null phrase one by one (each crop as Python
    /// reads a batch of one), then decoded together.
    pub fn read(&self, model: &IrisModel, image: &Rgb, boxes: &[[f64; 4]]) -> Result<Vec<String>> {
        let whole = [[0.0, 0.0, image.width as f64, image.height as f64]];
        let boxes = if boxes.is_empty() { &whole[..] } else { boxes };
        if boxes.len() > MAX_CROPS {
            return Err(Error::Refused(format!(
                "at most {MAX_CROPS} crops per read"
            )));
        }
        let size = model.config.image_size;
        let mut states = Vec::with_capacity(boxes.len());
        for b in boxes {
            let crop = crop(image, *b)?;
            let tokens = model.encode_frame(&Frame::new(crop, size))?;
            states.push(model.fuse_null(&tokens)?);
        }
        let memory = Tensor::cat(&states, 0)?;
        Ok(self
            .generate(&memory)?
            .into_iter()
            .map(|(_, text)| text)
            .collect())
    }
}

/// The pixels of `image` inside `b` (x0 y0 x1 y1, clamped to the image, at least one pixel).
pub fn crop(image: &Rgb, b: [f64; 4]) -> Result<Rgb> {
    if b.iter().any(|v| !v.is_finite()) {
        return Err(Error::Refused("a box is not finite".into()));
    }
    let clamp = |v: f64, limit: usize| v.round().clamp(0.0, limit as f64) as usize;
    let (x0, x1) = (
        clamp(b[0].min(b[2]), image.width),
        clamp(b[0].max(b[2]), image.width),
    );
    let (y0, y1) = (
        clamp(b[1].min(b[3]), image.height),
        clamp(b[1].max(b[3]), image.height),
    );
    let (x1, y1) = (
        x1.max(x0 + 1).min(image.width),
        y1.max(y0 + 1).min(image.height),
    );
    let (x0, y0) = (x0.min(x1 - 1), y0.min(y1 - 1));
    let (width, height) = (x1 - x0, y1 - y0);
    let mut pixels = Vec::with_capacity(width * height * 3);
    for y in y0..y1 {
        let start = (y * image.width + x0) * 3;
        pixels.extend_from_slice(&image.pixels[start..start + width * 3]);
    }
    Rgb::new(width, height, pixels)
}

/// `x0,y0,x1,y1;x0,y0,x1,y1;…` as pixel boxes.
pub fn parse_boxes(text: &str) -> std::result::Result<Vec<[f64; 4]>, Error> {
    text.split(';')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let values: Vec<f64> = part
                .split(',')
                .map(|v| v.trim().parse::<f64>())
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| Error::Refused("boxes are x0,y0,x1,y1 numbers".into()))?;
            match values.as_slice() {
                [a, b, c, d] if values.iter().all(|v| v.is_finite()) => Ok([*a, *b, *c, *d]),
                _ => Err(Error::Refused("a box has four finite numbers".into())),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alphabet_is_the_trained_one() {
        let alphabet = characters();
        assert_eq!(alphabet.len(), 95 + 21);
        assert_eq!(alphabet[0], ' ');
        assert_eq!(alphabet[94], '~');
        assert_eq!(*alphabet.last().unwrap(), 'Ç');
        let tokens = [4 + 33, 4 + 95 + 10, READ_UNK, READ_EOS, 4 + 40];
        assert_eq!(decode_text(&tokens, &alphabet), "Añ?");
    }

    #[test]
    fn crops_are_clamped_and_never_empty() {
        let image = Rgb::new(4, 3, (0..36).collect()).unwrap();
        let c = crop(&image, [1.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.pixels, vec![15, 16, 17, 18, 19, 20]);
        let edge = crop(&image, [10.0, 10.0, 20.0, 20.0]).unwrap();
        assert_eq!((edge.width, edge.height), (1, 1));
        assert_eq!(edge.pixels, vec![33, 34, 35]);
        assert!(crop(&image, [f64::NAN, 0.0, 1.0, 1.0]).is_err());
        assert_eq!(
            parse_boxes("0,0,10,5; 1,2,3,4").unwrap(),
            vec![[0.0, 0.0, 10.0, 5.0], [1.0, 2.0, 3.0, 4.0]]
        );
        assert!(parse_boxes("1,2,3").is_err());
    }
}
