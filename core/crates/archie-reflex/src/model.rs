//! The model as it is deployed: whole numbers, a checkpoint and a manifest.
//!
//! Training happens in fractions; running does not have to. The weights are scaled into
//! bytes, one scale to a layer, which is what IRIS's deployment path asks for and what
//! makes a checkpoint mean the same thing on every machine that loads it.
//!
//! A checkpoint is believed only if its digest matches the manifest beside it.

use crate::net::{Conv, Detector, Prediction};
use crate::{ControlKind, ReflexError, Tile, TILE_HEIGHT, TILE_WIDTH};
use archie_iris::{ModelManifest, Rect, ReflexDetection, ReflexSample, VisualReflexModel};
use std::path::Path;

/// What every checkpoint starts with, so a file that is not one is refused before it is
/// read as one.
const MAGIC: &[u8; 16] = b"archie-reflex/1\0";
/// Weights are scaled into this range, which is what fits a byte.
const SCALE_RANGE: f32 = 127.0;

/// One layer in whole numbers: bytes for the weights, and the scale that brings them back.
#[derive(Debug, Clone, PartialEq)]
struct Layer {
    in_channels: usize,
    out_channels: usize,
    kernel: usize,
    stride: usize,
    pad: usize,
    weights: Vec<i8>,
    scale: f32,
    biases: Vec<f32>,
}

impl Layer {
    fn of(conv: &Conv) -> Self {
        let largest = conv
            .weights
            .iter()
            .fold(0.0_f32, |largest, weight| largest.max(weight.abs()))
            .max(1e-6);
        let scale = largest / SCALE_RANGE;
        Self {
            in_channels: conv.in_channels,
            out_channels: conv.out_channels,
            kernel: conv.kernel,
            stride: conv.stride,
            pad: conv.pad,
            weights: conv
                .weights
                .iter()
                .map(|weight| (weight / scale).round().clamp(-127.0, 127.0) as i8)
                .collect(),
            scale,
            biases: conv.biases.clone(),
        }
    }

    fn as_conv(&self) -> Conv {
        Conv {
            in_channels: self.in_channels,
            out_channels: self.out_channels,
            kernel: self.kernel,
            stride: self.stride,
            pad: self.pad,
            weights: self
                .weights
                .iter()
                .map(|weight| f32::from(*weight) * self.scale)
                .collect(),
            biases: self.biases.clone(),
        }
    }
}

/// A trained model, ready to be loaded and run.
#[derive(Debug, Clone, PartialEq)]
pub struct Deployed {
    manifest: ModelManifest,
    layers: Vec<Layer>,
}

impl Deployed {
    /// Scales a trained detector into bytes.
    pub fn of(detector: &Detector, model_id: &str, revision: &str) -> Result<Self, ReflexError> {
        let mut deployed = Self {
            manifest: ModelManifest {
                model_id: model_id.to_string(),
                revision: revision.to_string(),
                checkpoint_sha256: String::new(),
                input_width: TILE_WIDTH as u32,
                input_height: TILE_HEIGHT as u32,
                // IRIS knows three words for this; the layer-wise scaling is ours.
                quantization: Some("int8".into()),
            },
            layers: detector.layers.iter().map(Layer::of).collect(),
        };
        let bytes = deployed.checkpoint();
        deployed.manifest.checkpoint_sha256 =
            hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
        deployed
            .manifest
            .validate()
            .map_err(|_| ReflexError::Checkpoint)?;
        Ok(deployed)
    }

    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    /// What the model says about one tile, in the tile's own coordinates.
    pub fn detect(&self, tile: &Tile, least_confidence_permille: u16) -> Vec<Prediction> {
        self.as_detector()
            .detect(tile.pixels(), least_confidence_permille)
    }

    fn as_detector(&self) -> Detector {
        Detector {
            layers: self.layers.iter().map(Layer::as_conv).collect(),
        }
    }

    /// The checkpoint's bytes: each layer's shape, its scale, its weights and its biases.
    pub fn checkpoint(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8_192);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(self.layers.len() as u32).to_le_bytes());
        for layer in &self.layers {
            for value in [
                layer.in_channels,
                layer.out_channels,
                layer.kernel,
                layer.stride,
                layer.pad,
            ] {
                bytes.extend_from_slice(&(value as u32).to_le_bytes());
            }
            bytes.extend_from_slice(&layer.scale.to_le_bytes());
            bytes.extend_from_slice(&(layer.weights.len() as u32).to_le_bytes());
            bytes.extend(layer.weights.iter().map(|weight| *weight as u8));
            bytes.extend_from_slice(&(layer.biases.len() as u32).to_le_bytes());
            for bias in &layer.biases {
                bytes.extend_from_slice(&bias.to_le_bytes());
            }
        }
        bytes
    }
}

impl Deployed {
    /// Reads a checkpoint, believing it only if it is what the manifest says it is.
    pub fn read(manifest: ModelManifest, bytes: &[u8]) -> Result<Self, ReflexError> {
        manifest.validate().map_err(|_| ReflexError::Checkpoint)?;
        let digest = hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref());
        if digest != manifest.checkpoint_sha256 {
            return Err(ReflexError::Checkpoint);
        }
        if manifest.input_width as usize != TILE_WIDTH
            || manifest.input_height as usize != TILE_HEIGHT
        {
            return Err(ReflexError::Shape);
        }
        let mut at = 0_usize;
        let mut take = |count: usize| -> Result<&[u8], ReflexError> {
            let slice = bytes
                .get(at..at.checked_add(count).ok_or(ReflexError::Checkpoint)?)
                .ok_or(ReflexError::Checkpoint)?;
            at += count;
            Ok(slice)
        };
        if take(MAGIC.len())? != MAGIC {
            return Err(ReflexError::Checkpoint);
        }
        let count = number(take(4)?)? as usize;
        if count == 0 || count > 8 {
            return Err(ReflexError::Shape);
        }
        let mut layers = Vec::with_capacity(count);
        for _ in 0..count {
            let in_channels = number(take(4)?)? as usize;
            let out_channels = number(take(4)?)? as usize;
            let kernel = number(take(4)?)? as usize;
            let stride = number(take(4)?)? as usize;
            let pad = number(take(4)?)? as usize;
            let scale =
                f32::from_le_bytes(take(4)?.try_into().map_err(|_| ReflexError::Checkpoint)?);
            let weight_count = number(take(4)?)? as usize;
            if in_channels == 0
                || out_channels == 0
                || kernel == 0
                || kernel > 7
                || stride == 0
                || stride > 4
                || pad > 3
                || !scale.is_finite()
                || scale <= 0.0
                || weight_count != out_channels * in_channels * kernel * kernel
                || weight_count > 1_000_000
            {
                return Err(ReflexError::Shape);
            }
            let weights = take(weight_count)?.iter().map(|byte| *byte as i8).collect();
            let bias_count = number(take(4)?)? as usize;
            if bias_count != out_channels {
                return Err(ReflexError::Shape);
            }
            let mut biases = Vec::with_capacity(bias_count);
            for _ in 0..bias_count {
                biases.push(f32::from_le_bytes(
                    take(4)?.try_into().map_err(|_| ReflexError::Checkpoint)?,
                ));
            }
            if biases.iter().any(|bias| !bias.is_finite()) {
                return Err(ReflexError::Shape);
            }
            layers.push(Layer {
                in_channels,
                out_channels,
                kernel,
                stride,
                pad,
                weights,
                scale,
                biases,
            });
        }
        if at != bytes.len() {
            return Err(ReflexError::Checkpoint);
        }
        Ok(Self { manifest, layers })
    }

    /// Writes the checkpoint and its manifest beside each other.
    pub fn write(&self, directory: &Path) -> Result<(), ReflexError> {
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join("reflex.checkpoint"), self.checkpoint())?;
        std::fs::write(
            directory.join("reflex.manifest.json"),
            serde_json::to_string_pretty(&ManifestFile::of(&self.manifest))
                .map_err(|_| ReflexError::Checkpoint)?,
        )?;
        Ok(())
    }

    /// Loads what `write` left.
    pub fn load(directory: &Path) -> Result<Self, ReflexError> {
        let manifest: ManifestFile =
            serde_json::from_slice(&std::fs::read(directory.join("reflex.manifest.json"))?)
                .map_err(|_| ReflexError::Checkpoint)?;
        let bytes = std::fs::read(directory.join("reflex.checkpoint"))?;
        Self::read(manifest.into(), &bytes)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ManifestFile {
    model_id: String,
    revision: String,
    checkpoint_sha256: String,
    input_width: u32,
    input_height: u32,
    quantization: Option<String>,
}

impl ManifestFile {
    fn of(manifest: &ModelManifest) -> Self {
        Self {
            model_id: manifest.model_id.clone(),
            revision: manifest.revision.clone(),
            checkpoint_sha256: manifest.checkpoint_sha256.clone(),
            input_width: manifest.input_width,
            input_height: manifest.input_height,
            quantization: manifest.quantization.clone(),
        }
    }
}

impl From<ManifestFile> for ModelManifest {
    fn from(file: ManifestFile) -> Self {
        Self {
            model_id: file.model_id,
            revision: file.revision,
            checkpoint_sha256: file.checkpoint_sha256,
            input_width: file.input_width,
            input_height: file.input_height,
            quantization: file.quantization,
        }
    }
}

fn number(bytes: &[u8]) -> Result<u32, ReflexError> {
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| ReflexError::Checkpoint)?,
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// IRIS's reflex, as IRIS asks for it: a model behind a runtime that batches it, bounds
/// what it says and escalates what it is unsure of.
impl VisualReflexModel for Deployed {
    fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    fn infer(
        &self,
        samples: &[ReflexSample<'_>],
    ) -> Result<Vec<ReflexDetection>, archie_iris::ReflexError> {
        let detector = self.as_detector();
        let mut detections = Vec::new();
        for sample in samples {
            let frame = sample.frame;
            if frame.width() as usize != TILE_WIDTH
                || frame.height() as usize != TILE_HEIGHT
                || frame.format() != archie_iris::PixelFormat::Gray8
            {
                return Err(archie_iris::ReflexError::InvalidInput);
            }
            for found in detector.detect(frame.pixels(), 300) {
                let x = found.control.x.round().max(0.0) as i32;
                let y = found.control.y.round().max(0.0) as i32;
                let width = found.control.width.round().max(1.0) as u32;
                let height = found.control.height.round().max(1.0) as u32;
                let Ok(bbox) = Rect::new(x.max(sample.roi.x), y.max(sample.roi.y), width, height)
                else {
                    continue;
                };
                // What is asked about is a region; an answer outside it is not an answer.
                if bbox.right() > sample.roi.right() || bbox.bottom() > sample.roi.bottom() {
                    continue;
                }
                detections.push(ReflexDetection {
                    source: sample.source.clone(),
                    kind: found.control.kind.as_node_kind(),
                    bbox,
                    confidence_permille: found.confidence_permille,
                    uncertainty_permille: found.uncertainty_permille,
                    embedding: Vec::new(),
                });
            }
        }
        Ok(detections)
    }
}

/// What a kind of scene node is to this model.
pub fn kind_of(node: archie_iris::NodeKind) -> ControlKind {
    ControlKind::of(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Detector;

    #[test]
    fn a_checkpoint_is_believed_only_as_far_as_its_digest() {
        let detector = Detector::new(11);
        let deployed = Deployed::of(&detector, "archie.reflex-controls", "test").unwrap();
        let bytes = deployed.checkpoint();
        let read = Deployed::read(deployed.manifest().clone(), &bytes).unwrap();
        assert_eq!(read, deployed, "the same model comes back");

        let mut tampered = bytes.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x40;
        assert!(
            matches!(
                Deployed::read(deployed.manifest().clone(), &tampered),
                Err(ReflexError::Checkpoint)
            ),
            "a changed weight is a different model"
        );
        let mut manifest = deployed.manifest().clone();
        manifest.input_width = 64;
        assert!(
            matches!(Deployed::read(manifest, &bytes), Err(ReflexError::Shape)),
            "a model that takes a different picture is a different model"
        );
        assert!(matches!(
            Deployed::read(deployed.manifest().clone(), &bytes[..bytes.len() - 4]),
            Err(ReflexError::Checkpoint)
        ));
    }

    #[test]
    fn scaling_into_bytes_keeps_what_the_model_saw() {
        // A model that has learned nothing still has to survive being made small: the same
        // tile through both should say nearly the same thing.
        let detector = Detector::new(3);
        let deployed = Deployed::of(&detector, "archie.reflex-controls", "test").unwrap();
        let mut pixels = vec![200_u8; TILE_WIDTH * TILE_HEIGHT];
        for y in 30..50 {
            for x in 40..90 {
                pixels[y * TILE_WIDTH + x] = 60;
            }
        }
        let tile = Tile::from_pixels(pixels, 1_600, 960).unwrap();
        let before = detector.detect(tile.pixels(), 0);
        let after = deployed.detect(&tile, 0);
        assert_eq!(before.len(), after.len());
        for (float, byte) in before.iter().zip(after.iter()) {
            assert!(
                (i32::from(float.confidence_permille) - i32::from(byte.confidence_permille)).abs()
                    <= 40,
                "{float:?} became {byte:?}"
            );
        }
    }
}
