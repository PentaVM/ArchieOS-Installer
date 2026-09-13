use crate::{
    ModelManifest, NodeKind, ReflexDetection, ReflexError, ReflexSample, VisualReflexModel,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_CHECKPOINT_BYTES: usize = 1_048_576;
const FEATURES: usize = 8;
const HIDDEN: usize = 16;
const LABELS: [&str; 4] = ["button", "input", "text", "unknown_object"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentManifest {
    model_id: String,
    revision: String,
    checkpoint_sha256: String,
    input_width: u32,
    input_height: u32,
    quantization: String,
    training_data: Vec<String>,
    synthetic_only: bool,
    training_checkpoint_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Int8Checkpoint {
    format: String,
    source_epochs: u32,
    scale_micros: i64,
    input_weights: Vec<Vec<i16>>,
    input_bias: Vec<i64>,
    output_weights: Vec<Vec<i16>>,
    output_bias: Vec<i64>,
    labels: Vec<String>,
}

/// Deployed fixed-point model exported by the separate `PentaVM/archie-iris` Foundry.
pub struct FoundryReflexModel {
    manifest: ModelManifest,
    checkpoint: Int8Checkpoint,
}

impl FoundryReflexModel {
    pub fn load(manifest_json: &[u8], checkpoint_json: &[u8]) -> Result<Self, ReflexError> {
        if manifest_json.is_empty()
            || checkpoint_json.is_empty()
            || manifest_json.len() > 64 * 1024
            || checkpoint_json.len() > MAX_CHECKPOINT_BYTES
        {
            return Err(ReflexError::InvalidManifest);
        }
        let deployment: DeploymentManifest =
            serde_json::from_slice(manifest_json).map_err(|_| ReflexError::InvalidManifest)?;
        let observed = format!("{:x}", Sha256::digest(checkpoint_json));
        if observed != deployment.checkpoint_sha256
            || deployment.quantization != "int8"
            || !deployment.synthetic_only
            || deployment.training_data != ["archie-synthetic-v1"]
            || deployment.training_checkpoint_sha256.len() != 64
            || !deployment
                .training_checkpoint_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ReflexError::InvalidManifest);
        }
        let manifest = ModelManifest {
            model_id: deployment.model_id,
            revision: deployment.revision,
            checkpoint_sha256: deployment.checkpoint_sha256,
            input_width: deployment.input_width,
            input_height: deployment.input_height,
            quantization: Some(deployment.quantization),
        };
        manifest.validate()?;
        let checkpoint: Int8Checkpoint =
            serde_json::from_slice(checkpoint_json).map_err(|_| ReflexError::InvalidManifest)?;
        if checkpoint.format != "archie-iris-int8-reflex-v1"
            || checkpoint.source_epochs == 0
            || checkpoint.scale_micros <= 0
            || checkpoint.labels != LABELS
            || checkpoint.input_weights.len() != HIDDEN
            || checkpoint.input_weights.iter().any(|row| {
                row.len() != FEATURES || row.iter().any(|value| !(-127..=127).contains(value))
            })
            || checkpoint.input_bias.len() != HIDDEN
            || checkpoint.output_weights.len() != LABELS.len()
            || checkpoint.output_weights.iter().any(|row| {
                row.len() != HIDDEN || row.iter().any(|value| !(-127..=127).contains(value))
            })
            || checkpoint.output_bias.len() != LABELS.len()
        {
            return Err(ReflexError::InvalidManifest);
        }
        Ok(Self {
            manifest,
            checkpoint,
        })
    }

    fn restore(&self, value: i16) -> i64 {
        i64::from(value) * self.checkpoint.scale_micros / 1_000_000
    }

    fn features(sample: &ReflexSample<'_>) -> Result<[i64; FEATURES], ReflexError> {
        let crop = sample
            .frame
            .crop(sample.roi)
            .map_err(|_| ReflexError::InvalidInput)?;
        let pixels = crop.pixels();
        let channels = match crop.format() {
            crate::PixelFormat::Gray8 => 1,
            crate::PixelFormat::Rgb8 | crate::PixelFormat::Bgr8 => 3,
        };
        let mut sums = [0_u64; 3];
        let mut edges = 0_u64;
        let mut edge_count = 0_u64;
        for chunk in pixels.chunks_exact(channels) {
            if channels == 1 {
                let next = sums[0] + u64::from(chunk[0]);
                sums.fill(next);
            } else {
                let (red, green, blue) = if crop.format() == crate::PixelFormat::Bgr8 {
                    (chunk[2], chunk[1], chunk[0])
                } else {
                    (chunk[0], chunk[1], chunk[2])
                };
                sums[0] += u64::from(red);
                sums[1] += u64::from(green);
                sums[2] += u64::from(blue);
            }
        }
        for pair in pixels.windows(2) {
            edges += u64::from(pair[0].abs_diff(pair[1]));
            edge_count += 1;
        }
        let count = u64::from(crop.width()) * u64::from(crop.height());
        let source = sample.source.as_str();
        let source_code = if source.starts_with("browser") {
            100
        } else if source.starts_with("desktop") {
            300
        } else if source.starts_with("freecad") {
            500
        } else if source.starts_with("blender") {
            700
        } else {
            800
        };
        Ok([
            i64::from(sample.roi.width) * 1_000 / i64::from(sample.frame.width()),
            i64::from(sample.roi.height) * 1_000 / i64::from(sample.frame.height()),
            (sums[0] * 1_000 / count / 255) as i64,
            (sums[1] * 1_000 / count / 255) as i64,
            (sums[2] * 1_000 / count / 255) as i64,
            (edges * 1_000).checked_div(edge_count).unwrap_or(0) as i64 / 255,
            0,
            source_code,
        ])
    }

    fn classify(&self, sample: &ReflexSample<'_>) -> Result<(usize, u16, Vec<i16>), ReflexError> {
        let features = Self::features(sample)?;
        let hidden: Vec<i64> = self
            .checkpoint
            .input_weights
            .iter()
            .zip(&self.checkpoint.input_bias)
            .map(|(row, bias)| {
                (bias
                    + row
                        .iter()
                        .zip(features)
                        .map(|(weight, feature)| self.restore(*weight) * feature)
                        .sum::<i64>()
                        / 1_000)
                    .max(0)
            })
            .collect();
        let scores: Vec<i64> = self
            .checkpoint
            .output_weights
            .iter()
            .zip(&self.checkpoint.output_bias)
            .map(|(row, bias)| {
                bias + row
                    .iter()
                    .zip(&hidden)
                    .map(|(weight, value)| self.restore(*weight) * value)
                    .sum::<i64>()
                    / 1_000
            })
            .collect();
        let mut ranked: Vec<_> = scores.iter().copied().enumerate().collect();
        ranked.sort_by(|(left_index, left), (right_index, right)| {
            right.cmp(left).then(left_index.cmp(right_index))
        });
        let margin = ranked[0].1.saturating_sub(ranked[1].1).clamp(0, 500) as u16;
        let confidence = 500 + margin;
        let embedding = hidden
            .into_iter()
            .map(|value| value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16)
            .collect();
        Ok((ranked[0].0, confidence, embedding))
    }
}

impl VisualReflexModel for FoundryReflexModel {
    fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    fn infer(&self, samples: &[ReflexSample<'_>]) -> Result<Vec<ReflexDetection>, ReflexError> {
        samples
            .iter()
            .map(|sample| {
                let (label, confidence_permille, embedding) = self.classify(sample)?;
                Ok(ReflexDetection {
                    source: sample.source.clone(),
                    kind: match LABELS[label] {
                        "button" => NodeKind::Button,
                        "input" => NodeKind::Input,
                        "text" => NodeKind::Text,
                        _ => NodeKind::UnknownObject,
                    },
                    bbox: sample.roi,
                    confidence_permille,
                    uncertainty_permille: 1_000 - confidence_permille,
                    embedding,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EphemeralFrame, PixelFormat, Rect, SourceId, VisualReflexRuntime};

    const MANIFEST: &[u8] = include_bytes!("../tests/fixtures/foundry/model-manifest.json");
    const CHECKPOINT: &[u8] = include_bytes!("../tests/fixtures/foundry/model.int8.json");

    #[test]
    fn foundry_checkpoint_loads_and_runs_in_archieos() {
        let model = FoundryReflexModel::load(MANIFEST, CHECKPOINT).unwrap();
        let source = SourceId::new("camera:0").unwrap();
        let frame =
            EphemeralFrame::new(source.clone(), 1, 4, 4, PixelFormat::Rgb8, vec![20; 48]).unwrap();
        let samples = [ReflexSample {
            source: &source,
            frame: &frame,
            roi: Rect::new(0, 0, 4, 4).unwrap(),
        }];
        let result = VisualReflexRuntime::new(model, 1, 500)
            .unwrap()
            .run(&samples)
            .unwrap();
        assert_eq!(result.detections.len(), 1);
        assert!(result.detections[0].confidence_permille <= 1_000);
    }

    #[test]
    fn checkpoint_hash_mismatch_fails_before_inference() {
        let mut changed = CHECKPOINT.to_vec();
        changed[10] ^= 1;
        assert!(matches!(
            FoundryReflexModel::load(MANIFEST, &changed),
            Err(ReflexError::InvalidManifest)
        ));
    }
}
