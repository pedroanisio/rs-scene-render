//! Foam mixed into the water's albedo (`MaterialParams::foam_mix`): the share of foam each vertex carries in the alpha of its
//! colour turns the water into a white diffuse surface that does not let light through.

use super::common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, FoamMix, MaterialParams};
use sr_gpu::pathtrace::PathOpts;
use sr_gpu::three::*;

const W: u32 = 64;

fn engine() -> Option<ThreeEngine> {
    let g = common::gpu()?;
    Some(ThreeEngine::new(g.device.clone(), g.queue.clone()))
}

fn lum(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

const DARK_WATER: [f32; 4] = [0.0018, 0.0152, 0.0252, 1.0];

fn water(foam: Option<FoamMix>) -> MaterialParams {
    MaterialParams {
        base_color: DARK_WATER,
        roughness: 0.06,
        transmission: 1.0,
        ior: 1.333,
        double_sided: true,
        foam_mix: foam,
        ..Default::default()
    }
}

/// A horizontal plane of 40 x 40 at depth `y` (the scene's up is -y), its vertices' colour alpha from `share(x)`.
fn plane(eng: &ThreeEngine, y: f32, material: MaterialParams, share: impl Fn(f32) -> f32) -> Draw3 {
    plane_of(eng, 40.0, 40, y, material, share)
}

/// The same plane, `size` wide in `segments` cells: the share of foam changes from one vertex to the next, so a finer plane has a
/// sharper edge.
fn plane_of(
    eng: &ThreeEngine,
    size: f32,
    segments: u32,
    y: f32,
    material: MaterialParams,
    share: impl Fn(f32) -> f32,
) -> Draw3 {
    let mut p = prim::plane(size, size, segments);
    for v in &mut p.vertices {
        v.color = [1.0, 1.0, 1.0, share(v.pos[0])];
    }
    Draw3 {
        mesh: MeshSrc::Cached(eng.upload_mesh(&p.vertices, &p.indices)),
        model: Mat4::from_translation(Vec3::new(0.0, y, 0.0)) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        material,
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
        shadow_catcher: false,
    }
}

fn scene(eng: &ThreeEngine, draws: Vec<Draw3>, dome: Option<f32>, sun: bool, eye: Vec3) -> Scene3 {
    let env = dome.map(|l| {
        let e = sr_3d::env::Equirect { width: 64, height: 32, rgb: vec![[l, l, l]; 64 * 32] };
        Env3 { env: eng.upload_env(&e), intensity: 1.0, rotation: Mat4::IDENTITY, visible: true }
    });
    let lights = if sun {
        vec![Light3 {
            kind: LightKind::Directional,
            pos: Vec3::ZERO,
            dir: Vec3::new(0.8, 0.6, 0.0).normalize(),
            right: Vec3::X,
            color: Vec3::splat(3.0),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 0.0,
            bias: 0.0005,
            map_size: 1024,
            size: [0.0; 3],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }]
    } else {
        Vec::new()
    };
    Scene3 {
        cam: resolve(
            &CameraParams { fov: 20.0, position: Some(eye), target: Some(Vec3::ZERO), ..Default::default() },
            W as f32,
            W as f32,
        ),
        clip_fix: Mat4::IDENTITY,
        size: [W, W],
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws,
        lights,
        env,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: Some(PathOpts { samples: 256, bounces: 4, denoise: false }),
        geodesic: None,
    }
}

fn mean(px: &[[f32; 4]], x0: u32, x1: u32, y0: u32, y1: u32) -> f32 {
    let v: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| lum(px[(y * W + x) as usize]))).collect();
    v.iter().sum::<f32>() / v.len() as f32
}

const FOAM: FoamMix = FoamMix { albedo: 0.9, roughness: 0.8 };
const TOP: Vec3 = Vec3::new(0.0, -8.0, -0.5);

#[test]
fn water_wholly_covered_by_foam_is_a_white_diffuse_surface_under_a_dome() {
    let Some(mut eng) = engine() else { return };
    let dome = srgb_to_linear(128.0 / 255.0);
    let draw = plane(&eng, 0.0, water(Some(FOAM)), |_| 1.0);
    let px = eng.render_now(&scene(&eng, vec![draw], Some(dome), false, TOP), None);
    let got = mean(&px, 24, 40, 24, 40);
    // seen almost along the normal: the directional albedo of a dielectric of index 1.333 and roughness 0.8, from the
    // quadrature of its BRDF (the diffuse share 0.9 times (1 - F))
    let (spec, diffuse) = common::directional_albedo(0.998, 0.8, 1.333);
    let want = dome * (spec + 0.9 * diffuse) as f32;
    println!("covered water: picture {got:.4}, quadrature {want:.4}");
    assert!((got - want).abs() <= 0.02 * want, "{got} against {want}");
}

#[test]
fn foam_on_a_surface_that_lets_no_light_through_gives_the_same_white_and_changes_nothing_where_it_is_absent() {
    let Some(mut eng) = engine() else { return };
    let dome = srgb_to_linear(128.0 / 255.0);
    let sea = |foam: Option<FoamMix>| MaterialParams { transmission: 0.0, ..water(foam) };
    let covered = plane(&eng, 0.0, sea(Some(FOAM)), |_| 1.0);
    let px = eng.render_now(&scene(&eng, vec![covered], Some(dome), false, TOP), None);
    let got = mean(&px, 24, 40, 24, 40);
    let (spec, diffuse) = common::directional_albedo(0.998, 0.8, 1.333);
    let want = dome * (spec + 0.9 * diffuse) as f32;
    println!("covered opaque sea: picture {got:.4}, quadrature {want:.4}");
    assert!((got - want).abs() <= 0.02 * want, "{got} against {want}");
    // with no foam anywhere the picture is the one of the same material without the mix, bit for bit
    let bare = plane(&eng, 0.0, sea(Some(FOAM)), |_| 0.0);
    let plain = plane(&eng, 0.0, sea(None), |_| 0.0);
    let a = eng.render_now(&scene(&eng, vec![bare], Some(dome), true, TOP), None);
    let b = eng.render_now(&scene(&eng, vec![plain], Some(dome), true, TOP), None);
    assert!(a == b, "a mix with no foam changes the picture");
}

#[test]
fn the_roughness_of_the_foam_sets_the_specular_lobe_of_covered_water() {
    let Some(mut eng) = engine() else { return };
    let dome = srgb_to_linear(128.0 / 255.0);
    // seen from 10 degrees over the horizon, where the lobe of a smooth surface is far brighter than a rough one's
    let eye = Vec3::new(0.0, -1.5, -8.0);
    let nv = 1.5 / eye.length() as f64;
    let mut got = Vec::new();
    for roughness in [0.2f32, 0.9] {
        let sea = MaterialParams { transmission: 0.0, ..water(Some(FoamMix { albedo: 0.9, roughness })) };
        let draw = plane(&eng, 0.0, sea, |_| 1.0);
        let px = eng.render_now(&scene(&eng, vec![draw], Some(dome), false, eye), None);
        let (spec, diffuse) = common::directional_albedo(nv, roughness as f64, 1.333);
        let want = dome * (spec + 0.9 * diffuse) as f32;
        got.push((mean(&px, 30, 34, 30, 34), want));
        println!("foam roughness {roughness}: picture {:.4}, quadrature {want:.4} (nv {nv:.3})", got.last().unwrap().0);
    }
    let (smooth, rough) = (got[0], got[1]);
    assert!(
        smooth.0 > 1.25 * rough.0,
        "the smoother foam has the brighter lobe at a grazing view: {smooth:?} against {rough:?}"
    );
    for (picture, want) in got {
        assert!((picture - want).abs() <= 0.03 * want, "{picture} against {want}");
    }
}

/// The mean luminance of the middle of a view from under a plane at depth 0 (the camera 6 below it, looking up) whose water has
/// a grey tint and a uniform share `share` of foam, under a uniform dome.
fn seen_from_below(eng: &mut ThreeEngine, share: f32) -> f32 {
    let dome = srgb_to_linear(128.0 / 255.0);
    let tinted = MaterialParams { base_color: [0.1, 0.1, 0.1, 1.0], ..water(Some(FOAM)) };
    let draw = plane(eng, 0.0, tinted, move |_| share);
    let px = eng.render_now(&scene(eng, vec![draw], Some(dome), false, Vec3::new(0.0, 6.0, -0.5)), None);
    mean(&px, 24, 40, 24, 40)
}

#[test]
fn the_light_that_gets_through_partly_covered_water_keeps_the_waters_tint() {
    let Some(mut eng) = engine() else { return };
    // Seen from below, the uncovered share of the surface lets the light through with the water's tint and the covered share is a
    // white diffuse surface: the picture is the mean of the bare water's and the wholly covered water's, weighted by the share.
    // Taking the foam's colour for the tint of the light that gets through (0.5 in place of 0.1 at half) makes the middle far brighter.
    let (bare, half, covered) =
        (seen_from_below(&mut eng, 0.0), seen_from_below(&mut eng, 0.5), seen_from_below(&mut eng, 1.0));
    let want = 0.5 * (bare + covered);
    println!("from below: share 0 {bare:.4}, 0.5 {half:.4}, 1 {covered:.4}; the weighted mean {want:.4}");
    assert!(covered > 2.0 * bare, "the foam is brighter than the tinted water: {covered} against {bare}");
    assert!((half - want).abs() <= 0.03 * want, "{half} against {want}");
}

#[test]
fn the_denoiser_keeps_the_edge_of_the_foam_because_its_albedo_guide_is_the_mixed_albedo() {
    let Some(mut eng) = engine() else { return };
    // Half of a dark sea is covered by foam, with an edge of one pixel (a plane of 6 in 120 cells seen 2.9 wide in 64 pixels): a step
    // from 0.006 to 0.19. At 4 samples a pixel the denoiser must not smear it: its filter keeps an edge where the guide (the albedo at
    // the first hit) steps, and that is the albedo after the mix.
    let dome = srgb_to_linear(128.0 / 255.0);
    let render = |eng: &mut ThreeEngine, samples: u32, denoise: bool| {
        let draw = plane_of(eng, 6.0, 120, 0.0, water(Some(FOAM)), |x| if x < 0.0 { 1.0 } else { 0.0 });
        let mut s = scene(eng, vec![draw], Some(dome), false, TOP);
        s.path = Some(PathOpts { samples, bounces: 4, denoise });
        eng.render_now(&s, None)
    };
    let reference = render(&mut eng, 1024, false);
    let denoised = render(&mut eng, 4, true);
    let noisy = render(&mut eng, 4, false);
    println!(
        "PROFILE {:?}",
        (0..64).step_by(2).map(|x| (mean(&reference, x, x + 1, 20, 44) * 1000.0).round() / 1000.0).collect::<Vec<_>>()
    );
    // the error of a picture against the reference in the columns 2 either side of the edge (the foam is on the left of the middle)
    let edge = (0..64).find(|x| mean(&reference, *x, *x + 1, 20, 44) < 0.1).unwrap_or(32);
    let error = |px: &[[f32; 4]]| -> f32 {
        let mut sum = 0.0;
        for x in edge.saturating_sub(2)..edge + 2 {
            sum += (mean(px, x, x + 1, 20, 44) - mean(&reference, x, x + 1, 20, 44)).abs();
        }
        sum / 4.0
    };
    let (noisy_error, denoised_error) = (error(&noisy), error(&denoised));
    println!("edge at column {edge}; mean error there: noisy {noisy_error:.4}, denoised {denoised_error:.4}");
    assert!(
        mean(&reference, 5, 15, 20, 44) > 20.0 * mean(&reference, 40, 50, 20, 44),
        "a step from foam to dark water"
    );
    assert!(
        denoised_error <= noisy_error,
        "the denoiser smears the edge of the foam: error {denoised_error} against {noisy_error} without it"
    );
}

#[test]
fn bare_water_is_the_same_with_or_without_the_mix_and_foam_brightens_it_by_share() {
    let Some(mut eng) = engine() else { return };
    let dome = srgb_to_linear(128.0 / 255.0);
    let render = |eng: &mut ThreeEngine, foam: Option<FoamMix>, share: f32| {
        let draw = plane(eng, 0.0, water(foam), |_| share);
        eng.render_now(&scene(eng, vec![draw], Some(dome), false, TOP), None)
    };
    // the transmissive water takes the foam variant of the shader, and with no foam it is the picture of the water without the mix,
    // bit for bit (the random numbers a sample draws are the same)
    let plain = render(&mut eng, None, 0.0);
    let mixed = render(&mut eng, Some(FOAM), 0.0);
    assert!(plain == mixed, "a mix with no foam changes the picture of transmissive water");
    let level = |px: &[[f32; 4]]| mean(px, 24, 40, 24, 40);
    let shares: Vec<f32> =
        [0.0, 0.25, 0.5, 0.75, 1.0].iter().map(|s| level(&render(&mut eng, Some(FOAM), *s))).collect();
    println!("by share 0, 0.25, 0.5, 0.75, 1: {shares:?}");
    assert!(shares.windows(2).all(|w| w[1] > w[0]), "foam brightens water step by step: {shares:?}");
    assert!(shares[4] > 20.0 * shares[0], "from a dark sea to white foam: {shares:?}");
    // the share is the area the foam covers: the picture is the mean of the water's and the foam's weighted by it
    for (i, share) in [0.25f32, 0.5, 0.75].iter().enumerate() {
        let want = (1.0 - share) * shares[0] + share * shares[4];
        assert!((shares[i + 1] - want).abs() <= 0.05 * want, "share {share}: {} against {want}", shares[i + 1]);
    }
}

#[test]
fn foam_casts_the_shadow_of_an_opaque_surface_on_the_floor_under_the_water() {
    let Some(mut eng) = engine() else { return };
    // Clear water over a floor 2 below it, the sun travelling to the right and down. The sun's rays refract to 37 degrees
    // from the vertical in the water, so the floor at x is lit through the surface 1.5 to the left of it: the floor under
    // the right of the middle, up to x = 1.5, is lit through the left half of the water. The camera looks at the floor
    // between x = 0.3 and 2.7 through the right half.
    let render = |eng: &mut ThreeEngine, mix: Option<FoamMix>, foam_below: f32| {
        let covered = move |x: f32| if x < foam_below { 1.0 } else { 0.0 };
        let clear = MaterialParams { base_color: [1.0; 4], ..water(mix) };
        let sea = plane(eng, 0.0, clear, covered);
        let floor = plane(
            eng,
            2.0,
            MaterialParams { base_color: [0.7, 0.7, 0.7, 1.0], roughness: 1.0, ..Default::default() },
            |_| 0.0,
        );
        let eye = Vec3::new(1.5, -8.0, -0.5);
        let mut s = scene(eng, vec![sea, floor], None, true, eye);
        s.cam = resolve(
            &CameraParams {
                fov: 14.0,
                position: Some(eye),
                target: Some(Vec3::new(1.5, 0.0, 0.0)),
                ..Default::default()
            },
            W as f32,
            W as f32,
        );
        eng.render_now(&s, None)
    };
    let columns = |px: &[[f32; 4]]| -> Vec<f32> { (0..8).map(|c| mean(px, c * 8, c * 8 + 8, 20, 44)).collect() };
    let plain = render(&mut eng, None, -100.0);
    let bare = render(&mut eng, Some(FOAM), -100.0);
    let beyond = render(&mut eng, Some(FOAM), -18.0);
    let foamy = render(&mut eng, Some(FOAM), 0.0);
    let (bare_c, foamy_c) = (columns(&bare), columns(&foamy));
    println!("floor by column, bare: {bare_c:.4?}");
    println!("floor by column, foam on the left half: {foamy_c:.4?}");
    let all = |px: &[[f32; 4]]| mean(px, 0, 64, 20, 44);
    assert!(
        (all(&bare) - all(&plain)).abs() <= 0.005 * all(&plain),
        "the mix without foam changes nothing: {} against {}",
        all(&bare),
        all(&plain)
    );
    assert!(bare_c[0] > 0.2, "the sun reaches the floor through bare water: {bare_c:?}");
    assert!(foamy_c[0] < 0.25 * bare_c[0], "foam shades the floor it covers: {} against {}", foamy_c[0], bare_c[0]);
    assert!(
        foamy_c.windows(2).take(4).all(|w| w[1] > w[0]),
        "the shade fades with the distance from the foam: {foamy_c:?}"
    );
    assert!(
        foamy_c[7] > 0.8 * bare_c[7],
        "and little is left of it past the sun's reach: {} against {}",
        foamy_c[7],
        bare_c[7]
    );
    assert!(
        (all(&beyond) - all(&bare)).abs() <= 0.005 * all(&bare),
        "foam out of the window's reach changes nothing: {} against {}",
        all(&beyond),
        all(&bare)
    );
}
