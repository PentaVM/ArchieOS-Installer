//! Open-vocabulary detection: `IrisDetector.detect` and `detect_everything`.

use crate::model::{EncodedPhrases, IrisModel};
use crate::nn::Result;
use crate::phrases::PhraseSource;
use crate::preprocess::{letterbox, to_tensor, Rgb};
use crate::roi::nms;
use candle_core::{DType, Tensor};
use serde::Serialize;

/// The broad everyday vocabulary "everything" mode reads in one batched pass
/// (`iris_infer.EVERYDAY_VOCABULARY`, in its order).
pub const EVERYDAY_VOCABULARY: &str = include_str!("vocabulary.txt");

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Detection {
    pub phrase: String,
    pub score: f64,
    /// Pixels in the input image, x0 y0 x1 y1.
    #[serde(rename = "box")]
    pub bbox: [f64; 4],
    pub point: [f64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f64>>,
}

/// Python's `round(value, digits)`: correctly rounded, halves to even.
pub fn round_to(value: f64, digits: i32) -> f64 {
    let text = format!("{value:.*}", digits as usize);
    text.parse().unwrap_or(value)
}

/// Canvas-normalised xyxy back to input pixels (`canvas_to_pixels`), clipped to the image.
fn to_pixels(b: [f32; 4], width: usize, height: usize, size: usize) -> [f64; 4] {
    let scale = size as f64 / width.max(height) as f64;
    let f = |v: f32| f64::from(v) * size as f64 / scale;
    [
        f(b[0]).clamp(0.0, width as f64),
        f(b[1]).clamp(0.0, height as f64),
        f(b[2]).clamp(0.0, width as f64),
        f(b[3]).clamp(0.0, height as f64),
    ]
}

fn xyxy(b: &[f32]) -> [f32; 4] {
    let (cx, cy, w, h) = (b[0], b[1], b[2], b[3]);
    [
        (cx - w / 2.0).clamp(0.0, 1.0),
        (cy - h / 2.0).clamp(0.0, 1.0),
        (cx + w / 2.0).clamp(0.0, 1.0),
        (cy + h / 2.0).clamp(0.0, 1.0),
    ]
}

fn detection(
    phrase: &str,
    score: f32,
    canvas_box: [f32; 4],
    image: &Rgb,
    size: usize,
    embedding: Option<Vec<f64>>,
) -> Detection {
    let pixels = to_pixels(canvas_box, image.width, image.height, size);
    let b = pixels.map(|v| round_to(v, 1));
    Detection {
        phrase: phrase.to_owned(),
        score: round_to(f64::from(score), 4),
        bbox: b,
        point: [
            round_to((b[0] + b[2]) / 2.0, 1),
            round_to((b[1] + b[3]) / 2.0, 1),
        ],
        embedding,
    }
}

pub struct Detector {
    pub model: IrisModel,
    pub phrases: Box<dyn PhraseSource>,
}

impl Detector {
    pub fn canvas(&self, image: &Rgb) -> Result<Tensor> {
        let (canvas, _) = letterbox(image, self.model.config.image_size);
        to_tensor(&canvas, &self.model.device)
    }

    /// Encoded phrases for one batch row per entry of `chunks`.
    pub fn encode_chunks(&mut self, chunks: &[Vec<String>]) -> Result<EncodedPhrases> {
        let features = chunks
            .iter()
            .map(|chunk| self.phrases.encode(chunk))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.model.encode_phrases(&features)
    }

    /// `detect`: the phrases asked for, each with its own boxes.
    pub fn detect(
        &mut self,
        image: &Rgb,
        phrases: &[String],
        threshold: f32,
        max_per_phrase: usize,
        nms_iou: f32,
        with_embeddings: bool,
    ) -> Result<Vec<Detection>> {
        let phrases: Vec<String> = phrases
            .iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .take(self.model.config.max_phrases)
            .collect();
        if phrases.is_empty() {
            return Ok(Vec::new());
        }
        let tokens = self.model.encode_image(&self.canvas(image)?)?;
        let encoded = self.encode_chunks(std::slice::from_ref(&phrases))?;
        let (fused, text) = self
            .model
            .fuse(&tokens, &encoded.text, &encoded.text_padding)?;
        let decoded = self.model.decode(&fused, &text, &encoded)?;
        let scores: Vec<Vec<f32>> = candle_nn::ops::sigmoid(&decoded.logits.get(0)?)?.to_vec2()?;
        let boxes: Vec<Vec<f32>> = decoded.boxes.get(0)?.to_dtype(DType::F32)?.to_vec2()?;
        let boxes: Vec<[f32; 4]> = boxes.iter().map(|b| xyxy(b)).collect();
        let size = self.model.config.image_size;
        let mut found = Vec::new();
        for (column, phrase) in phrases.iter().enumerate() {
            let candidates: Vec<usize> = (0..scores.len())
                .filter(|&q| scores[q][column] >= threshold)
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let kept_boxes: Vec<[f32; 4]> = candidates.iter().map(|&q| boxes[q]).collect();
            let kept_scores: Vec<f32> = candidates.iter().map(|&q| scores[q][column]).collect();
            let kept: Vec<usize> = nms(&kept_boxes, &kept_scores, nms_iou)
                .into_iter()
                .take(max_per_phrase)
                .map(|i| candidates[i])
                .collect();
            let embeddings = if with_embeddings {
                let chosen: Vec<[f32; 4]> = kept.iter().map(|&q| boxes[q]).collect();
                Some(self.model.instance_embeddings(&fused.get(0)?, &chosen)?)
            } else {
                None
            };
            for (position, &query) in kept.iter().enumerate() {
                let embedding = embeddings.as_ref().map(|all| {
                    all[position]
                        .iter()
                        .map(|&v| round_to(f64::from(v), 5))
                        .collect()
                });
                found.push(detection(
                    phrase,
                    scores[query][column],
                    boxes[query],
                    image,
                    size,
                    embedding,
                ));
            }
        }
        // `sorted(key=-score)` is stable: equal scores keep phrase order.
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(found)
    }

    /// `detect_everything`: the everyday vocabulary (after `extra`) in chunks of
    /// `max_phrases`, one batch row each, read against one encoding of the image.
    pub fn detect_everything(
        &mut self,
        image: &Rgb,
        threshold: f32,
        extra: &[String],
        max_detections: usize,
        nms_iou: f32,
        with_embeddings: bool,
    ) -> Result<Vec<Detection>> {
        let mut vocabulary: Vec<String> = Vec::new();
        for phrase in extra
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .chain(EVERYDAY_VOCABULARY.lines())
        {
            if !vocabulary.iter().any(|v| v == phrase) {
                vocabulary.push(phrase.to_owned());
            }
        }
        let chunks: Vec<Vec<String>> = vocabulary
            .chunks(self.model.config.max_phrases)
            .map(<[String]>::to_vec)
            .collect();
        let tokens = self.model.encode_image(&self.canvas(image)?)?;
        let encoded = self.encode_chunks(&chunks)?;
        let tokens = tokens
            .broadcast_as((chunks.len(), tokens.dim(1)?, tokens.dim(2)?))?
            .contiguous()?;
        let (fused, text) = self
            .model
            .fuse(&tokens, &encoded.text, &encoded.text_padding)?;
        let decoded = self.model.decode(&fused, &text, &encoded)?;
        let scores = candle_nn::ops::sigmoid(&decoded.logits)?;
        let (batch, queries, width) = scores.dims3()?;
        let padding = encoded
            .phrase_padding
            .unsqueeze(1)?
            .broadcast_as((batch, queries, width))?;
        let scores = padding.where_cond(
            &Tensor::zeros((batch, queries, width), DType::F32, scores.device())?,
            &scores,
        )?;
        let best: Vec<f32> = scores.max(2)?.flatten_all()?.to_vec1()?;
        let column: Vec<u32> = scores.argmax(2)?.flatten_all()?.to_vec1()?;
        let boxes: Vec<[f32; 4]> = decoded
            .boxes
            .to_dtype(DType::F32)?
            .reshape((batch * queries, 4))?
            .to_vec2::<f32>()?
            .iter()
            .map(|b| xyxy(b))
            .collect();
        let candidates: Vec<usize> = (0..best.len()).filter(|&i| best[i] >= threshold).collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let kept: Vec<usize> = nms(
            &candidates.iter().map(|&i| boxes[i]).collect::<Vec<_>>(),
            &candidates.iter().map(|&i| best[i]).collect::<Vec<_>>(),
            nms_iou,
        )
        .into_iter()
        .take(max_detections)
        .map(|i| candidates[i])
        .collect();
        let size = self.model.config.image_size;
        let mut found = Vec::with_capacity(kept.len());
        for index in kept {
            let chunk = index / queries;
            let phrase = &chunks[chunk][column[index] as usize];
            let embedding = if with_embeddings {
                let vector = self
                    .model
                    .instance_embeddings(&fused.get(chunk)?, &[boxes[index]])?;
                Some(
                    vector[0]
                        .iter()
                        .map(|&v| round_to(f64::from(v), 4))
                        .collect(),
                )
            } else {
                None
            };
            found.push(detection(
                phrase,
                best[index],
                boxes[index],
                image,
                size,
                embedding,
            ));
        }
        Ok(found)
    }
}
