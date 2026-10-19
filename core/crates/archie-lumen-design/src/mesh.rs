//! Rust geometry for the explicit shapes in XaviCottier materials.js:67.
//! No replacements for original/category fallback meshes (styles 0 and 8).
use crate::{DesignVec3 as V, NodeStyle};
use std::f32::consts::{PI, TAU};
#[derive(Debug)]
pub struct Mesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}
impl Mesh {
    fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }
    fn vertex(&mut self, p: V, n: V, uv: [f32; 2]) -> u32 {
        let i = (self.vertices.len() / 8) as u32;
        self.vertices
            .extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, uv[0], uv[1]]);
        i
    }
    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }
}
fn v(x: f32, y: f32, z: f32) -> V {
    V { x, y, z }
}
pub fn geometry(style: NodeStyle, folder: bool) -> Option<Mesh> {
    Some(match style {
        NodeStyle::Original | NodeStyle::MatteCeramic => return None,
        NodeStyle::PrismCut => lathe(
            &[
                (0., -0.9),
                (0.7, -0.9),
                (0.86, -0.76),
                (0.86, 0.76),
                (0.7, 0.9),
                (0., 0.9),
            ],
            6,
        ),
        NodeStyle::Obsidian => polyhedron(true, 0),
        NodeStyle::BrushedTitanium => cylinder(0.78, 0.78, if folder { 1.4 } else { 1.1 }, 8),
        NodeStyle::Hologram => polyhedron(false, 1),
        NodeStyle::FracturedCrystal => cylinder(0.24, 0.85, 1.9, 5),
        NodeStyle::Microcircuit => cuboid(if folder { 1.35 } else { 1.15 }, 0.72, 1.1),
        NodeStyle::PhotonSkin => sphere(12, 8),
    })
}
fn lathe(points: &[(f32, f32)], segments: usize) -> Mesh {
    let mut mesh = Mesh::new();
    let len = points.len();
    let mut normals = Vec::new();
    for j in 0..len {
        let slope = |a: usize, b: usize| {
            v(points[b].1 - points[a].1, -(points[b].0 - points[a].0), 0.).unit()
        };
        normals.push(if j == 0 {
            slope(0, 1)
        } else if j == len - 1 {
            slope(j - 1, j)
        } else {
            (slope(j - 1, j) + slope(j, j + 1)).unit()
        });
    }
    for i in 0..=segments {
        let u = i as f32 / segments as f32;
        let phi = u * TAU;
        for (j, &(r, y)) in points.iter().enumerate() {
            let n = normals[j];
            mesh.vertex(
                v(r * phi.sin(), y, r * phi.cos()),
                v(n.x * phi.sin(), n.y, n.x * phi.cos()),
                [u, j as f32 / (len - 1) as f32],
            );
        }
    }
    for i in 0..segments {
        for j in 0..len - 1 {
            let a = (i * len + j) as u32;
            let b = a + len as u32;
            mesh.triangle(a, b, a + 1);
            mesh.triangle(b, b + 1, a + 1);
        }
    }
    mesh
}
fn cylinder(top: f32, bottom: f32, height: f32, segments: usize) -> Mesh {
    let mut mesh = Mesh::new();
    let slope = (bottom - top) / height;
    for y in 0..=1 {
        let fraction = y as f32;
        let radius = top + (bottom - top) * fraction;
        for x in 0..=segments {
            let u = x as f32 / segments as f32;
            let theta = u * TAU;
            mesh.vertex(
                v(
                    radius * theta.sin(),
                    height * (0.5 - fraction),
                    radius * theta.cos(),
                ),
                v(theta.sin(), slope, theta.cos()).unit(),
                [u, 1. - fraction],
            );
        }
    }
    for x in 0..segments {
        let a = x as u32;
        let b = a + (segments + 1) as u32;
        mesh.triangle(a, b, a + 1);
        mesh.triangle(b, b + 1, a + 1);
    }
    for upper in [true, false] {
        let sign = if upper { 1. } else { -1. };
        let radius = if upper { top } else { bottom };
        let yy = sign * height * 0.5;
        let center = mesh.vertex(v(0., yy, 0.), v(0., sign, 0.), [0.5, 0.5]);
        let first = (mesh.vertices.len() / 8) as u32;
        for x in 0..=segments {
            let theta = x as f32 / segments as f32 * TAU;
            mesh.vertex(
                v(radius * theta.sin(), yy, radius * theta.cos()),
                v(0., sign, 0.),
                [theta.sin() * 0.5 + 0.5, theta.cos() * 0.5 * sign + 0.5],
            );
        }
        for x in 0..segments {
            let a = first + x as u32;
            if upper {
                mesh.triangle(a, a + 1, center);
            } else {
                mesh.triangle(a + 1, a, center);
            }
        }
    }
    mesh
}
fn cuboid(width: f32, height: f32, depth: f32) -> Mesh {
    let mut mesh = Mesh::new();
    let x = width * 0.5;
    let y = height * 0.5;
    let z = depth * 0.5;
    let faces = [
        (
            [v(x, -y, z), v(x, -y, -z), v(x, y, -z), v(x, y, z)],
            v(1., 0., 0.),
        ),
        (
            [v(-x, -y, -z), v(-x, -y, z), v(-x, y, z), v(-x, y, -z)],
            v(-1., 0., 0.),
        ),
        (
            [v(-x, y, z), v(x, y, z), v(x, y, -z), v(-x, y, -z)],
            v(0., 1., 0.),
        ),
        (
            [v(-x, -y, -z), v(x, -y, -z), v(x, -y, z), v(-x, -y, z)],
            v(0., -1., 0.),
        ),
        (
            [v(-x, -y, z), v(x, -y, z), v(x, y, z), v(-x, y, z)],
            v(0., 0., 1.),
        ),
        (
            [v(x, -y, -z), v(-x, -y, -z), v(-x, y, -z), v(x, y, -z)],
            v(0., 0., -1.),
        ),
    ];
    for (face, n) in faces {
        let start = (mesh.vertices.len() / 8) as u32;
        for (p, uv) in face
            .into_iter()
            .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
        {
            mesh.vertex(p, n, uv);
        }
        mesh.triangle(start, start + 1, start + 2);
        mesh.triangle(start, start + 2, start + 3);
    }
    mesh
}
fn sphere(width: usize, height: usize) -> Mesh {
    let mut mesh = Mesh::new();
    for iy in 0..=height {
        let vv = iy as f32 / height as f32;
        let theta = vv * PI;
        let offset = if iy == 0 {
            0.5 / width as f32
        } else if iy == height {
            -0.5 / width as f32
        } else {
            0.
        };
        for ix in 0..=width {
            let u = ix as f32 / width as f32;
            let phi = u * TAU;
            let p = v(
                -phi.cos() * theta.sin(),
                theta.cos(),
                phi.sin() * theta.sin(),
            );
            mesh.vertex(p, p.unit(), [u + offset, 1. - vv]);
        }
    }
    for iy in 0..height {
        for ix in 0..width {
            let a = (iy * (width + 1) + ix + 1) as u32;
            let b = a - 1;
            let c = b + (width + 1) as u32;
            let d = c + 1;
            if iy != 0 {
                mesh.triangle(a, b, d);
            }
            if iy != height - 1 {
                mesh.triangle(b, c, d);
            }
        }
    }
    mesh
}
fn polyhedron(ico: bool, detail: usize) -> Mesh {
    let (vertices, faces) = if ico {
        let t = (1. + 5f32.sqrt()) / 2.;
        (
            vec![
                v(-1., t, 0.),
                v(1., t, 0.),
                v(-1., -t, 0.),
                v(1., -t, 0.),
                v(0., -1., t),
                v(0., 1., t),
                v(0., -1., -t),
                v(0., 1., -t),
                v(t, 0., -1.),
                v(t, 0., 1.),
                v(-t, 0., -1.),
                v(-t, 0., 1.),
            ],
            vec![
                [0, 11, 5],
                [0, 5, 1],
                [0, 1, 7],
                [0, 7, 10],
                [0, 10, 11],
                [1, 5, 9],
                [5, 11, 4],
                [11, 10, 2],
                [10, 7, 6],
                [7, 1, 8],
                [3, 9, 4],
                [3, 4, 2],
                [3, 2, 6],
                [3, 6, 8],
                [3, 8, 9],
                [4, 9, 5],
                [2, 4, 11],
                [6, 2, 10],
                [8, 6, 7],
                [9, 8, 1],
            ],
        )
    } else {
        (
            vec![
                v(1., 0., 0.),
                v(-1., 0., 0.),
                v(0., 1., 0.),
                v(0., -1., 0.),
                v(0., 0., 1.),
                v(0., 0., -1.),
            ],
            vec![
                [0, 2, 4],
                [0, 4, 3],
                [0, 3, 5],
                [0, 5, 2],
                [1, 2, 5],
                [1, 5, 3],
                [1, 3, 4],
                [1, 4, 2],
            ],
        )
    };
    let mut mesh = Mesh::new();
    for f in faces {
        let a = vertices[f[0]];
        let b = vertices[f[1]];
        let c = vertices[f[2]];
        let triangles = if detail == 0 {
            vec![[a, b, c]]
        } else {
            let ab = (a + b) * 0.5;
            let bc = (b + c) * 0.5;
            let ca = (c + a) * 0.5;
            vec![[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]
        };
        for triangle in triangles {
            let points = triangle.map(|p| p.unit());
            let normal = (points[1] - points[0]).cross(points[2] - points[0]).unit();
            let mut uv = points.map(|p| {
                [
                    (p.z.atan2(-p.x) / TAU) + 0.5,
                    0.5 - (-p.y).atan2((p.x * p.x + p.z * p.z).sqrt()) / PI,
                ]
            });
            let min = uv.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min);
            let max = uv.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max);
            if max - min > 0.5 {
                for q in &mut uv {
                    if q[0] < 0.5 {
                        q[0] += 1.;
                    }
                }
            }
            let start = (mesh.vertices.len() / 8) as u32;
            for i in 0..3 {
                mesh.vertex(
                    points[i],
                    if detail == 0 { normal } else { points[i] },
                    uv[i],
                );
            }
            mesh.triangle(start, start + 1, start + 2);
        }
    }
    mesh
}
