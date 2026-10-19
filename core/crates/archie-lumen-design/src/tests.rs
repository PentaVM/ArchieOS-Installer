use super::*;
use policy::*;
use simulation::*;
use std::ffi::CStr;

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn v(x: f32, y: f32, z: f32) -> DesignVec3 {
    DesignVec3 { x, y, z }
}
fn body(x: f32, target: f32) -> DesignBody {
    DesignBody {
        position: v(x, 0., 0.),
        velocity: v(0., 0., 0.),
        anchor: v(target, 0., 0.),
        fixed: 0,
    }
}

#[test]
fn exported_catalog_has_real_flattened_source_profiles() {
    let text = unsafe { CStr::from_ptr(design_presets_json()) }
        .to_str()
        .unwrap();
    assert!(text.starts_with('{'));
    for id in [
        "polar-prisma",
        "polar-obsidiana",
        "polar-titanio",
        "polar-holograma",
        "polar-cristal",
        "polar-circuitos",
        "polar-fotonico",
        "polar-cartografia",
    ] {
        assert!(text.contains(id));
    }
    assert!(text.contains("visualNodeStyle"));
    assert_eq!(NodeStyle::from_id(9), None);
    assert_eq!(NodeStyle::parse("../../shader"), None);
    assert_eq!(NodeStyle::parse("prism_cut"), Some(NodeStyle::PrismCut));
}

#[test]
fn poles_are_spatial_and_finite_at_center() {
    let w = v(-170., 100., 180.);
    let c = v(165., -105., -85.);
    near(pole_coordinate(w, w, c, 200.), 1.);
    near(pole_coordinate(c, w, c, 200.), 0.);
    near(pole_coordinate(v(-2.5, -2.5, 47.5), w, c, 200.), 0.5);
    near(pole_coordinate(w, w, w, 200.), 0.5);
    assert_eq!(bloom_weight(BloomMode::Selection, 0.8, false), 0.);
    assert_eq!(bloom_weight(BloomMode::Polar, 1., false), 1.);
    assert_eq!(bloom_weight(BloomMode::Off, 1., true), 0.);
}

#[test]
fn texture_tiers_have_source_dead_band_and_capabilities() {
    let p = TextureRequest {
        max_resolution: 4096,
        max_capability: 8192,
        quality: Quality::Original,
        projected_size: 3.,
        current: 512,
    };
    assert_eq!(material_texture_size(p), 512);
    assert_eq!(
        material_texture_size(TextureRequest {
            projected_size: 40.,
            ..p
        }),
        1024
    );
    assert_eq!(
        material_texture_size(TextureRequest {
            projected_size: 220.,
            ..p
        }),
        4096
    );
    assert_eq!(
        material_texture_size(TextureRequest {
            projected_size: 18.,
            current: 2048,
            ..p
        }),
        2048
    );
    assert_eq!(
        material_texture_size(TextureRequest {
            projected_size: 220.,
            max_capability: 1024,
            ..p
        }),
        1024
    );
    assert_eq!(
        material_texture_size(TextureRequest {
            projected_size: 220.,
            quality: Quality::Performance,
            ..p
        }),
        1024
    );
    assert_eq!(
        material_texture_size(TextureRequest {
            max_resolution: 0,
            ..p
        }),
        0
    );
    assert_eq!(texture_bytes(4096, 4096, 4, true), Some(89_478_484));
    assert_eq!(texture_bytes(4096, 4096, 4, false), Some(67_108_864));
}

#[test]
fn quality_uses_sustained_samples_and_preserves_content_limit() {
    let mut q = AdaptiveQuality::default();
    for _ in 0..3 {
        assert_eq!(q.sample(20.), None);
    }
    assert_eq!(q.sample(20.), Some(Quality::Performance));
    for _ in 0..7 {
        assert_eq!(q.sample(60.), None);
    }
    assert_eq!(q.sample(60.), Some(Quality::Balanced));
    for _ in 0..4 {
        q.sample(20.);
    }
    q.sample(40.);
    assert_eq!(q.sample(f32::NAN), None);
    let base = Decoration {
        ambient_count: 420,
        shell_density: 1.,
        bloom: true,
        bloom_strength: 0.5,
        pixel_ratio: 1.25,
        texture_resolution: 4096,
        visible_limit: 36000,
    };
    let fast = effective_quality(base, Quality::Performance);
    assert_eq!(fast.visible_limit, 36000);
    assert!(!fast.bloom);
    assert_eq!(fast.ambient_count, 80);
    near(fast.pixel_ratio, 0.8);
    assert_eq!(fast.texture_resolution, 1024);
}

#[test]
fn source_hybrid_parameters_and_near_lod_are_preserved() {
    let p = hybrid_parameters(NodeStyle::PrismCut, 0.035, 0.18, 1.3);
    near(p[0], 0.0567);
    near(p[1], 0.022);
    near(p[2], 0.1296);
    near(p[3], 1.3);
    assert_eq!(hybrid_lod(0.), [0., 0.]);
    assert_eq!(hybrid_lod(112.), [1., 1.]);
    assert_eq!(hybrid_state(true, false, false), 1.);
    near(hybrid_state(false, true, false), 0.78);
    near(hybrid_state(false, false, true), 0.55);
}

#[test]
fn source_surface_bake_is_linear_repeatable_and_has_packed_normals() {
    let mut signatures = std::collections::BTreeSet::new();
    for id in 1..=8 {
        let style = NodeStyle::from_id(id).unwrap();
        let a = surface::bake(style, 32, 1.).unwrap();
        let b = surface::bake(style, 32, 1.).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.albedo.len(), 4096);
        assert!(a.surface.chunks_exact(4).all(|p| p[2] >= 128));
        signatures.insert(a.albedo);
    }
    assert_eq!(signatures.len(), 8);
    let ceramic = surface::bake(NodeStyle::MatteCeramic, 32, 1.).unwrap();
    // At UV0 the authored matte height is 0.5: albedo=(0.94+0.03*0.5)^2.2.
    assert_eq!(&ceramic.albedo[..4], &[230, 230, 230, 255]);
    assert_eq!(ceramic.surface[3], 242);
    near(design_height(4, 0., 0., 2.), 0.5);
    assert!(surface::bake(NodeStyle::PrismCut, 0, 1.).is_none());
}

#[test]
fn ffi_surface_rejects_short_null_and_overlapping_buffers() {
    let mut a = vec![0; 16];
    let mut s = vec![0; 16];
    unsafe {
        assert!(!design_surface(
            1,
            2,
            1.,
            std::ptr::null_mut(),
            s.as_mut_ptr(),
            16
        ));
        assert!(!design_surface(
            1,
            2,
            1.,
            a.as_mut_ptr(),
            s.as_mut_ptr(),
            15
        ));
        assert!(!design_surface(
            1,
            2,
            1.,
            a.as_mut_ptr(),
            a.as_mut_ptr(),
            16
        ));
        assert!(!design_surface(
            99,
            2,
            1.,
            a.as_mut_ptr(),
            s.as_mut_ptr(),
            16
        ));
        assert!(design_surface(1, 2, 1., a.as_mut_ptr(), s.as_mut_ptr(), 16));
    }
    assert!(a.iter().any(|&b| b != 0));
}

#[test]
fn simulation_reports_small_position_changes_and_exact_final_rest() {
    let mut b = [body(0., 1.)];
    let m = DesignMotion {
        spring: 0.0001,
        ..DesignMotion::default()
    };
    assert_eq!(
        integrate(&mut b, None, 1. / 60., m) & POSITIONS_CHANGED,
        POSITIONS_CHANGED
    );
    assert!(b[0].position.x > 0.);
    let mut b = [body(-15., 10.)];
    let m = DesignMotion::default();
    for _ in 0..3000 {
        integrate(&mut b, None, 1. / 60., m);
    }
    assert_eq!(b[0].position, v(10., 0., 0.));
    assert_eq!(b[0].velocity, v(0., 0., 0.));
    assert_eq!(integrate(&mut b, None, 1. / 60., m), 0);
}

#[test]
fn simulation_cannot_sleep_at_a_turning_point_or_zero_delta() {
    let mut b = [body(1., 2.)];
    b[0].velocity.x = -23. / 60.;
    let result = integrate(&mut b, None, 1. / 60., DesignMotion::default());
    assert_eq!(result & SIMULATION_ACTIVE, SIMULATION_ACTIVE);
    let mut b = [body(0., 10.)];
    let before = b;
    assert_eq!(
        integrate(&mut b, None, 0., DesignMotion::default()),
        SIMULATION_ACTIVE
    );
    assert_eq!(before, b);
}

#[test]
fn free_motion_preserves_weak_damping_and_fixed_root() {
    let mut b = [body(2., 10.)];
    b[0].velocity.x = 0.00001;
    let m = DesignMotion {
        spring: 0.,
        damping: 1.,
        ..DesignMotion::default()
    };
    assert_eq!(
        integrate(&mut b, None, 1. / 60., m) & SIMULATION_ACTIVE,
        SIMULATION_ACTIVE
    );
    near(b[0].velocity.x, 0.00001);
    b[0].fixed = 1;
    integrate(&mut b, Some(v(10., 0., 0.)), 1. / 60., m);
    assert_eq!(b[0].position, b[0].anchor);
    assert_eq!(b[0].velocity, v(0., 0., 0.));
}

#[test]
fn simulation_sleeps_and_wakes_on_input_or_revision() {
    let mut b = [body(10., 10.)];
    let mut sim = Simulation::default();
    let m = DesignMotion::default();
    assert_eq!(sim.step(&mut b, None, 1. / 60., m, 0), 0);
    assert!(sim.sleeping());
    assert_eq!(
        sim.step(&mut b, Some(v(10., 0., 0.)), 1. / 60., m, 0) & SIMULATION_ACTIVE,
        SIMULATION_ACTIVE
    );
    assert!(!sim.sleeping());
    b[0] = body(10., 10.);
    sim.step(&mut b, None, 1. / 60., m, 1);
    assert!(sim.sleeping());
    b[0].anchor.x = 12.;
    assert_ne!(sim.step(&mut b, None, 1. / 60., m, 2), 0);
}

#[test]
fn ffi_step_rejects_invalid_input_without_partial_mutation() {
    let mut b = [body(0., 10.)];
    let before = b;
    let m = DesignMotion::default();
    unsafe {
        assert_eq!(
            design_step(b.as_mut_ptr(), 1, std::ptr::null(), f32::NAN, &m),
            0
        );
        assert_eq!(
            design_step(std::ptr::null_mut(), 1, std::ptr::null(), 0.02, &m),
            0
        );
        assert_ne!(
            design_step(b.as_mut_ptr(), 1, std::ptr::null(), 0.02, &m),
            0
        );
    }
    assert_ne!(before, b);
}

#[test]
fn continuity_is_root_scoped_and_bounded() {
    let mut memory = SpatialMemory::new(2);
    let mut old = body(20., 10.);
    old.velocity.x = 0.3;
    memory.remember(
        "root-a",
        [("one", old), ("two", body(3., 4.)), ("three", body(5., 6.))],
    );
    assert!(memory.restore("root-a", "one", 10.).is_none());
    let got = memory.restore("root-a", "three", 2.).unwrap();
    near(got.position.x, 2.1);
    assert!(memory.restore("root-b", "three", 2.).is_none());
}

#[test]
fn delivered_geometry_forms_have_uvs_normals_and_dimensions() {
    for id in 1..=7 {
        let m = mesh::geometry(NodeStyle::from_id(id).unwrap(), false).unwrap();
        assert_eq!(m.vertices.len() % 8, 0);
        assert_eq!(m.indices.len() % 3, 0);
        assert!(m
            .indices
            .iter()
            .all(|&i| (i as usize) < m.vertices.len() / 8));
        for vertex in m.vertices.chunks_exact(8) {
            assert!(vertex.iter().all(|x| x.is_finite()));
            near(v(vertex[3], vertex[4], vertex[5]).length(), 1.);
        }
    }
    assert!(mesh::geometry(NodeStyle::Original, false).is_none());
    assert!(mesh::geometry(NodeStyle::MatteCeramic, false).is_none());
    let crystal = mesh::geometry(NodeStyle::FracturedCrystal, false).unwrap();
    near(
        crystal
            .vertices
            .chunks_exact(8)
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max),
        0.95,
    );
    let box_file = mesh::geometry(NodeStyle::Microcircuit, false).unwrap();
    let box_dir = mesh::geometry(NodeStyle::Microcircuit, true).unwrap();
    near(
        box_file
            .vertices
            .chunks_exact(8)
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max),
        0.575,
    );
    near(
        box_dir
            .vertices
            .chunks_exact(8)
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max),
        0.675,
    );
}

#[test]
fn typed_setting_validation_and_invalidation_do_not_rebuild_anchors_for_color() {
    let s = VisualSettings {
        pole_color_mix: 42.,
        line_width_px: -1.,
        detail_frequency: f32::NAN,
        ..VisualSettings::default()
    }
    .validated();
    assert_eq!(s.pole_color_mix, 1.);
    assert_eq!(s.line_width_px, 0.5);
    assert_eq!(s.detail_frequency, 1.);
    assert_eq!(change_kind(["bloomStrength"]), ChangeKind::Uniform);
    assert_eq!(change_kind(["visualNodeStyle"]), ChangeKind::Material);
    assert_eq!(change_kind(["visualLinkStyle"]), ChangeKind::Links);
    assert_eq!(
        change_kind(["shellDensity", "visualLinkStyle"]),
        ChangeKind::Structure
    );
    assert_eq!(original_texture_resolution(4096, 2048, 4.), 1024);
    assert_eq!(visual_texture_resolution(4096, 512, Some(1.)), 1024);
    assert_eq!(bloom_mask_scale(Quality::Balanced), 0.375);
}

#[test]
fn extreme_finite_ffi_inputs_are_rejected_without_nan_or_mutation() {
    let mut bodies = [body(f32::MAX, 10.)];
    let before = bodies;
    unsafe {
        assert_eq!(
            design_step(
                bodies.as_mut_ptr(),
                1,
                std::ptr::null(),
                0.02,
                &DesignMotion::default()
            ),
            0
        );
    }
    assert_eq!(before, bodies);
}

#[test]
fn continuity_recovers_each_velocity_component_independently() {
    let mut old = body(2., 99.);
    old.velocity = v(0.3, f32::NAN, 0.7);
    let mut memory = SpatialMemory::new(4);
    memory.remember("root", [("id", old)]);
    let restored = memory.restore("root", "id", 20.).unwrap();
    assert_eq!(restored.velocity, v(0.3, 0., 0.7));
}

#[test]
fn mesh_ffi_query_fill_and_short_buffers_are_consistent() {
    let (mut nv, mut ni) = (0, 0);
    unsafe {
        assert!(design_mesh(
            1,
            false,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut nv,
            &mut ni
        ));
        assert_eq!(nv, 42);
        assert_eq!(ni, 180);
        let mut vertices = vec![0.; nv * 8];
        let mut indices = vec![0; ni];
        assert!(!design_mesh(
            1,
            false,
            vertices.as_mut_ptr(),
            nv * 8 - 1,
            indices.as_mut_ptr(),
            ni,
            &mut nv,
            &mut ni
        ));
        assert!(design_mesh(
            1,
            false,
            vertices.as_mut_ptr(),
            nv * 8,
            indices.as_mut_ptr(),
            ni,
            &mut nv,
            &mut ni
        ));
        assert!(vertices.iter().all(|x| x.is_finite()));
        assert!(!design_mesh(
            0,
            false,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut nv,
            &mut ni
        ));
    }
}

#[test]
fn ffi_material_policy_respects_capabilities_and_rejects_invalid_input() {
    assert_eq!(design_texture_size(4096, 1024, 0, 220., 512), 1024);
    assert_eq!(design_texture_size(4096, 8192, 1, 220., 512), 2048);
    assert_eq!(design_texture_size(4096, 8192, 2, 220., 512), 1024);
    assert_eq!(design_texture_size(4096, 8192, 99, 220., 512), 0);
    let mut out = [0.; 4];
    unsafe {
        assert!(design_hybrid(1, 0.035, 0.18, 1.3, out.as_mut_ptr(), 4));
        near(out[0], 0.0567);
        assert!(!design_hybrid(1, f32::NAN, 0.18, 1.3, out.as_mut_ptr(), 4));
    }
}

#[test]
fn springless_free_motion_and_constrained_targets_reach_source_rest() {
    let mut bodies = [body(2., 10.)];
    bodies[0].velocity.x = 1.;
    let motion = DesignMotion {
        spring: 0.,
        ..DesignMotion::default()
    };
    let mut flags = 3;
    for _ in 0..4000 {
        flags = integrate(&mut bodies, None, 1. / 60., motion);
        if flags == 0 {
            break;
        }
    }
    assert_eq!(flags, 0);
    assert!(bodies[0].position.x > 2. && bodies[0].position.x < 3.);
    assert_eq!(bodies[0].velocity, DesignVec3::default());
    let mut bodies = [body(0., 20.)];
    let motion = DesignMotion {
        radius: 5.,
        ..DesignMotion::default()
    };
    for _ in 0..3000 {
        integrate(&mut bodies, None, 1. / 60., motion);
    }
    assert_eq!(bodies[0].position, v(5., 0., 0.));
    assert_eq!(bodies[0].velocity, DesignVec3::default());
    let mut bodies = [body(2., 10.)];
    let motion = DesignMotion {
        damping: 0.,
        ..DesignMotion::default()
    };
    assert_eq!(integrate(&mut bodies, None, 1. / 60., motion), 0);
    assert_eq!(bodies[0].position.x, 2.);
}
