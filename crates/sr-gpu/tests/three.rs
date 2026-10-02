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
        ao: None,
        ssr: false,
        path: None,
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
        contact: 0.0,
    }
}

fn at(px: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    px[(y * W + x) as usize]
}

fn lum(p: [f32; 4]) -> f32 {
    p[0] + p[1] + p[2]
}

#[test]
fn imported_texture_maps_use_independent_transformed_coordinates() {
    let Some(mut eng) = engine() else { return };
    let mut p = prim::plane(70.0, 70.0, 1);
    for v in &mut p.vertices {
        v.uv = [0.25, 0.5];
    }
    let mut material = sr_3d::ImportedMaterial::default();
    material.params.unlit = false;
    material.params.separate_uvs = true;
    material.params.emissive = [1.0; 3];
    material.texture_transforms[0].offset = [0.5, 0.0];
    // Emissive keeps the original UV, while base colour moves to the second texel.
    let texture = eng.upload_rgba8(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255], false);
    let mut vertices = p.vertices.clone();
    material.apply_texture_coordinates(&p, &mut vertices);
    p.vertices = vertices;
    let mut dr = draw(&eng, &p, Vec3::new(64.0, 64.0, 0.0), material.params);
    dr.maps[0] = Some(texture.clone());
    dr.maps[4] = Some(texture);
    let mut ambient = sun(Vec3::Z, 1.0, false);
    ambient.kind = LightKind::Ambient;
    let pixels = eng.render_now(&scene(vec![dr], vec![ambient]), None);
    let center = at(&pixels, 64, 64);
    assert!(center[0] > 0.8 && center[1] > 0.2 && center[2] < 0.1, "base green + emissive red: {center:?}");
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
    // a directional light under a perspective camera casts through four cascades
    assert_eq!(eng.stats.shadow_views, 4);
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
        ..Default::default()
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
    s.env = Some(Env3 { env: gpu, intensity: 1.0, rotation: Mat4::IDENTITY, visible: true });
    let px = eng.render_now(&s, None);
    let bg = at(&px, 2, 2);
    assert!((bg[0] - 0.5).abs() < 0.02 && bg[3] == 1.0, "{bg:?}");
    let c = at(&px, 64, 64);
    assert!((c[0] - 0.5).abs() < 0.12, "a white diffuse sphere in a uniform 0.5 dome reflects ≈0.5: {c:?}");
}

#[test]
fn visible_dome_renders_without_any_draws() {
    // a pass that draws only the environment (every object culled or outside its time range) still needs a
    // material binding for the dome; materials are bound per render, so none is left over from earlier passes
    let Some(mut eng) = engine() else { return };
    let env = sr_3d::env::Equirect { width: 64, height: 32, rgb: vec![[0.25, 0.5, 0.75]; 64 * 32] };
    let gpu = eng.upload_env(&env);
    let mut s = scene(Vec::new(), Vec::new());
    s.env = Some(Env3 { env: gpu, intensity: 1.0, rotation: Mat4::IDENTITY, visible: true });
    let px = eng.render_now(&s, None);
    let bg = at(&px, 64, 64);
    assert!((bg[1] - 0.5).abs() < 0.02 && (bg[2] - 0.75).abs() < 0.02 && bg[3] == 1.0, "{bg:?}");
}

#[test]
fn splats_beside_the_camera_are_culled() {
    // just in front of the camera but far to the side: a tiny depth gives a footprint larger than the frame,
    // which must not paint the frame (3DGS frustum culling with a guard band)
    let Some(mut eng) = engine() else { return };
    let sc0 = scene(Vec::new(), Vec::new());
    let eye = sc0.cam.view.inverse().transform_point3(Vec3::ZERO);
    let s = sr_3d::Splats {
        pos: vec![[eye.x + 500.0, eye.y, eye.z + 20.0]],
        scale: vec![[60.0, 60.0, 60.0]],
        rot: vec![[0.0, 0.0, 0.0, 1.0]],
        color: vec![[1.0, 1.0, 1.0, 0.9]],
        basis: Mat4::IDENTITY,
        ..Default::default()
    };
    let gpu = eng.upload_splats(&s);
    let mut sc = sc0;
    sc.splats.push(SplatDraw { gpu, model: Mat4::IDENTITY, opacity: 1.0 });
    let px = eng.render_now(&sc, None);
    assert_eq!(at(&px, 64, 64)[3], 0.0, "the off-frame splat must not cover the frame");
}

/// 3D Gaussian Splatting's SH colour (reference implementation's basis), encoded, clamped at 0.
fn sh_reference(c: &[f32; 48], deg: u32, d: Vec3) -> [f32; 3] {
    let k = |i: usize| Vec3::new(c[i * 3], c[i * 3 + 1], c[i * 3 + 2]);
    let (x, y, z) = (d.x, d.y, d.z);
    let mut r = 0.282_094_8 * k(0);
    if deg >= 1 {
        r += -0.488_602_5 * y * k(1) + 0.488_602_5 * z * k(2) - 0.488_602_5 * x * k(3);
    }
    if deg >= 2 {
        let (xx, yy, zz) = (x * x, y * y, z * z);
        r += 1.092_548_4 * x * y * k(4) - 1.092_548_4 * y * z * k(5) + 0.315_391_57 * (2.0 * zz - xx - yy) * k(6)
            - 1.092_548_4 * x * z * k(7)
            + 0.546_274_2 * (xx - yy) * k(8);
        if deg >= 3 {
            r += -0.590_043_6 * y * (3.0 * xx - yy) * k(9) + 2.890_611_4 * x * y * z * k(10)
                - 0.457_045_8 * y * (4.0 * zz - xx - yy) * k(11)
                + 0.373_176_33 * z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * k(12)
                - 0.457_045_8 * x * (4.0 * zz - xx - yy) * k(13)
                + 1.445_305_7 * z * (xx - yy) * k(14)
                - 0.590_043_6 * x * (xx - 3.0 * yy) * k(15);
        }
    }
    (r + Vec3::splat(0.5)).max(Vec3::ZERO).to_array()
}

#[test]
fn splat_colour_follows_its_spherical_harmonics() {
    splat_colour_check(false);
}

#[test]
fn path_traced_splat_colour_follows_its_spherical_harmonics() {
    splat_colour_check(true);
}

fn splat_colour_check(path: bool) {
    // one opaque degree-3 splat, turned so the camera sees it from two directions in its own
    // frame: the centre pixel is the reference SH colour for each direction
    let Some(mut eng) = engine() else { return };
    let mut c = [0.0f32; 48];
    for (i, v) in c.iter_mut().enumerate() {
        *v = ((i as f32 * 1.618).fract() - 0.5) * 0.6;
    }
    c[0] = 0.4;
    c[1] = -0.2;
    c[2] = 0.1;
    let s = sr_3d::Splats {
        pos: vec![[0.0, 0.0, 0.0]],
        scale: vec![[14.0, 14.0, 14.0]],
        rot: vec![[0.0, 0.0, 0.0, 1.0]],
        color: vec![[0.5, 0.5, 0.5, 1.0]],
        sh: vec![c],
        sh_degree: 3,
        basis: Mat4::IDENTITY,
    };
    let gpu = eng.upload_splats(&s);
    let mut seen = Vec::new();
    for yaw in [0.0f32, 70.0] {
        let sc0 = scene(Vec::new(), Vec::new());
        let eye = sc0.cam.view.inverse().transform_point3(Vec3::ZERO);
        let model = Mat4::from_translation(Vec3::new(64.0, 64.0, 0.0)) * Mat4::from_rotation_y(yaw.to_radians());
        let mut sc = sc0;
        sc.splats.push(SplatDraw { gpu: gpu.clone(), model, opacity: 1.0 });
        if path {
            sc.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
        }
        let px = eng.render_now(&sc, None);
        let got = at(&px, 64, 64);
        let cam_local = model.inverse().transform_point3(eye);
        let dir = (Vec3::ZERO - cam_local).normalize();
        let enc = sh_reference(&c, 3, dir);
        // at the centre the splat is at its full (capped) opacity: premultiplied linear colour
        let a = got[3];
        assert!((a - 0.99).abs() < 0.01, "{got:?}");
        for ch in 0..3 {
            let e = enc[ch].min(1.0);
            let lin = if e <= 0.04045 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) };
            let want = lin * a;
            assert!((got[ch] - want).abs() < 0.01, "yaw {yaw}: channel {ch}: {} vs reference {want}", got[ch]);
        }
        seen.push(got);
    }
    let change = (0..3).map(|k| (seen[0][k] - seen[1][k]).abs()).fold(0.0, f32::max);
    assert!(change > 0.05, "the colour must depend on the view: {seen:?}");
}

#[test]
fn path_tracing_samples_texture_maps() {
    let Some(mut eng) = engine() else { return };
    let mut plane = prim::plane(70.0, 70.0, 1);
    for v in &mut plane.vertices {
        v.uv = [0.75, 0.5];
    }
    let mut dr = draw(&eng, &plane, Vec3::new(64.0, 64.0, 0.0), MaterialParams { unlit: true, ..Default::default() });
    dr.maps[0] = Some(eng.upload_rgba8(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255], true));
    let mut s = scene(vec![dr], vec![]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let pixels = eng.render_now(&s, None);
    let p = at(&pixels, 64, 64);
    assert!(p[1] > 0.9 && p[0] < 0.1 && p[2] < 0.1, "green texel in path-traced material: {p:?}");
    assert!(!sr_gpu::pathtrace::notes(&s).iter().any(|n| n.contains("texture maps")));
}

#[test]
fn path_traced_cutout_textures_do_not_cast_solid_shadows() {
    let Some(mut eng) = engine() else { return };
    let plane = prim::plane(100.0, 100.0, 1);
    let receiver = draw(&eng, &plane, Vec3::new(64.0, 64.0, 0.0), MaterialParams::default());
    let mut s = scene(vec![receiver], vec![sun(Vec3::Z, 5.0, true)]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let reference = lum(at(&eng.render_now(&s, None), 64, 64));
    let mut cutout = draw(
        &eng,
        &plane,
        Vec3::new(64.0, 64.0, -20.0),
        MaterialParams { alpha_mode: sr_3d::AlphaMode::Mask, ..Default::default() },
    );
    cutout.maps[0] = Some(eng.upload_rgba8(1, 1, &[255, 255, 255, 0], true));
    s.draws.push(cutout);
    let actual = lum(at(&eng.render_now(&s, None), 64, 64));
    assert!(
        reference > 0.1 && actual > reference * 0.9,
        "transparent texel blocked the light: {actual} vs {reference}"
    );
}

#[test]
fn path_tracing_honours_orthographic_cameras_and_ies_profiles() {
    let Some(mut eng) = engine() else { return };
    let plane = prim::plane(70.0, 70.0, 1);
    let dr = draw(&eng, &plane, Vec3::new(64.0, 64.0, 100.0), MaterialParams { unlit: true, ..Default::default() });
    let mut s = scene(vec![dr], vec![]);
    s.cam =
        resolve(&CameraParams { orthographic: true, ortho_height: Some(128.0), ..Default::default() }, 128.0, 128.0);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let pixels = eng.render_now(&s, None);
    assert!(at(&pixels, 96, 64)[3] > 0.9, "orthographic width must not shrink with depth");
    let mut light = sun(Vec3::Z, 10.0, false);
    light.kind = LightKind::Point;
    light.pos = Vec3::new(64.0, 64.0, -100.0);
    light.ies = Some(std::sync::Arc::new(vec![0.0; 128 * 32]));
    s.draws = vec![draw(&eng, &plane, Vec3::new(64.0, 64.0, 0.0), MaterialParams::default())];
    s.lights = vec![light];
    s.lens_k1 = 0.1;
    let pixels = eng.render_now(&s, None);
    assert!(lum(at(&pixels, 64, 64)) < 0.001, "zero IES profile must emit no light");
    assert!(sr_gpu::pathtrace::notes(&s).is_empty());
}

#[test]
fn path_tracing_composites_gaussian_splats_in_depth_order() {
    let Some(mut eng) = engine() else { return };
    let splats = sr_3d::Splats {
        pos: vec![[64.0, 64.0, 50.0], [64.0, 64.0, 0.0]],
        scale: vec![[12.0; 3]; 2],
        rot: vec![[0.0, 0.0, 0.0, 1.0]; 2],
        color: vec![[0.0, 0.0, 1.0, 0.95], [1.0, 0.0, 0.0, 0.95]],
        basis: Mat4::IDENTITY,
        ..Default::default()
    };
    let mut s = scene(vec![], vec![]);
    s.splats.push(SplatDraw { gpu: eng.upload_splats(&splats), model: Mat4::IDENTITY, opacity: 1.0 });
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let pixels = eng.render_now(&s, None);
    let center = at(&pixels, 64, 64);
    assert!(center[0] > 0.8 && center[2] < 0.15, "near red splat must cover the far blue splat: {center:?}");
    assert_eq!(at(&pixels, 1, 1)[3], 0.0);
    assert!(sr_gpu::pathtrace::notes(&s).is_empty());
}

#[test]
fn pathtrace_honors_object_shadow_flags() {
    let Some(mut eng) = engine() else { return };
    let receiver = draw(&eng, &prim::plane(110.0, 110.0, 1), Vec3::new(64.0, 64.0, 0.0), MaterialParams::default());
    let occluder = draw(&eng, &prim::plane(20.0, 30.0, 1), Vec3::new(44.0, 64.0, -20.0), MaterialParams::default());
    let mut s = scene(vec![receiver, occluder], vec![sun(Vec3::new(1.0, 0.0, 1.0), 5.0, true)]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let shadow = lum(at(&eng.render_now(&s, None), 64, 64));
    s.draws[1].cast_shadow = false;
    let no_cast = lum(at(&eng.render_now(&s, None), 64, 64));
    s.draws[1].cast_shadow = true;
    s.draws[0].receive_shadow = false;
    let no_receive = lum(at(&eng.render_now(&s, None), 64, 64));
    s.draws.pop();
    let reference = lum(at(&eng.render_now(&s, None), 64, 64));
    assert!(reference > shadow + 1.0);
    assert!((no_cast - reference).abs() < 0.01, "castShadow=false: {no_cast}, expected {reference}");
    assert!((no_receive - reference).abs() < 0.01, "receiveShadow=false: {no_receive}, expected {reference}");
}

#[test]
fn pathtrace_honors_light_lobe_flags() {
    let Some(mut eng) = engine() else { return };
    let receiver = draw(&eng, &prim::plane(110.0, 110.0, 1), Vec3::new(64.0, 64.0, 0.0), MaterialParams::default());
    let mut s = scene(vec![receiver], vec![sun(Vec3::new(1.0, 0.0, 1.0), 5.0, false)]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let full = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = false;
    let diffuse = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = true;
    s.lights[0].affects_diffuse = false;
    let specular = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = false;
    let disabled = lum(at(&eng.render_now(&s, None), 64, 64));
    assert!(diffuse > 0.0 && specular > 0.0);
    assert!(disabled < 1e-5, "disabled light contributes {disabled}");
    assert!((full - diffuse - specular).abs() < 0.01, "full={full} diffuse={diffuse} specular={specular}");
}

#[test]
fn pathtrace_honors_ambient_light_lobe_flags() {
    let Some(mut eng) = engine() else { return };
    let receiver = draw(&eng, &prim::plane(110.0, 110.0, 1), Vec3::new(64.0, 64.0, 0.0), MaterialParams::default());
    let mut s = scene(vec![receiver], vec![sun(Vec3::new(1.0, 0.0, 1.0), 5.0, false)]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    s.lights[0].kind = LightKind::Ambient;
    let full = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = false;
    let diffuse = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = true;
    s.lights[0].affects_diffuse = false;
    let specular = lum(at(&eng.render_now(&s, None), 64, 64));
    s.lights[0].affects_specular = false;
    let disabled = lum(at(&eng.render_now(&s, None), 64, 64));
    assert!(diffuse > 0.0 && specular > 0.0);
    assert!(disabled < 1e-5, "disabled light contributes {disabled}");
    assert!((full - diffuse - specular).abs() < 0.01, "full={full} diffuse={diffuse} specular={specular}");
}
