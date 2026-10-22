//! Bounded C ABI. Caller owns valid aligned live allocations; Rust never retains them.
use crate::{catalog, mesh, policy, surface, DesignBody, DesignMotion, DesignVec3, NodeStyle};
use std::{
    ffi::c_char,
    mem::{align_of, size_of},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr, slice,
};

#[repr(C)]
pub struct DesignShellInput {
    pub kind: u32,
    pub nodes: *const crate::shell::ShellNode,
    pub count: usize,
    pub radius: f32,
    pub density: f32,
    pub scale: f32,
    pub warm: DesignVec3,
    pub cool: DesignVec3,
    pub tint: DesignVec3,
}
/// # Safety
/// Input and count outputs must be valid aligned allocations. Optional fill buffers
/// are distinct, writable allocations with the declared capacities; none alias input.
#[no_mangle]
pub unsafe extern "C" fn design_shell(
    input: *const DesignShellInput,
    vertices: *mut f32,
    vertex_capacity: usize,
    indices: *mut u32,
    index_capacity: usize,
    nv: *mut usize,
    ni: *mut usize,
    radius: *mut f32,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if !aligned(input) || !aligned(nv) || !aligned(ni) || !aligned(radius) {
            return false;
        }
        let p = unsafe { &*input };
        if p.count > 420 || (p.count > 0 && !aligned(p.nodes)) {
            return false;
        }
        let nodes = if p.count == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(p.nodes, p.count) }
        };
        let Some(shell) = crate::shell::build(
            p.kind, nodes, p.radius, p.density, p.scale, p.warm, p.cool, p.tint,
        ) else {
            return false;
        };
        unsafe {
            nv.write(shell.vertices.len() / 10);
            ni.write(shell.indices.len());
            radius.write(shell.radius);
        }
        if vertices.is_null() && indices.is_null() && vertex_capacity == 0 && index_capacity == 0 {
            return true;
        }
        if !aligned(vertices)
            || !aligned(indices)
            || vertex_capacity < shell.vertices.len()
            || index_capacity < shell.indices.len()
        {
            return false;
        }
        unsafe {
            ptr::copy_nonoverlapping(shell.vertices.as_ptr(), vertices, shell.vertices.len());
            ptr::copy_nonoverlapping(shell.indices.as_ptr(), indices, shell.indices.len());
        }
        true
    }))
    .unwrap_or(false)
}
#[repr(C)]
pub struct DesignContentOptions {
    pub limit: u32,
    pub files_per_folder: u32,
    pub category_pull: f32,
    pub hierarchy_pull: f32,
    pub radial_layers: f32,
    pub size_influence: f32,
    pub scale: f32,
}
/// Lists the folder `root` (UTF-8, `root_len` bytes, no NUL needed) read-only and writes
/// its laid-out scene JSON to `out`. Returns the bytes written, or 0 when the folder
/// cannot be listed or the scene does not fit in `capacity`.
///
/// # Safety
/// `root` points to `root_len` readable bytes, `options` to a valid aligned value and
/// `out` to `capacity` writable bytes that overlap neither.
#[no_mangle]
pub unsafe extern "C" fn design_content_scene(
    root: *const u8,
    root_len: usize,
    options: *const DesignContentOptions,
    generated_at_ms: i64,
    out: *mut u8,
    capacity: usize,
) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        if root.is_null() || root_len == 0 || root_len > 4096 || !aligned(options) || out.is_null()
        {
            return 0;
        }
        let Ok(path) = std::str::from_utf8(unsafe { slice::from_raw_parts(root, root_len) }) else {
            return 0;
        };
        let o = unsafe { options.read() };
        let options = crate::content::ContentOptions {
            limit: o.limit as usize,
            files_per_folder: o.files_per_folder as usize,
            category_pull: f64::from(o.category_pull),
            hierarchy_pull: f64::from(o.hierarchy_pull),
            radial_layers: f64::from(o.radial_layers),
            size_influence: f64::from(o.size_influence),
            scale: f64::from(o.scale),
        };
        let Some(field) = crate::content::scan(std::path::Path::new(path), options) else {
            return 0;
        };
        let json = crate::content::scene_json(&field, generated_at_ms);
        if json.len() > capacity {
            return 0;
        }
        unsafe { ptr::copy_nonoverlapping(json.as_ptr(), out, json.len()) };
        json.len()
    }))
    .unwrap_or(0)
}
fn aligned<T>(p: *const T) -> bool {
    !p.is_null() && (p as usize) % align_of::<T>() == 0
}
fn range(p: usize, len: usize) -> Option<(usize, usize)> {
    Some((p, p.checked_add(len)?))
}
fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}
#[no_mangle]
pub extern "C" fn design_presets_json() -> *const c_char {
    catalog::CATALOG_C.as_ptr().cast()
}
#[no_mangle]
pub extern "C" fn design_height(style: u32, u: f32, v: f32, f: f32) -> f32 {
    if !u.is_finite()
        || !v.is_finite()
        || !f.is_finite()
        || u.abs() > 1e6
        || v.abs() > 1e6
        || !(0. ..=6.).contains(&f)
    {
        return 0.;
    }
    NodeStyle::from_id(style).map_or(0., |s| {
        surface::height(s, u as f64, v as f64, f as f64) as f32
    })
}
/// # Safety
/// albedo and surface must each have buffer_len writable bytes and not overlap.
#[no_mangle]
pub unsafe extern "C" fn design_surface(
    style: u32,
    size: u32,
    frequency: f32,
    albedo: *mut u8,
    packed: *mut u8,
    buffer_len: usize,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(style) = NodeStyle::from_id(style) else {
            return false;
        };
        let Some(bytes) = (size as usize)
            .checked_mul(size as usize)
            .and_then(|n| n.checked_mul(4))
        else {
            return false;
        };
        if bytes == 0 || buffer_len < bytes || !aligned(albedo) || !aligned(packed) {
            return false;
        }
        let (Some(a), Some(s)) = (range(albedo as usize, bytes), range(packed as usize, bytes))
        else {
            return false;
        };
        if overlaps(a, s) {
            return false;
        }
        let Some(pack) = surface::bake(style, size, frequency) else {
            return false;
        };
        unsafe {
            ptr::copy_nonoverlapping(pack.albedo.as_ptr(), albedo, bytes);
            ptr::copy_nonoverlapping(pack.surface.as_ptr(), packed, bytes);
        }
        true
    }))
    .unwrap_or(false)
}
/// # Safety
/// bodies points to count writable DesignBody values; optional pointer and motion
/// point to valid aligned initialized values. Inputs are copied before mutation.
#[no_mangle]
pub unsafe extern "C" fn design_step(
    bodies: *mut DesignBody,
    count: usize,
    pointer: *const DesignVec3,
    dt: f32,
    motion: *const DesignMotion,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        if count == 0
            || count > crate::simulation::MAX_BODIES
            || !aligned(bodies)
            || !aligned(motion)
            || (!pointer.is_null() && !aligned(pointer))
        {
            return 0;
        }
        if range(bodies as usize, count * size_of::<DesignBody>()).is_none() {
            return 0;
        }
        let motion = unsafe { motion.read() };
        let pointer = if pointer.is_null() {
            None
        } else {
            Some(unsafe { pointer.read() })
        };
        crate::simulation::integrate(
            unsafe { slice::from_raw_parts_mut(bodies, count) },
            pointer,
            dt,
            motion,
        )
    }))
    .unwrap_or(0)
}
/// # Safety
/// Output count pointers are valid, writable, distinct and aligned. For fill mode,
/// vertices/indices each refer to writable, nonoverlapping allocations of capacities.
#[no_mangle]
pub unsafe extern "C" fn design_mesh(
    style: u32,
    folder: bool,
    vertices: *mut f32,
    vertex_capacity: usize,
    indices: *mut u32,
    index_capacity: usize,
    out_vertex_count: *mut usize,
    out_index_count: *mut usize,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if !aligned(out_vertex_count)
            || !aligned(out_index_count)
            || out_vertex_count == out_index_count
        {
            return false;
        }
        let Some(style) = NodeStyle::from_id(style) else {
            return false;
        };
        let Some(mesh) = mesh::geometry(style, folder) else {
            return false;
        };
        unsafe {
            out_vertex_count.write(mesh.vertices.len() / 8);
            out_index_count.write(mesh.indices.len());
        }
        if vertices.is_null() && indices.is_null() && vertex_capacity == 0 && index_capacity == 0 {
            return true;
        }
        if !aligned(vertices)
            || !aligned(indices)
            || vertex_capacity < mesh.vertices.len()
            || index_capacity < mesh.indices.len()
        {
            return false;
        }
        let (Some(a), Some(b)) = (
            range(vertices as usize, mesh.vertices.len() * 4),
            range(indices as usize, mesh.indices.len() * 4),
        ) else {
            return false;
        };
        if overlaps(a, b) {
            return false;
        }
        unsafe {
            ptr::copy_nonoverlapping(mesh.vertices.as_ptr(), vertices, mesh.vertices.len());
            ptr::copy_nonoverlapping(mesh.indices.as_ptr(), indices, mesh.indices.len());
        }
        true
    }))
    .unwrap_or(false)
}
#[no_mangle]
pub extern "C" fn design_texture_size(
    max_resolution: u32,
    max_capability: u32,
    quality: u32,
    projected_size: f32,
    current: u32,
) -> u32 {
    if !projected_size.is_finite() {
        return 0;
    }
    let quality = match quality {
        0 => policy::Quality::Original,
        1 => policy::Quality::Balanced,
        2 => policy::Quality::Performance,
        _ => return 0,
    };
    policy::material_texture_size(policy::TextureRequest {
        max_resolution,
        max_capability,
        quality,
        projected_size,
        current,
    })
}
/// # Safety
/// output points to len writable f32 values.
#[no_mangle]
pub unsafe extern "C" fn design_hybrid(
    style: u32,
    depth: f32,
    emissive: f32,
    contrast: f32,
    output: *mut f32,
    len: usize,
) -> bool {
    if !aligned(output)
        || len < 4
        || ![depth, emissive, contrast]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e6)
    {
        return false;
    }
    let Some(style) = NodeStyle::from_id(style) else {
        return false;
    };
    let p = policy::hybrid_parameters(style, depth, emissive, contrast);
    unsafe {
        ptr::copy_nonoverlapping(p.as_ptr(), output, 4);
    }
    true
}
