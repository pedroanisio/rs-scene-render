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
        Env3 { env: eng.upload_env(&e), intensity: 1.0, rotation: Mat4::IDENTITY, visible: true, sky: None }
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
    // a metallic water is covered by foam as well: the foam sample is a non-metal, so a wholly covered metallic sea is the same white
    let metal = MaterialParams { metallic: 1.0, ..sea(Some(FOAM)) };
    let px = eng.render_now(&scene(&eng, vec![plane(&eng, 0.0, metal, |_| 1.0)], Some(dome), false, TOP), None);
    let got = mean(&px, 24, 40, 24, 40);
    println!("covered metallic sea: picture {got:.4}, quadrature {want:.4}");
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

#[test]
fn the_refracted_shadow_ray_scales_the_light_by_one_minus_the_share_and_not_by_a_step_or_a_square() {
    let Some(mut eng) = engine() else { return };
    // A camera under the water between the surface (depth 0) and a floor 2 below it, looking down at the floor, which the sun lights
    // through the water, the foam a uniform share of the surface: the light on the floor is what the refracted shadow ray lets
    // through, one minus the share of it (there is no dome; the floor sees the foam only from below, lit by nothing)
    let floor_under = |eng: &mut ThreeEngine, share: f32| {
        let clear = MaterialParams { base_color: [1.0; 4], ..water(Some(FOAM)) };
        let sea = plane(eng, 0.0, clear, move |_| share);
        let floor = plane(
            eng,
            2.0,
            MaterialParams { base_color: [0.7, 0.7, 0.7, 1.0], roughness: 1.0, ..Default::default() },
            |_| 0.0,
        );
        let eye = Vec3::new(0.0, 1.0, -0.2);
        let mut s = scene(eng, vec![sea, floor], None, true, eye);
        s.cam = resolve(
            &CameraParams {
                fov: 30.0,
                position: Some(eye),
                target: Some(Vec3::new(0.0, 2.0, 0.0)),
                ..Default::default()
            },
            W as f32,
            W as f32,
        );
        let px = eng.render_now(&s, None);
        mean(&px, 16, 48, 16, 48)
    };
    let (open, half, covered) = (floor_under(&mut eng, 0.0), floor_under(&mut eng, 0.5), floor_under(&mut eng, 1.0));
    println!("floor: share 0 {open:.4}, 0.5 {half:.4}, 1 {covered:.4}");
    assert!(open > 0.2 && covered < 0.35 * open, "from the sun's light to the shade: {open} against {covered}");
    // one minus the share of what the open water lets through (0.2985 against 0.2697 at a half: the light the floor gives back to the
    // surface and the surface to the floor adds a tenth; 0.3103 is the edge of the band); a step at one half would give the whole or nothing, and one minus the share
    // squared a quarter of the open water's
    let want = 0.5 * (open + covered);
    assert!((half - want).abs() <= 0.2 * want, "{half} against the mean {want} of {open} and {covered}");
    assert!(half > 0.4 * open && half < 0.7 * open, "neither a step nor a square: {half} of {open}");
}

/// The mean and the standard deviation of the luminance of the middle 16 x 16 pixels of a sea wholly at `share` of foam, rendered at
/// `samples` a pixel under a uniform dome.
fn noise_at(eng: &mut ThreeEngine, share: f32, samples: u32, denoise: bool) -> (f32, f32) {
    noise_of(eng, Some(FOAM), share, samples, denoise)
}

/// The same for the water with the foam mix `mix` (none: the water as it was before the mix existed).
fn noise_of(eng: &mut ThreeEngine, mix: Option<FoamMix>, share: f32, samples: u32, denoise: bool) -> (f32, f32) {
    let dome = srgb_to_linear(128.0 / 255.0);
    let draw = plane(eng, 0.0, water(mix), move |_| share);
    let mut s = scene(eng, vec![draw], Some(dome), false, TOP);
    s.path = Some(PathOpts { samples, bounces: 4, denoise });
    let px = eng.render_now(&s, None);
    let mut v: Vec<f32> = Vec::new();
    for y in 24..40u32 {
        for x in 24..40u32 {
            v.push(lum(px[(y * W + x) as usize]));
        }
    }
    let m = v.iter().sum::<f32>() / v.len() as f32;
    (m, (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt())
}

#[test]
#[ignore = "measures the noise of the random coverage; run it on the GPU queue with --ignored"]
fn the_noise_of_the_foam_mix_at_a_half_share_is_what_the_srep_states() {
    let Some(mut eng) = engine() else { return };
    // 8 samples a pixel, a window of 16 x 16 pixels, a uniform grey dome: the numbers of the SREP (NVIDIA), with the room that the
    // adapters and the random numbers give
    let (half, half_sd) = noise_at(&mut eng, 0.5, 8, false);
    let (denoised, denoised_sd) = noise_at(&mut eng, 0.5, 8, true);
    let (open, open_sd) = noise_at(&mut eng, 0.0, 8, false);
    // a share of 0 is the water without the mix, in the same run: the same mean and the same noise, to the bit
    let (plain, plain_sd) = noise_of(&mut eng, None, 0.0, 8, false);
    assert_eq!((open, open_sd), (plain, plain_sd), "no foam, no change");
    let (covered, covered_sd) = noise_at(&mut eng, 1.0, 8, false);
    println!(
        "noise at 8 samples: half {half:.4} sd {half_sd:.4} (cv {:.3}); denoised {denoised:.4} sd {denoised_sd:.4} (cv {:.3}); open sd {open_sd:.4}; covered {covered:.4} sd {covered_sd:.4}",
        half_sd / half,
        denoised_sd / denoised
    );
    // the coefficient of variation of the mix at a half: 0.156 measured with the lobe of the first hit drawn in strata (0.345 when it was
    // drawn at random)
    assert!(half_sd / half < 0.2, "cv {}", half_sd / half);
    // with the denoiser: 0.003 measured (0.125 at random), and its mean within 3 % of the true one (0.0996; 5 % under at random)
    assert!(denoised_sd / denoised < 0.02, "cv {}", denoised_sd / denoised);
    assert!((denoised - 0.0996).abs() < 0.0996 * 0.03, "the denoised mean {denoised}");
    // a share of 0 or of 1 is as noisy as it was before the mix existed: 0.0101 and 0.0091 measured
    assert!(open_sd < 0.013 && covered_sd < 0.012, "{open_sd} {covered_sd}");
}

/// The luminance of every pixel of the middle 32 x 32 of a sea wholly at a half share of foam, at `samples` a pixel.
fn half_sea_window(eng: &mut ThreeEngine, samples: u32) -> Vec<f32> {
    let dome = srgb_to_linear(128.0 / 255.0);
    let draw = plane(eng, 0.0, water(Some(FOAM)), |_| 0.5);
    let mut s = scene(eng, vec![draw], Some(dome), false, TOP);
    s.path = Some(PathOpts { samples, bounces: 4, denoise: false });
    let px = eng.render_now(&s, None);
    let mut v = Vec::new();
    for y in 16..48u32 {
        for x in 16..48u32 {
            v.push(lum(px[(y * W + x) as usize]));
        }
    }
    v
}

#[test]
fn the_lobe_of_the_first_hit_is_drawn_in_strata_so_the_half_share_is_far_less_noisy_than_a_random_draw() {
    let Some(mut eng) = engine() else { return };
    // 8 samples a pixel at a half share: a random draw of the lobe gave a coefficient of variation of 0.345 over the window; drawing it
    // from a sequence that has a different offset in every pixel puts nearly half of a pixel's samples on each
    let v = half_sea_window(&mut eng, 8);
    let m = v.iter().sum::<f32>() / v.len() as f32;
    let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt();
    println!("half share, 8 samples: mean {m:.4} sd {sd:.4} cv {:.3}", sd / m);
    assert!(sd / m < 0.2, "cv {}", sd / m);
}

#[test]
fn the_strata_leave_no_pattern_in_the_picture_and_no_bias_in_its_mean() {
    let Some(mut eng) = engine() else { return };
    // the offset of a pixel's sequence is a hash of the pixel: neighbours are not correlated (a lattice of offsets would be a grid in the
    // picture), and the mean of many samples is the mean of the lobes weighted by the share: 0.0996 from the bare and covered pictures
    let v = half_sea_window(&mut eng, 8);
    let m = v.iter().sum::<f32>() / v.len() as f32;
    let residual: Vec<f32> = v.iter().map(|x| x - m).collect();
    let var = residual.iter().map(|r| r * r).sum::<f32>() / residual.len() as f32;
    let corr = |dx: usize, dy: usize| -> f32 {
        let mut sum = 0.0;
        let mut n = 0;
        for y in 0..32 - dy {
            for x in 0..32 - dx {
                sum += residual[y * 32 + x] * residual[(y + dy) * 32 + x + dx];
                n += 1;
            }
        }
        sum / n as f32 / var
    };
    let (right, below, diagonal) = (corr(1, 0), corr(0, 1), corr(1, 1));
    println!("correlation of neighbours: right {right:.3}, below {below:.3}, diagonal {diagonal:.3}");
    assert!(right.abs() < 0.15 && below.abs() < 0.15 && diagonal.abs() < 0.15, "{right} {below} {diagonal}");
    let many = half_sea_window(&mut eng, 128);
    let mean = many.iter().sum::<f32>() / many.len() as f32;
    assert!((mean - 0.0996).abs() <= 0.05 * 0.0996, "the mean of 128 samples: {mean}");
}
