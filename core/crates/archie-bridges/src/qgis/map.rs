//! Maps drawn without a QGIS project: GDAL puts every layer in the map's CRS and extent,
//! and this module paints them — rasters underneath, then each vector layer in its own
//! colour, with translucent fills, crisp outlines and ringed points — supersampled twice
//! in each direction so lines stay smooth.

use serde_json::Value;

/// A layer ready to paint, already in the map's CRS.
pub enum Layer {
    /// Pixels covering exactly the map's extent, `width × height`, RGBA.
    Raster(Vec<u8>),
    /// A GeoJSON FeatureCollection in map coordinates.
    Vector(Value),
}

const SUPERSAMPLE: usize = 2;
const PALETTE: [[u8; 3]; 6] = [
    [47, 111, 222],
    [222, 94, 47],
    [42, 157, 110],
    [170, 72, 196],
    [214, 162, 30],
    [30, 160, 190],
];
const BACKGROUND: [u8; 3] = [246, 246, 243];

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 3]>,
    extent: [f64; 4],
}

impl Canvas {
    fn to_pixel(&self, x: f64, y: f64) -> (f64, f64) {
        let [xmin, xmax, ymin, ymax] = self.extent;
        (
            (x - xmin) / (xmax - xmin) * self.width as f64,
            (ymax - y) / (ymax - ymin) * self.height as f64,
        )
    }

    fn blend(&mut self, x: usize, y: usize, colour: [u8; 3], alpha: f32) {
        if x < self.width && y < self.height {
            let p = &mut self.pixels[y * self.width + x];
            for c in 0..3 {
                p[c] = p[c] * (1.0 - alpha) + colour[c] as f32 / 255.0 * alpha;
            }
        }
    }

    /// Even-odd fill of rings (outer and holes together), by scanline.
    fn fill(&mut self, rings: &[Vec<(f64, f64)>], colour: [u8; 3], alpha: f32) {
        let ys = rings.iter().flatten().map(|p| p.1);
        let (lo, hi) = ys.fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        if lo > hi {
            return;
        }
        let top = lo.floor().max(0.0) as usize;
        let bottom = (hi.ceil() as usize).min(self.height.saturating_sub(1));
        for row in top..=bottom {
            let y = row as f64 + 0.5;
            let mut crossings = Vec::new();
            for ring in rings {
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    if (a.1 <= y) != (b.1 <= y) {
                        crossings.push(a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
            }
            crossings.sort_by(|a, b| a.total_cmp(b));
            for pair in crossings.chunks(2) {
                if let [from, to] = pair {
                    let start = from.round().max(0.0) as usize;
                    let end = (to.round().max(0.0) as usize).min(self.width);
                    for x in start..end {
                        self.blend(x, row, colour, alpha);
                    }
                }
            }
        }
    }

    fn disc(&mut self, cx: f64, cy: f64, radius: f64, colour: [u8; 3], alpha: f32) {
        let r = radius.ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (cx + dx as f64, cy + dy as f64);
                if (dx * dx + dy * dy) as f64 <= radius * radius && x >= 0.0 && y >= 0.0 {
                    self.blend(x as usize, y as usize, colour, alpha);
                }
            }
        }
    }

    /// A line of `width` pixels, as discs along it.
    fn line(&mut self, a: (f64, f64), b: (f64, f64), width: f64, colour: [u8; 3]) {
        let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = (length / (width / 3.0).max(0.5)).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            self.disc(
                a.0 + (b.0 - a.0) * t,
                a.1 + (b.1 - a.1) * t,
                width / 2.0,
                colour,
                1.0,
            );
        }
    }
}

fn points(coords: &Value) -> Vec<(f64, f64)> {
    coords
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
        .collect()
}

fn draw_geometry(canvas: &mut Canvas, geometry: &Value, colour: [u8; 3]) {
    let s = SUPERSAMPLE as f64;
    let px = |canvas: &Canvas, pts: Vec<(f64, f64)>| {
        pts.into_iter()
            .map(|(x, y)| canvas.to_pixel(x, y))
            .collect::<Vec<_>>()
    };
    let coords = &geometry["coordinates"];
    let polygon = |canvas: &mut Canvas, rings: &Value| {
        let rings: Vec<Vec<(f64, f64)>> = rings
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| px(canvas, points(r)))
            .collect();
        canvas.fill(&rings, colour, 0.32);
        for ring in &rings {
            for i in 0..ring.len() {
                canvas.line(ring[i], ring[(i + 1) % ring.len()], 1.6 * s, colour);
            }
        }
    };
    let line = |canvas: &mut Canvas, pts: &Value| {
        let pts = px(canvas, points(pts));
        for pair in pts.windows(2) {
            canvas.line(pair[0], pair[1], 2.2 * s, colour);
        }
    };
    let point = |canvas: &mut Canvas, p: &Value| {
        if let (Some(x), Some(y)) = (
            p.get(0).and_then(Value::as_f64),
            p.get(1).and_then(Value::as_f64),
        ) {
            let (x, y) = canvas.to_pixel(x, y);
            canvas.disc(x, y, 5.5 * s, [255, 255, 255], 1.0);
            canvas.disc(x, y, 4.0 * s, colour, 1.0);
        }
    };
    match geometry["type"].as_str().unwrap_or("") {
        "Point" => point(canvas, coords),
        "MultiPoint" => coords
            .as_array()
            .into_iter()
            .flatten()
            .for_each(|p| point(canvas, p)),
        "LineString" => line(canvas, coords),
        "MultiLineString" => coords
            .as_array()
            .into_iter()
            .flatten()
            .for_each(|l| line(canvas, l)),
        "Polygon" => polygon(canvas, coords),
        "MultiPolygon" => coords
            .as_array()
            .into_iter()
            .flatten()
            .for_each(|p| polygon(canvas, p)),
        "GeometryCollection" => geometry["geometries"]
            .as_array()
            .into_iter()
            .flatten()
            .for_each(|g| draw_geometry(canvas, g, colour)),
        _ => {}
    }
}

/// Paints `layers`, first at the bottom, over `extent` [xmin, xmax, ymin, ymax] into a PNG.
pub fn paint(
    layers: &[Layer],
    extent: [f64; 4],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let (w, h) = (width as usize, height as usize);
    let mut canvas = Canvas {
        width: w * SUPERSAMPLE,
        height: h * SUPERSAMPLE,
        pixels: vec![BACKGROUND.map(|c| c as f32 / 255.0); w * h * SUPERSAMPLE * SUPERSAMPLE],
        extent,
    };
    let mut colours = PALETTE.iter().cycle();
    for layer in layers {
        match layer {
            Layer::Raster(rgba) => {
                for y in 0..canvas.height {
                    for x in 0..canvas.width {
                        let i = ((y / SUPERSAMPLE) * w + x / SUPERSAMPLE) * 4;
                        if let Some(px) = rgba.get(i..i + 4) {
                            canvas.blend(x, y, [px[0], px[1], px[2]], px[3] as f32 / 255.0);
                        }
                    }
                }
            }
            Layer::Vector(collection) => {
                let colour = *colours.next().expect("cycle");
                for feature in collection["features"].as_array().into_iter().flatten() {
                    draw_geometry(&mut canvas, &feature["geometry"], colour);
                }
            }
        }
    }
    let mut pixels = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for dy in 0..SUPERSAMPLE {
                for dx in 0..SUPERSAMPLE {
                    let p =
                        canvas.pixels[(y * SUPERSAMPLE + dy) * canvas.width + x * SUPERSAMPLE + dx];
                    for c in 0..3 {
                        sum[c] += p[c];
                    }
                }
            }
            for s in sum {
                pixels.push(
                    (s / (SUPERSAMPLE * SUPERSAMPLE) as f32 * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8,
                );
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A PNG decoded to RGBA, whatever its colour type.
pub fn rgba(png_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("the picture is too large")?
    ];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    buffer.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let out = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        _ => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    };
    Ok((out, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn features_land_where_their_coordinates_say() {
        let layer = Layer::Vector(json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [50, 0], [50, 50], [0, 50], [0, 0]]]}},
            {"type": "Feature", "geometry": {"type": "Point", "coordinates": [75, 75]}}
        ]}));
        let png = paint(&[layer], [0.0, 100.0, 0.0, 100.0], 100, 100).unwrap();
        let (pixels, w, _) = rgba(&png).unwrap();
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let inside = at(25, 75);
        assert!(
            inside[2] > inside[0],
            "the square (bottom left in map terms) is tinted blue: {inside:?}"
        );
        assert_eq!(at(90, 90), BACKGROUND, "empty map is background");
        let point = at(75, 25);
        assert!(
            point[2] > 150 && point[0] < 120,
            "the point is drawn solid: {point:?}"
        );
    }
}
