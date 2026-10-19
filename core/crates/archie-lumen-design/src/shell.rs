//! Delivered scene.js/visuals.js shell geometry, driven by observed OS nodes.
use crate::DesignVec3 as V;
use std::f32::consts::{PI, TAU};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShellNode {
    pub position: V,
    pub color: V,
    pub branch: u32,
}
pub struct Shell {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub radius: f32,
}
fn v(x: f32, y: f32, z: f32) -> V {
    V { x, y, z }
}
fn mix(a: V, b: V, t: f32) -> V {
    a + (b - a) * t
}
impl Shell {
    fn vertex(&mut self, p: V, n: V, c: V, a: f32) -> u32 {
        let i = (self.vertices.len() / 10) as u32;
        self.vertices
            .extend([p.x, p.y, p.z, n.x, n.y, n.z, c.x, c.y, c.z, a]);
        i
    }
    fn tube(&mut self, a: V, b: V, r: f32, c: V, opacity: f32) {
        let axis = (b - a).unit();
        let t = axis
            .cross(if axis.y.abs() < 0.9 {
                v(0., 1., 0.)
            } else {
                v(1., 0., 0.)
            })
            .unit();
        let bt = axis.cross(t).unit();
        for k in 0..5 {
            let n = t * (TAU * k as f32 / 5.).cos() + bt * (TAU * k as f32 / 5.).sin();
            let q = t * (TAU * (k + 1) as f32 / 5.).cos() + bt * (TAU * (k + 1) as f32 / 5.).sin();
            let first = self.vertex(a + n * r, n, c, opacity);
            self.vertex(b + n * r, n, c, opacity);
            self.vertex(a + q * r, q, c, opacity);
            self.vertex(b + q * r, q, c, opacity);
            self.indices
                .extend([first, first + 1, first + 2, first + 2, first + 1, first + 3]);
        }
    }
}
fn seeded(index: u32) -> f32 {
    let mut x = index
        .wrapping_add(1)
        .wrapping_mul(1664525)
        .wrapping_add(1013904223);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}
pub fn build(
    kind: u32,
    nodes: &[ShellNode],
    radius: f32,
    density: f32,
    scale: f32,
    warm: V,
    cool: V,
    tint: V,
) -> Option<Shell> {
    if kind > 3
        || nodes.len() > 420
        || !radius.is_finite()
        || !(0. ..=10000.).contains(&radius)
        || !density.is_finite()
        || !(0. ..=3.).contains(&density)
        || !scale.is_finite()
        || !(0. ..=2.).contains(&scale)
        || ![warm, cool, tint].iter().all(|c| c.finite())
        || nodes
            .iter()
            .any(|n| !n.position.finite() || !n.color.finite() || n.position.length() > 10000.)
    {
        return None;
    }
    let mut distances: Vec<_> = nodes.iter().map(|n| n.position.length()).collect();
    distances.sort_by(f32::total_cmp);
    let occupied = distances
        .get((distances.len() as f32 * 0.96) as usize)
        .copied()
        .unwrap_or(radius * 0.7);
    let radius = radius.min(occupied * 1.15) * scale;
    let mut out = Shell {
        vertices: Vec::new(),
        indices: Vec::new(),
        radius,
    };
    if kind == 0 {
        let candidates: Vec<_> = nodes
            .iter()
            .filter(|n| n.position.length() > 0.01)
            .collect();
        let count = ((135. * density).round() as usize).min(candidates.len());
        for i in 0..count {
            let node = candidates
                [((i as f32 + 0.5) * candidates.len() as f32 / count as f32).floor() as usize];
            let direction = node.position.unit();
            let tangent = direction
                .cross(if direction.y.abs() < 0.86 {
                    v(0., 1., 0.)
                } else {
                    v(1., 0., 0.)
                })
                .unit();
            let bitangent = direction.cross(tangent).unit();
            let width = radius * if node.branch != 0 { 0.021 } else { 0.012 };
            let center = direction * (radius * (1. + (i as f32 * 5.1).sin() * 0.025));
            let points = [(-0.75, -0.55), (0.9, -0.28), (0.04, 1.)]
                .map(|(u, w)| center + tangent * (width * u) + bitangent * (width * w));
            let normal = (points[1] - points[0]).cross(points[2] - points[0]).unit();
            let first = out.vertices.len() as u32 / 10;
            for p in points {
                out.vertex(p, normal, mix(cool, node.color, 0.18), 0.32);
            }
            out.indices.extend([first, first + 1, first + 2]);
        }
    } else if kind == 2 {
        let count = (40. + 60. * density).round() as u32;
        for (width, opacity) in [(0.0048, 0.44 * 2.65), (0.0022, 2.65)] {
            for i in 0..count {
                let y = 1. - 2. * (i as f32 + 0.5) / count as f32;
                let theta = i as f32 * 2.3999631;
                let radial = (1. - y * y).sqrt();
                let direction = v(radial * theta.cos(), y, radial * theta.sin());
                let tangent = direction
                    .cross(if y.abs() < 0.9 {
                        v(0., 1., 0.)
                    } else {
                        v(1., 0., 0.)
                    })
                    .unit();
                let bitangent = direction.cross(tangent).unit();
                let phase = seeded(i * 13 + 9) * TAU;
                let span = 0.12 + seeded(i * 13 + 10) * 0.19;
                let point = |f: f32| {
                    let angle = (f - 0.5) * span;
                    (direction * angle.cos()
                        + tangent * (angle.sin() * phase.cos())
                        + bitangent * (angle.sin() * phase.sin()))
                        * (radius * (1.01 + 0.035 * (i as f32 * 2.3).sin()))
                };
                let color = if i % 2 == 0 {
                    tint
                } else {
                    mix(tint, warm, 0.38)
                };
                for segment in 0..8 {
                    out.tube(
                        point(segment as f32 / 8.),
                        point((segment + 1) as f32 / 8.),
                        radius * width,
                        color,
                        opacity,
                    );
                }
            }
        }
    } else {
        let w = if kind == 3 { 64 } else { 56 };
        let h = 40;
        for iy in 0..=h {
            let theta = iy as f32 / h as f32 * PI;
            for ix in 0..=w {
                let phi = ix as f32 / w as f32 * TAU;
                let n = v(
                    -phi.cos() * theta.sin(),
                    theta.cos(),
                    phi.sin() * theta.sin(),
                );
                let (mut displacement, mut color) = (1., warm);
                if kind == 1 {
                    let (mut density, mut highest) = (0., -1.);
                    for node in nodes.iter().take(320) {
                        let similarity = n.dot(node.position.unit());
                        if similarity > 0.83 {
                            density += ((similarity - 0.83) / 0.17).powi(2);
                        }
                        if similarity > highest {
                            highest = similarity;
                            color = node.color;
                        }
                    }
                    displacement +=
                        (density * 0.004).min(0.11) + (n.x * 19. + n.y * 14.).sin() * 0.009;
                    color = mix(color, warm, 0.46);
                }
                out.vertex(
                    n * (radius * displacement * if kind == 3 { 1.04 } else { 1. }),
                    n,
                    if kind == 3 { tint } else { color },
                    1.,
                );
            }
        }
        for iy in 0..h {
            for ix in 0..w {
                let a = (iy * (w + 1) + ix) as u32;
                let b = a + (w + 1) as u32;
                out.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_shells_are_bounded_repeatable_and_keep_observed_radius() {
        let nodes = [
            ShellNode {
                position: v(50., 0., 0.),
                color: v(0.3, 0.4, 0.5),
                branch: 0,
            },
            ShellNode {
                position: v(0., 70., 0.),
                color: v(0.7, 0.4, 0.2),
                branch: 1,
            },
        ];
        let mut counts = Vec::new();
        for kind in 0..4 {
            let s = build(
                kind,
                &nodes,
                100.,
                1.,
                1.,
                v(1., 0.7, 0.4),
                v(0.3, 0.7, 1.),
                v(0.4, 0.7, 0.8),
            )
            .unwrap();
            assert_eq!(s.radius, 80.5);
            assert!(s.vertices.iter().all(|x| x.is_finite()));
            assert!(s
                .indices
                .iter()
                .all(|i| (*i as usize) < s.vertices.len() / 10));
            assert_eq!(
                s.vertices,
                build(
                    kind,
                    &nodes,
                    100.,
                    1.,
                    1.,
                    v(1., 0.7, 0.4),
                    v(0.3, 0.7, 1.),
                    v(0.4, 0.7, 0.8)
                )
                .unwrap()
                .vertices
            );
            counts.push(s.vertices.len());
        }
        assert_eq!(counts[0], 60);
        assert_eq!(counts[2], 100 * 8 * 2 * 5 * 4 * 10);
        assert_ne!(counts[1], counts[3]);
        assert!(build(
            4,
            &nodes,
            100.,
            1.,
            1.,
            V::default(),
            V::default(),
            V::default()
        )
        .is_none());
        assert!(build(
            0,
            &nodes,
            100.,
            f32::NAN,
            1.,
            V::default(),
            V::default(),
            V::default()
        )
        .is_none());
    }
}
