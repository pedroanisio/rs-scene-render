mod common;

use glam::{Mat4, Quat, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_3d::{prim, AlphaMode, MaterialParams};
use sr_gpu::pathtrace::{self, PathOpts};
use sr_gpu::three::{Draw3, Light3, LightKind, MeshSrc, Scene3, ThreeEngine};

#[test]
fn repeated_prototypes_fit_without_triangle_expansion_and_match_expanded_pixels() {
    let Some(gpu) = common::gpu() else { return };
    let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let mesh = prim::sphere(4., 12);
    let prototype = engine.upload_mesh(&mesh.vertices, &mesh.indices);
    let mut scene = Scene3 {
        cam: resolve(&CameraParams::default(), 128., 128.),
        clip_fix: Mat4::IDENTITY,
        size: [128, 128],
        exposure: 1.,
        dof: None,
        lens_k1: 0.,
        draws: (0..64)
            .map(|i| Draw3 {
                mesh: MeshSrc::Cached(prototype.clone()),
                model: Mat4::from_scale_rotation_translation(
                    Vec3::new(if i % 3 == 0 { -1.2 } else { 1. }, 0.7 + (i % 4) as f32 * 0.1, 1.3),
                    Quat::from_rotation_y(i as f32 * 0.19),
                    Vec3::new(22. + (i % 8) as f32 * 12., 22. + (i / 8) as f32 * 12., (i % 5) as f32 * 3.),
                ),
                material: MaterialParams {
                    base_color: [0.2 + (i % 4) as f32 * 0.15, 0.3, 0.1, 1.],
                    unlit: true,
                    ..Default::default()
                },
                maps: Default::default(),
                opacity: 1.,
                cast_shadow: i % 2 == 0,
                receive_shadow: true,
            })
            .collect(),
        lights: Vec::new(),
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: Some(PathOpts { samples: 4, bounces: 2, denoise: false }),
    };
    let mut limits = gpu.device.limits();
    limits.max_storage_buffer_binding_size = 2 << 20;
    assert!(pathtrace::limit_note(&scene, &limits).is_none(), "shared geometry must fit the declared binding budget");
    let packed = pathtrace::build(&scene);
    assert!(packed.tri_mat.len() <= mesh.indices.len() / 3 + 64, "prototype triangles must occur once");
    assert!(packed.timing.bvh_seconds > 0.0 && packed.timing.assemble_seconds > 0.0, "{:?}", packed.timing);
    let color =
        engine.upload_rgba8(2, 2, &[255, 64, 32, 255, 32, 192, 64, 255, 64, 32, 255, 255, 255, 192, 64, 255], true);
    let normal = engine.upload_rgba8(1, 1, &[160, 112, 248, 255], false);
    for mode in 0..5 {
        let textured = mode == 1 || mode == 3;
        scene.volumes = if mode == 3 {
            use sr_volume::{
                medium::{Bounds, March, Medium, Optical},
                SparseGrid, Transform,
            };
            let density = std::sync::Arc::new(SparseGrid::new(Transform::identity(), 0.02, 0).unwrap());
            let medium = Medium::new(
                density,
                Some(Bounds::new([10., 10., -10.], [120., 120., 40.]).unwrap()),
                Transform::identity(),
                Optical { extinction: 0.5, albedo: [0.5; 3], ..Default::default() },
            )
            .unwrap();
            vec![sr_gpu::volume::VolumeDraw::new(std::sync::Arc::new(medium), March { step_size: 1., max_steps: 512 })
                .unwrap()]
        } else {
            Vec::new()
        };
        scene.lights = if textured {
            vec![Light3 {
                kind: LightKind::Directional,
                pos: Vec3::ZERO,
                dir: Vec3::new(0.4, 0.2, 1.).normalize(),
                right: Vec3::X,
                color: Vec3::splat(1.),
                range: 0.,
                falloff: 2.,
                cos_outer: 0.,
                cos_inner: 0.,
                cast_shadow: true,
                softness: 0.,
                bias: 0.0005,
                map_size: 128,
                size: [0.; 3],
                ies: None,
                affects_diffuse: true,
                affects_specular: true,
                contact: 0.,
            }]
        } else {
            Vec::new()
        };
        for (i, draw) in scene.draws.iter_mut().enumerate() {
            draw.mesh = MeshSrc::Cached(prototype.clone());
            if mode == 4 {
                draw.model = Mat4::from_translation(Vec3::new(64., 64., 0.));
                draw.material.base_color = [0.7, 0.3, 0.1, 1.];
            }
            draw.material.unlit = !textured;
            draw.material.uv_scale = if textured { [1. + (i % 2) as f32, 1.] } else { [1.; 2] };
            draw.material.separate_uvs = textured && i % 3 == 0;
            draw.maps[0] = textured.then(|| color.clone());
            draw.maps[1] = textured.then(|| normal.clone());
            draw.material.alpha_mode = if mode == 2 { AlphaMode::Blend } else { AlphaMode::Opaque };
            draw.opacity = if mode == 2 { 0.3 + (i % 3) as f32 * 0.2 } else { 1. };
            draw.receive_shadow = i % 3 != 0;
        }
        assert!(pathtrace::limit_note(&scene, &limits).is_none());
        let instanced = engine.render_now(&scene, None);
        assert!(instanced.iter().any(|p| p[3] > 0.1), "scene must contain visible instances");
        for draw in &mut scene.draws {
            draw.mesh = MeshSrc::Deformed(mesh.vertices.clone(), prototype.clone());
        }
        assert!(pathtrace::limit_note(&scene, &limits).is_some(), "expanded geometry exceeds this budget");
        let expanded = engine.render_now(&scene, None);
        let max =
            instanced.iter().flatten().zip(expanded.iter().flatten()).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(max < 0.003, "mode {mode}: transformed prototype pixels differ from expanded geometry: {max}");
    }
}
