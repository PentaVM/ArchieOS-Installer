use crate::geometry::Rect;
use crate::scene::{NodeId, SceneGraph, SourceId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use thiserror::Error;

const MAX_FRAME_PIXELS: usize = 16_777_216;
const MAX_DAMAGE_REGIONS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Gray8,
    Rgb8,
    Bgr8,
}

impl PixelFormat {
    fn channels(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Rgb8 | Self::Bgr8 => 3,
        }
    }
}

/// A bounded pixel buffer. It deliberately implements neither `Serialize`, `Debug` nor
/// `Clone`, and it is zeroed when released.
///
/// ```compile_fail
/// use archie_iris::{EphemeralFrame, PixelFormat};
/// fn persist(frame: &EphemeralFrame) { let _ = serde_json::to_string(frame); }
/// ```
pub struct EphemeralFrame {
    source: SourceId,
    captured_at_ms: i64,
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: Vec<u8>,
}

impl EphemeralFrame {
    pub fn new(
        source: SourceId,
        captured_at_ms: i64,
        width: u32,
        height: u32,
        format: PixelFormat,
        pixels: Vec<u8>,
    ) -> Result<Self, DamageError> {
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(DamageError::InvalidFrame)?;
        let bytes = pixel_count
            .checked_mul(format.channels())
            .ok_or(DamageError::InvalidFrame)?;
        if captured_at_ms < 0
            || width == 0
            || height == 0
            || pixel_count > MAX_FRAME_PIXELS
            || bytes != pixels.len()
        {
            return Err(DamageError::InvalidFrame);
        }
        Ok(Self {
            source,
            captured_at_ms,
            width,
            height,
            format,
            pixels,
        })
    }

    pub fn source(&self) -> &SourceId {
        &self.source
    }

    pub fn captured_at_ms(&self) -> i64 {
        self.captured_at_ms
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Borrows the ephemeral buffer for a local specialist. The borrow cannot outlive
    /// the frame and no persistence API is provided.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn crop(&self, rect: Rect) -> Result<Self, DamageError> {
        rect.validate().map_err(|_| DamageError::InvalidRoi)?;
        if rect.x < 0
            || rect.y < 0
            || rect.right() > i64::from(self.width)
            || rect.bottom() > i64::from(self.height)
        {
            return Err(DamageError::InvalidRoi);
        }
        let channels = self.format.channels();
        let mut output = Vec::with_capacity(rect.area() as usize * channels);
        for y in rect.y as usize..rect.bottom() as usize {
            let start = (y * self.width as usize + rect.x as usize) * channels;
            let end = start + rect.width as usize * channels;
            output.extend_from_slice(&self.pixels[start..end]);
        }
        Self::new(
            self.source.clone(),
            self.captured_at_ms,
            rect.width,
            rect.height,
            self.format,
            output,
        )
    }
}

impl Drop for EphemeralFrame {
    fn drop(&mut self) {
        self.pixels.fill(0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageConfig {
    pub channel_threshold: u8,
    pub minimum_changed_pixels: u32,
    pub maximum_regions: usize,
}

impl Default for DamageConfig {
    fn default() -> Self {
        Self {
            channel_threshold: 12,
            minimum_changed_pixels: 2,
            maximum_regions: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageRegion {
    pub rect: Rect,
    pub changed_pixels: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticRoi {
    pub source: SourceId,
    pub rect: Rect,
    pub semantic_node: Option<NodeId>,
    pub scene_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DamageError {
    #[error("frame is malformed or outside its bound")]
    InvalidFrame,
    #[error("frames do not share dimensions, source and format")]
    FrameMismatch,
    #[error("damage configuration is invalid")]
    InvalidConfig,
    #[error("ROI is outside the frame")]
    InvalidRoi,
    #[error("damage output exceeded its region budget")]
    TooManyRegions,
}

pub fn detect_damage(
    previous: &EphemeralFrame,
    current: &EphemeralFrame,
    config: DamageConfig,
) -> Result<Vec<DamageRegion>, DamageError> {
    if config.maximum_regions == 0
        || config.maximum_regions > MAX_DAMAGE_REGIONS
        || config.minimum_changed_pixels == 0
    {
        return Err(DamageError::InvalidConfig);
    }
    if previous.source != current.source
        || previous.width != current.width
        || previous.height != current.height
        || previous.format != current.format
    {
        return Err(DamageError::FrameMismatch);
    }
    let channels = current.format.channels();
    let pixel_count = current.width as usize * current.height as usize;
    let mut changed = vec![false; pixel_count];
    for (index, changed_pixel) in changed.iter_mut().enumerate() {
        let offset = index * channels;
        *changed_pixel = (0..channels).any(|channel| {
            previous.pixels[offset + channel].abs_diff(current.pixels[offset + channel])
                >= config.channel_threshold
        });
    }

    let mut visited = vec![false; pixel_count];
    let mut regions = Vec::new();
    for seed in 0..pixel_count {
        if !changed[seed] || visited[seed] {
            continue;
        }
        let mut queue = VecDeque::from([seed]);
        visited[seed] = true;
        let mut count = 0_u32;
        let mut min_x = current.width;
        let mut min_y = current.height;
        let mut max_x = 0_u32;
        let mut max_y = 0_u32;
        while let Some(index) = queue.pop_front() {
            let x = (index % current.width as usize) as u32;
            let y = (index / current.width as usize) as u32;
            count += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            for neighbor in neighbors(x, y, current.width, current.height) {
                if changed[neighbor] && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        if count < config.minimum_changed_pixels {
            continue;
        }
        regions.push(DamageRegion {
            rect: Rect::new(
                min_x as i32,
                min_y as i32,
                max_x - min_x + 1,
                max_y - min_y + 1,
            )
            .map_err(|_| DamageError::InvalidRoi)?,
            changed_pixels: count,
        });
        if regions.len() > config.maximum_regions {
            return Err(DamageError::TooManyRegions);
        }
    }
    regions.sort_by_key(|region| (region.rect.y, region.rect.x));
    Ok(regions)
}

fn neighbors(x: u32, y: u32, width: u32, height: u32) -> Vec<usize> {
    let mut output = Vec::with_capacity(4);
    if x > 0 {
        output.push((y * width + x - 1) as usize);
    }
    if x + 1 < width {
        output.push((y * width + x + 1) as usize);
    }
    if y > 0 {
        output.push(((y - 1) * width + x) as usize);
    }
    if y + 1 < height {
        output.push(((y + 1) * width + x) as usize);
    }
    output
}

/// Snaps pixel damage to the smallest intersecting semantic node. A changed caret thus
/// selects its input field instead of becoming a two-pixel crop.
pub fn semantic_rois(
    source: &SourceId,
    damage: &[DamageRegion],
    scene: &SceneGraph,
    include_parent: bool,
) -> Result<Vec<SemanticRoi>, DamageError> {
    let mut output = Vec::new();
    for region in damage {
        let target = scene
            .nodes()
            .filter(|node| node.source == *source)
            .filter_map(|node| node.bbox.map(|bbox| (bbox, node)))
            .filter(|(bbox, _)| bbox.intersects(&region.rect))
            .min_by_key(|(bbox, node)| (bbox.area(), node.id.as_str().to_owned()));
        let (mut rect, mut semantic_node) = match target {
            Some((bbox, node)) => (bbox, Some(node.id.clone())),
            None => (region.rect, None),
        };
        if include_parent {
            if let Some(node) = semantic_node.as_ref().and_then(|id| scene.node(id)) {
                if let Some(parent) = node.parent.as_ref().and_then(|id| scene.node(id)) {
                    if let Some(parent_box) = parent.bbox {
                        rect = rect
                            .union(&parent_box)
                            .map_err(|_| DamageError::InvalidRoi)?;
                        semantic_node = Some(parent.id.clone());
                    }
                }
            }
        }
        if output.iter().any(|existing: &SemanticRoi| {
            existing.semantic_node == semantic_node && existing.rect == rect
        }) {
            continue;
        }
        output.push(SemanticRoi {
            source: source.clone(),
            rect,
            semantic_node,
            scene_revision: scene.revision(),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{NodeKind, SceneEvent, SceneNode, TrackId};

    fn frame(pixels: Vec<u8>) -> EphemeralFrame {
        EphemeralFrame::new(
            SourceId::new("display:0").unwrap(),
            1,
            10,
            10,
            PixelFormat::Gray8,
            pixels,
        )
        .unwrap()
    }

    fn node(id: &str, kind: NodeKind, bbox: Rect, parent: Option<&str>) -> SceneNode {
        SceneNode {
            id: NodeId::new(id).unwrap(),
            source: SourceId::new("display:0").unwrap(),
            kind,
            bbox: Some(bbox),
            parent: parent.map(|id| NodeId::new(id).unwrap()),
            track_id: Some(TrackId::new(format!("track:{id}")).unwrap()),
            confidence_permille: 1_000,
            uncertainty_permille: 0,
            last_seen_ms: 1,
            label: None,
            revision: 0,
        }
    }

    #[test]
    fn no_change_and_single_pixel_noise_produce_no_regions() {
        let a = frame(vec![0; 100]);
        let mut noisy = vec![0; 100];
        noisy[5] = 255;
        let b = frame(noisy);
        assert!(detect_damage(&a, &a, DamageConfig::default())
            .unwrap()
            .is_empty());
        assert!(detect_damage(&a, &b, DamageConfig::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn connected_pixels_become_one_native_resolution_roi() {
        let a = frame(vec![0; 100]);
        let mut pixels = vec![0; 100];
        pixels[44] = 255;
        pixels[45] = 255;
        pixels[54] = 255;
        pixels[55] = 255;
        let b = frame(pixels);
        let damage = detect_damage(&a, &b, DamageConfig::default()).unwrap();
        assert_eq!(damage.len(), 1);
        assert_eq!(damage[0].rect, Rect::new(4, 4, 2, 2).unwrap());
        let crop = b.crop(damage[0].rect).unwrap();
        assert_eq!((crop.width(), crop.height()), (2, 2));
    }

    #[test]
    fn tiny_caret_damage_expands_to_the_input_semantics() {
        let mut graph = SceneGraph::default();
        graph
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node(
                        "dialog:signup",
                        NodeKind::Dialog,
                        Rect::new(0, 0, 10, 10).unwrap(),
                        None,
                    )),
                    SceneEvent::Upsert(node(
                        "input:email",
                        NodeKind::Input,
                        Rect::new(2, 3, 7, 3).unwrap(),
                        Some("dialog:signup"),
                    )),
                ],
            )
            .unwrap();
        let damage = [DamageRegion {
            rect: Rect::new(4, 4, 1, 2).unwrap(),
            changed_pixels: 2,
        }];
        let rois =
            semantic_rois(&SourceId::new("display:0").unwrap(), &damage, &graph, false).unwrap();
        assert_eq!(
            rois[0].semantic_node.as_ref().unwrap().as_str(),
            "input:email"
        );
        assert_eq!(rois[0].rect, Rect::new(2, 3, 7, 3).unwrap());
    }

    #[test]
    fn modal_expansion_can_include_the_parent_container() {
        let mut graph = SceneGraph::default();
        graph
            .apply(
                0,
                &[
                    SceneEvent::Upsert(node(
                        "dialog:confirm",
                        NodeKind::Dialog,
                        Rect::new(0, 0, 10, 10).unwrap(),
                        None,
                    )),
                    SceneEvent::Upsert(node(
                        "button:ok",
                        NodeKind::Button,
                        Rect::new(6, 7, 3, 2).unwrap(),
                        Some("dialog:confirm"),
                    )),
                ],
            )
            .unwrap();
        let damage = [DamageRegion {
            rect: Rect::new(7, 7, 2, 2).unwrap(),
            changed_pixels: 4,
        }];
        let rois =
            semantic_rois(&SourceId::new("display:0").unwrap(), &damage, &graph, true).unwrap();
        assert_eq!(
            rois[0].semantic_node.as_ref().unwrap().as_str(),
            "dialog:confirm"
        );
        assert_eq!(rois[0].rect, Rect::new(0, 0, 10, 10).unwrap());
    }
}
