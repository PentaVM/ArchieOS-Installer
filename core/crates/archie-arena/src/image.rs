//! Captures as pixels: decoded, scaled for a model to look at, cut to the canvas.

use archie_paint::bench::Sheet;

/// Decodes a capture into a sheet of RGB pixels: a PNG (RGB or RGBA, 8 bits), or the binary
/// PPM a live frame arrives as.
pub fn decode(png_bytes: &[u8]) -> Result<Sheet, String> {
    if png_bytes.starts_with(b"P6") {
        return decode_ppm(png_bytes);
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("capture: {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("capture: too large")?];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| format!("capture: {e}"))?;
    let channels = info.color_type.samples();
    if channels < 3 {
        return Err("capture: not a colour picture".into());
    }
    let pixels = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Ok(Sheet {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// A binary PPM (`P6`, maxval 255): three whitespace-separated numbers after the magic, one
/// whitespace byte, then the rows.
pub fn decode_ppm(bytes: &[u8]) -> Result<Sheet, String> {
    let mut fields = Vec::with_capacity(3);
    let mut at = 2;
    while fields.len() < 3 {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        let number: u32 = std::str::from_utf8(&bytes[start..at])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or("capture: bad PPM header")?;
        fields.push(number);
    }
    let (width, height) = (fields[0], fields[1]);
    if fields[2] != 255 || width == 0 || height == 0 || at >= bytes.len() {
        return Err("capture: bad PPM header".into());
    }
    let body = &bytes[at + 1..];
    let needed = width as usize * height as usize * 3;
    if body.len() != needed {
        return Err("capture: PPM size does not match its header".into());
    }
    Ok(Sheet {
        width,
        height,
        pixels: body.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect(),
    })
}

/// The sheet as a binary PPM: what IRIS reads without decoding a picture format.
pub fn ppm(sheet: &Sheet) -> Vec<u8> {
    let mut bytes = format!("P6\n{} {}\n255\n", sheet.width, sheet.height).into_bytes();
    bytes.reserve(sheet.pixels.len() * 3);
    for p in &sheet.pixels {
        bytes.extend_from_slice(p);
    }
    bytes
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
            let (x0, x1) = (
                (x as f32 * scale) as u32,
                (((x + 1) as f32 * scale) as u32)
                    .min(sheet.width)
                    .max((x as f32 * scale) as u32 + 1),
            );
            let (y0, y1) = (
                (y as f32 * scale) as u32,
                (((y + 1) as f32 * scale) as u32)
                    .min(sheet.height)
                    .max((y as f32 * scale) as u32 + 1),
            );
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
    (
        Sheet {
            width: max_width,
            height,
            pixels,
        },
        scale,
    )
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
    Sheet {
        width: x1 - x,
        height: y1 - y,
        pixels,
    }
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

fn luma(p: [u8; 3]) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

/// A rectangle as `(x, y, width, height)` in pixels.
pub type Rect = (u32, u32, u32, u32);

/// A white page an editor shows: [`paper`], and only when it sits inside the editor's own
/// darker workspace — so a page still loading, all white, is not taken for a document.
/// Returns the page and the workspace's colour around it.
pub fn page_in_editor(sheet: &Sheet) -> Option<(Rect, [u8; 3])> {
    let (x, y, w, h) = paper(sheet)?;
    if w > sheet.width * 7 / 10 || x < 8 || x + w + 8 >= sheet.width {
        return None;
    }
    let mid = y + h / 2;
    let left = sheet.pixels[(mid * sheet.width + x - 6) as usize];
    let right = sheet.pixels[(mid * sheet.width + x + w + 5) as usize];
    (luma(left) < 110.0 && luma(right) < 110.0 && (luma(left) - luma(right)).abs() < 25.0)
        .then_some(((x, y, w, h), left))
}

/// Where the document is when the run ends: everything around the first page's place that
/// is not the workspace's colour, if it keeps the page's proportions — the document after
/// Archie painted, zoomed or moved it. Otherwise the page's first place.
pub fn document_now(
    sheet: &Sheet,
    first: (u32, u32, u32, u32),
    workspace: [u8; 3],
) -> (u32, u32, u32, u32) {
    let (x, y, w, h) = first;
    let (gx, gy) = (w / 3, h / 3);
    let (x0, y0) = (x.saturating_sub(gx), y.saturating_sub(gy));
    let (x1, y1) = (
        (x + w + gx).min(sheet.width),
        (y + h + gy).min(sheet.height),
    );
    let differs = |p: [u8; 3]| {
        (0..3)
            .map(|k| (p[k] as i32 - workspace[k] as i32).abs())
            .max()
            .unwrap_or(0)
            > 14
    };
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for row in y0..y1 {
        for col in x0..x1 {
            if differs(sheet.pixels[(row * sheet.width + col) as usize]) {
                left = left.min(col);
                right = right.max(col);
                top = top.min(row);
                bottom = bottom.max(row);
            }
        }
    }
    if left >= right || top >= bottom {
        return first;
    }
    let (nw, nh) = (right - left + 1, bottom - top + 1);
    let ratio = (nw as f32 / nh as f32) / (w as f32 / h as f32);
    if (0.9..=1.1).contains(&ratio) {
        (left, top, nw, nh)
    } else {
        first
    }
}

/// A sheet as a PNG data URL, for a model.
pub fn data_url(sheet: &Sheet) -> String {
    use base64::Engine;
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(sheet.png())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Sheet {
        // Grey editor with a white page from (40,30) to (99,109).
        let mut sheet = Sheet {
            width: 160,
            height: 140,
            pixels: vec![[60, 60, 60]; 160 * 140],
        };
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
        let grey = Sheet {
            width: 50,
            height: 50,
            pixels: vec![[60, 60, 60]; 2500],
        };
        assert_eq!(paper(&grey), None);
    }

    #[test]
    fn a_page_still_loading_is_not_a_document_and_a_moved_document_is_found() {
        let loading = Sheet {
            width: 160,
            height: 140,
            pixels: vec![[250, 250, 252]; 160 * 140],
        };
        assert_eq!(page_in_editor(&loading), None, "all white: still loading");
        let (page, workspace) = page_in_editor(&window()).unwrap();
        assert_eq!((page, workspace), ((40, 30, 60, 80), [60, 60, 60]));
        // Painted over and nudged 5 px right: found where it is now.
        let mut after = Sheet {
            width: 160,
            height: 140,
            pixels: vec![[60, 60, 60]; 160 * 140],
        };
        for y in 30..110 {
            for x in 45..105 {
                after.pixels[(y * 160 + x) as usize] = [120, 90, 40];
            }
        }
        assert_eq!(document_now(&after, page, workspace), (45, 30, 60, 80));
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

    #[test]
    fn a_live_frame_round_trips_as_ppm_and_a_bad_one_is_refused() {
        let frame = window();
        assert_eq!(decode(&ppm(&frame)).unwrap(), frame);
        // The header the compositor writes, with a comment-free single newline before the rows.
        let two = decode(b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00").unwrap();
        assert_eq!(
            (two.width, two.height, two.pixels.clone()),
            (2, 1, vec![[255, 0, 0], [0, 255, 0]])
        );
        assert!(decode(b"P6\n2 1\n255\n\xff\x00").is_err(), "short rows");
        assert!(
            decode(b"P6\n2 1\n65535\n\xff\x00\x00\x00\xff\x00").is_err(),
            "16-bit is not what the stage writes"
        );
    }
}
