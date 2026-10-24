//! Pixel conversions for uncompressed camera formats.

use crate::Error;

/// Packed YUYV 4:2:2 (BT.601, limited range, as UVC cameras send it) to packed RGB.
pub fn yuyv_to_rgb(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, Error> {
    let (w, h) = (width as usize, height as usize);
    if w % 2 != 0 || data.len() < w * h * 2 {
        return Err(Error::Frame(format!(
            "YUYV {width}x{height} needs an even width and {} bytes, got {}",
            w * h * 2,
            data.len()
        )));
    }
    let mut rgb = vec![0u8; w * h * 3];
    for (pair, out) in data[..w * h * 2]
        .chunks_exact(4)
        .zip(rgb.chunks_exact_mut(6))
    {
        let (y0, u, y1, v) = (pair[0], pair[1], pair[2], pair[3]);
        out[..3].copy_from_slice(&pixel(y0, u, v));
        out[3..].copy_from_slice(&pixel(y1, u, v));
    }
    Ok(rgb)
}

/// BT.601 limited range in fixed point (8 fractional bits).
fn pixel(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let clamp = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
    [
        clamp(298 * c + 409 * e),
        clamp(298 * c - 100 * d - 208 * e),
        clamp(298 * c + 516 * d),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_range_black_white_and_grey() {
        // Y=16 is black, Y=235 white, neutral chroma at 128.
        let rgb = yuyv_to_rgb(&[16, 128, 235, 128, 126, 128, 126, 128], 4, 1).unwrap();
        assert_eq!(&rgb[0..3], &[0, 0, 0]);
        assert_eq!(&rgb[3..6], &[255, 255, 255]);
        assert_eq!(&rgb[6..9], &[128, 128, 128]);
    }

    #[test]
    fn red_has_red_on_top() {
        // BT.601 limited-range red: Y=81 U=90 V=240.
        let rgb = yuyv_to_rgb(&[81, 90, 81, 240], 2, 1).unwrap();
        assert!(rgb[0] > 250 && rgb[1] < 5 && rgb[2] < 5, "{rgb:?}");
    }

    #[test]
    fn short_or_odd_frames_are_refused() {
        assert!(yuyv_to_rgb(&[0; 7], 2, 2).is_err());
        assert!(yuyv_to_rgb(&[0; 6], 3, 1).is_err());
    }
}
