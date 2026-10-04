//! Volume lighting from per-light grids (`medium@lighting="grid"`) against the exact shadow march.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
use sr_gpu::three::{Light3, LightKind, Scene3, ThreeEngine};
use sr_gpu::volume::VolumeDraw;
use sr_volume::medium::{Bounds, March, Medium, Optical};
use sr_volume::{SparseGrid, Transform};

/// A soft ball of density: 1 at the centre, falling to 0 at `radius`, in voxels of `voxel` units.
fn ball_cache(name: &str, center: [f32; 3], radius: f32, voxel: f64) -> PathBuf {
    let scale = Transform::new(glam::DMat4::from_scale(glam::DVec3::splat(voxel)).to_cols_array()).unwrap();
    let mut grid = SparseGrid::new(scale, 0.0, 100_000).unwrap();
    let span = (radius / voxel as f32).ceil() as i32 + 1;
    let at = |c: f32| (c / voxel as f32).round() as i32;
    for z in at(center[2]) - span..=at(center[2]) + span {
        for y in at(center[1]) - span..=at(center[1]) + span {
            for x in at(center[0]) - span..=at(center[0]) + span {
                let d = Vec3::new(
                    x as f32 * voxel as f32 - center[0],
                    y as f32 * voxel as f32 - center[1],
                    z as f32 * voxel as f32 - center[2],
                )
                .length();
                if d <= radius {
                    grid.set([x, y, z], 1.0 - d / radius).unwrap();
                }
            }
        }
    }
    let mut cache = sr_volume::Volume::new();
    cache.insert("density", grid).unwrap();
    let path = common::fixtures().join(name);
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    path
}

/// [`ball_cache`] with a constant velocity field, for advected sequences.
fn moving_ball_cache(name: &str, center: [f32; 3], velocity: [f32; 3]) -> PathBuf {
    let path = ball_cache(name, center, 6.0, 1.0);
    let mut cache = sr_volume::Volume::read(std::fs::File::open(&path).unwrap(), Default::default()).unwrap();
    for (axis, v) in ["velocity.x", "velocity.y", "velocity.z"].iter().zip(velocity) {
        cache.insert(axis, SparseGrid::new(Transform::identity(), v, 0).unwrap()).unwrap();
    }
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    path
}

struct Setup<'a> {
    medium: &'a str,
    lights: &'a str,
    extra: &'a str,
    camera: &'a str,
    bounds: f32,
    samples: u32,
    size: u32,
    time: f64,
    interpolation: &'a str,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Setup {
            medium: r##"<medium extinction="0.6" albedo="#FFFFFF" stepSize="0.5" maxSteps="2048"/>"##,
            lights: r##"<light id="sun" type="directional" yaw="-35" pitch="-50" intensity="3"/>"##,
            extra: "",
            camera: r##"<camera id="camera" x="0" y="0" z="-30" fov="45" renderer="pathtrace" pathSamples="{samples}" maxBounces="2" denoise="false"/>"##,
            bounds: 8.0,
            samples: 4,
            size: 48,
            time: 0.0,
            interpolation: "linear",
        }
    }
}

fn xml(cache: &std::path::Path, s: &Setup) -> String {
    let b = s.bounds;
    let camera = s.camera.replace("{samples}", &s.samples.to_string());
    format!(
        r##"<scene version="1.3"><project width="{size}" height="{size}" fps="10" duration="1" background="#00000000"/>
      <assets><volume id="smoke" src="{cache}"{sequence} boundsMinX="-{b}" boundsMinY="-{b}" boundsMinZ="-{b}" boundsMaxX="{b}" boundsMaxY="{b}" boundsMaxZ="{b}"/></assets>
      <materials><material id="plateMat" baseColor="#000000" roughness="1"/></materials>
      <composition>{camera}{extra}<object3D id="cloud" primitive="volume" volume="smoke">{medium}</object3D></composition>
      <lights>{lights}</lights></scene>"##,
        size = s.size,
        cache = cache.display(),
        extra = s.extra,
        medium = s.medium,
        lights = s.lights,
        sequence = if cache.to_string_lossy().contains('%') {
            let velocity = if s.interpolation == "advect" {
                r#" velocityGridX="velocity.x" velocityGridY="velocity.y" velocityGridZ="velocity.z""#
            } else {
                ""
            };
            format!(r#" first="0" last="1" fps="1" interpolation="{}"{velocity}"#, s.interpolation)
        } else {
            String::new()
        },
    )
}

fn render(gpu: &sr_gpu::Gpu, cache: &std::path::Path, s: &Setup) -> common::Rendered {
    let doc = sr_model::load_str(
        &xml(cache, s),
        &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) },
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let shot = common::render_times_on(gpu.clone(), &doc, &[s.time]).unwrap();
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    shot
}

fn grid(medium: &str) -> String {
    medium.replace("<medium ", r#"<medium lighting="grid" "#)
}

fn mean(r: &common::Rendered) -> f64 {
    r.px.iter().map(|p| f64::from(p[0] + p[1] + p[2])).sum::<f64>() / r.px.len() as f64
}

/// Each analytic light's grid reproduces the exact march: with no dome nothing is random, so the
/// images differ only by the grid's interpolation.
#[test]
fn grid_lighting_matches_the_exact_march_for_every_light_type() {
    let Some(gpu) = common::gpu() else { return };
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    let lights = [
        ("directional", r##"<light id="l" type="directional" yaw="-35" pitch="-50" intensity="3"/>"##),
        ("point", r##"<light id="l" type="point" x="-30" y="-30" z="-25" intensity="6"/>"##),
        (
            "spot",
            r##"<light id="l" type="spot" x="-12" y="-12" z="-30" yaw="20" pitch="20" spotAngle="90" intensity="6"/>"##,
        ),
        (
            "rect-area",
            r##"<light id="l" type="rect-area" x="-30" y="-30" z="-25" yaw="-35" pitch="-30" width="8" height="8" intensity="6"/>"##,
        ),
        ("sphere-area", r##"<light id="l" type="sphere-area" x="-30" y="-30" z="-25" radius="4" intensity="6"/>"##),
    ];
    for (name, light) in lights {
        let base = Setup { lights: light, samples: 1, ..Default::default() };
        let exact = render(&gpu, &cache, &base);
        let gridded = render(&gpu, &cache, &Setup { medium: &grid(base.medium), ..base });
        assert!(mean(&exact) > 0.02, "{name}: the lit ball is visible ({})", mean(&exact));
        let psnr = common::psnr(&exact.px, &gridded.px);
        // area lights are sampled at random points by the exact march and at their centre by the grid
        let floor = if name.ends_with("area") { 38.0 } else { 45.0 };
        assert!(psnr >= floor, "{name}: grid differs from exact: {psnr:.1} dB");
    }
}

/// Omitting `lighting` and writing `lighting="exact"` are the same path, bit for bit, dome included.
#[test]
fn exact_lighting_is_the_default_and_unchanged() {
    let Some(gpu) = common::gpu() else { return };
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    let lights = r##"<light id="sun" type="directional" yaw="-35" pitch="-50" intensity="3"/><light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    let base = Setup { lights, samples: 8, ..Default::default() };
    let plain = render(&gpu, &cache, &base);
    let spelled = render(
        &gpu,
        &cache,
        &Setup { medium: &base.medium.replace("<medium ", r#"<medium lighting="exact" "#), ..base },
    );
    assert_eq!(plain.px, spelled.px, "lighting=\"exact\" must not change a pixel");
}

/// The grids are built from fixed data in a fixed order, so two renders agree to the bit.
#[test]
fn grid_lighting_is_deterministic() {
    let Some(gpu) = common::gpu() else { return };
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    let lights = r##"<light id="sun" type="directional" yaw="-35" pitch="-50" intensity="3"/><light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    let setup = Setup { lights, medium: &grid(Setup::default().medium), samples: 8, ..Default::default() };
    assert_eq!(render(&gpu, &cache, &setup).px, render(&gpu, &cache, &setup).px);
}

/// The dome, built as pre-integrated radiance for isotropic scattering and as one grid per fixed
/// direction for anisotropic scattering, converges to what the random dome sample averages to.
/// Not run on a software adapter: its exact reference sums 256 samples per pixel of long marches,
/// and llvmpipe (LLVM 15) drops samples from invocations that run that long.
#[test]
fn grid_dome_matches_the_exact_dome_on_average_for_isotropic_and_anisotropic_media() {
    let Some(gpu) = common::gpu() else { return };
    if gpu.info.device_type == wgpu::DeviceType::Cpu {
        eprintln!("skipped: the software adapter drops samples from long invocations");
        return;
    }
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    let lights = r##"<light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    for anisotropy in ["0", "0.5"] {
        let medium = format!(
            r##"<medium extinction="0.6" albedo="#FFFFFF" anisotropy="{anisotropy}" stepSize="0.5" maxSteps="2048"/>"##
        );
        let base = Setup { lights, medium: &medium, samples: 256, size: 24, ..Default::default() };
        let exact = render(&gpu, &cache, &base);
        let gridded = render(&gpu, &cache, &Setup { medium: &grid(&medium), ..base });
        let (a, b) = (mean(&exact), mean(&gridded));
        assert!((a - b).abs() < 0.03 * a, "anisotropy {anisotropy}: exact mean {a}, grid mean {b}");
        let psnr = common::psnr(&exact.px, &gridded.px);
        assert!(psnr >= 40.0, "anisotropy {anisotropy}: {psnr:.1} dB");
    }
}

/// A medium that asks for more memory than allowed is an error naming the attribute, never a
/// silent exact render; a larger allowance renders.
#[test]
fn grid_over_its_memory_is_a_render_error() {
    let Some(gpu) = common::gpu() else { return };
    // 8 units at voxel 0.06: 134 nodes a side, tens of MiB of grids
    let cache = ball_cache("grid_fine_ball.srvol", [0.0; 3], 6.0, 0.06);
    let medium = grid(Setup::default().medium).replace("<medium ", r#"<medium lightGridMemoryMiB="1" "#);
    let setup = Setup { medium: &medium, samples: 1, size: 16, bounds: 4.0, ..Default::default() };
    let doc = sr_model::load_str(
        &xml(&cache, &setup),
        &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) },
    )
    .unwrap();
    let shot = common::render_times_on(gpu.clone(), &doc, &[0.0]).unwrap();
    assert!(
        shot.stats.errors.iter().any(|e| e.contains("lightGridMemoryMiB allows 1.0 MiB")),
        "{:?}",
        shot.stats.errors
    );
    let roomy =
        grid(Setup::default().medium).replace("<medium ", r#"<medium lightGridMemoryMiB="512" lightGridCell="4" "#);
    render(&gpu, &cache, &Setup { medium: &roomy, samples: 1, size: 16, bounds: 4.0, ..Default::default() });
}

/// A surface inside the domain shades the volume through the grid's baked visibility.
#[test]
fn a_surface_inside_the_domain_shadows_the_volume_in_the_grid() {
    let Some(gpu) = common::gpu() else { return };
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    // a black plate inside the domain, in front of the ball's left side, with the light behind the camera
    let plate = r##"<object3D id="plate" primitive="box" x="-3" y="0" z="-7" width="5" height="12" depth="0.5" material="plateMat"/>"##;
    let lights = r##"<light id="sun" type="directional" intensity="3" castShadow="true"/>"##;
    let free = Setup { lights, samples: 1, ..Default::default() };
    let shaded = Setup { extra: plate, ..free };
    let exact = render(&gpu, &cache, &shaded);
    let gridded = render(&gpu, &cache, &Setup { medium: &grid(shaded.medium), ..shaded });
    // the plate and its shadow change the picture, and the grid agrees about both
    assert!(
        common::psnr(&render(&gpu, &cache, &free).px, &exact.px) < 40.0,
        "the plate is visible in the exact render"
    );
    let psnr = common::psnr(&exact.px, &gridded.px);
    assert!(psnr >= 35.0, "plate: grid differs from exact: {psnr:.1} dB");
}

/// Two overlapping media light each other's shadows through one grid.
#[test]
fn overlapping_media_share_one_grid() {
    let Some(gpu) = common::gpu() else { return };
    let first = ball_cache("grid_ball_a.srvol", [-2.0, 0.0, 0.0], 5.0, 1.0);
    let second = ball_cache("grid_ball_b.srvol", [2.0, 0.0, 0.0], 5.0, 1.0);
    let body = |lighting: bool| {
        let medium = if lighting { grid(Setup::default().medium) } else { Setup::default().medium.to_string() };
        let xml = format!(
            r##"<scene version="1.3"><project width="48" height="48" fps="10" duration="1" background="#00000000"/>
          <assets><volume id="a" src="{}" boundsMinX="-8" boundsMinY="-8" boundsMinZ="-8" boundsMaxX="8" boundsMaxY="8" boundsMaxZ="8"/>
          <volume id="b" src="{}" boundsMinX="-8" boundsMinY="-8" boundsMinZ="-8" boundsMaxX="8" boundsMaxY="8" boundsMaxZ="8"/></assets>
          <composition><camera id="camera" x="0" y="0" z="-30" fov="45" renderer="pathtrace" pathSamples="1" maxBounces="2" denoise="false"/>
          <object3D id="ca" primitive="volume" volume="a">{medium}</object3D>
          <object3D id="cb" primitive="volume" volume="b">{medium}</object3D></composition>
          <lights><light id="sun" type="directional" yaw="-35" pitch="-50" intensity="3"/></lights></scene>"##,
            first.display(),
            second.display()
        );
        let doc = sr_model::load_str(
            &xml,
            &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) },
        )
        .unwrap();
        let shot = common::render_times_on(gpu.clone(), &doc, &[0.0]).unwrap();
        assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
        shot
    };
    let (exact, gridded) = (body(false), body(true));
    let psnr = common::psnr(&exact.px, &gridded.px);
    assert!(psnr >= 45.0, "overlapping media: {psnr:.1} dB");
}

/// Camera inside the volume: the grid is in world space and does not care where the camera is.
#[test]
fn a_camera_inside_the_volume_is_lit_from_the_grid() {
    let Some(gpu) = common::gpu() else { return };
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    let camera = r##"<camera id="camera" x="0" y="0" z="-3" fov="60" renderer="pathtrace" pathSamples="{samples}" maxBounces="2" denoise="false"/>"##;
    let base = Setup { camera, samples: 1, ..Default::default() };
    let exact = render(&gpu, &cache, &base);
    let gridded = render(&gpu, &cache, &Setup { medium: &grid(base.medium), ..base });
    assert!(mean(&exact) > 0.05);
    let psnr = common::psnr(&exact.px, &gridded.px);
    assert!(psnr >= 45.0, "camera inside: {psnr:.1} dB");
}

/// A medium blended between two frames, or advected along a velocity field, is lit from grids built
/// from the same blended density, so they agree with the exact march at fractional times.
#[test]
fn media_with_a_second_frame_or_advection_are_lit_from_the_grid() {
    let Some(gpu) = common::gpu() else { return };
    let centers = [[-3.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
    for (frame, center) in centers.iter().enumerate() {
        moving_ball_cache(&format!("grid_seq_{frame:02}.srvol"), *center, [2.0, 0.0, 0.0]);
    }
    let pattern = common::fixtures().join("grid_seq_%02d.srvol");
    for interpolation in ["linear", "advect"] {
        let base = Setup { samples: 1, time: 0.5, interpolation, ..Default::default() };
        if interpolation == "advect" && gpu.info.device_type == wgpu::DeviceType::Cpu {
            // llvmpipe (LLVM 15) cuts short the exact march of an advected medium with shadows, so
            // it cannot serve as the reference there; the NVIDIA adapter agrees with the grid
            eprintln!("skipped advect: the software adapter truncates long marches");
            continue;
        }
        let exact = render(&gpu, &pattern, &base);
        let gridded = render(&gpu, &pattern, &Setup { medium: &grid(base.medium), ..base });
        assert!(mean(&exact) > 0.02, "{interpolation}: the blended ball is visible");
        let psnr = common::psnr(&exact.px, &gridded.px);
        assert!(psnr >= 45.0, "{interpolation}: grid differs from exact: {psnr:.1} dB");
    }
}

/// Tiles share the grids that were built once before them: a tiled frame equals the whole frame.
#[test]
fn tiled_frames_equal_whole_frames_in_grid_mode() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let smp = g.device.create_sampler(&Default::default());
    let input = PtInputs { env: &black, backdrop: None, black: &black, sampler: &smp };
    let size = [200, 120];
    let cache = Arc::new({
        let scale = Transform::identity();
        let mut grid = SparseGrid::new(scale, 0.0, 128).unwrap();
        for z in -6..=6 {
            for y in -6..=6 {
                for x in -6..=6 {
                    let r = ((x * x + y * y + z * z) as f32).sqrt();
                    if r <= 6.0 {
                        grid.set([x, y, z], 1.0 - r / 6.0).unwrap();
                    }
                }
            }
        }
        grid
    });
    let medium = Medium::new(
        cache,
        Some(Bounds::new([-8.0; 3], [8.0; 3]).unwrap()),
        Transform::identity(),
        Optical { extinction: 0.6, albedo: [1.0; 3], ..Default::default() },
    )
    .unwrap();
    let volume = VolumeDraw::new(Arc::new(medium), March { step_size: 0.5, max_steps: 4096 })
        .unwrap()
        .with_light_grid(1, 64, 128)
        .unwrap();
    let scene = Scene3 {
        cam: resolve(
            &CameraParams { position: Some(Vec3::new(0.0, 0.0, -30.0)), ..Default::default() },
            size[0] as f32,
            size[1] as f32,
        ),
        clip_fix: Mat4::IDENTITY,
        size,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: Vec::new(),
        lights: vec![Light3 {
            kind: LightKind::Directional,
            pos: Vec3::ZERO,
            dir: Vec3::new(0.4, 0.5, 0.7).normalize(),
            right: Vec3::X,
            color: Vec3::splat(std::f32::consts::PI),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: false,
            softness: 0.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0; 3],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        splats: Vec::new(),
        volumes: vec![volume],
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: Some(PathOpts { samples: 2, bounces: 2, denoise: false }),
    };
    assert!(pathtrace::limit_note(&scene, &g.device.limits()).is_none());
    let data = pathtrace::build(&scene);
    let render = |tile_bytes: Option<u64>| {
        let mut pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
        if let Some(bytes) = tile_bytes {
            pt.limit_buffers(bytes).unwrap();
        }
        let target = eng.target(size);
        let mut enc = g.device.create_command_encoder(&Default::default());
        pathtrace::render(
            &pt,
            &g.device,
            &mut enc,
            &scene,
            &data,
            scene.path.unwrap(),
            &input,
            &target.create_view(&Default::default()),
        );
        g.queue.submit([enc.finish()]);
        eng.read(&target)
    };
    let whole = render(None);
    assert!(whole.iter().any(|p| p[3] > 0.1), "the ball is visible");
    let tiled = render(Some(1_200_000));
    assert_eq!(whole, tiled, "tiles must not change a pixel in grid mode");
}

/// An area light is sampled at a random point by the exact march and at its centre by the grid, so
/// near the volume the grid's shadows lose the exact march's penumbra. The error is bounded, and
/// grows as the light nears. Measured on NVIDIA against 256 exact samples (8-unit rectangle, radius-4
/// sphere, ball of radius 6): 65 dB at 60 units, 53-56 dB at 14, 43 dB (rectangle) and 50 dB (sphere) at 9.
#[test]
fn an_area_light_near_the_volume_has_a_known_error() {
    let Some(gpu) = common::gpu() else { return };
    if gpu.info.device_type == wgpu::DeviceType::Cpu {
        eprintln!("skipped: the software adapter drops samples from long invocations");
        return;
    }
    let cache = ball_cache("grid_ball.srvol", [0.0; 3], 6.0, 1.0);
    // (kind, distance of the light's centre from the ball's centre, least PSNR against 256 exact samples)
    for (kind, distance, floor) in [
        ("rect-area", 60.0, 60.0),
        ("rect-area", 14.0, 48.0),
        ("rect-area", 9.0, 38.0),
        ("sphere-area", 60.0, 60.0),
        ("sphere-area", 14.0, 50.0),
        ("sphere-area", 9.0, 45.0),
    ] {
        let light = format!(
            r##"<light id="l" type="{kind}" x="-4" y="-4" z="-{distance}" width="8" height="8" radius="4" intensity="{}"/>"##,
            0.06 * distance * distance / 10.0
        );
        let base = Setup { lights: &light, samples: 256, size: 32, ..Default::default() };
        let exact = render(&gpu, &cache, &base);
        let gridded = render(&gpu, &cache, &Setup { medium: &grid(base.medium), ..base });
        let psnr = common::psnr(&exact.px, &gridded.px);
        assert!(psnr >= floor, "{kind} at {distance}: {psnr:.1} dB");
    }
}
