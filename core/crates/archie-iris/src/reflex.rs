use crate::{EphemeralFrame, NodeKind, Rect, SourceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_BATCH: usize = 64;
const MAX_DETECTIONS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub model_id: String,
    pub revision: String,
    pub checkpoint_sha256: String,
    pub input_width: u32,
    pub input_height: u32,
    pub quantization: Option<String>,
}

impl ModelManifest {
    pub fn validate(&self) -> Result<(), ReflexError> {
        let bounded =
            |value: &str| !value.is_empty() && value.len() <= 128 && !value.contains('\0');
        if !bounded(&self.model_id)
            || !bounded(&self.revision)
            || self.checkpoint_sha256.len() != 64
            || !self
                .checkpoint_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.input_width == 0
            || self.input_height == 0
            || self.input_width > 8_192
            || self.input_height > 8_192
            || self
                .quantization
                .as_ref()
                .is_some_and(|value| !matches!(value.as_str(), "int8" | "fp16" | "fp32"))
        {
            return Err(ReflexError::InvalidManifest);
        }
        Ok(())
    }
}

pub struct ReflexSample<'a> {
    pub source: &'a SourceId,
    pub frame: &'a EphemeralFrame,
    pub roi: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflexDetection {
    pub source: SourceId,
    pub kind: NodeKind,
    pub bbox: Rect,
    pub confidence_permille: u16,
    pub uncertainty_permille: u16,
    pub embedding: Vec<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflexBatchResult {
    pub detections: Vec<ReflexDetection>,
    pub escalate_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReflexError {
    #[error("visual reflex model manifest is invalid")]
    InvalidManifest,
    #[error("visual reflex input is malformed or outside its bound")]
    InvalidInput,
    #[error("visual reflex output is malformed or outside its bound")]
    InvalidOutput,
    #[error("visual reflex provider failed")]
    Provider,
}

pub trait VisualReflexModel: Send + Sync {
    fn manifest(&self) -> &ModelManifest;
    fn infer(&self, samples: &[ReflexSample<'_>]) -> Result<Vec<ReflexDetection>, ReflexError>;
}

pub struct VisualReflexRuntime<M> {
    model: M,
    batch_size: usize,
    escalation_uncertainty_permille: u16,
}

impl<M: VisualReflexModel> VisualReflexRuntime<M> {
    pub fn new(
        model: M,
        batch_size: usize,
        escalation_uncertainty_permille: u16,
    ) -> Result<Self, ReflexError> {
        model.manifest().validate()?;
        if batch_size == 0 || batch_size > MAX_BATCH || escalation_uncertainty_permille > 1_000 {
            return Err(ReflexError::InvalidInput);
        }
        Ok(Self {
            model,
            batch_size,
            escalation_uncertainty_permille,
        })
    }

    pub fn run(&self, samples: &[ReflexSample<'_>]) -> Result<ReflexBatchResult, ReflexError> {
        if samples.is_empty() || samples.len() > MAX_BATCH * 16 {
            return Err(ReflexError::InvalidInput);
        }
        for sample in samples {
            sample
                .roi
                .validate()
                .map_err(|_| ReflexError::InvalidInput)?;
            if sample.frame.source() != sample.source
                || sample.roi.x < 0
                || sample.roi.y < 0
                || sample.roi.right() > i64::from(sample.frame.width())
                || sample.roi.bottom() > i64::from(sample.frame.height())
            {
                return Err(ReflexError::InvalidInput);
            }
        }

        let mut detections = Vec::new();
        for batch in samples.chunks(self.batch_size) {
            let output = self.model.infer(batch)?;
            if detections.len().saturating_add(output.len()) > MAX_DETECTIONS {
                return Err(ReflexError::InvalidOutput);
            }
            for detection in output {
                validate_detection(&detection, samples)?;
                detections.push(detection);
            }
        }
        let escalate_indices = detections
            .iter()
            .enumerate()
            .filter_map(|(index, detection)| {
                (detection.uncertainty_permille >= self.escalation_uncertainty_permille)
                    .then_some(index)
            })
            .collect();
        Ok(ReflexBatchResult {
            detections,
            escalate_indices,
        })
    }

    pub fn model(&self) -> &M {
        &self.model
    }
}

fn validate_detection(
    detection: &ReflexDetection,
    samples: &[ReflexSample<'_>],
) -> Result<(), ReflexError> {
    detection
        .bbox
        .validate()
        .map_err(|_| ReflexError::InvalidOutput)?;
    if detection.confidence_permille > 1_000
        || detection.uncertainty_permille > 1_000
        || detection.embedding.len() > 256
        || !samples.iter().any(|sample| {
            *sample.source == detection.source
                && detection.bbox.x >= sample.roi.x
                && detection.bbox.y >= sample.roi.y
                && detection.bbox.right() <= sample.roi.right()
                && detection.bbox.bottom() <= sample.roi.bottom()
        })
    {
        return Err(ReflexError::InvalidOutput);
    }
    Ok(())
}

/// Honest no-training baseline. It emits only `UnknownObject` and derives an uncertainty
/// score from local intensity spread. It is replaceable by a Foundry checkpoint.
pub struct DeterministicReflexBaseline {
    manifest: ModelManifest,
}

impl Default for DeterministicReflexBaseline {
    fn default() -> Self {
        Self {
            manifest: ModelManifest {
                model_id: "iris-deterministic-baseline".into(),
                revision: "1".into(),
                checkpoint_sha256: "0".repeat(64),
                input_width: 256,
                input_height: 256,
                quantization: None,
            },
        }
    }
}

impl VisualReflexModel for DeterministicReflexBaseline {
    fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    fn infer(&self, samples: &[ReflexSample<'_>]) -> Result<Vec<ReflexDetection>, ReflexError> {
        let mut output = Vec::with_capacity(samples.len());
        for sample in samples {
            let crop = sample
                .frame
                .crop(sample.roi)
                .map_err(|_| ReflexError::InvalidInput)?;
            let pixels = crop.pixels();
            let minimum = pixels.iter().copied().min().unwrap_or(0);
            let maximum = pixels.iter().copied().max().unwrap_or(0);
            let spread = u16::from(maximum.saturating_sub(minimum));
            let confidence = 100_u16.saturating_add(spread.saturating_mul(2)).min(600);
            output.push(ReflexDetection {
                source: sample.source.clone(),
                kind: NodeKind::UnknownObject,
                bbox: sample.roi,
                confidence_permille: confidence,
                uncertainty_permille: 1_000 - confidence,
                embedding: vec![i16::from(minimum), i16::from(maximum)],
            });
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingModel {
        manifest: ModelManifest,
        calls: AtomicUsize,
    }

    struct EscapingModel {
        manifest: ModelManifest,
    }

    impl VisualReflexModel for EscapingModel {
        fn manifest(&self) -> &ModelManifest {
            &self.manifest
        }

        fn infer(&self, samples: &[ReflexSample<'_>]) -> Result<Vec<ReflexDetection>, ReflexError> {
            Ok(vec![ReflexDetection {
                source: samples[0].source.clone(),
                kind: NodeKind::Button,
                bbox: Rect::new(0, 0, 4, 4).unwrap(),
                confidence_permille: 900,
                uncertainty_permille: 100,
                embedding: Vec::new(),
            }])
        }
    }

    impl VisualReflexModel for CountingModel {
        fn manifest(&self) -> &ModelManifest {
            &self.manifest
        }

        fn infer(&self, samples: &[ReflexSample<'_>]) -> Result<Vec<ReflexDetection>, ReflexError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(samples
                .iter()
                .map(|sample| ReflexDetection {
                    source: sample.source.clone(),
                    kind: NodeKind::Button,
                    bbox: sample.roi,
                    confidence_permille: 800,
                    uncertainty_permille: 200,
                    embedding: vec![1, 2],
                })
                .collect())
        }
    }

    fn manifest() -> ModelManifest {
        ModelManifest {
            model_id: "fixture".into(),
            revision: "1".into(),
            checkpoint_sha256: "a".repeat(64),
            input_width: 32,
            input_height: 32,
            quantization: Some("int8".into()),
        }
    }

    #[test]
    fn runtime_micro_batches_and_marks_uncertainty() {
        let source = SourceId::new("image:1").unwrap();
        let frame = EphemeralFrame::new(
            source.clone(),
            1,
            4,
            4,
            crate::PixelFormat::Gray8,
            vec![0; 16],
        )
        .unwrap();
        let roi = Rect::new(0, 0, 2, 2).unwrap();
        let samples: Vec<_> = (0..5)
            .map(|_| ReflexSample {
                source: &source,
                frame: &frame,
                roi,
            })
            .collect();
        let runtime = VisualReflexRuntime::new(
            CountingModel {
                manifest: manifest(),
                calls: AtomicUsize::new(0),
            },
            2,
            200,
        )
        .unwrap();
        let result = runtime.run(&samples).unwrap();
        assert_eq!(runtime.model().calls.load(Ordering::SeqCst), 3);
        assert_eq!(result.detections.len(), 5);
        assert_eq!(result.escalate_indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn deterministic_baseline_is_bounded_and_does_not_claim_a_class() {
        let source = SourceId::new("image:1").unwrap();
        let frame = EphemeralFrame::new(
            source.clone(),
            1,
            2,
            2,
            PixelFormat::Gray8,
            vec![0, 10, 20, 30],
        )
        .unwrap();
        let samples = [ReflexSample {
            source: &source,
            frame: &frame,
            roi: Rect::new(0, 0, 2, 2).unwrap(),
        }];
        let output = VisualReflexRuntime::new(DeterministicReflexBaseline::default(), 1, 400)
            .unwrap()
            .run(&samples)
            .unwrap();
        assert_eq!(output.detections[0].kind, NodeKind::UnknownObject);
        assert_eq!(output.escalate_indices, vec![0]);
    }

    #[test]
    fn output_outside_requested_roi_is_rejected() {
        let source = SourceId::new("image:1").unwrap();
        let frame =
            EphemeralFrame::new(source.clone(), 1, 4, 4, PixelFormat::Gray8, vec![0; 16]).unwrap();
        let model = EscapingModel {
            manifest: manifest(),
        };
        let sample = [ReflexSample {
            source: &source,
            frame: &frame,
            roi: Rect::new(0, 0, 2, 2).unwrap(),
        }];
        assert_eq!(
            VisualReflexRuntime::new(model, 1, 500)
                .unwrap()
                .run(&sample),
            Err(ReflexError::InvalidOutput)
        );
    }
}
