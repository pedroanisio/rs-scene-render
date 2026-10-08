//! `material/@unevenness`: a smooth noise fixed to the object that tilts the shading normal and varies the roughness.

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
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
        geodesic: None,
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
        shadow_catcher: false,
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

fn render(eng: &mut ThreeEngine, x: f32, material: MaterialParams) -> Vec<[f32; 4]> {
    let sphere = prim::sphere(30.0, 64);
    let mut ambient = sun(Vec3::Z, 0.4, false);
    ambient.kind = LightKind::Ambient;
    let s = scene(
        vec![draw(eng, &sphere, Vec3::new(x, 64.0, 0.0), material)],
        vec![sun(Vec3::new(1.0, 0.0, 0.3), 3.0, false), ambient],
    );
    eng.render_now(&s, None)
}

fn clay(unevenness: f32, seed: u32) -> MaterialParams {
    MaterialParams { roughness: 0.85, unevenness, unevenness_scale: 8.0, unevenness_seed: seed, ..Default::default() }
}

#[test]
fn off_is_the_baseline_whatever_the_seed() {
    let Some(mut eng) = engine() else { return };
    let a = render(&mut eng, 64.0, clay(0.0, 1));
    let b = render(&mut eng, 64.0, clay(0.0, 99));
    let c = render(&mut eng, 64.0, MaterialParams { roughness: 0.85, ..Default::default() });
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn unevenness_varies_the_lit_face() {
    let Some(mut eng) = engine() else { return };
    let smooth = render(&mut eng, 64.0, clay(0.0, 0));
    let uneven = render(&mut eng, 64.0, clay(0.8, 0));
    // the lit left face: many pixels change, in both directions
    let (mut changed, mut up, mut down) = (0, 0, 0);
    for y in 44..84 {
        for x in 40..64 {
            let d = lum(at(&uneven, x, y)) - lum(at(&smooth, x, y));
            if d.abs() > 0.02 {
                changed += 1;
                if d > 0.0 {
                    up += 1;
                } else {
                    down += 1;
                }
            }
        }
    }
    assert!(changed > 300 && up > 50 && down > 50, "changed {changed}, brighter {up}, darker {down}");
    // the silhouette is untouched
    for y in 0..W {
        for x in 0..W {
            assert_eq!(at(&uneven, x, y)[3], at(&smooth, x, y)[3]);
        }
    }
}

#[test]
fn the_pattern_belongs_to_the_object_and_the_seed() {
    let Some(mut eng) = engine() else { return };
    let here = render(&mut eng, 64.0, clay(0.8, 3));
    let again = render(&mut eng, 64.0, clay(0.8, 3));
    assert_eq!(here, again, "the same seed repeats exactly");
    let other = render(&mut eng, 64.0, clay(0.8, 4));
    let differs = (0..W * W).filter(|i| (lum(here[*i as usize]) - lum(other[*i as usize])).abs() > 0.02).count();
    assert!(differs > 300, "another seed is another pattern: {differs}");
    // moved 8 units to the right the object carries its pattern with it. The front of the sphere is nearer the camera
    // than the plane the move is measured in, so it shifts by a little more on screen: the best shift is searched.
    let moved = render(&mut eng, 72.0, clay(0.8, 3));
    let crop = |px: &[[f32; 4]], dx: u32| -> Vec<f32> {
        (48..80).flat_map(|y| (44..56).map(move |x| (x, y))).map(|(x, y)| lum(at(px, x + dx, y))).collect()
    };
    let best = |other: &[[f32; 4]], dxs: std::ops::Range<u32>| {
        dxs.map(|dx| correlation(&crop(&here, 0), &crop(other, dx))).fold(-1.0f32, f32::max)
    };
    let (follows, differs) = (best(&moved, 6..14), best(&render(&mut eng, 72.0, clay(0.8, 4)), 6..14));
    assert!(follows > 0.9, "the pattern follows the object: {follows}");
    assert!(differs < 0.6, "another seed does not: {differs}");
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    sab / (saa.sqrt() * sbb.sqrt()).max(1e-9)
}

#[test]
fn the_path_tracer_says_it_does_not_apply_unevenness() {
    let Some(eng) = engine() else { return };
    let sphere = prim::sphere(30.0, 16);
    let s = scene(vec![draw(&eng, &sphere, Vec3::new(64.0, 64.0, 0.0), clay(0.5, 0))], vec![sun(Vec3::Z, 3.0, false)]);
    assert_eq!(sr_gpu::pathtrace::notes(&s), vec!["material unevenness is not applied by the path tracer".to_string()]);
    let s = scene(vec![draw(&eng, &sphere, Vec3::new(64.0, 64.0, 0.0), clay(0.0, 0))], vec![sun(Vec3::Z, 3.0, false)]);
    assert!(sr_gpu::pathtrace::notes(&s).is_empty());
}

#[test]
fn features_below_a_pixel_fade_out_instead_of_aliasing() {
    let Some(mut eng) = engine() else { return };
    let sphere = prim::sphere(30.0, 64);
    let shot = |eng: &mut ThreeEngine, scale: f32| {
        let mut ambient = sun(Vec3::Z, 0.4, false);
        ambient.kind = LightKind::Ambient;
        let m = MaterialParams { unevenness_scale: scale, ..clay(0.8, 0) };
        let s = scene(
            vec![draw(eng, &sphere, Vec3::new(64.0, 64.0, 0.0), m)],
            vec![sun(Vec3::new(1.0, 0.0, 0.3), 3.0, false), ambient],
        );
        eng.render_now(&s, None)
    };
    let smooth = render(&mut eng, 64.0, clay(0.0, 0));
    let change = |a: &[[f32; 4]]| {
        (44..84)
            .flat_map(|y| (40..64).map(move |x| (x, y)))
            .map(|(x, y)| (lum(at(a, x, y)) - lum(at(&smooth, x, y))).abs())
            .fold(0.0f32, f32::max)
    };
    // features of 1 px, ½ px and ¼ px (octaves 0 to 2 at s = 1 unit; about 1 px per unit here): all below the 2 px limit
    let fine = change(&shot(&mut eng, 1.0));
    assert!(fine < 0.02, "sub-pixel features must not show: largest change {fine}");
    // coarse features still show
    let coarse = change(&shot(&mut eng, 16.0));
    assert!(coarse > 0.15, "coarse features show: {coarse}");
}
