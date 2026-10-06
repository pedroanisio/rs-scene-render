//! The Schwarzschild geodesic pass: the shadow, the disk's symmetry and redshift, the secondary image,
//! the integration against an f64 reference, finite output, and bands against the whole frame.

mod common;

use glam::{DVec3, Vec3};
use sr_gpu::geodesic::{Disk, GeodesicGpu, GeodesicScene, Output, Pattern};

const STEP: f64 = 0.02;
const MAX_STEPS: usize = 4096;

/// The frame an observer at `eye` sees looking at `hole`, the scene's +y being down in the image.
fn camera(size: [u32; 2], eye: Vec3, hole: Vec3, mass: f32, focal_px: f32, disk: Option<Disk>) -> GeodesicScene {
    let forward = (hole - eye).normalize();
    let down = (Vec3::Y - forward * Vec3::Y.dot(forward)).normalize();
    let right = down.cross(forward);
    GeodesicScene {
        size,
        eye,
        right,
        down,
        forward,
        focal_px,
        hole,
        mass,
        disk,
        samples: 1,
        star_seed: 0,
        exposure: 1.0,
        encode_srgb: false,
    }
}

fn disk(inner: f32, outer: f32, axis: Vec3) -> Disk {
    Disk {
        inner,
        outer,
        peak_kelvin: 9000.0,
        intensity: 1.0,
        contrast: 0.0,
        pattern: Pattern::None,
        seed: 1,
        axis,
        time: 0.0,
    }
}

/// Renders into an RGBA32F texture and reads it back, rows of `[r, g, b, a]`.
fn render(scene: &GeodesicScene, output: Output, rows: Option<u32>) -> Option<Vec<[f32; 4]>> {
    let g = common::gpu()?;
    let (w, h) = (scene.size[0], scene.size[1]);
    assert_eq!(w * 16 % 256, 0, "a width whose rows are 256-byte aligned");
    let tex = g.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let pass = GeodesicGpu::new(&g.device, wgpu::TextureFormat::Rgba32Float);
    let mut enc = g.device.create_command_encoder(&Default::default());
    pass.render(&g.device, &mut enc, scene, output, rows, &view);
    let buf = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(w) * u64::from(h) * 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    g.queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    g.wait();
    let data = buf.slice(..).get_mapped_range().expect("mapped");
    let floats: &[f32] = bytemuck::cast_slice(&data);
    Some(floats.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect())
}

// ---------------------------------------------------------------- the f64 reference

struct Reference {
    escaped: bool,
    phi_inf: f64,
    crossings: Vec<(f64, f64)>,
}

/// Fixed-step RK4 of u'' = -u + 3 M u^2 in the angle phi from the observer, with the last step before
/// each of the first four crossings of the plane (at phi0 + k pi) shortened to land on it.
fn reference(mass: f64, r_obs: f64, b: f64, ingoing: bool, phi0: f64) -> Reference {
    let accel = |u: f64| -u + 3.0 * mass * u * u;
    let u0 = 1.0 / r_obs;
    let mut u = u0;
    let mut w = (1.0 / (b * b) - u0 * u0 + 2.0 * mass * u0 * u0 * u0).max(0.0).sqrt();
    if !ingoing {
        w = -w;
    }
    let (mut phi, mut k) = (0.0, 0usize);
    let mut out = Reference { escaped: false, phi_inf: 0.0, crossings: Vec::new() };
    for _ in 0..MAX_STEPS {
        let (mut h, mut landing) = (STEP, false);
        let target = phi0 + k as f64 * std::f64::consts::PI;
        if k < 4 && target - phi <= STEP {
            h = target - phi;
            landing = true;
        }
        let (k1u, k1w) = (w, accel(u));
        let (k2u, k2w) = (w + 0.5 * h * k1w, accel(u + 0.5 * h * k1u));
        let (k3u, k3w) = (w + 0.5 * h * k2w, accel(u + 0.5 * h * k2u));
        let (k4u, k4w) = (w + h * k3w, accel(u + h * k3u));
        let un = u + h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
        let wn = w + h / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        if un >= 0.5 / mass {
            return out;
        }
        if un <= 0.0 {
            out.escaped = true;
            out.phi_inf = phi + h * u / (u - un);
            return out;
        }
        (u, w, phi) = (un, wn, phi + h);
        if landing {
            phi = target;
            out.crossings.push((phi, 1.0 / u));
            k += 1;
        }
    }
    out
}

/// What the shader computes before the integration: the ray's plane and impact parameter, as f64.
struct RayFrame {
    b: f64,
    ingoing: bool,
    phi0: f64,
}

fn ray_frame(scene: &GeodesicScene, axis: Vec3, x: u32, y: u32) -> RayFrame {
    let v = |a: Vec3| DVec3::new(a.x as f64, a.y as f64, a.z as f64);
    let (w, h) = (scene.size[0] as f64, scene.size[1] as f64);
    let d = (v(scene.forward) * scene.focal_px as f64
        + v(scene.right) * (x as f64 + 0.5 - w * 0.5)
        + v(scene.down) * (y as f64 + 0.5 - h * 0.5))
        .normalize();
    let to_eye = v(scene.eye) - v(scene.hole);
    let r_o = to_eye.length();
    let e1 = to_eye / r_o;
    let cosa = -e1.dot(d);
    let perp = d + e1 * cosa;
    let e2 = perp.normalize();
    let m = scene.mass as f64;
    let b = r_o * perp.length() / (1.0 - 2.0 * m / r_o).sqrt();
    let z = v(axis).normalize();
    let (a, c) = (e1.dot(z), e2.dot(z));
    let mut phi0 = (-a).atan2(c);
    if phi0 < 0.0 {
        phi0 += std::f64::consts::PI;
    }
    if phi0 < 1e-6 {
        phi0 += std::f64::consts::PI;
    }
    RayFrame { b, ingoing: cosa > 0.0, phi0 }
}

// ---------------------------------------------------------------- the tests

/// An observer on the -z axis, `distance` from a hole at the origin, looking at it.
fn on_axis(distance: f32, mass: f32, size: [u32; 2], focal_px: f32, disk: Option<Disk>) -> GeodesicScene {
    camera(size, Vec3::new(0.0, 0.0, -distance), Vec3::ZERO, mass, focal_px, disk)
}

#[test]
fn the_shadow_of_the_hole_has_the_radius_of_the_critical_impact_parameter() {
    let (mass, distance) = (1.0f32, 1.0e4f32);
    // a ray at impact parameter b is at the angle b / r from the axis: 40 pixels for sqrt(27) M
    let critical = 27.0f64.sqrt() * mass as f64;
    let focal = (40.0 / (critical / distance as f64)) as f32;
    let scene = on_axis(distance, mass, [160, 160], focal, None);
    let Some(px) = render(&scene, Output::Crossings, None) else { return };
    let captured = px.iter().filter(|p| p[0] == 0.0).count();
    let radius = (captured as f64 / std::f64::consts::PI).sqrt();
    println!("shadow: {captured} pixels captured, radius {radius:.3} px, expected 40.000 px");
    assert!((radius - 40.0).abs() <= 1.0, "shadow radius {radius} px, expected 40 +- 1");
    // and along the axes the edge is within a pixel too
    let row = &px[80 * 160..81 * 160];
    let (first, last) = (row.iter().position(|p| p[0] == 0.0).unwrap(), row.iter().rposition(|p| p[0] == 0.0).unwrap());
    let half = (last - first + 1) as f64 / 2.0;
    assert!((half - 40.0).abs() <= 1.0, "the shadow is {half} px wide at its centre row");
}

#[test]
fn a_disk_seen_face_on_is_symmetric_and_redshifted_by_the_orbital_speed_and_gravity_alone() {
    let mass = 1.0f32;
    // the observer on the disk's axis, the disk from 6 M to 20 M
    let d = disk(6.0, 20.0, Vec3::new(0.0, 0.0, -1.0));
    let scene = on_axis(100.0, mass, [64, 64], 330.0, Some(d));
    let Some(px) = render(&scene, Output::FirstHit, None) else { return };
    let at = |x: usize, y: usize| px[y * 64 + x];
    let mut hits = 0;
    for y in 0..64 {
        for x in 0..32 {
            let (a, b) = (at(x, y), at(63 - x, y));
            assert_eq!(a[3] >= 0.0, b[3] >= 0.0, "pixel ({x},{y}) and its mirror differ in whether they see the disk");
            if a[3] >= 0.0 {
                hits += 1;
                assert!((a[1] - b[1]).abs() <= 2e-4 * a[1], "radius at ({x},{y}): {} against {}", a[1], b[1]);
                let expected = (1.0 - 3.0 * mass / a[1]).sqrt();
                assert!((a[0] - expected).abs() <= 2e-4, "g at ({x},{y}) is {} and sqrt(1 - 3M/r) = {expected}", a[0]);
            }
        }
    }
    assert!(hits > 500, "the disk covers part of the left half: {hits} pixels");
}

#[test]
fn the_side_of_the_disk_that_turns_toward_the_observer_is_brighter_and_bluer() {
    // far from the hole the path is nearly straight: g is 1 / (1 + v . d) for the flat direction d
    let (mass, inclination) = (0.05f32, 60.0f32.to_radians());
    let axis = Vec3::NEG_Y;
    let distance = 200.0;
    let eye = Vec3::new(inclination.sin(), -inclination.cos(), 0.0) * distance;
    let scene = camera([64, 64], eye, Vec3::ZERO, mass, 220.0, Some(disk(20.0, 40.0, axis)));
    let Some(px) = render(&scene, Output::FirstHit, None) else { return };
    let (mut checked, mut approaching, mut receding) = (0, false, false);
    for y in 0..64u32 {
        for x in 0..64u32 {
            let p = px[(y * 64 + x) as usize];
            if p[3] != 0.0 {
                continue;
            }
            // the straight ray from the eye meets the plane y = 0 at q
            let d = dir(&scene, x, y);
            let eye64 = DVec3::new(eye.x as f64, eye.y as f64, eye.z as f64);
            let t = -eye64.y / d.y;
            let q = eye64 + d * t;
            let r = q.length();
            if !(20.0..=40.0).contains(&r) {
                continue;
            }
            let omega = (mass as f64 / (r * r * r)).sqrt();
            let v = DVec3::new(0.0, -1.0, 0.0).cross(q / r) * omega * r;
            let flat = (1.0 - 3.0 * mass as f64 / r).sqrt() / (1.0 + v.dot(d));
            assert!(
                (p[0] as f64 - flat).abs() <= 4e-3,
                "g at ({x},{y}) is {} and the flat-space value is {flat}",
                p[0]
            );
            checked += 1;
            approaching |= p[0] > 1.01;
            receding |= p[0] < 0.97;
        }
    }
    assert!(checked > 300, "pixels checked: {checked}");
    assert!(approaching && receding, "both sides of the disk are seen, one blueshifted and one redshifted");
}

fn dir(scene: &GeodesicScene, x: u32, y: u32) -> DVec3 {
    let v = |a: Vec3| DVec3::new(a.x as f64, a.y as f64, a.z as f64);
    let (w, h) = (scene.size[0] as f64, scene.size[1] as f64);
    (v(scene.forward) * scene.focal_px as f64
        + v(scene.right) * (x as f64 + 0.5 - w * 0.5)
        + v(scene.down) * (y as f64 + 0.5 - h * 0.5))
        .normalize()
}

#[test]
fn a_disk_seen_at_75_degrees_has_a_secondary_image_above_and_below_the_shadow() {
    let mass = 1.0f32;
    let inclination = 75.0f32.to_radians();
    let axis = Vec3::NEG_Y;
    let eye = Vec3::new(inclination.sin(), -inclination.cos(), 0.0) * 100.0;
    // 100 M away, the shadow is about 5.2 / 100 rad: 52 px at 1000 px per radian
    let scene = camera([160, 160], eye, Vec3::ZERO, mass, 1000.0, Some(disk(6.0, 20.0, axis)));
    let Some(px) = render(&scene, Output::FirstHit, None) else { return };
    let order = |x: usize, y: usize| px[y * 160 + x][3];
    let shadow = 27.0f64.sqrt() / 100.0 * 1000.0;
    let (mut above, mut below) = (0, 0);
    for y in 0..160usize {
        for x in 0..160usize {
            if order(x, y) < 1.0 {
                continue;
            }
            // the image's centre is the hole
            let (dx, dy) = (x as f64 + 0.5 - 80.0, y as f64 + 0.5 - 80.0);
            let rho = (dx * dx + dy * dy).sqrt();
            if rho < 1.6 * shadow && rho > 0.9 * shadow {
                if dy < 0.0 {
                    above += 1;
                } else {
                    below += 1;
                }
            }
        }
    }
    println!("pixels showing the disk after one more crossing, near the shadow: {above} above, {below} below");
    assert!(above > 40, "a secondary image above the shadow: {above} pixels");
    assert!(below > 40, "a secondary image below the shadow: {below} pixels");
}

#[test]
fn the_shader_agrees_with_an_f64_reference_of_the_same_integration() {
    let mass = 1.0f32;
    let axis = Vec3::new(0.2, -1.0, 0.1).normalize();
    let eye = Vec3::new(-30.0, -12.0, -45.0);
    // 64 x 40 pixels covering about 30 degrees: the shadow, the lensed rim and the far field
    let scene = camera([64, 40], eye, Vec3::new(1.0, 0.5, 0.0), mass, 150.0, Some(disk(6.0, 14.0, axis)));
    let (Some(a), Some(b)) = (render(&scene, Output::Crossings, None), render(&scene, Output::MoreCrossings, None))
    else {
        return;
    };
    let (mut worst_phi, mut worst_r, mut worst_far, mut compared, mut flips) = (0.0f64, 0.0f64, 0.0f64, 0, 0);
    for y in 0..40u32 {
        for x in 0..64u32 {
            let f = ray_frame(&scene, axis, x, y);
            let reference =
                reference(mass as f64, (eye - Vec3::new(1.0, 0.5, 0.0)).length() as f64, f.b, f.ingoing, f.phi0);
            let (sa, sb) = (a[(y * 64 + x) as usize], b[(y * 64 + x) as usize]);
            if (sa[0] == 1.0) != reference.escaped {
                // only the rays within a thousandth of the critical impact parameter may differ
                let critical = 27.0f64.sqrt();
                assert!(
                    (f.b / critical - 1.0).abs() < 1e-3,
                    "the outcome at ({x},{y}) differs away from the critical ray, b = {}",
                    f.b
                );
                flips += 1;
                continue;
            }
            compared += 1;
            if reference.escaped {
                let e = (sa[1] as f64 - reference.phi_inf).abs() / reference.phi_inf.abs().max(1.0);
                worst_phi = worst_phi.max(e);
                assert!(e <= 2e-4, "phi_inf at ({x},{y}): {} against {}", sa[1], reference.phi_inf);
            }
            let shader_r = [sa[2], sa[3], sb[0], sb[1]];
            assert_eq!(sb[2] as usize, reference.crossings.len(), "crossings counted at ({x},{y})");
            for (k, (_, r)) in reference.crossings.iter().enumerate() {
                let e = (shader_r[k] as f64 - r).abs() / r;
                // A crossing after the ray has wound round the hole (k >= 2), or one beyond a few times the
                // disk's outer radius where a ray is close to leaving, is ill-conditioned: a relative change of
                // 1e-7 in the f32 direction (a different impact parameter) grows by about e^phi along the ray
                // near the critical curve, so these two kinds are held to a tenth of a percent.
                if *r <= 56.0 && k <= 1 {
                    worst_r = worst_r.max(e);
                    assert!(e <= 2e-4, "radius of crossing {k} at ({x},{y}): {} against {r}", shader_r[k]);
                } else {
                    worst_far = worst_far.max(e);
                    assert!(e <= 2e-3, "radius of crossing {k} at ({x},{y}): {} against {r}", shader_r[k]);
                }
            }
        }
    }
    println!("{compared} rays compared, {flips} at the critical curve; worst relative error: phi_inf {worst_phi:.2e}, crossing radius {worst_r:.2e} (crossings after the first two, or beyond 56 M: {worst_far:.2e})");
    assert!(compared > 2000);
}

#[test]
fn no_pixel_is_not_a_number_and_a_ray_at_the_hole_is_black() {
    let mass = 1.0f32;
    let scene =
        camera([160, 80], Vec3::new(0.0, -3.0, -10.0), Vec3::ZERO, mass, 260.0, Some(disk(6.0, 12.0, Vec3::NEG_Y)));
    for output in [Output::Picture, Output::Crossings, Output::MoreCrossings, Output::FirstHit] {
        let Some(px) = render(&scene, output, None) else { return };
        let bad = px.iter().filter(|p| p.iter().any(|c| !c.is_finite())).count();
        assert_eq!(bad, 0, "{output:?}: {bad} pixels are not finite");
    }
    // an exact hit on the hole: the ray through the middle of the image falls in
    let mut head_on = on_axis(50.0, mass, [16, 2], 100.0, None);
    head_on.samples = 1;
    let Some(px) = render(&head_on, Output::Crossings, None) else { return };
    assert_eq!(px[8][0], 0.0, "a ray aimed at the centre of the hole is captured: {:?}", px[8]);
}

#[test]
fn drawing_in_bands_changes_nothing() {
    let mass = 1.0f32;
    let mut d = disk(6.0, 14.0, Vec3::new(0.1, -1.0, 0.0));
    d.pattern = Pattern::Clumps;
    d.contrast = 0.6;
    d.time = 3.0;
    let mut scene = camera([64, 40], Vec3::new(0.0, -20.0, -50.0), Vec3::ZERO, mass, 220.0, Some(d));
    scene.samples = 4;
    let (Some(whole), Some(banded)) = (render(&scene, Output::Picture, None), render(&scene, Output::Picture, Some(7)))
    else {
        return;
    };
    assert!(whole.iter().any(|p| p[0] > 0.0), "the picture is not empty");
    assert!(whole == banded, "the bands differ from the whole frame");
}

/// A face-on disk of clumps from 6 M to 20 M, seen from 100 M, at geometric time `time`.
fn clumpy(seed: u32, time: f32) -> GeodesicScene {
    let mut d = disk(6.0, 20.0, Vec3::new(0.0, 0.0, -1.0));
    d.pattern = Pattern::Clumps;
    d.contrast = 0.8;
    d.seed = seed;
    d.time = time;
    on_axis(100.0, 1.0, [64, 64], 330.0, Some(d))
}

/// The shader's integer hash, for the arms' phase.
fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let word = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (word >> 22) ^ word
}

#[test]
fn the_pattern_of_the_disk_turns_at_the_keplerian_rate_of_each_radius() {
    // spiral arms have a closed form: exp(1.5 c cos(m (psi - Omega t) + 3 ln(r / r_in) + phase)), so the
    // brightness of a pixel at time t over its brightness at time 0 is known from its radius and azimuth
    let (seed, contrast, time) = (4u32, 0.8f32, 40.0f32);
    let spiral = |t: f32| {
        let mut d = disk(6.0, 20.0, Vec3::new(0.0, 0.0, -1.0));
        d.pattern = Pattern::Spiral;
        d.contrast = contrast;
        d.seed = seed;
        d.time = t;
        on_axis(100.0, 1.0, [64, 64], 330.0, Some(d))
    };
    let (Some(before), Some(after), Some(hits)) = (
        render(&spiral(0.0), Output::Picture, None),
        render(&spiral(time), Output::Picture, None),
        render(&spiral(0.0), Output::FirstHit, None),
    ) else {
        return;
    };
    let arms = (2 + seed % 3) as f64;
    let phase = std::f64::consts::TAU * ((pcg(seed + 11) >> 8) as f64 / 16777216.0);
    let (mut checked, mut worst, mut turned) = (0, 0.0f64, 0.0f64);
    for i in 0..64 * 64 {
        if hits[i][3] < 0.0 {
            continue;
        }
        let (r, psi) = (hits[i][1] as f64, hits[i][2] as f64);
        let omega = (1.0 / (r * r * r)).sqrt();
        let arg = |t: f64| arms * (psi - omega * t) + 3.0 * (r / 6.0).ln() + phase;
        let expected = (1.5 * contrast as f64 * (arg(time as f64).cos() - arg(0.0).cos())).exp();
        let measured = after[i][0] as f64 / before[i][0] as f64;
        worst = worst.max((measured / expected - 1.0).abs());
        turned = turned.max((expected - 1.0).abs());
        checked += 1;
    }
    println!("{checked} pixels: the brightness ratio after {time} agrees with the arms' closed form to {worst:.2e} (largest change {turned:.2})");
    assert!(checked > 1000);
    assert!(turned > 0.5, "the pattern moves enough to tell: {turned}");
    assert!(worst < 2e-3, "relative error of the turned pattern: {worst}");
}

#[test]
fn the_same_seed_gives_the_same_pattern_and_another_seed_another() {
    let (Some(a), Some(b), Some(c)) = (
        render(&clumpy(5, 7.0), Output::Picture, None),
        render(&clumpy(5, 7.0), Output::Picture, None),
        render(&clumpy(6, 7.0), Output::Picture, None),
    ) else {
        return;
    };
    assert!(a == b, "the same seed and time give the same picture");
    let differ = a.iter().zip(&c).filter(|(p, q)| (p[0] - q[0]).abs() > 0.02 * p[0].max(0.01)).count();
    assert!(differ > 200, "another seed changes the pattern: {differ} pixels");
}

/// The axes of the disk's plane the pass derives from its spin axis, as unit vectors.
fn plane_axes(axis: Vec3) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let z = axis.normalize();
    let helper = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = (helper - z * helper.dot(z)).normalize();
    let y = z.cross(x);
    (x.to_array().map(f64::from), y.to_array().map(f64::from), z.to_array().map(f64::from))
}

#[test]
fn the_shader_agrees_with_the_cpu_reference_image_pixel_by_pixel() {
    use sr_sim::gr::image::{self, Class};
    let (mass, inclination) = (1.0f64, 75.0f64.to_radians());
    let camera = image::Camera::orbiting(60.0, inclination, 40.0f64.to_radians(), [96, 60]);
    let (dx, dy, dz) = plane_axes(Vec3::Y);
    let reference_disk = image::Disk { r_in: 6.0, r_out: 16.0, dx, dy, dz };
    let to_vec = |a: [f64; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
    let scene = GeodesicScene {
        size: [96, 60],
        eye: to_vec(camera.eye),
        right: to_vec(camera.right),
        down: to_vec(camera.down),
        forward: to_vec(camera.fwd),
        focal_px: camera.focal as f32,
        hole: Vec3::ZERO,
        mass: mass as f32,
        disk: Some(disk(6.0, 16.0, Vec3::Y)),
        samples: 1,
        star_seed: 0,
        exposure: 1.0,
        encode_srgb: false,
    };
    let (Some(crossings), Some(hits)) =
        (render(&scene, Output::Crossings, None), render(&scene, Output::FirstHit, None))
    else {
        return;
    };
    let (mut class_flips, mut compared, mut worst_r, mut worst_g, mut worst_psi, mut worst_phi) =
        (0, 0, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut by_class = [0usize; 3];
    for y in 0..60usize {
        for x in 0..96usize {
            let want = image::trace_pixel(&camera, mass, Some(&reference_disk), x, y);
            let (c, h) = (crossings[y * 96 + x], hits[y * 96 + x]);
            let got = if h[3] >= 0.0 {
                Class::Disk
            } else if c[0] == 1.0 {
                Class::Background
            } else {
                Class::Captured
            };
            if got != want.class {
                class_flips += 1;
                continue;
            }
            compared += 1;
            match want.class {
                Class::Disk => {
                    by_class[2] += 1;
                    assert_eq!(h[3] as usize, want.order.unwrap(), "which crossing at ({x},{y})");
                    let er = (h[1] as f64 - want.r).abs() / want.r;
                    let eg = (h[0] as f64 - want.g).abs() / want.g;
                    let mut dpsi = (h[2] as f64 - want.psi).abs();
                    dpsi = dpsi.min(std::f64::consts::TAU - dpsi);
                    (worst_r, worst_g, worst_psi) = (worst_r.max(er), worst_g.max(eg), worst_psi.max(dpsi));
                    assert!(
                        er <= 2e-3 && eg <= 2e-3 && dpsi <= 2e-3,
                        "disk pixel ({x},{y}): shader {h:?}, reference {want:?}"
                    );
                }
                Class::Background => {
                    by_class[1] += 1;
                    let e = (c[1] as f64 - want.phi_inf).abs() / want.phi_inf.abs().max(1.0);
                    worst_phi = worst_phi.max(e);
                    assert!(e <= 2e-4, "escape angle at ({x},{y}): {} against {}", c[1], want.phi_inf);
                }
                Class::Captured => by_class[0] += 1,
            }
        }
    }
    println!(
        "{compared} pixels agree in class ({} captured, {} background, {} disk), {class_flips} differ; worst relative error: escape angle {worst_phi:.2e}, disk radius {worst_r:.2e}, g {worst_g:.2e}, azimuth {worst_psi:.2e} rad",
        by_class[0], by_class[1], by_class[2]
    );
    assert!(class_flips <= 6, "{class_flips} pixels differ in class (f32 against f64 at a boundary)");
    assert!(by_class.iter().all(|n| *n > 100), "every class is present: {by_class:?}");
}

#[test]
fn the_shader_matches_the_single_precision_reference_integrator_far_from_the_critical_curve() {
    // the same arithmetic in the same order: for rays whose impact parameter is not within 5 % of the critical one
    // (where an f32 rounding of the direction grows exponentially) the f32 reference and the shader differ only by
    // how the GPU compiler contracts and rounds the operations
    let mass = 1.0f32;
    let axis = Vec3::new(0.2, -1.0, 0.1).normalize();
    let eye = Vec3::new(-30.0, -12.0, -45.0);
    let hole = Vec3::new(1.0, 0.5, 0.0);
    let scene = camera([64, 40], eye, hole, mass, 150.0, Some(disk(6.0, 14.0, axis)));
    let (Some(a), Some(b)) = (render(&scene, Output::Crossings, None), render(&scene, Output::MoreCrossings, None))
    else {
        return;
    };
    let r_obs = (eye - hole).length();
    let critical = 27.0f32.sqrt();
    let (mut worst, mut worst_far, mut worst_phi, mut within_1e6, mut total) = (0.0f64, 0.0f64, 0.0f64, 0usize, 0usize);
    for y in 0..40u32 {
        for x in 0..64u32 {
            let f = ray_frame(&scene, axis, x, y);
            if (f.b as f32 / critical - 1.0).abs() < 0.05 {
                continue;
            }
            let reference = sr_sim::gr::f32::trace(mass, r_obs, f.b as f32, f.ingoing, f.phi0 as f32, 4);
            let (sa, sb) = (a[(y * 64 + x) as usize], b[(y * 64 + x) as usize]);
            assert_eq!((sa[0] == 1.0), reference.outcome == sr_sim::gr::Outcome::Escaped, "outcome at ({x},{y})");
            if sa[0] == 1.0 {
                worst_phi =
                    worst_phi.max((sa[1] - reference.phi_inf).abs() as f64 / reference.phi_inf.abs().max(1.0) as f64);
            }
            let shader_r = [sa[2], sa[3], sb[0], sb[1]];
            for (k, (_, r)) in reference.crossings.iter().enumerate() {
                let e = ((shader_r[k] - r).abs() / r) as f64;
                // beyond a few disk radii a ray is about to leave and the radius changes quickly with the angle
                if *r <= 56.0 {
                    worst = worst.max(e);
                } else {
                    worst_far = worst_far.max(e);
                }
                total += 1;
                within_1e6 += usize::from(e <= 1e-6);
            }
        }
    }
    println!(
        "{total} crossing radii against gr::f32::trace: {within_1e6} within 1e-6; worst {worst:.2e} up to 56 M, {worst_far:.2e} beyond; worst escape angle {worst_phi:.2e}"
    );
    assert!(total > 500);
    assert!(worst <= 3e-5 && worst_phi <= 5e-6, "worst crossing radius {worst}, escape angle {worst_phi}");
    assert!(worst_far <= 2e-4, "worst far crossing radius {worst_far}");
}
