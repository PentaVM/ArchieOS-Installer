//! XaviCottier materials.js / gpu-surface-bake.js height and packed surface formulas.
//! CPU execution reproduces the production linear RGBA contract, not a GPU test.
//! Pixel centers match the source fullscreen bake. Trigonometric GPU rounding is
//! implementation-dependent; this deterministic f64 reference is not bitwise GPU parity.
use crate::NodeStyle;
use std::f64::consts::TAU;
use std::sync::OnceLock;
#[derive(Debug, PartialEq)]
pub struct PackedSurface {
    pub albedo: Vec<u8>,
    pub surface: Vec<u8>,
}
fn fract(x: f64) -> f64 {
    x - x.floor()
}
fn cells() -> &'static [[f64; 2]; 64] {
    static CELLS: OnceLock<[[f64; 2]; 64]> = OnceLock::new();
    CELLS.get_or_init(|| {
        std::array::from_fn(|i| {
            let x = (i % 8) as f64;
            let y = (i / 8) as f64;
            // Source CELLS stores each seed as Float32Array before the height pass.
            [
                fract((x * 127.1 + y * 311.7).sin() * 43758.5453) as f32 as f64,
                fract((x * 269.5 + y * 183.3).sin() * 43758.5453) as f32 as f64,
            ]
        })
    })
}
/// Effective frequency is an integer from SurfaceCache, not the UI frequency.
/// This function is periodic in u/v with that frequency.
pub fn height(style: NodeStyle, u: f64, v: f64, frequency: f64) -> f64 {
    let u = u * frequency;
    let v = v * frequency;
    match style {
        NodeStyle::PrismCut => {
            ((TAU * u * 12.).sin() + (TAU * v * 12.).sin()).abs() * 0.3
                + (TAU * (u + v) * 6.).sin().abs() * 0.3
        }
        NodeStyle::Obsidian => {
            let x = u * 8.;
            let y = v * 8.;
            let ix = x.floor();
            let iy = y.floor();
            let mut first = 9.;
            let mut second = 9.;
            for j in -1..=1 {
                for i in -1..=1 {
                    let cx = ix + i as f64;
                    let cy = iy + j as f64;
                    let seed =
                        cells()[(cy.rem_euclid(8.) as usize) * 8 + cx.rem_euclid(8.) as usize];
                    let d = (cx + seed[0] - x).powi(2) + (cy + seed[1] - y).powi(2);
                    if d < first {
                        second = first;
                        first = d;
                    } else if d < second {
                        second = d;
                    }
                }
            }
            (-(second.sqrt() - first.sqrt()).abs() * 55.).exp()
        }
        NodeStyle::BrushedTitanium => {
            0.5 + 0.22 * (TAU * v * 128.).sin() + 0.12 * (TAU * (v * 61. + u * 2.)).sin()
        }
        NodeStyle::Hologram => 0.5 + 0.5 * (TAU * v * 32.).sin(),
        NodeStyle::FracturedCrystal => {
            (TAU * (u + v) * 8.).sin().abs() * (TAU * (u - v) * 8.).sin().abs()
        }
        NodeStyle::Microcircuit => {
            let x = fract(u * 10.);
            let y = fract(v * 10.);
            if ((x - 0.5).abs() < 0.045 && y > 0.18)
                || ((y - 0.25).abs() < 0.045 && x > 0.5)
                || (x - 0.5).hypot(y - 0.8) < 0.095
            {
                1.
            } else {
                0.
            }
        }
        NodeStyle::PhotonSkin => {
            0.5 + 0.25 * (TAU * u * 3. + (TAU * v * 4.).sin()).sin() + 0.25 * (TAU * v * 5.).cos()
        }
        _ => 0.5 + 0.04 * (TAU * u * 48.).sin() * (TAU * v * 48.).sin(),
    }
}
fn byte(x: f64) -> u8 {
    (x.clamp(0., 1.) * 255.).round() as u8
}
pub fn bake(style: NodeStyle, size: u32, frequency: f32) -> Option<PackedSurface> {
    if style == NodeStyle::Original
        || size == 0
        || size > 4096
        || !frequency.is_finite()
        || !(0. ..=3.).contains(&frequency)
    {
        return None;
    }
    let size = size as usize;
    let count = size.checked_mul(size)?;
    let f = (frequency as f64 * 2.).round().max(1.);
    let mut heights = Vec::new();
    heights.try_reserve_exact(count).ok()?;
    for y in 0..size {
        for x in 0..size {
            heights.push(height(
                style,
                (x as f64 + 0.5) / size as f64,
                (y as f64 + 0.5) / size as f64,
                f,
            ));
        }
    }
    let bytes = count.checked_mul(4)?;
    let mut albedo = Vec::new();
    albedo.try_reserve_exact(bytes).ok()?;
    albedo.resize(bytes, 0);
    let mut surface = Vec::new();
    surface.try_reserve_exact(bytes).ok()?;
    surface.resize(bytes, 0);
    let sample = |x: usize, y: usize| heights[(y % size) * size + x % size];
    for y in 0..size {
        for x in 0..size {
            let h = sample(x, y);
            let (base, rough) = match style {
                NodeStyle::Obsidian => (0.13 + 0.45 * h, 0.18 + 0.27 * (1. - h)),
                NodeStyle::BrushedTitanium => (0.69 + 0.24 * h, 0.4 + 0.22 * h),
                NodeStyle::Microcircuit => (0.26 + 0.7 * h, 0.7 - 0.4 * h),
                NodeStyle::MatteCeramic => (0.94 + 0.03 * h, 0.95),
                NodeStyle::FracturedCrystal => (0.58 + 0.4 * h, 0.12 + 0.2 * h),
                _ => (0.76 + 0.2 * h, 0.4 + 0.25 * h),
            };
            let dx = (sample(x + size - 1, y) - sample(x + 1, y)) * 0.8;
            let dy = (sample(x, y + size - 1) - sample(x, y + 1)) * 0.8;
            let l = (dx * dx + dy * dy + 1.).sqrt();
            let k = (y * size + x) * 4;
            let value = byte(base.clamp(0., 1.).powf(2.2));
            albedo[k..k + 4].copy_from_slice(&[value, value, value, 255]);
            surface[k..k + 4].copy_from_slice(&[
                byte(dx / l * 0.5 + 0.5),
                byte(dy / l * 0.5 + 0.5),
                byte(1. / l * 0.5 + 0.5),
                byte(rough),
            ]);
        }
    }
    Some(PackedSurface { albedo, surface })
}
