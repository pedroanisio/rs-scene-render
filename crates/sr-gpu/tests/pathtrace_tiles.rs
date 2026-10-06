//! Tiling must preserve camera rays, sampling, denoising and pixel placement.

mod common;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, MaterialParams};
use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

#[test]
fn uhd_document_renders_pathtraced_without_fallback() {
    let d = common::doc(
        r##"width="3840" height="2160" background="#00000000""##,
        r##"<materials><material id="m" baseColor="#D04020" unlit="true"/></materials>"##,
        r#"<camera id="cam" x="1920" y="1080" z="-3000" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/>
           <object3D id="box" primitive="box" x="1920" y="1080" width="1000" height="800" depth="50" material="m"/>"#,
    );
    let Some(r) = common::render(&d) else { return };
    assert_eq!(r.size, [3840, 2160]);
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    let center = r.at(1920, 1080);
    assert!(center[0] > 0.1 && center[0] > center[1] * 2.0 && center[3] > 0.99, "{center:?}");
    assert_eq!(r.at(0, 0)[3], 0.0);
}

#[test]
fn tiled_paths_match_whole_frame_at_edges_and_denoise_seams() {
    let Some(g) = common::gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let size = [257, 193]; // non-workgroup-aligned edges, multiple tiles in both axes
    let mesh = prim::sphere(75.0, 24);
    let mut sc = Scene3 {
        size,
        cam: resolve(&CameraParams::default(), size[0] as f32, size[1] as f32),
        clip_fix: Mat4::IDENTITY,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.12,
        draws: vec![Draw3 {
            mesh: MeshSrc::Cached(eng.upload_mesh(&mesh.vertices, &mesh.indices)),
            model: Mat4::from_translation(Vec3::new(128.0, 96.0, 10.0)),
            material: MaterialParams { base_color: [0.7, 0.3, 0.1, 1.0], roughness: 0.6, ..Default::default() },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: true,
            receive_shadow: true,
        }],
        lights: vec![Light3 {
            kind: LightKind::Sphere,
            pos: Vec3::new(50.0, 10.0, -170.0),
            dir: Vec3::Z,
            right: Vec3::X,
            color: Vec3::splat(200.0),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 0.0,
            cast_shadow: true,
            softness: 1.0,
            bias: 0.0005,
            map_size: 128,
            size: [0.0, 0.0, 40.0],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        }],
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
        geodesic: None,
    };
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let sampler = g.device.create_sampler(&Default::default());
    let inputs = PtInputs { env: &black, backdrop: None, black: &black, sampler: &sampler };
    let mut pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    assert!(pt.limit_buffers(1024).is_err(), "must reject a budget unable to hold denoising halos");
    assert!(
        pt.limit_buffers(125 * 125 * 32).is_err(),
        "a one-pixel core would amplify UHD into millions of overlapping tile dispatches"
    );
    for orthographic in [false, true] {
        sc.cam = resolve(&CameraParams { orthographic, ..Default::default() }, size[0] as f32, size[1] as f32);
        let data = pathtrace::build(&sc);
        for denoise in [false, true] {
            let opts = PathOpts { samples: 5, bounces: 2, denoise };
            let mut images = Vec::new();
            for budget in [32 << 20, 192 * 192 * 32, 192 * 192 * 32] {
                pt.limit_buffers(budget).unwrap();
                let target = eng.target(size);
                let mut enc = g.device.create_command_encoder(&Default::default());
                pathtrace::render(
                    &pt,
                    &g.device,
                    &mut enc,
                    &sc,
                    &data,
                    opts,
                    &inputs,
                    &target.create_view(&Default::default()),
                );
                g.queue.submit([enc.finish()]);
                images.push(eng.read(&target));
            }
            assert!(images[0].iter().any(|p| p[0] > 0.01 && p[3] > 0.9), "must render lit geometry");
            assert!(images[0].iter().any(|p| p[3] == 0.0), "must preserve transparent background");
            assert_eq!(images[1], images[2], "repeated tiled render must not accumulate old samples");
            for (index, (a, b)) in images[0].iter().zip(&images[1]).enumerate() {
                assert_eq!(a, b, "pixel {index}, denoise {denoise}, orthographic {orthographic}");
            }
        }
    }
}
