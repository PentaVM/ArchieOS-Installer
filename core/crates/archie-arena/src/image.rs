//! Captures as pixels: decoded, scaled for a model to look at, cut to the canvas.

use archie_paint::bench::Sheet;

/// Decodes a PNG (RGB or RGBA, 8 bits) into a sheet of RGB pixels.
pub fn decode(png_bytes: &[u8]) -> Result<Sheet, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("capture: {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("capture: too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| format!("capture: {e}"))?;
    let channels = info.color_type.samples();
    if channels < 3 {
        return Err("capture: not a colour picture".into());
    }
    let pixels = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Ok(Sheet { width: info.width, height: info.height, pixels })
}

/// Scaled so that it is at most `max_width` wide (box filter); returns the sheet and how many
/// original pixels one of its pixels spans.
pub fn fit_width(sheet: &Sheet, max_width: u32) -> (Sheet, f32) {
    if sheet.width <= max_width {
        return (sheet.clone(), 1.0);
    }
    let scale = sheet.width as f32 / max_width as f32;
    let height = (sheet.height as f32 / scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity((max_width * height) as usize);
    for y in 0..height {
        for x in 0..max_width {
            let (x0, x1) = ((x as f32 * scale) as u32, (((x + 1) as f32 * scale) as u32).min(sheet.width).max((x as f32 * scale) as u32 + 1));
            let (y0, y1) = ((y as f32 * scale) as u32, (((y + 1) as f32 * scale) as u32).min(sheet.height).max((y as f32 * scale) as u32 + 1));
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            for sy in y0..y1.min(sheet.height) {
                for sx in x0..x1.min(sheet.width) {
                    let p = sheet.pixels[(sy * sheet.width + sx) as usize];
                    for k in 0..3 {
                        sum[k] += p[k] as u32;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            pixels.push([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]);
        }
    }
    (Sheet { width: max_width, height, pixels }, scale)
}

/// The part of a sheet inside a rectangle (clamped to it).
pub fn crop(sheet: &Sheet, x: u32, y: u32, width: u32, height: u32) -> Sheet {
    let x1 = (x + width).min(sheet.width);
    let y1 = (y + height).min(sheet.height);
    let (x, y) = (x.min(x1), y.min(y1));
    let mut pixels = Vec::with_capacity(((x1 - x) * (y1 - y)) as usize);
    for row in y..y1 {
        for col in x..x1 {
            pixels.push(sheet.pixels[(row * sheet.width + col) as usize]);
        }
    }
    Sheet { width: x1 - x, height: y1 - y, pixels }
}

/// The largest near-white rectangle, where a new document's paper is: grown from the most
/// central white row run, as long as rows stay mostly white. `None` when there is none of
/// at least a tenth of the sheet.
pub fn paper(sheet: &Sheet) -> Option<(u32, u32, u32, u32)> {
    let white = |p: [u8; 3]| p.iter().all(|c| *c >= 245);
    let row_run = |y: u32| -> (u32, u32) {
        // The longest run of white in this row.
        let (mut best, mut start, mut length) = ((0, 0), 0, 0);
        for x in 0..sheet.width {
            if white(sheet.pixels[(y * sheet.width + x) as usize]) {
                if length == 0 {
                    start = x;
                }
                length += 1;
                if length > best.1 {
                    best = (start, length);
                }
            } else {
                length = 0;
            }
        }
        best
    };
    let mid = sheet.height / 2;
    let (x0, run) = row_run(mid);
    if run < sheet.width / 10 {
        return None;
    }
    let same = |y: u32| {
        let (x, length) = row_run(y);
        x <= x0 + 2 && x + length + 2 >= x0 + run
    };
    let mut top = mid;
    while top > 0 && same(top - 1) {
        top -= 1;
    }
    let mut bottom = mid;
    while bottom + 1 < sheet.height && same(bottom + 1) {
        bottom += 1;
    }
    let height = bottom - top + 1;
    (height >= sheet.height / 10).then_some((x0, top, run, height))
}

/// A sheet as a PNG data URL, for a model.
pub fn data_url(sheet: &Sheet) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(sheet.png()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Sheet {
        // Grey editor with a white page from (40,30) to (99,109).
        let mut sheet = Sheet { width: 160, height: 140, pixels: vec![[60, 60, 60]; 160 * 140] };
        for y in 30..110 {
            for x in 40..100 {
                sheet.pixels[(y * 160 + x) as usize] = [255, 255, 255];
            }
        }
        sheet
    }

    #[test]
    fn the_white_page_is_found_in_an_editor() {
        assert_eq!(paper(&window()), Some((40, 30, 60, 80)));
        let grey = Sheet { width: 50, height: 50, pixels: vec![[60, 60, 60]; 2500] };
        assert_eq!(paper(&grey), None);
    }

    #[test]
    fn scaling_keeps_the_proportions_and_says_by_how_much() {
        let (small, scale) = fit_width(&window(), 80);
        assert_eq!((small.width, small.height), (80, 70));
        assert!((scale - 2.0).abs() < 1e-6);
        let page = crop(&window(), 40, 30, 60, 80);
        assert!(page.pixels.iter().all(|p| *p == [255, 255, 255]));
        let round = decode(&window().png()).unwrap();
        assert_eq!(round, window());
    }
}
