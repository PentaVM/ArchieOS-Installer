//! A frame as IRIS was trained to see it: decoded to RGB, letterboxed onto a square canvas.
//!
//! The resize is Pillow's `BILINEAR` (what `beta_data.letterbox` calls), reproduced to the
//! integer: the same support-scaled triangle filter, the same 22-bit fixed-point weights, the
//! horizontal pass before the vertical one and the same rounding between them. A different
//! bilinear (OpenCV's, or one without Pillow's antialiasing when shrinking) would hand the
//! encoders a different picture than the one the weights learned on.

use crate::Error;

/// The grey the canvas is padded with (`beta_data.PAD_VALUE`).
pub const PAD_VALUE: u8 = 124;

/// An RGB image, row-major, three bytes a pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Rgb {
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Result<Self, Error> {
        if width == 0 || height == 0 || pixels.len() != width * height * 3 {
            return Err(Error::Image("pixel buffer does not match its size".into()));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
}

/// Decode a JPEG or PNG from memory. Nothing is written anywhere.
pub fn decode(bytes: &[u8]) -> Result<Rgb, Error> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(bytes)
    } else {
        Err(Error::Image("only JPEG and PNG frames are read".into()))
    }
}

fn decode_jpeg(bytes: &[u8]) -> Result<Rgb, Error> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    let data = decoder
        .decode()
        .map_err(|error| Error::Image(format!("jpeg: {error}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| Error::Image("jpeg without a header".into()))?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    let pixels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => data,
        jpeg_decoder::PixelFormat::L8 => data.iter().flat_map(|&v| [v, v, v]).collect(),
        jpeg_decoder::PixelFormat::L16 => data
            .chunks_exact(2)
            .flat_map(|pair| {
                let v = pair[0];
                [v, v, v]
            })
            .collect(),
        jpeg_decoder::PixelFormat::CMYK32 => data
            .chunks_exact(4)
            .flat_map(|p| {
                // Adobe CMYK JPEGs are stored inverted, as Pillow reads them.
                let k = u16::from(p[3]);
                let channel = |c: u8| ((u16::from(c) * k) / 255) as u8;
                [channel(p[0]), channel(p[1]), channel(p[2])]
            })
            .collect(),
    };
    Rgb::new(width, height, pixels)
}

fn decode_png(bytes: &[u8]) -> Result<Rgb, Error> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| Error::Image(format!("png: {error}")))?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| Error::Image("png too large".into()))?
    ];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| Error::Image(format!("png: {error}")))?;
    let data = &buffer[..frame.buffer_size()];
    let (width, height) = (frame.width as usize, frame.height as usize);
    let pixels: Vec<u8> = match frame.color_type {
        png::ColorType::Rgb => data.to_vec(),
        // Pillow's `convert("RGB")` drops alpha without compositing.
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&v| [v, v, v]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        png::ColorType::Indexed => return Err(Error::Image("png palette was not expanded".into())),
    };
    Rgb::new(width, height, pixels)
}

const PRECISION_BITS: u32 = 32 - 8 - 2;

/// Pillow's `precompute_coeffs` for the bilinear filter (support 1), then
/// `normalize_coeffs_8bpc`. Returns each output's first input index, its tap count, and
/// `taps` fixed-point weights per output.
fn coefficients(input: usize, output: usize) -> (Vec<(usize, usize)>, Vec<i32>, usize) {
    let scale = input as f64 / output as f64;
    let filterscale = scale.max(1.0);
    let support = filterscale;
    let taps = support.ceil() as usize * 2 + 1;
    let mut bounds = Vec::with_capacity(output);
    let mut weights = vec![0i32; output * taps];
    let mut row = vec![0f64; taps];
    for xx in 0..output {
        let center = (xx as f64 + 0.5) * scale;
        let ss = 1.0 / filterscale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(input as i64) as usize - xmin;
        let mut total = 0.0;
        for (x, slot) in row.iter_mut().enumerate().take(xmax) {
            let t = ((x + xmin) as f64 - center + 0.5) * ss;
            let w = if t.abs() < 1.0 { 1.0 - t.abs() } else { 0.0 };
            *slot = w;
            total += w;
        }
        for (x, slot) in row.iter_mut().enumerate() {
            let w = if x < xmax && total != 0.0 {
                *slot / total
            } else {
                0.0
            };
            let fixed = w * f64::from(1u32 << PRECISION_BITS);
            weights[xx * taps + x] = if w < 0.0 {
                (-0.5 + fixed) as i32
            } else {
                (0.5 + fixed) as i32
            };
        }
        bounds.push((xmin, xmax));
    }
    (bounds, weights, taps)
}

fn clip8(value: i64) -> u8 {
    (value >> PRECISION_BITS).clamp(0, 255) as u8
}

/// Pillow `Image.resize((width, height), Image.BILINEAR)` for an RGB image.
pub fn resize_bilinear(image: &Rgb, width: usize, height: usize) -> Rgb {
    let mut current = image.clone();
    if width != image.width {
        let (bounds, weights, taps) = coefficients(image.width, width);
        let mut out = vec![0u8; width * current.height * 3];
        for y in 0..current.height {
            let row = &current.pixels[y * current.width * 3..(y + 1) * current.width * 3];
            for (xx, &(xmin, count)) in bounds.iter().enumerate() {
                let k = &weights[xx * taps..xx * taps + count];
                for channel in 0..3 {
                    let mut sum: i64 = 1 << (PRECISION_BITS - 1);
                    for (x, &w) in k.iter().enumerate() {
                        sum += i64::from(row[(xmin + x) * 3 + channel]) * i64::from(w);
                    }
                    out[(y * width + xx) * 3 + channel] = clip8(sum);
                }
            }
        }
        current = Rgb {
            width,
            height: current.height,
            pixels: out,
        };
    }
    if height != image.height {
        let (bounds, weights, taps) = coefficients(current.height, height);
        let mut out = vec![0u8; current.width * height * 3];
        for (yy, &(ymin, count)) in bounds.iter().enumerate() {
            let k = &weights[yy * taps..yy * taps + count];
            for x in 0..current.width {
                for channel in 0..3 {
                    let mut sum: i64 = 1 << (PRECISION_BITS - 1);
                    for (y, &w) in k.iter().enumerate() {
                        sum += i64::from(
                            current.pixels[((ymin + y) * current.width + x) * 3 + channel],
                        ) * i64::from(w);
                    }
                    out[(yy * current.width + x) * 3 + channel] = clip8(sum);
                }
            }
        }
        current = Rgb {
            width: current.width,
            height,
            pixels: out,
        };
    }
    current
}

/// Python's `round` for the resized side: halves go to the even neighbour.
fn round_half_even(value: f64) -> usize {
    let floor = value.floor();
    let fraction = value - floor;
    let rounded = if fraction > 0.5 || (fraction == 0.5 && floor % 2.0 != 0.0) {
        floor + 1.0
    } else {
        floor
    };
    rounded.max(1.0) as usize
}

/// `beta_data.letterbox`: the longest side scaled to `size`, the image at the top left of a
/// `size` x `size` canvas, the rest `PAD_VALUE`. Returns the canvas and the scale used.
pub fn letterbox(image: &Rgb, size: usize) -> (Rgb, f64) {
    let scale = size as f64 / image.width.max(image.height) as f64;
    let new_w = round_half_even(image.width as f64 * scale);
    let new_h = round_half_even(image.height as f64 * scale);
    let resized = resize_bilinear(image, new_w, new_h);
    let mut canvas = vec![PAD_VALUE; size * size * 3];
    for y in 0..new_h.min(size) {
        let from = &resized.pixels[y * new_w * 3..(y * new_w + new_w.min(size)) * 3];
        canvas[y * size * 3..y * size * 3 + from.len()].copy_from_slice(from);
    }
    (
        Rgb {
            width: size,
            height: size,
            pixels: canvas,
        },
        scale,
    )
}

/// The canvas as a `[1, 3, S, S]` tensor in [0, 1].
pub fn to_tensor(canvas: &Rgb, device: &candle_core::Device) -> Result<candle_core::Tensor, Error> {
    let size = canvas.width;
    let plane = size * size;
    let mut data = vec![0f32; 3 * plane];
    for (index, pixel) in canvas.pixels.chunks_exact(3).enumerate() {
        for channel in 0..3 {
            data[channel * plane + index] = f32::from(pixel[channel]) / 255.0;
        }
    }
    Ok(candle_core::Tensor::from_vec(
        data,
        (1, 3, size, canvas.height),
        device,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_to_even_like_python() {
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round_half_even(0.2), 1);
    }

    #[test]
    fn a_flat_image_stays_flat_through_the_resize() {
        let image = Rgb::new(7, 5, vec![200; 7 * 5 * 3]).unwrap();
        let resized = resize_bilinear(&image, 3, 11);
        assert!(resized.pixels.iter().all(|&v| v == 200));
    }

    #[test]
    fn the_letterbox_pads_below_and_to_the_right() {
        let image = Rgb::new(4, 2, vec![10; 4 * 2 * 3]).unwrap();
        let (canvas, scale) = letterbox(&image, 8);
        assert_eq!(scale, 2.0);
        assert_eq!(canvas.pixels[0], 10);
        assert_eq!(canvas.pixels[(7 * 8) * 3], PAD_VALUE);
        assert_eq!(canvas.pixels[(3 * 8 + 7) * 3], 10);
    }
}
