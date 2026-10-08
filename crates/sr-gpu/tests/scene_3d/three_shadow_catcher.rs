//! A shadow catcher draws no colour of its own: black, with alpha `opacity · (1 − E_sh / E_un)`.

use super::common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::three::*;

const W: u32 = 128;

fn engine() -> Option<ThreeEngine> {
    let g = common::gpu()?;
    Some(ThreeEngine::new(g.device.clone(), g.queue.clone()))
}

fn light(kind: LightKind, dir: Vec3, lux: f32, shadow: bool) -> Light3 {
    Light3 {
        kind,
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

fn draw(eng: &ThreeEngine, p: &sr_3d::Primitive, at: Vec3) -> Draw3 {
    Draw3 {
        mesh: MeshSrc::Cached(eng.upload_mesh(&p.vertices, &p.indices)),
        model: Mat4::from_translation(at),
        material: MaterialParams { roughness: 1.0, ..Default::default() },
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
        shadow_catcher: false,
    }
}

fn scene(draws: Vec<Draw3>, lights: Vec<Light3>) -> Scene3 {
    Scene3 {
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
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
        geodesic: None,
    }
}

fn at(px: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    px[(y * W + x) as usize]
}

/// The lit ground at x = 20 and the shadowed ground at x = 119 (the sun travels right and away, as in the shadow test
/// of `three.rs`: the sphere's shadow lands to its right).
const LIT: (u32, u32) = (20, 64);
const SHADOW: (u32, u32) = (64 + 55, 64);

fn rig(eng: &ThreeEngine, catcher: bool, opacity: f32, lights: Vec<Light3>) -> Scene3 {
    let mut ground = draw(eng, &prim::plane(400.0, 400.0, 1), Vec3::new(64.0, 64.0, 60.0));
    ground.shadow_catcher = catcher;
    ground.opacity = opacity;
    let ball = draw(eng, &prim::sphere(15.0, 32), Vec3::new(64.0, 64.0, 0.0));
    scene(vec![ball, ground], lights)
}

fn sun() -> Light3 {
    light(LightKind::Directional, Vec3::new(1.0, 0.0, 1.0), 3.0, true)
}

fn ambient(lux: f32) -> Light3 {
    light(LightKind::Ambient, Vec3::Z, lux, false)
}

fn y(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

#[test]
fn a_catcher_is_transparent_in_the_light_and_opaque_black_in_the_full_shadow() {
    let Some(mut eng) = engine() else { return };
    let px = eng.render_now(&rig(&eng, true, 1.0, vec![sun()]), None);
    let (lit, shadow) = (at(&px, LIT.0, LIT.1), at(&px, SHADOW.0, SHADOW.1));
    assert!(lit[3] < 0.02, "lit ground draws nothing: {lit:?}");
    assert!(shadow[3] > 0.97 && shadow[0] + shadow[1] + shadow[2] < 0.02, "shadow is black and opaque: {shadow:?}");
}

#[test]
fn ambient_light_keeps_the_shadow_lighter_by_the_ratio_of_the_lights() {
    let Some(mut eng) = engine() else { return };
    let lights = || vec![sun(), ambient(1.0)];
    // the ordinary ground's own lit and shadowed brightness give E_sh / E_un: albedo cancels
    let plain = eng.render_now(&rig(&eng, false, 1.0, lights()), None);
    let ratio = y(at(&plain, SHADOW.0, SHADOW.1)) / y(at(&plain, LIT.0, LIT.1));
    assert!(ratio > 0.1 && ratio < 0.9, "the scene has both direct and ambient light: {ratio}");
    let px = eng.render_now(&rig(&eng, true, 1.0, lights()), None);
    let shadow = at(&px, SHADOW.0, SHADOW.1);
    assert!((shadow[3] - (1.0 - ratio)).abs() < 0.03, "alpha {} expected {}", shadow[3], 1.0 - ratio);
    assert!(shadow[0] + shadow[1] + shadow[2] < 0.02, "colour stays black: {shadow:?}");
    assert!(at(&px, LIT.0, LIT.1)[3] < 0.02);
}

#[test]
fn opacity_scales_the_darkening() {
    let Some(mut eng) = engine() else { return };
    let full = at(&eng.render_now(&rig(&eng, true, 1.0, vec![sun(), ambient(1.0)]), None), SHADOW.0, SHADOW.1)[3];
    let half = at(&eng.render_now(&rig(&eng, true, 0.5, vec![sun(), ambient(1.0)]), None), SHADOW.0, SHADOW.1)[3];
    assert!((half - full * 0.5).abs() < 0.02, "half opacity halves alpha: {half} vs {full}");
}

#[test]
fn lights_that_cast_no_shadow_leave_the_catcher_transparent() {
    let Some(mut eng) = engine() else { return };
    let mut s = light(LightKind::Directional, Vec3::new(1.0, 0.0, 1.0), 3.0, false);
    s.cast_shadow = false;
    let px = eng.render_now(&rig(&eng, true, 1.0, vec![s]), None);
    assert!(at(&px, SHADOW.0, SHADOW.1)[3] < 0.02, "{:?}", at(&px, SHADOW.0, SHADOW.1));
}

#[test]
fn a_catcher_casts_no_shadow() {
    let Some(mut eng) = engine() else { return };
    // the catcher floats between the light and a second ground: the ground behind it stays lit
    let mut veil = draw(&eng, &prim::plane(60.0, 60.0, 1), Vec3::new(44.0, 64.0, -20.0));
    veil.shadow_catcher = true;
    let ground = draw(&eng, &prim::plane(400.0, 400.0, 1), Vec3::new(64.0, 64.0, 60.0));
    let with = eng.render_now(&scene(vec![veil, ground], vec![sun()]), None);
    let ground_only = draw(&eng, &prim::plane(400.0, 400.0, 1), Vec3::new(64.0, 64.0, 60.0));
    let without = eng.render_now(&scene(vec![ground_only], vec![sun()]), None);
    for (x, yy) in [(64, 64), (74, 64), (90, 64)] {
        let (a, b) = (y(at(&with, x, yy)), y(at(&without, x, yy)));
        assert!((a - b).abs() < 0.01 + b * 0.02, "({x},{yy}) {a} vs {b}");
    }
}

#[test]
fn a_catcher_that_is_not_a_catcher_still_draws_its_colour() {
    let Some(mut eng) = engine() else { return };
    let px = eng.render_now(&rig(&eng, false, 1.0, vec![sun(), ambient(1.0)]), None);
    assert!(y(at(&px, LIT.0, LIT.1)) > 0.2 && at(&px, LIT.0, LIT.1)[3] > 0.97);
}

#[test]
fn the_path_tracer_does_not_draw_a_catcher_and_says_so() {
    let Some(mut eng) = engine() else { return };
    let mut s = rig(&eng, true, 1.0, vec![sun(), ambient(1.0)]);
    s.path = Some(sr_gpu::pathtrace::PathOpts { samples: 4, bounces: 1, denoise: false });
    let px = eng.render_now(&s, None);
    assert!(at(&px, LIT.0, LIT.1)[3] < 0.02, "the catcher's material is not drawn: {:?}", at(&px, LIT.0, LIT.1));
    let notes = sr_gpu::pathtrace::notes(&s);
    assert!(notes.iter().any(|n| n.contains("shadow catcher")), "{notes:?}");
}
