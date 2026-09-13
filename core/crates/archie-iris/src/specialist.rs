use crate::{EphemeralFrame, NodeId, NodeKind, Rect, SourceId};
use archie_types::Locality;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_REGIONS: usize = 256;
const MAX_RESULTS: usize = 512;
const MAX_TEXT_BYTES: usize = 8_192;
const MAX_QUERY_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistKind {
    Ocr,
    Object,
    Grounding,
    BiometricVerification,
    Pose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecialistDescriptor {
    pub kind: SpecialistKind,
    pub provider: String,
    pub locality: Locality,
    pub model_revision: String,
}

impl SpecialistDescriptor {
    pub fn validate(&self) -> Result<(), SpecialistError> {
        if self.provider.is_empty()
            || self.provider.len() > 128
            || self.model_revision.is_empty()
            || self.model_revision.len() > 128
        {
            return Err(SpecialistError::InvalidInput);
        }
        Ok(())
    }
}

/// Bounded visual input. The borrowed frame cannot be serialized or retained by this API.
pub struct VisualRequest<'a> {
    pub source: &'a SourceId,
    pub frame: &'a EphemeralFrame,
    pub regions: &'a [Rect],
}

impl VisualRequest<'_> {
    fn validate(&self) -> Result<(), SpecialistError> {
        if self.regions.is_empty() || self.regions.len() > MAX_REGIONS {
            return Err(SpecialistError::InvalidInput);
        }
        for region in self.regions {
            region
                .validate()
                .map_err(|_| SpecialistError::InvalidInput)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrSpan {
    pub text: String,
    pub bbox: Rect,
    pub confidence_permille: u16,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDetection {
    pub kind: NodeKind,
    pub bbox: Rect,
    pub confidence_permille: u16,
    pub uncertainty_permille: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingMatch {
    pub node: Option<NodeId>,
    pub bbox: Rect,
    pub confidence_permille: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseLandmark {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub confidence_permille: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Match,
    NoMatch,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpecialistError {
    #[error("specialist input is malformed or outside its bound")]
    InvalidInput,
    #[error("specialist is not configured")]
    NotConfigured,
    #[error("specialist output is malformed or outside its bound")]
    InvalidOutput,
    #[error("biometric processing is local only")]
    LocalityDenied,
}

pub trait OcrSpecialist: Send + Sync {
    fn descriptor(&self) -> &SpecialistDescriptor;
    fn recognize(&self, request: VisualRequest<'_>) -> Result<Vec<OcrSpan>, SpecialistError>;
}

pub trait ObjectSpecialist: Send + Sync {
    fn descriptor(&self) -> &SpecialistDescriptor;
    fn detect(&self, request: VisualRequest<'_>) -> Result<Vec<ObjectDetection>, SpecialistError>;
}

pub trait GroundingSpecialist: Send + Sync {
    fn descriptor(&self) -> &SpecialistDescriptor;
    fn ground(
        &self,
        request: VisualRequest<'_>,
        query: &str,
    ) -> Result<Vec<GroundingMatch>, SpecialistError>;
}

pub trait PoseSpecialist: Send + Sync {
    fn descriptor(&self) -> &SpecialistDescriptor;
    fn estimate(&self, request: VisualRequest<'_>) -> Result<Vec<PoseLandmark>, SpecialistError>;
}

/// Biometric implementations receive only a borrowed frame and an opaque enrolled-template
/// handle. The generic IRIS crate has no API for exporting samples or template material.
pub trait BiometricVerifier: Send + Sync {
    fn descriptor(&self) -> &SpecialistDescriptor;
    fn verify(
        &self,
        request: VisualRequest<'_>,
        enrolled_template: &str,
    ) -> Result<VerificationOutcome, SpecialistError>;
}

/// Native document text bypasses OCR. It is useful for PDF/DOM sources that already expose
/// text and prevents a pixel round-trip. The caller remains responsible for privacy policy.
#[derive(Debug, Default)]
pub struct NativeTextAdapter;

impl NativeTextAdapter {
    pub fn spans(
        &self,
        text: &str,
        bbox: Rect,
        language: Option<&str>,
    ) -> Result<Vec<OcrSpan>, SpecialistError> {
        if text.is_empty()
            || text.len() > MAX_TEXT_BYTES
            || text.contains('\0')
            || language.is_some_and(|value| value.is_empty() || value.len() > 32)
        {
            return Err(SpecialistError::InvalidInput);
        }
        bbox.validate().map_err(|_| SpecialistError::InvalidInput)?;
        Ok(vec![OcrSpan {
            text: text.to_owned(),
            bbox,
            confidence_permille: 1_000,
            language: language.map(str::to_owned),
        }])
    }
}

/// Explicit placeholder used when a real provider is absent. It never emits invented
/// detections. One type implements each interface so configuration can remain model-agnostic.
pub struct UnconfiguredSpecialist {
    descriptor: SpecialistDescriptor,
}

impl UnconfiguredSpecialist {
    pub fn new(
        kind: SpecialistKind,
        provider: impl Into<String>,
        locality: Locality,
    ) -> Result<Self, SpecialistError> {
        if kind == SpecialistKind::BiometricVerification && locality != Locality::Local {
            return Err(SpecialistError::LocalityDenied);
        }
        let descriptor = SpecialistDescriptor {
            kind,
            provider: provider.into(),
            locality,
            model_revision: "unconfigured".into(),
        };
        descriptor.validate()?;
        Ok(Self { descriptor })
    }

    fn check(&self, request: &VisualRequest<'_>) -> Result<(), SpecialistError> {
        self.descriptor.validate()?;
        request.validate()?;
        Err(SpecialistError::NotConfigured)
    }
}

macro_rules! unavailable_visual_impl {
    ($trait_name:ident, $method:ident, $output:ty) => {
        impl $trait_name for UnconfiguredSpecialist {
            fn descriptor(&self) -> &SpecialistDescriptor {
                &self.descriptor
            }

            fn $method(&self, request: VisualRequest<'_>) -> Result<Vec<$output>, SpecialistError> {
                self.check(&request)?;
                unreachable!()
            }
        }
    };
}

unavailable_visual_impl!(OcrSpecialist, recognize, OcrSpan);
unavailable_visual_impl!(ObjectSpecialist, detect, ObjectDetection);
unavailable_visual_impl!(PoseSpecialist, estimate, PoseLandmark);

impl GroundingSpecialist for UnconfiguredSpecialist {
    fn descriptor(&self) -> &SpecialistDescriptor {
        &self.descriptor
    }

    fn ground(
        &self,
        request: VisualRequest<'_>,
        query: &str,
    ) -> Result<Vec<GroundingMatch>, SpecialistError> {
        if query.is_empty() || query.len() > MAX_QUERY_BYTES || query.contains('\0') {
            return Err(SpecialistError::InvalidInput);
        }
        self.check(&request)?;
        unreachable!()
    }
}

impl BiometricVerifier for UnconfiguredSpecialist {
    fn descriptor(&self) -> &SpecialistDescriptor {
        &self.descriptor
    }

    fn verify(
        &self,
        request: VisualRequest<'_>,
        enrolled_template: &str,
    ) -> Result<VerificationOutcome, SpecialistError> {
        if self.descriptor.locality != Locality::Local {
            return Err(SpecialistError::LocalityDenied);
        }
        if enrolled_template.is_empty()
            || enrolled_template.len() > 128
            || enrolled_template.contains('\0')
        {
            return Err(SpecialistError::InvalidInput);
        }
        self.check(&request)?;
        unreachable!()
    }
}

pub fn validate_ocr_output(output: &[OcrSpan]) -> Result<(), SpecialistError> {
    if output.len() > MAX_RESULTS {
        return Err(SpecialistError::InvalidOutput);
    }
    for span in output {
        if span.text.is_empty()
            || span.text.len() > MAX_TEXT_BYTES
            || span.text.contains('\0')
            || span.confidence_permille > 1_000
        {
            return Err(SpecialistError::InvalidOutput);
        }
        span.bbox
            .validate()
            .map_err(|_| SpecialistError::InvalidOutput)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;

    fn frame() -> EphemeralFrame {
        EphemeralFrame::new(
            SourceId::new("camera:0").unwrap(),
            1,
            2,
            2,
            PixelFormat::Gray8,
            vec![0; 4],
        )
        .unwrap()
    }

    #[test]
    fn native_text_bypasses_ocr_without_inventing_content() {
        let spans = NativeTextAdapter
            .spans(
                "Export model",
                Rect::new(10, 20, 100, 20).unwrap(),
                Some("en"),
            )
            .unwrap();
        validate_ocr_output(&spans).unwrap();
        assert_eq!(spans[0].text, "Export model");
        assert_eq!(spans[0].confidence_permille, 1_000);
    }

    #[test]
    fn absent_visual_provider_fails_explicitly() {
        let source = SourceId::new("camera:0").unwrap();
        let pixels = frame();
        let regions = [Rect::new(0, 0, 2, 2).unwrap()];
        let request = VisualRequest {
            source: &source,
            frame: &pixels,
            regions: &regions,
        };
        let adapter =
            UnconfiguredSpecialist::new(SpecialistKind::Object, "onnx-runtime", Locality::Local)
                .unwrap();
        assert_eq!(adapter.detect(request), Err(SpecialistError::NotConfigured));
    }

    #[test]
    fn biometric_provider_cannot_be_cloud_or_fabric() {
        assert!(matches!(
            UnconfiguredSpecialist::new(
                SpecialistKind::BiometricVerification,
                "remote-face",
                Locality::Cloud
            ),
            Err(SpecialistError::LocalityDenied)
        ));
    }

    #[test]
    fn oversized_or_invalid_specialist_output_is_rejected() {
        let invalid = [OcrSpan {
            text: "x".into(),
            bbox: Rect::new(0, 0, 1, 1).unwrap(),
            confidence_permille: 1_001,
            language: None,
        }];
        assert_eq!(
            validate_ocr_output(&invalid),
            Err(SpecialistError::InvalidOutput)
        );
    }
}
