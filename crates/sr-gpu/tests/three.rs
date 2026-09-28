//! The 3D renderer on the GPU: shading, orientation, shadows, transmission,
//! splats, depth of field and dome lighting.

mod common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::three::*;

const W: u32 = 128;

fn engine() -> Option<ThreeEngine> {
    let g = common::gpu()?;
    Some(ThreeEngine::new(g.device.clone(), g.queue.clone()))
}

fn scene(draws: Vec<Draw3>, lights: Vec<Light3>) -> Scene3 {
    Scene3 {
        // these renderer-level tests were laid out for a 39.6° lens; the document default is now 60°
        cam: resolve(&CameraParams { fov: 39.6, ..Default::default() }, W as f32, W as f32),
        clip_fix: Mat4::IDENTITY,
        size: [W, W],
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws,
        lights,
        env: None,
        splats: Vec::new(),
        encode_srgb: false,
    }
}

fn draw(eng: &ThreeEngine, p: &sr_3d::Primitive, at: Vec3, mat: MaterialParams) -> Draw3 {
    Draw3 {
        mesh: MeshSrc::Cached(eng.upload_mesh(&p.vertices, &p.indices)),
        model: Mat4::from_translation(at),
        material: mat,
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
        instances: 1,
    }
}

fn sun(dir: Vec3, lux: f32, shadow: bool) -> Light3 {
    Light3 {
        kind: LightKind::Directional,
        pos: Vec3::ZERO,
        dir: dir.normalize(),
        right: Vec3::X,
        color: Vec3::splat(lux),
        range: 0.0,
        falloff: 2.0,
        cos_outer: 0.0,
        cos_inner: 0.0,
        cast_shadow: shadow,
        softness: 1.0,
        bias: 0.0005,
        map_size: 1024,
        size: [0.0; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
    }
}

fn at(px: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    px[(y * W + x) as usize]
}

fn lum(p: [f32; 4]) -> f32 {
    p[0] + p[1] + p[2]
}

#[test]
fn sphere_faces_outward_and_is_lit_from_the_light() {
    let Some(mut eng) = engine() else { return };
    let sphere = prim::sphere(30.0, 48);
    let mat = MaterialParams { roughness: 0.8, ..Default::default() };
    // light travelling +x: it comes from the left
    let s = scene(
        vec![draw(&eng, &sphere, Vec3::new(64.0, 64.0, 0.0), mat)],
        vec![sun(Vec3::new(1.0, 0.0, 0.3), 3.0, false)],
    );
    let px = eng.render_now(&s, None);
    assert!((at(&px, 64, 64)[3] - 1.0).abs() < 1e-3, "covered: {:?}", at(&px, 64, 64));
    assert_eq!(at(&px, 2, 2)[3], 0.0, "background stays transparent");
    let (left, right) = (lum(at(&px, 44, 64)), lum(at(&px, 84, 64)));
    assert!(left > right * 3.0 + 0.05, "lit from the left: {left} vs {right}");
    assert_eq!(eng.stats.draws, 1);
    assert!(eng.stats.triangles > 2000);
}

#[test]
fn shadows_fall_on_the_plane_behind() {
    let Some(mut eng) = engine() else { return };
    let sphere = prim::sphere(15.0, 32);
    let plane = prim::plane(400.0, 400.0, 1);
    let mat = MaterialParams { roughness: 1.0, ..Default::default() };
    let draws = vec![
        draw(&eng, &sphere, Vec3::new(64.0, 64.0, 0.0), mat.clone()),
        draw(&eng, &plane, Vec3::new(64.0, 64.0, 60.0), mat),
    ];
    // travelling right and away: the shadow lands to the right of the sphere on the plane
    let lit = eng.render_now(&scene(draws, vec![sun(Vec3::new(1.0, 0.0, 1.0), 3.0, true)]), None);
    let shadowed = lum(at(&lit, 64 + 55, 64));
    let open = lum(at(&lit, 64 - 45, 64));
    assert!(open > 0.1 && shadowed < open * 0.4, "shadow {shadowed} vs open {open}");
    assert_eq!(eng.stats.shadow_views, 1);
}

#[test]
fn transmission_refracts_the_backdrop() {
    let Some(mut eng) = engine() else { return };
    let backdrop = eng.upload_f16([W, W], &vec![[1.0, 0.0, 0.0, 1.0]; (W * W) as usize]);
    let glass = MaterialParams { transmission: 1.0, roughness: 0.05, ior: 1.5, thickness: 5.0, ..Default::default() };
    let plate = prim::cuboid(60.0, 60.0, 5.0);
    let px = eng.render_now(
        &scene(
            vec![draw(&eng, &plate, Vec3::new(64.0, 64.0, 0.0), glass)],
            vec![sun(Vec3::new(1.0, 0.0, 1.0), 1.0, false)],
        ),
        Some(&backdrop.create_view(&Default::default())),
    );
    let c = at(&px, 64, 64);
    assert!(c[0] > 0.5 && c[1] < 0.2 && c[3] > 0.99, "red through glass: {c:?}");
    assert_eq!(eng.stats.transmissive, 1);
}

#[test]
fn splats_sort_back_to_front() {
    let Some(mut eng) = engine() else { return };
    let mk = |z: f32, c: [f32; 4]| sr_3d::Splats {
        pos: vec![[64.0, 64.0, z]],
        scale: vec![[12.0, 12.0, 12.0]],
        rot: vec![[0.0, 0.0, 0.0, 1.0]],
        color: vec![c],
        basis: Mat4::IDENTITY,
    };
    let mut s = mk(0.0, [1.0, 0.0, 0.0, 0.95]);
    let far = mk(50.0, [0.0, 0.0, 1.0, 0.95]);
    // blue (far) first in memory: the sort must still draw it first
    s.pos.insert(0, far.pos[0]);
    s.scale.insert(0, far.scale[0]);
    s.rot.insert(0, far.rot[0]);
    s.color.insert(0, far.color[0]);
    let gpu = eng.upload_splats(&s);
    let mut sc = scene(Vec::new(), Vec::new());
    sc.splats.push(SplatDraw { gpu, model: Mat4::IDENTITY, opacity: 1.0 });
    let px = eng.render_now(&sc, None);
    let c = at(&px, 64, 64);
    assert!(c[0] > 0.8 && c[2] < 0.15, "red in front: {c:?}");
    assert_eq!(at(&px, 1, 1)[3], 0.0);
    // many splats: the radix sort orders 2000 random depths
    let n = 2000;
    let mut big = sr_3d::Splats::default();
    for i in 0..n {
        let z = ((i * 7919) % n) as f32;
        big.pos.push([64.0, 64.0, z]);
        big.scale.push([3.0; 3]);
        big.rot.push([0.0, 0.0, 0.0, 1.0]);
        big.color.push(if z < 1.0 { [0.0, 1.0, 0.0, 1.0] } else { [1.0, 0.0, 1.0, 0.5] });
    }
    let gpu = eng.upload_splats(&big);
    let mut sc = scene(Vec::new(), Vec::new());
    sc.splats.push(SplatDraw { gpu, model: Mat4::IDENTITY, opacity: 1.0 });
    let c = at(&eng.render_now(&sc, None), 64, 64);
    assert!(c[1] > 0.9, "the nearest (green, opaque) splat ends on top: {c:?}");
}

#[test]
fn depth_of_field_blurs_out_of_focus() {
    let Some(mut eng) = engine() else { return };
    let sphere = prim::sphere(10.0, 32);
    let mat = MaterialParams { unlit: true, ..Default::default() };
    let mut s = scene(vec![draw(&eng, &sphere, Vec3::new(64.0, 64.0, 400.0), mat)], Vec::new());
    let sharp = eng.render_now(&s, None);
    let edge = |px: &[[f32; 4]]| (0..W).filter(|x| at(px, *x, 64)[3] > 0.02).count();
    s.dof = Some(Dof { coc_scale: 2000.0, focus: 50.0, max_coc: 12.0, blades: 6 });
    let blurred = eng.render_now(&s, None);
    assert!(edge(&blurred) > edge(&sharp) + 6, "{} vs {}", edge(&blurred), edge(&sharp));
    assert!(at(&blurred, 64, 64)[3] < 0.999, "the centre softens too");
}

#[test]
fn dome_lights_and_fills_the_background() {
    let Some(mut eng) = engine() else { return };
    let env = sr_3d::env::Equirect { width: 64, height: 32, rgb: vec![[0.5, 0.5, 0.5]; 64 * 32] };
    let gpu = eng.upload_env(&env);
    let sphere = prim::sphere(30.0, 48);
    let mut s = scene(
        vec![draw(&eng, &sphere, Vec3::new(64.0, 64.0, 0.0), MaterialParams { roughness: 1.0, ..Default::default() })],
        Vec::new(),
    );
    s.env = Some(Env3 { env: gpu, intensity: 1.0, rotation: 0.0, visible: true });
    let px = eng.render_now(&s, None);
    let bg = at(&px, 2, 2);
    assert!((bg[0] - 0.5).abs() < 0.02 && bg[3] == 1.0, "{bg:?}");
    let c = at(&px, 64, 64);
    assert!((c[0] - 0.5).abs() < 0.12, "a white diffuse sphere in a uniform 0.5 dome reflects ≈0.5: {c:?}");
}
