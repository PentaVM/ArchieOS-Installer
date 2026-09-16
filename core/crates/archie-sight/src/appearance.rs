//! What a control looked like: small enough to keep, cheap enough to check again.
//!
//! IRIS's fast path is a local model that re-finds in a new frame what it already knows.
//! Until one is trained for interfaces, this stands in for it. It reduces the box a
//! grounding model returned to a 9x8 grid of average luminance. Drawing a control the way
//! hover, focus and a press do shifts the whole box and leaves the grid's shape alone;
//! different writing, a different icon or a different control does not.
//!
//! It perceives nothing and grants nothing. A match means the old position is worth using
//! once more; a mismatch means asking the grounding model again.

use archie_iris::Rect;

const CELLS_X: usize = 9;
const CELLS_Y: usize = 8;
const CELLS: usize = CELLS_X * CELLS_Y;
/// How far one cell may move, once the whole box is brought to the same brightness. A
/// cell's average moves by the contrast of what covers it times the fraction it covers,
/// so a caret or a one-pixel border moves it by a tenth of full contrast at most, while
/// different writing in the same box moves it by a third or more.
const CELL_TOLERANCE: i32 = 32;
/// How many of the 72 cells may move that far and still be the same control.
const SAME_CELLS: usize = 4;

/// The shape of one box: what each part of it averages, and how bright it is overall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    cells: [u8; CELLS],
    mean: u8,
}

impl Appearance {
    /// How many parts of the box disagree, once both are brought to the same brightness.
    fn differing(&self, other: &Self) -> usize {
        let shift = i32::from(other.mean) - i32::from(self.mean);
        self.cells
            .iter()
            .zip(other.cells.iter())
            .filter(|(mine, theirs)| {
                (i32::from(**mine) + shift - i32::from(**theirs)).abs() > CELL_TOLERANCE
            })
            .count()
    }

    /// Whether this is still the same control, allowing for one drawn differently while
    /// it is hovered, focused or held down.
    pub fn matches(&self, other: &Self) -> bool {
        self.differing(other) <= SAME_CELLS
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppearanceError {
    #[error("the capture could not be read as a PNG")]
    Unreadable,
    #[error("that box is not inside the capture")]
    Outside,
}

/// Reduces `area` of a PNG capture. Only the rows the box covers are decoded, and no pixel
/// outside it is read.
pub fn appearance_of(png: &[u8], area: Rect) -> Result<Appearance, AppearanceError> {
    if area.width == 0 || area.height == 0 || area.x < 0 || area.y < 0 {
        return Err(AppearanceError::Outside);
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // Small colour types and 16-bit channels become plain 8-bit samples, so one loop below
    // handles every capture a compositor might write.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|_| AppearanceError::Unreadable)?;
    let (width, height) = (reader.info().width, reader.info().height);
    let samples = reader.output_color_type().0.samples();
    let (top, left) = (area.y as u32, area.x as u32);
    let bottom = top
        .checked_add(area.height)
        .ok_or(AppearanceError::Outside)?;
    let right = left
        .checked_add(area.width)
        .ok_or(AppearanceError::Outside)?;
    if bottom > height || right > width || samples == 0 {
        return Err(AppearanceError::Outside);
    }

    let mut sums = [0_u64; CELLS];
    let mut counts = [0_u64; CELLS];
    let mut y = 0_u32;
    while y < bottom {
        let Some(row) = reader.next_row().map_err(|_| AppearanceError::Unreadable)? else {
            return Err(AppearanceError::Unreadable);
        };
        if y >= top {
            let data = row.data();
            let cell_y = ((y - top) as usize * CELLS_Y) / area.height as usize;
            for x in left..right {
                let at = x as usize * samples;
                let pixel = data
                    .get(at..at + samples)
                    .ok_or(AppearanceError::Unreadable)?;
                let luma = if samples < 3 {
                    u64::from(pixel[0])
                } else {
                    (u64::from(pixel[0]) * 299
                        + u64::from(pixel[1]) * 587
                        + u64::from(pixel[2]) * 114)
                        / 1_000
                };
                let cell = cell_y * CELLS_X + ((x - left) as usize * CELLS_X) / area.width as usize;
                sums[cell] += luma;
                counts[cell] += 1;
            }
        }
        y += 1;
    }

    let mut cells = [0_u8; CELLS];
    let mut total = 0_u64;
    let mut seen = 0_u64;
    for (cell, average) in cells.iter_mut().enumerate() {
        if let Some(value) = sums[cell].checked_div(counts[cell]) {
            *average = value as u8;
            total += value;
            seen += 1;
        }
    }
    Ok(Appearance {
        cells,
        mean: (total / seen.max(1)) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey window with one control in it: a bordered box holding `bars` marks, drawn
    /// `shade` levels darker, with or without a caret blinking in it.
    fn window(bars: usize, shade: u8, caret: bool) -> Vec<u8> {
        let (width, height) = (120_u32, 60_u32);
        let (bx, by, bw, bh) = (20_u32, 10_u32, 80_u32, 30_u32);
        let mut pixels = vec![0xf2_u8; (width * height) as usize];
        for y in by..by + bh {
            for x in bx..bx + bw {
                let edge = x == bx || x == bx + bw - 1 || y == by || y == by + bh - 1;
                pixels[(y * width + x) as usize] = if edge {
                    0x40
                } else {
                    0xd8_u8.saturating_sub(shade)
                };
            }
        }
        for bar in 0..bars {
            let left = bx + 8 + bar as u32 * 9;
            for y in by + 8..by + bh - 8 {
                for x in left..left + 3 {
                    pixels[(y * width + x) as usize] = 0x10;
                }
            }
        }
        if caret {
            for y in by + 6..by + bh - 6 {
                pixels[(y * width + bx + 70) as usize] = 0x00;
            }
        }
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        png
    }

    fn control() -> Rect {
        Rect::new(20, 10, 80, 30).unwrap()
    }

    #[test]
    fn a_control_drawn_differently_is_still_that_control() {
        let plain = appearance_of(&window(3, 0, false), control()).unwrap();
        let shaded = appearance_of(&window(3, 24, false), control()).unwrap();
        let caret = appearance_of(&window(3, 0, true), control()).unwrap();
        let rewritten = appearance_of(&window(7, 0, false), control()).unwrap();
        assert!(
            plain.matches(&shaded),
            "hovered or held down is the same control ({} parts apart)",
            plain.differing(&shaded)
        );
        assert!(
            plain.matches(&caret),
            "a caret blinking in it is not a different control ({} parts apart)",
            plain.differing(&caret)
        );
        assert!(
            !plain.matches(&rewritten),
            "different writing in the same box is a different control ({} parts apart)",
            plain.differing(&rewritten)
        );
    }

    #[test]
    fn a_box_outside_the_capture_says_nothing() {
        assert!(matches!(
            appearance_of(&window(3, 0, false), Rect::new(60, 10, 200, 30).unwrap()),
            Err(AppearanceError::Outside)
        ));
        assert!(matches!(
            appearance_of(b"not a png at all", control()),
            Err(AppearanceError::Unreadable)
        ));
    }
}
