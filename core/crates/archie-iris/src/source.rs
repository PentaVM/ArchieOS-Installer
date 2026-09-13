use crate::damage::{detect_damage, semantic_rois, DamageConfig, EphemeralFrame};
use crate::geometry::Rect;
use crate::scene::{NodeId, NodeKind, SceneEvent, SceneGraph, SceneNode, SourceId, TrackId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

const MAX_STRUCTURED_NODES: usize = 16_384;
const MAX_QUEUE_FRAMES: usize = 4;
const MAX_PPM_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Desktop,
    Browser,
    Camera,
    Video,
    Image,
    FreeCad,
    Blender,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionPath {
    CompositorDamage,
    Accessibility,
    Dom,
    ApplicationApi,
    ExplicitHostBridge,
    PixelFallback,
    CameraStream,
    DecodedImage,
    RemoteDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub source_id: SourceId,
    pub source_kind: SourceKind,
    pub acquisition: AcquisitionPath,
    pub revision: u64,
    pub semantic_nodes: usize,
    pub pending_frames: usize,
}

/// One source result. The metadata and scene events may cross process boundaries; the
/// optional frame cannot be serialized and must be consumed or dropped locally.
pub struct SourceObservation {
    pub source_id: SourceId,
    pub source_kind: SourceKind,
    pub acquisition: AcquisitionPath,
    pub events: Vec<SceneEvent>,
    pub frame: Option<EphemeralFrame>,
}

pub trait PerceptionSource {
    fn source_id(&self) -> &SourceId;
    fn source_kind(&self) -> SourceKind;
    fn poll(&mut self, scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError>;
    fn snapshot(&self) -> SourceSnapshot;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceError {
    #[error("source input is malformed or outside its bound")]
    InvalidInput,
    #[error("source is not authorized")]
    NotAuthorized,
    #[error("source has no new state")]
    NoState,
    #[error("source revision is stale")]
    StaleRevision,
    #[error("image format is unsupported")]
    UnsupportedImage,
    #[error("image payload is malformed")]
    MalformedImage,
    #[error("desktop damage failed")]
    Damage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredNode {
    pub native_id: String,
    pub role: String,
    pub bounds: Option<Rect>,
    pub parent_native_id: Option<String>,
    pub label: Option<String>,
    pub focused: bool,
    pub selected: bool,
}

impl StructuredNode {
    fn validate(&self) -> Result<(), SourceError> {
        let valid_id = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && !value.contains('\0')
                && !value.contains(['\r', '\n'])
        };
        if !valid_id(&self.native_id)
            || self
                .parent_native_id
                .as_deref()
                .is_some_and(|parent| !valid_id(parent) || parent == self.native_id)
            || self.role.is_empty()
            || self.role.len() > 64
            || self
                .label
                .as_ref()
                .is_some_and(|label| label.is_empty() || label.len() > 512 || label.contains('\0'))
        {
            return Err(SourceError::InvalidInput);
        }
        if let Some(bounds) = self.bounds {
            bounds.validate().map_err(|_| SourceError::InvalidInput)?;
        }
        Ok(())
    }
}

struct StructuredSource {
    id: SourceId,
    kind: SourceKind,
    acquisition: AcquisitionPath,
    namespace: &'static str,
    revision: u64,
    nodes: BTreeMap<String, StructuredNode>,
    previous_ids: BTreeSet<String>,
    dirty: bool,
}

impl StructuredSource {
    fn new(
        id: SourceId,
        kind: SourceKind,
        acquisition: AcquisitionPath,
        namespace: &'static str,
    ) -> Self {
        Self {
            id,
            kind,
            acquisition,
            namespace,
            revision: 0,
            nodes: BTreeMap::new(),
            previous_ids: BTreeSet::new(),
            dirty: false,
        }
    }

    fn update(&mut self, revision: u64, nodes: Vec<StructuredNode>) -> Result<(), SourceError> {
        if revision <= self.revision || nodes.len() > MAX_STRUCTURED_NODES {
            return Err(SourceError::StaleRevision);
        }
        let mut next = BTreeMap::new();
        for node in nodes {
            node.validate()?;
            if next.insert(node.native_id.clone(), node).is_some() {
                return Err(SourceError::InvalidInput);
            }
        }
        for node in next.values() {
            if node
                .parent_native_id
                .as_ref()
                .is_some_and(|parent| !next.contains_key(parent))
            {
                return Err(SourceError::InvalidInput);
            }
        }
        self.nodes = next;
        self.revision = revision;
        self.dirty = true;
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<SourceObservation>, SourceError> {
        if !self.dirty {
            return Ok(Vec::new());
        }
        let current_ids: BTreeSet<_> = self.nodes.keys().cloned().collect();
        let mut events = Vec::new();
        for removed in self.previous_ids.difference(&current_ids) {
            events.push(SceneEvent::Delete(self.node_id(removed)?));
        }
        // Parents precede children so one atomic SceneGraph update can validate them.
        let mut pending: BTreeSet<_> = current_ids.clone();
        while !pending.is_empty() {
            let ready: Vec<_> = pending
                .iter()
                .filter(|id| {
                    self.nodes[*id]
                        .parent_native_id
                        .as_ref()
                        .is_none_or(|parent| !pending.contains(parent))
                })
                .cloned()
                .collect();
            if ready.is_empty() {
                return Err(SourceError::InvalidInput);
            }
            for native_id in ready {
                let node = &self.nodes[&native_id];
                events.push(SceneEvent::Upsert(SceneNode {
                    id: self.node_id(&native_id)?,
                    source: self.id.clone(),
                    kind: role_kind(&node.role),
                    bbox: node.bounds,
                    parent: node
                        .parent_native_id
                        .as_ref()
                        .map(|parent| self.node_id(parent))
                        .transpose()?,
                    track_id: Some(
                        TrackId::new(format!("{}:{}", self.namespace, native_id))
                            .map_err(|_| SourceError::InvalidInput)?,
                    ),
                    confidence_permille: 1_000,
                    uncertainty_permille: 0,
                    last_seen_ms: self.revision as i64,
                    label: node.label.clone(),
                    revision: 0,
                }));
                pending.remove(&native_id);
            }
        }
        self.previous_ids = current_ids;
        self.dirty = false;
        Ok(vec![SourceObservation {
            source_id: self.id.clone(),
            source_kind: self.kind,
            acquisition: self.acquisition,
            events,
            frame: None,
        }])
    }

    fn node_id(&self, native_id: &str) -> Result<NodeId, SourceError> {
        let sanitized: String = native_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        NodeId::new(format!("{}:{sanitized}", self.namespace))
            .map_err(|_| SourceError::InvalidInput)
    }

    fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            source_id: self.id.clone(),
            source_kind: self.kind,
            acquisition: self.acquisition,
            revision: self.revision,
            semantic_nodes: self.nodes.len(),
            pending_frames: 0,
        }
    }
}

fn role_kind(role: &str) -> NodeKind {
    match role.to_ascii_lowercase().as_str() {
        "display" => NodeKind::Display,
        "window" => NodeKind::Window,
        "application" => NodeKind::Application,
        "document" | "body" | "sketch" | "pad" | "pocket" | "fillet" => NodeKind::Document,
        "page" => NodeKind::BrowserPage,
        "button" => NodeKind::Button,
        "input" | "textbox" => NodeKind::Input,
        "link" => NodeKind::Link,
        "checkbox" => NodeKind::Checkbox,
        "radio" => NodeKind::Radio,
        "menu" => NodeKind::Menu,
        "menuitem" | "menu_item" => NodeKind::MenuItem,
        "tab" => NodeKind::Tab,
        "dialog" => NodeKind::Dialog,
        "toolbar" => NodeKind::Toolbar,
        "treeitem" | "tree_item" => NodeKind::TreeItem,
        "viewport" => NodeKind::Viewport,
        "icon" => NodeKind::Icon,
        "image" => NodeKind::Image,
        "canvas" => NodeKind::Canvas,
        "text" => NodeKind::Text,
        "object" | "mesh" | "light" | "camera" | "modifier" => NodeKind::Object3d,
        _ => NodeKind::UnknownUi,
    }
}

macro_rules! structured_adapter {
    ($name:ident, $kind:expr, $acquisition:expr, $namespace:expr) => {
        pub struct $name(StructuredSource);

        impl $name {
            pub fn new(id: SourceId) -> Self {
                Self(StructuredSource::new(id, $kind, $acquisition, $namespace))
            }

            pub fn update(
                &mut self,
                revision: u64,
                nodes: Vec<StructuredNode>,
            ) -> Result<(), SourceError> {
                self.0.update(revision, nodes)
            }
        }

        impl PerceptionSource for $name {
            fn source_id(&self) -> &SourceId {
                &self.0.id
            }
            fn source_kind(&self) -> SourceKind {
                self.0.kind
            }
            fn poll(&mut self, _scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError> {
                self.0.poll()
            }
            fn snapshot(&self) -> SourceSnapshot {
                self.0.snapshot()
            }
        }
    };
}

structured_adapter!(
    BrowserDomSource,
    SourceKind::Browser,
    AcquisitionPath::Dom,
    "dom"
);
structured_adapter!(
    FreeCadSource,
    SourceKind::FreeCad,
    AcquisitionPath::ApplicationApi,
    "freecad"
);
structured_adapter!(
    BlenderSource,
    SourceKind::Blender,
    AcquisitionPath::ApplicationApi,
    "blender"
);

pub struct RemoteSceneSource(StructuredSource);

impl RemoteSceneSource {
    pub fn new(id: SourceId) -> Self {
        Self(StructuredSource::new(
            id,
            SourceKind::Remote,
            AcquisitionPath::RemoteDelta,
            "remote",
        ))
    }

    pub fn update(&mut self, revision: u64, nodes: Vec<StructuredNode>) -> Result<(), SourceError> {
        self.0.update(revision, nodes)
    }
}

impl PerceptionSource for RemoteSceneSource {
    fn source_id(&self) -> &SourceId {
        &self.0.id
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Remote
    }
    fn poll(&mut self, _scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError> {
        self.0.poll()
    }
    fn snapshot(&self) -> SourceSnapshot {
        self.0.snapshot()
    }
}

/// Desktop source supporting compositor damage and a bounded pixel fallback. The host
/// bridge flag records an explicit source contract; it does not enable Windows automount.
pub struct DesktopSource {
    id: SourceId,
    revision: u64,
    pending_damage: Vec<Rect>,
    previous: Option<EphemeralFrame>,
    current: Option<EphemeralFrame>,
    explicit_host_bridge: bool,
}

impl DesktopSource {
    pub fn new(id: SourceId, explicit_host_bridge: bool) -> Self {
        Self {
            id,
            revision: 0,
            pending_damage: Vec::new(),
            previous: None,
            current: None,
            explicit_host_bridge,
        }
    }

    pub fn compositor_damage(
        &mut self,
        revision: u64,
        regions: Vec<Rect>,
    ) -> Result<(), SourceError> {
        if revision <= self.revision || regions.len() > 256 {
            return Err(SourceError::StaleRevision);
        }
        for region in &regions {
            region.validate().map_err(|_| SourceError::InvalidInput)?;
        }
        self.revision = revision;
        self.pending_damage = regions;
        Ok(())
    }

    pub fn fallback_frame(&mut self, frame: EphemeralFrame) -> Result<(), SourceError> {
        if frame.source() != &self.id {
            return Err(SourceError::InvalidInput);
        }
        self.previous = self.current.take();
        self.current = Some(frame);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

impl PerceptionSource for DesktopSource {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Desktop
    }
    fn poll(&mut self, scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError> {
        if !self.pending_damage.is_empty() {
            let damage = std::mem::take(&mut self.pending_damage)
                .into_iter()
                .map(|rect| crate::damage::DamageRegion {
                    changed_pixels: rect.area().min(u64::from(u32::MAX)) as u32,
                    rect,
                })
                .collect::<Vec<_>>();
            let rois =
                semantic_rois(&self.id, &damage, scene, false).map_err(|_| SourceError::Damage)?;
            return Ok(vec![SourceObservation {
                source_id: self.id.clone(),
                source_kind: SourceKind::Desktop,
                acquisition: if self.explicit_host_bridge {
                    AcquisitionPath::ExplicitHostBridge
                } else {
                    AcquisitionPath::CompositorDamage
                },
                events: roi_events(&rois, self.revision as i64)?,
                frame: None,
            }]);
        }
        let Some(current) = self.current.take() else {
            return Ok(Vec::new());
        };
        let events = if let Some(previous) = &self.previous {
            let damage = detect_damage(previous, &current, DamageConfig::default())
                .map_err(|_| SourceError::Damage)?;
            let rois =
                semantic_rois(&self.id, &damage, scene, false).map_err(|_| SourceError::Damage)?;
            roi_events(&rois, self.revision as i64)?
        } else {
            Vec::new()
        };
        Ok(vec![SourceObservation {
            source_id: self.id.clone(),
            source_kind: SourceKind::Desktop,
            acquisition: AcquisitionPath::PixelFallback,
            events,
            frame: Some(current),
        }])
    }
    fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            source_id: self.id.clone(),
            source_kind: SourceKind::Desktop,
            acquisition: if self.explicit_host_bridge {
                AcquisitionPath::ExplicitHostBridge
            } else if self.pending_damage.is_empty() {
                AcquisitionPath::PixelFallback
            } else {
                AcquisitionPath::CompositorDamage
            },
            revision: self.revision,
            semantic_nodes: 0,
            pending_frames: usize::from(self.current.is_some()),
        }
    }
}

fn roi_events(
    rois: &[crate::damage::SemanticRoi],
    time: i64,
) -> Result<Vec<SceneEvent>, SourceError> {
    rois.iter()
        .enumerate()
        .map(|(index, roi)| {
            Ok(SceneEvent::Upsert(SceneNode {
                id: NodeId::new(format!("roi:{}:{index}", roi.source.as_str()))
                    .map_err(|_| SourceError::InvalidInput)?,
                source: roi.source.clone(),
                kind: NodeKind::Image,
                bbox: Some(roi.rect),
                parent: roi.semantic_node.clone(),
                track_id: None,
                confidence_permille: 1_000,
                uncertainty_permille: 0,
                last_seen_ms: time,
                label: None,
                revision: 0,
            }))
        })
        .collect()
}

/// A latest-frame queue. Capacity is deliberately tiny: slow consumers drop old pixels
/// instead of accumulating camera history.
pub struct CameraSource {
    id: SourceId,
    kind: SourceKind,
    authorized: bool,
    revision: u64,
    frames: VecDeque<EphemeralFrame>,
}

impl CameraSource {
    pub fn new(id: SourceId, kind: SourceKind, authorized: bool) -> Result<Self, SourceError> {
        if !matches!(kind, SourceKind::Camera | SourceKind::Video) {
            return Err(SourceError::InvalidInput);
        }
        Ok(Self {
            id,
            kind,
            authorized,
            revision: 0,
            frames: VecDeque::new(),
        })
    }

    pub fn set_authorized(&mut self, authorized: bool) {
        self.authorized = authorized;
        if !authorized {
            self.frames.clear();
        }
    }

    pub fn push(&mut self, frame: EphemeralFrame) -> Result<(), SourceError> {
        if !self.authorized {
            return Err(SourceError::NotAuthorized);
        }
        if frame.source() != &self.id {
            return Err(SourceError::InvalidInput);
        }
        while self.frames.len() >= MAX_QUEUE_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

impl PerceptionSource for CameraSource {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn source_kind(&self) -> SourceKind {
        self.kind
    }
    fn poll(&mut self, _scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError> {
        if !self.authorized {
            self.frames.clear();
            return Err(SourceError::NotAuthorized);
        }
        let Some(frame) = self.frames.pop_back() else {
            return Ok(Vec::new());
        };
        self.frames.clear();
        Ok(vec![SourceObservation {
            source_id: self.id.clone(),
            source_kind: self.kind,
            acquisition: AcquisitionPath::CameraStream,
            events: Vec::new(),
            frame: Some(frame),
        }])
    }
    fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            source_id: self.id.clone(),
            source_kind: self.kind,
            acquisition: AcquisitionPath::CameraStream,
            revision: self.revision,
            semantic_nodes: 0,
            pending_frames: self.frames.len(),
        }
    }
}

pub struct StaticImageSource {
    id: SourceId,
    frame: Option<EphemeralFrame>,
    revision: u64,
}

impl StaticImageSource {
    /// Decodes a binary PPM (P6) once. Other codecs belong behind replaceable decoders.
    pub fn from_ppm(
        id: SourceId,
        captured_at_ms: i64,
        encoded: &[u8],
    ) -> Result<Self, SourceError> {
        let frame = decode_ppm(id.clone(), captured_at_ms, encoded)?;
        Ok(Self {
            id,
            frame: Some(frame),
            revision: 1,
        })
    }
}

impl PerceptionSource for StaticImageSource {
    fn source_id(&self) -> &SourceId {
        &self.id
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Image
    }
    fn poll(&mut self, _scene: &SceneGraph) -> Result<Vec<SourceObservation>, SourceError> {
        let Some(frame) = self.frame.take() else {
            return Ok(Vec::new());
        };
        Ok(vec![SourceObservation {
            source_id: self.id.clone(),
            source_kind: SourceKind::Image,
            acquisition: AcquisitionPath::DecodedImage,
            events: Vec::new(),
            frame: Some(frame),
        }])
    }
    fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            source_id: self.id.clone(),
            source_kind: SourceKind::Image,
            acquisition: AcquisitionPath::DecodedImage,
            revision: self.revision,
            semantic_nodes: 0,
            pending_frames: usize::from(self.frame.is_some()),
        }
    }
}

fn decode_ppm(
    id: SourceId,
    captured_at_ms: i64,
    encoded: &[u8],
) -> Result<EphemeralFrame, SourceError> {
    if !encoded.starts_with(b"P6\n") || encoded.len() < 12 {
        return Err(SourceError::UnsupportedImage);
    }
    let header_end = encoded
        .windows(5)
        .position(|window| window == b"\n255\n")
        .map(|index| index + 5)
        .ok_or(SourceError::MalformedImage)?;
    if header_end > MAX_PPM_HEADER_BYTES {
        return Err(SourceError::MalformedImage);
    }
    let header = std::str::from_utf8(&encoded[3..header_end - 5])
        .map_err(|_| SourceError::MalformedImage)?;
    let mut dimensions = header.split_ascii_whitespace();
    let width: u32 = dimensions
        .next()
        .ok_or(SourceError::MalformedImage)?
        .parse()
        .map_err(|_| SourceError::MalformedImage)?;
    let height: u32 = dimensions
        .next()
        .ok_or(SourceError::MalformedImage)?
        .parse()
        .map_err(|_| SourceError::MalformedImage)?;
    if dimensions.next().is_some() {
        return Err(SourceError::MalformedImage);
    }
    EphemeralFrame::new(
        id,
        captured_at_ms,
        width,
        height,
        crate::damage::PixelFormat::Rgb8,
        encoded[header_end..].to_vec(),
    )
    .map_err(|_| SourceError::MalformedImage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_uses_dom_text_and_coordinates_without_pixels_or_ocr() {
        let mut browser = BrowserDomSource::new(SourceId::new("browser:0").unwrap());
        browser
            .update(
                1,
                vec![
                    StructuredNode {
                        native_id: "page".into(),
                        role: "page".into(),
                        bounds: Some(Rect::new(0, 0, 800, 600).unwrap()),
                        parent_native_id: None,
                        label: Some("Signup".into()),
                        focused: false,
                        selected: false,
                    },
                    StructuredNode {
                        native_id: "email".into(),
                        role: "input".into(),
                        bounds: Some(Rect::new(20, 30, 200, 30).unwrap()),
                        parent_native_id: Some("page".into()),
                        label: Some("Email".into()),
                        focused: true,
                        selected: false,
                    },
                ],
            )
            .unwrap();
        let observations = browser.poll(&SceneGraph::default()).unwrap();
        assert_eq!(observations[0].acquisition, AcquisitionPath::Dom);
        assert!(observations[0].frame.is_none());
        assert_eq!(observations[0].events.len(), 2);
    }

    #[test]
    fn camera_queue_keeps_only_latest_bounded_frames_and_revocation_clears_it() {
        let id = SourceId::new("camera:0").unwrap();
        let mut camera = CameraSource::new(id.clone(), SourceKind::Camera, true).unwrap();
        for time in 0..10 {
            camera
                .push(
                    EphemeralFrame::new(
                        id.clone(),
                        time,
                        1,
                        1,
                        crate::damage::PixelFormat::Gray8,
                        vec![time as u8],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        assert_eq!(camera.snapshot().pending_frames, MAX_QUEUE_FRAMES);
        let observations = camera.poll(&SceneGraph::default()).unwrap();
        assert_eq!(observations[0].frame.as_ref().unwrap().captured_at_ms(), 9);
        camera.set_authorized(false);
        assert_eq!(camera.snapshot().pending_frames, 0);
        assert!(matches!(
            camera.poll(&SceneGraph::default()),
            Err(SourceError::NotAuthorized)
        ));
    }

    #[test]
    fn real_ppm_is_decoded_once_without_serializing_pixels() {
        let mut ppm = b"P6\n2 1\n255\n".to_vec();
        ppm.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
        let mut image =
            StaticImageSource::from_ppm(SourceId::new("image:fixture").unwrap(), 1, &ppm).unwrap();
        let first = image.poll(&SceneGraph::default()).unwrap();
        let frame = first[0].frame.as_ref().unwrap();
        assert_eq!((frame.width(), frame.height()), (2, 1));
        assert_eq!(frame.pixels(), &[255, 0, 0, 0, 255, 0]);
        assert!(image.poll(&SceneGraph::default()).unwrap().is_empty());
    }

    #[test]
    fn freecad_and_blender_are_structured_application_sources() {
        let mut freecad = FreeCadSource::new(SourceId::new("freecad:doc").unwrap());
        freecad
            .update(
                1,
                vec![StructuredNode {
                    native_id: "Sketch001".into(),
                    role: "sketch".into(),
                    bounds: None,
                    parent_native_id: None,
                    label: Some("Bracket profile".into()),
                    focused: false,
                    selected: true,
                }],
            )
            .unwrap();
        assert_eq!(
            freecad.snapshot().acquisition,
            AcquisitionPath::ApplicationApi
        );
        assert!(freecad.poll(&SceneGraph::default()).unwrap()[0]
            .frame
            .is_none());

        let blender = BlenderSource::new(SourceId::new("blender:scene").unwrap());
        assert_eq!(blender.snapshot().source_kind, SourceKind::Blender);
    }

    #[test]
    fn remote_source_accepts_structure_and_never_a_video_stream() {
        let remote = RemoteSceneSource::new(SourceId::new("remote:node1").unwrap());
        assert_eq!(remote.snapshot().acquisition, AcquisitionPath::RemoteDelta);
        assert_eq!(remote.snapshot().pending_frames, 0);
    }
}
