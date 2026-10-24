//! Which part of a frame changed.
//!
//! Work follows damage, not a clock (`docs/IRIS-SCENE-GRAPH.md`: "Caret noise and unchanged
//! frames produce no work"). A camera's damage comes from pixels: the frame is cut in
//! tiles, a pixel counts as changed when its luma moved more than the sensor noise, and a
//! tile is damaged when enough of its pixels changed. Damaged tiles join into connected
//! regions, and a region with fewer changed pixels than a blinking caret makes is dropped.
//! A screen's damage comes from the compositor (`archie-stage` damage rectangles) through
//! [`Damage::from_rects`], under the same small-region rule.

use crate::{Error, Rect};

/// An 8-bit luma image, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Luma {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Luma {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, Error> {
        if width == 0 || height == 0 || pixels.len() != (width as usize) * (height as usize) {
            return Err(Error::Frame(format!(
                "{} luma bytes for {width} x {height}",
                pixels.len()
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// BT.601 luma in integers, `(77 R + 150 G + 29 B) >> 8`, from packed RGB.
    pub fn from_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Self, Error> {
        if rgb.len() != 3 * (width as usize) * (height as usize) {
            return Err(Error::Frame(format!(
                "{} RGB bytes for {width} x {height}",
                rgb.len()
            )));
        }
        let pixels = rgb
            .chunks_exact(3)
            .map(|p| {
                ((77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2])) >> 8) as u8
            })
            .collect();
        Self::new(width, height, pixels)
    }

    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y as usize) * (self.width as usize) + x as usize]
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DamageConfig {
    /// Tile side in pixels. 16 matches the encoders' patches on the canvas.
    pub tile: u32,
    /// Luma change a pixel needs to count as changed (sensor noise sits below it).
    pub pixel_threshold: u8,
    /// Changed pixels a tile needs to be damaged.
    pub tile_pixels: u32,
    /// A connected region with fewer changed pixels than this is noise: a blinking caret
    /// (2 x 16 pixels) or a cursor blip.
    pub min_region_pixels: u32,
}

impl Default for DamageConfig {
    fn default() -> Self {
        Self {
            tile: 16,
            pixel_threshold: 12,
            tile_pixels: 4,
            min_region_pixels: 64,
        }
    }
}

/// Changed regions of a frame.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Damage {
    /// One rectangle per connected damaged region kept, in pixels.
    pub regions: Vec<Rect>,
    /// Damaged area over the frame's area, in [0, 1].
    pub fraction: f32,
}

impl Damage {
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Everything changed: a first frame, or a frame of another size.
    pub fn whole(frame: Rect) -> Self {
        Self {
            regions: vec![frame],
            fraction: 1.0,
        }
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.regions.iter().copied().reduce(|a, b| a.union(&b))
    }

    /// The damage between two frames of one source.
    pub fn between(previous: Option<&Luma>, current: &Luma, config: &DamageConfig) -> Self {
        let Some(previous) = previous else {
            return Self::whole(current.bounds());
        };
        if previous.width != current.width || previous.height != current.height {
            return Self::whole(current.bounds());
        }
        let tile = config.tile.max(1);
        let cols = current.width.div_ceil(tile);
        let rows = current.height.div_ceil(tile);
        let mut changed = vec![0u32; (cols * rows) as usize];
        let width = current.width as usize;
        for (index, (a, b)) in previous.pixels.iter().zip(&current.pixels).enumerate() {
            if a.abs_diff(*b) > config.pixel_threshold {
                let (x, y) = ((index % width) as u32, (index / width) as u32);
                changed[((y / tile) * cols + x / tile) as usize] += 1;
            }
        }
        let damaged: Vec<bool> = changed.iter().map(|&n| n >= config.tile_pixels).collect();
        // Connected regions of damaged tiles (8-neighbourhood), with their changed pixels.
        let mut label = vec![usize::MAX; damaged.len()];
        let mut regions = Vec::new();
        let mut area = 0u64;
        for start in 0..damaged.len() {
            if !damaged[start] || label[start] != usize::MAX {
                continue;
            }
            let id = regions.len();
            let mut stack = vec![start];
            label[start] = id;
            let (mut c0, mut r0, mut c1, mut r1) = (u32::MAX, u32::MAX, 0, 0);
            let (mut pixels, mut tiles) = (0u32, 0u64);
            while let Some(cell) = stack.pop() {
                let (c, r) = (cell as u32 % cols, cell as u32 / cols);
                (c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c), r1.max(r));
                pixels += changed[cell];
                tiles += 1;
                for dr in -1i64..=1 {
                    for dc in -1i64..=1 {
                        let (nc, nr) = (i64::from(c) + dc, i64::from(r) + dr);
                        if nc < 0 || nr < 0 || nc >= i64::from(cols) || nr >= i64::from(rows) {
                            continue;
                        }
                        let next = (nr as u32 * cols + nc as u32) as usize;
                        if damaged[next] && label[next] == usize::MAX {
                            label[next] = id;
                            stack.push(next);
                        }
                    }
                }
            }
            let rect = Rect::new(
                c0 * tile,
                r0 * tile,
                ((c1 + 1) * tile).min(current.width),
                ((r1 + 1) * tile).min(current.height),
            );
            regions.push((rect, pixels, tiles));
        }
        let kept: Vec<Rect> = regions
            .into_iter()
            .filter(|(_, pixels, _)| *pixels >= config.min_region_pixels)
            .map(|(rect, _, tiles)| {
                area += tiles * u64::from(tile) * u64::from(tile);
                rect
            })
            .collect();
        let total = u64::from(current.width) * u64::from(current.height);
        Self {
            regions: kept,
            fraction: (area as f64 / total as f64).min(1.0) as f32,
        }
    }

    /// Damage the compositor reported for a `frame`-sized surface. Rectangles are clipped
    /// to the frame; overlapping ones merge; a region smaller than the caret rule is noise.
    pub fn from_rects(frame: Rect, rects: &[Rect], config: &DamageConfig) -> Self {
        let mut merged: Vec<Rect> = Vec::new();
        for rect in rects {
            let clipped = Rect::new(
                rect.x0.min(frame.x1),
                rect.y0.min(frame.y1),
                rect.x1.min(frame.x1),
                rect.y1.min(frame.y1),
            );
            if clipped.area() == 0 {
                continue;
            }
            let mut current = clipped;
            let mut index = 0;
            while index < merged.len() {
                if merged[index].intersects(&current) {
                    current = current.union(&merged.swap_remove(index));
                    index = 0;
                } else {
                    index += 1;
                }
            }
            merged.push(current);
        }
        merged.retain(|rect| rect.area() >= u64::from(config.min_region_pixels));
        let area: u64 = merged.iter().map(Rect::area).sum();
        Self {
            fraction: (area as f64 / frame.area().max(1) as f64).min(1.0) as f32,
            regions: merged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, value: u8) -> Luma {
        Luma::new(width, height, vec![value; (width * height) as usize]).unwrap()
    }

    fn paint(frame: &mut Luma, rect: Rect, value: u8) {
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                frame.pixels[(y * frame.width + x) as usize] = value;
            }
        }
    }

    #[test]
    fn an_unchanged_frame_and_sensor_noise_are_no_damage() {
        let a = flat(64, 48, 100);
        let mut b = a.clone();
        for (i, p) in b.pixels.iter_mut().enumerate() {
            *p = (100 + (i % 7) as i32 - 3) as u8; // +-3 levels of noise
        }
        let config = DamageConfig::default();
        assert!(Damage::between(Some(&a), &a, &config).is_empty());
        assert!(Damage::between(Some(&a), &b, &config).is_empty());
    }

    #[test]
    fn a_blinking_caret_is_noise_but_a_moving_object_is_not() {
        let a = flat(128, 96, 30);
        let mut caret = a.clone();
        paint(&mut caret, Rect::new(40, 20, 42, 36), 250);
        let config = DamageConfig::default();
        assert!(Damage::between(Some(&a), &caret, &config).is_empty());

        let mut object = a.clone();
        paint(&mut object, Rect::new(70, 40, 100, 70), 200);
        let damage = Damage::between(Some(&a), &object, &config);
        assert_eq!(damage.regions.len(), 1);
        let bounds = damage.bounds().unwrap();
        assert!(bounds.x0 <= 70 && bounds.y0 <= 40 && bounds.x1 >= 100 && bounds.y1 >= 70);
        assert!(damage.fraction > 0.0 && damage.fraction < 0.3);
    }

    #[test]
    fn separate_changes_are_separate_regions() {
        let a = flat(128, 128, 0);
        let mut b = a.clone();
        paint(&mut b, Rect::new(0, 0, 20, 20), 255);
        paint(&mut b, Rect::new(90, 90, 120, 120), 255);
        let damage = Damage::between(Some(&a), &b, &DamageConfig::default());
        assert_eq!(damage.regions.len(), 2);
    }

    #[test]
    fn a_first_frame_or_a_new_size_is_whole_damage() {
        let a = flat(32, 32, 0);
        let b = flat(64, 32, 0);
        let config = DamageConfig::default();
        assert_eq!(Damage::between(None, &a, &config).fraction, 1.0);
        assert_eq!(Damage::between(Some(&a), &b, &config).fraction, 1.0);
    }

    #[test]
    fn compositor_rectangles_merge_clip_and_drop_carets() {
        let frame = Rect::new(0, 0, 1920, 1080);
        let config = DamageConfig::default();
        let damage = Damage::from_rects(
            frame,
            &[
                Rect::new(100, 100, 300, 200),
                Rect::new(250, 150, 400, 260),
                Rect::new(1900, 1000, 2100, 1200),
                Rect::new(800, 500, 802, 516),
            ],
            &config,
        );
        assert_eq!(damage.regions.len(), 2);
        assert!(damage.regions.contains(&Rect::new(100, 100, 400, 260)));
        assert!(damage.regions.contains(&Rect::new(1900, 1000, 1920, 1080)));
    }

    #[test]
    fn luma_from_rgb_weights_green_most() {
        let luma = Luma::from_rgb(3, 1, &[255, 0, 0, 0, 255, 0, 0, 0, 255]).unwrap();
        assert_eq!(luma.pixels, vec![76, 149, 28]);
        assert!(Luma::from_rgb(2, 2, &[0; 5]).is_err());
    }
}
