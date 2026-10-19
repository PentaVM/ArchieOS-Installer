//! XaviCottier's camera/quality/material policies. No hardware probing occurs here.
use crate::{BloomMode, DesignBody, DesignVec3, NodeStyle};
use std::collections::VecDeque;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Quality {
    Original,
    Quality,
    Balanced,
    Performance,
}
#[derive(Debug, Clone, Copy)]
pub struct TextureRequest {
    pub max_resolution: u32,
    pub max_capability: u32,
    pub quality: Quality,
    pub projected_size: f32,
    pub current: u32,
}
pub fn material_texture_size(p: TextureRequest) -> u32 {
    if p.max_resolution == 0 {
        return 0;
    }
    let ceiling = p.max_resolution.min(p.max_capability).min(match p.quality {
        Quality::Performance => 1024,
        Quality::Balanced => 2048,
        _ => 4096,
    });
    let desired = if p.projected_size > 140. {
        4096
    } else if p.projected_size > 70. {
        2048
    } else if p.projected_size > 22. {
        1024
    } else if p.projected_size < 14. {
        512
    } else {
        p.current
    };
    desired.min(ceiling).max(1)
}
pub fn texture_bytes(mut w: u32, mut h: u32, c: u32, m: bool) -> Option<u64> {
    if w == 0 || h == 0 || c == 0 {
        return Some(0);
    }
    let mut bytes = 0u64;
    loop {
        bytes = bytes.checked_add(
            u64::from(w)
                .checked_mul(u64::from(h))?
                .checked_mul(u64::from(c))?,
        )?;
        if !m || (w == 1 && h == 1) {
            return Some(bytes);
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}
pub(crate) fn smoothstep(lo: f32, hi: f32, value: f32) -> f32 {
    let t = ((value - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
pub fn pole_coordinate(p: DesignVec3, w: DesignVec3, c: DesignVec3, r: f32) -> f32 {
    let d = w - c;
    let axis = d * (1. / d.length().max(f32::MIN_POSITIVE));
    let q = (p - (w + c) * 0.5).dot(axis);
    smoothstep(0., 1., q / r.max(1.) * 0.5 + 0.5)
}
pub fn bloom_weight(mode: BloomMode, t: f32, focus: bool) -> f32 {
    match mode {
        BloomMode::Off => 0.,
        BloomMode::Selection => {
            if focus {
                1.
            } else {
                0.
            }
        }
        BloomMode::Polar => {
            if focus {
                1.
            } else {
                smoothstep(0.35, 1., (t - 0.5).abs() * 2.)
            }
        }
        _ => 1.,
    }
}
pub fn hybrid_parameters(style: NodeStyle, depth: f32, emissive: f32, contrast: f32) -> [f32; 4] {
    let [d, disp, e] = match style {
        NodeStyle::Original => [0.52, 0.012, 0.16],
        NodeStyle::PrismCut => [0.9, 0.022, 0.24],
        NodeStyle::Obsidian => [0.28, 0.008, 0.2],
        NodeStyle::BrushedTitanium => [0.08, 0.002, 0.035],
        NodeStyle::Hologram => [0.4, 0.016, 0.21],
        NodeStyle::FracturedCrystal => [1., 0.028, 0.22],
        NodeStyle::Microcircuit => [0.14, 0.003, 0.3],
        NodeStyle::PhotonSkin => [0.6, 0.018, 0.28],
        NodeStyle::MatteCeramic => [0., 0., 0.],
    };
    [
        d * depth.max(0.025) * 1.8,
        disp,
        e * (emissive * 3.).clamp(0.3, 2.),
        contrast.clamp(0.4, 2.),
    ]
}
pub fn hybrid_lod(pixels: f32) -> [f32; 2] {
    [smoothstep(7., 30., pixels), smoothstep(38., 112., pixels)]
}
pub fn hybrid_state(selected: bool, hovered: bool, focused: bool) -> f32 {
    if selected {
        1.
    } else if hovered {
        0.78
    } else if focused {
        0.55
    } else {
        0.
    }
}
/// scene.js original-map camera hysteresis, measured in field radii.
pub fn original_texture_resolution(max: u32, current: u32, distance: f32) -> u32 {
    if distance > 3.5 {
        max.min(1024)
    } else if distance < 2.4 {
        max
    } else {
        max.min(current)
    }
}
/// scene.js visual-surface camera hysteresis, independent of hybrid stored maps.
pub fn visual_texture_resolution(max: u32, current: u32, distance: Option<f32>) -> u32 {
    let max = if max == 0 { 1024 } else { max }.min(2048);
    match distance {
        None => max.min(512),
        Some(d) if d > 2.6 => 512,
        Some(d) if d < 0.85 => max,
        Some(d) if d < 2.1 => max.min(1024),
        _ => current,
    }
}
pub fn bloom_mask_scale(q: Quality) -> f32 {
    match q {
        Quality::Performance => 0.25,
        Quality::Balanced => 0.375,
        _ => 0.5,
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Decoration {
    pub ambient_count: u32,
    pub shell_density: f32,
    pub bloom: bool,
    pub bloom_strength: f32,
    pub pixel_ratio: f32,
    pub texture_resolution: u32,
    pub visible_limit: usize,
}
pub fn effective_quality(mut d: Decoration, q: Quality) -> Decoration {
    match q {
        Quality::Balanced => {
            d.ambient_count = d.ambient_count.min(250);
            d.shell_density = d.shell_density.min(0.85);
            d.bloom_strength = d.bloom_strength.min(0.3);
            d.pixel_ratio = d.pixel_ratio.min(1.);
            d.texture_resolution = d.texture_resolution.min(2048);
        }
        Quality::Performance => {
            d.ambient_count = d.ambient_count.min(80);
            d.shell_density = d.shell_density.min(0.5);
            d.bloom = false;
            d.pixel_ratio = d.pixel_ratio.min(0.8);
            d.texture_resolution = d.texture_resolution.min(1024);
        }
        _ => {}
    }
    d
}
#[derive(Debug)]
pub struct AdaptiveQuality {
    pub level: Quality,
    low: u8,
    high: u8,
}
impl Default for AdaptiveQuality {
    fn default() -> Self {
        Self {
            level: Quality::Balanced,
            low: 0,
            high: 0,
        }
    }
}
impl AdaptiveQuality {
    pub fn sample(&mut self, fps: f32) -> Option<Quality> {
        if !fps.is_finite() || fps < 0. {
            return None;
        }
        if fps < 35. {
            self.low += 1;
            self.high = 0;
        } else if fps > 54. {
            self.high += 1;
            self.low = 0;
        } else {
            self.low = 0;
            self.high = 0;
        }
        if self.low >= 4 {
            self.low = 0;
            let next = match self.level {
                Quality::Original | Quality::Quality => Quality::Balanced,
                Quality::Balanced => Quality::Performance,
                Quality::Performance => return None,
            };
            self.level = next;
            return Some(next);
        }
        if self.high >= 8 {
            self.high = 0;
            let next = match self.level {
                Quality::Performance => Quality::Balanced,
                Quality::Balanced => Quality::Quality,
                _ => return None,
            };
            self.level = next;
            return Some(next);
        }
        None
    }
    pub fn reset(&mut self, level: Quality) {
        self.level = if level == Quality::Original {
            Quality::Quality
        } else {
            level
        };
        self.low = 0;
        self.high = 0;
    }
}
/// Source root-scoped LRU continuity. Rendering anchors remain owned by the OS graph.
pub struct SpatialMemory {
    limit: usize,
    root: Option<String>,
    positions: VecDeque<(String, DesignBody)>,
}
impl SpatialMemory {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.min(36_000),
            root: None,
            positions: VecDeque::new(),
        }
    }
    fn enter(&mut self, root: &str) {
        if self.root.as_deref() != Some(root) {
            self.positions.clear();
            self.root = Some(root.to_owned());
        }
    }
    pub fn remember<'a>(
        &mut self,
        root: &str,
        bodies: impl IntoIterator<Item = (&'a str, DesignBody)>,
    ) {
        self.enter(root);
        for (id, mut body) in bodies {
            if !body.position.finite() {
                continue;
            }
            if !body.velocity.x.is_finite() {
                body.velocity.x = 0.;
            }
            if !body.velocity.y.is_finite() {
                body.velocity.y = 0.;
            }
            if !body.velocity.z.is_finite() {
                body.velocity.z = 0.;
            }
            self.positions.retain(|(old, _)| old != id);
            self.positions.push_back((id.to_owned(), body));
            if self.positions.len() > self.limit {
                self.positions.pop_front();
            }
        }
    }
    pub fn restore(&mut self, root: &str, id: &str, radius: f32) -> Option<DesignBody> {
        self.enter(root);
        let mut body = self.positions.iter().find(|(old, _)| old == id)?.1;
        let length = body.position.length();
        let limit = radius.max(0.) * 1.05;
        if length > limit {
            body.position = body.position * (limit / length);
        }
        Some(body)
    }
    pub fn len(&self) -> usize {
        self.positions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}
