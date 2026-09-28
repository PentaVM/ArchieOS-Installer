//! Pictures of 3D results without a GPU or a display: an STL mesh drawn in software.
//!
//! A bridge that makes a solid — FreeCAD, a STEP from KiCad — exports its tessellation as
//! STL and this module draws it: orthographic, fitted to the frame, z-buffered, lit from
//! two sides, and supersampled twice in each direction so edges are smooth. The picture is
//! for a person skimming stages on the desk and for a planner checking a shape, so it
//! favours legibility: a neutral background, a steel-blue part, clear light and shade.

use std::io::Write;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PreviewError {
    #[error("the mesh is not STL: {0}")]
    Mesh(String),
    #[error("the mesh has no triangles")]
    Empty,
    #[error("encoding the picture: {0}")]
    Png(String),
}

pub type Vec3 = [f64; 3];

/// A view direction: where the camera looks from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Iso,
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
}

impl View {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "iso" => Self::Iso,
            "front" => Self::Front,
            "back" => Self::Back,
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => return None,
        })
    }

    pub const NAMES: [&'static str; 7] = ["iso", "front", "back", "top", "bottom", "left", "right"];

    /// Camera axes (right, up, towards the viewer), Z up in the model as CAD has it.
    fn axes(self) -> (Vec3, Vec3, Vec3) {
        let toward: Vec3 = match self {
            Self::Iso => normalize([1.0, -1.0, 0.8]),
            Self::Front => [0.0, -1.0, 0.0],
            Self::Back => [0.0, 1.0, 0.0],
            Self::Top => [0.0, 0.0, 1.0],
            Self::Bottom => [0.0, 0.0, -1.0],
            Self::Left => [-1.0, 0.0, 0.0],
            Self::Right => [1.0, 0.0, 0.0],
        };
        let world_up = if matches!(self, Self::Top | Self::Bottom) {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let right = normalize(cross(world_up, toward));
        let up = cross(toward, right);
        (right, up, toward)
    }
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize(v: Vec3) -> Vec3 {
    let length = dot(v, v).sqrt();
    if length == 0.0 {
        v
    } else {
        [v[0] / length, v[1] / length, v[2] / length]
    }
}

/// The triangles of an STL file, binary or ASCII.
pub fn read_stl(bytes: &[u8]) -> Result<Vec<[Vec3; 3]>, PreviewError> {
    let binary_size = |n: usize| 84 + n * 50;
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if bytes.len() == binary_size(count) {
            let mut triangles = Vec::with_capacity(count);
            for i in 0..count {
                let at = 84 + i * 50 + 12;
                let float = |k: usize| {
                    let o = at + k * 4;
                    f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64
                };
                triangles.push([
                    [float(0), float(1), float(2)],
                    [float(3), float(4), float(5)],
                    [float(6), float(7), float(8)],
                ]);
            }
            return Ok(triangles);
        }
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PreviewError::Mesh("neither binary nor text".into()))?;
    if !text.trim_start().starts_with("solid") {
        return Err(PreviewError::Mesh(
            "text that does not start with solid".into(),
        ));
    }
    let mut vertices = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() == Some("vertex") {
            let v: Vec<f64> = words.filter_map(|w| w.parse().ok()).collect();
            if v.len() != 3 {
                return Err(PreviewError::Mesh(format!("a vertex line: {line}")));
            }
            vertices.push([v[0], v[1], v[2]]);
        }
    }
    if vertices.len() % 3 != 0 {
        return Err(PreviewError::Mesh("vertices are not in threes".into()));
    }
    Ok(vertices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

/// The mesh's bounding box, [min, max].
pub fn bounds(triangles: &[[Vec3; 3]]) -> Option<[Vec3; 2]> {
    let mut points = triangles.iter().flatten();
    let first = *points.next()?;
    Some(points.fold([first, first], |[lo, hi], p| {
        [
            [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
            [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
        ]
    }))
}

const SUPERSAMPLE: usize = 2;
const PART: Vec3 = [0.36, 0.52, 0.74];
const BACKGROUND_TOP: Vec3 = [0.97, 0.97, 0.96];
const BACKGROUND_BOTTOM: Vec3 = [0.86, 0.87, 0.88];

/// Draws `triangles` from `view` into a PNG of `width` × `height`.
pub fn render_png(
    triangles: &[[Vec3; 3]],
    view: View,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, PreviewError> {
    if triangles.is_empty() {
        return Err(PreviewError::Empty);
    }
    let (w, h) = (width as usize * SUPERSAMPLE, height as usize * SUPERSAMPLE);
    let (right, up, toward) = view.axes();
    let [lo, hi] = bounds(triangles).ok_or(PreviewError::Empty)?;
    let centre = [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ];
    // Fit the projected box with a margin.
    let mut extent = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
    for p in triangles.iter().flatten() {
        let d = sub(*p, centre);
        let (x, y) = (dot(d, right), dot(d, up));
        extent = [
            extent[0].min(x),
            extent[1].max(x),
            extent[2].min(y),
            extent[3].max(y),
        ];
    }
    let span_x = (extent[1] - extent[0]).max(1e-9);
    let span_y = (extent[3] - extent[2]).max(1e-9);
    let scale = 0.86 * (w as f64 / span_x).min(h as f64 / span_y);
    let mid_x = (extent[0] + extent[1]) / 2.0;
    let mid_y = (extent[2] + extent[3]) / 2.0;
    let project = |p: Vec3| {
        let d = sub(p, centre);
        (
            w as f64 / 2.0 + (dot(d, right) - mid_x) * scale,
            h as f64 / 2.0 - (dot(d, up) - mid_y) * scale,
            dot(d, toward),
        )
    };

    let mut depth = vec![f64::MIN; w * h];
    let mut colour = vec![[0.0f64; 3]; w * h];
    for (i, pixel) in colour.iter_mut().enumerate() {
        let t = (i / w) as f64 / h as f64;
        for c in 0..3 {
            pixel[c] = BACKGROUND_TOP[c] * (1.0 - t) + BACKGROUND_BOTTOM[c] * t;
        }
    }
    let key = normalize([0.4, 0.7, 1.0]);
    let fill = normalize([-0.8, -0.2, 0.4]);
    for triangle in triangles {
        let normal = normalize(cross(
            sub(triangle[1], triangle[0]),
            sub(triangle[2], triangle[0]),
        ));
        // Light in camera space, so every view is lit the same way; both faces shade, so
        // a mesh with flipped normals still reads.
        let n = [dot(normal, right), dot(normal, up), dot(normal, toward)];
        let n = if n[2] < 0.0 { [-n[0], -n[1], -n[2]] } else { n };
        let light = 0.28 + 0.62 * dot(n, key).max(0.0) + 0.18 * dot(n, fill).max(0.0);
        let specular = dot(n, normalize([key[0], key[1], key[2] + 1.0]))
            .max(0.0)
            .powi(24)
            * 0.25;
        let shade = [
            (PART[0] * light + specular).min(1.0),
            (PART[1] * light + specular).min(1.0),
            (PART[2] * light + specular).min(1.0),
        ];
        let [a, b, c] = triangle.map(project);
        let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
        let max_x = (a.0.max(b.0).max(c.0).ceil() as usize).min(w.saturating_sub(1));
        let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
        let max_y = (a.1.max(b.1).max(c.1).ceil() as usize).min(h.saturating_sub(1));
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area.abs() < 1e-12 || min_x > max_x || min_y > max_y {
            continue;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
                let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
                let index = y * w + x;
                if z > depth[index] {
                    depth[index] = z;
                    colour[index] = shade;
                }
            }
        }
    }
    // Outline where depth jumps: silhouettes and creases read at a glance.
    let range = (hi[0] - lo[0])
        .max(hi[1] - lo[1])
        .max(hi[2] - lo[2])
        .max(1e-9);
    let jump = range * 0.02;
    let mut outlined = colour.clone();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let here = depth[i];
            let edge = [i - 1, i + 1, i - w, i + w].iter().any(|&j| {
                let there = depth[j];
                (here == f64::MIN) != (there == f64::MIN)
                    || (here != f64::MIN && there != f64::MIN && (here - there).abs() > jump)
            });
            if edge && here != f64::MIN {
                outlined[i] = [0.13, 0.17, 0.24];
            }
        }
    }

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut sum = [0.0; 3];
            for dy in 0..SUPERSAMPLE {
                for dx in 0..SUPERSAMPLE {
                    let p = outlined[(y * SUPERSAMPLE + dy) * w + x * SUPERSAMPLE + dx];
                    for c in 0..3 {
                        sum[c] += p[c];
                    }
                }
            }
            for s in sum {
                let v = s / (SUPERSAMPLE * SUPERSAMPLE) as f64;
                pixels.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PreviewError::Png(e.to_string()))?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| PreviewError::Png(e.to_string()))?;
    }
    Ok(out)
}

/// A binary STL of `triangles`, for tests and for bridges that build meshes themselves.
pub fn write_stl(triangles: &[[Vec3; 3]]) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out.write_all(&(triangles.len() as u32).to_le_bytes())
        .unwrap();
    for t in triangles {
        let n = normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])));
        for v in std::iter::once(n).chain(t.iter().copied()) {
            for c in v {
                out.write_all(&(c as f32).to_le_bytes()).unwrap();
            }
        }
        out.write_all(&[0, 0]).unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 × 10 × 5 box as twelve triangles.
    pub fn cuboid() -> Vec<[Vec3; 3]> {
        let (x, y, z) = (20.0, 10.0, 5.0);
        let p = [
            [0.0, 0.0, 0.0],
            [x, 0.0, 0.0],
            [x, y, 0.0],
            [0.0, y, 0.0],
            [0.0, 0.0, z],
            [x, 0.0, z],
            [x, y, z],
            [0.0, y, z],
        ];
        let faces = [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        faces.iter().map(|f| [p[f[0]], p[f[1]], p[f[2]]]).collect()
    }

    #[test]
    fn binary_and_text_stl_read_the_same() {
        let triangles = cuboid();
        assert_eq!(read_stl(&write_stl(&triangles)).unwrap(), triangles);
        let text = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(read_stl(text.as_bytes()).unwrap().len(), 1);
        assert!(read_stl(b"garbage").is_err());
    }

    #[test]
    fn a_part_is_drawn_in_the_middle_of_a_light_background() {
        let png = render_png(&cuboid(), View::Iso, 160, 120).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!((info.width, info.height), (160, 120));
        let at = |x: usize, y: usize| {
            let i = (y * 160 + x) * 3;
            [buffer[i], buffer[i + 1], buffer[i + 2]]
        };
        let corner = at(2, 2);
        assert!(corner.iter().all(|&c| c > 230), "background {corner:?}");
        let middle = at(80, 60);
        assert!(middle[2] > middle[0], "the part is blue: {middle:?}");
        assert!(render_png(&[], View::Top, 10, 10).is_err());
    }
}
