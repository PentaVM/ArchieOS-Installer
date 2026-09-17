//! The picture a reflex model sees: one window, reduced to a fixed grey grid.
//!
//! A control is a shape — a border, a fill, a run of text inside it — and shape survives
//! being made small. Words do not, which is deliberate: this model finds where the controls
//! are, never what they say. Reading is what an application's own declarations are for, and
//! what a grounding model is asked when nothing declares anything.

use crate::ReflexError;

/// How wide and tall a tile is. A window of any shape is squeezed into this, and the same
/// squeeze is applied to every box that goes with it.
pub const TILE_WIDTH: usize = 160;
pub const TILE_HEIGHT: usize = 96;

/// One window as the model sees it, and the size it was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pixels: Vec<u8>,
    /// The window this came from, so a box in the tile can go back to where it belongs.
    pub source_width: u32,
    pub source_height: u32,
}

impl Tile {
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Reads a PNG capture and squeezes it into one tile, averaging over the pixels each
    /// cell covers so that a thin line does not disappear between samples.
    pub fn from_png(png: &[u8]) -> Result<Self, ReflexError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|_| ReflexError::Unreadable)?;
        let (width, height) = (reader.info().width, reader.info().height);
        let samples = reader.output_color_type().0.samples();
        if width == 0 || height == 0 || samples == 0 {
            return Err(ReflexError::Unreadable);
        }
        let mut sums = vec![0_u64; TILE_WIDTH * TILE_HEIGHT];
        let mut counts = vec![0_u64; TILE_WIDTH * TILE_HEIGHT];
        let mut y = 0_u32;
        while let Some(row) = reader.next_row().map_err(|_| ReflexError::Unreadable)? {
            let data = row.data();
            let cell_y = (y as usize * TILE_HEIGHT) / height as usize;
            if cell_y >= TILE_HEIGHT {
                break;
            }
            for x in 0..width as usize {
                let at = x * samples;
                let Some(pixel) = data.get(at..at + samples) else {
                    return Err(ReflexError::Unreadable);
                };
                let luma = if samples < 3 {
                    u64::from(pixel[0])
                } else {
                    (u64::from(pixel[0]) * 299
                        + u64::from(pixel[1]) * 587
                        + u64::from(pixel[2]) * 114)
                        / 1_000
                };
                let cell = cell_y * TILE_WIDTH + (x * TILE_WIDTH) / width as usize;
                sums[cell] += luma;
                counts[cell] += 1;
            }
            y += 1;
        }
        let pixels = sums
            .iter()
            .zip(counts.iter())
            .map(|(sum, count)| sum.checked_div(*count).unwrap_or(0) as u8)
            .collect();
        Ok(Self {
            pixels,
            source_width: width,
            source_height: height,
        })
    }

    /// The same window as IRIS's ephemeral frame type, for the reflex runtime's checks.
    pub fn frame(
        &self,
        source: archie_iris::SourceId,
        captured_at_ms: i64,
    ) -> Result<archie_iris::EphemeralFrame, ReflexError> {
        archie_iris::EphemeralFrame::new(
            source,
            captured_at_ms,
            TILE_WIDTH as u32,
            TILE_HEIGHT as u32,
            archie_iris::PixelFormat::Gray8,
            self.pixels.clone(),
        )
        .map_err(|_| ReflexError::Unreadable)
    }
}

impl Tile {
    /// A tile read back from a corpus.
    pub fn from_pixels(
        pixels: Vec<u8>,
        source_width: u32,
        source_height: u32,
    ) -> Result<Self, ReflexError> {
        if pixels.len() != TILE_WIDTH * TILE_HEIGHT {
            return Err(ReflexError::Corpus);
        }
        Ok(Self {
            pixels,
            source_width,
            source_height,
        })
    }
}
