//! Device limits and hostile inputs: sizes are checked against what the device has, and what
//! does not fit is split, reduced or reported instead of failing wgpu validation.

mod common;
use common::*;

use glam::Vec3;
use sr_gpu::gpu::device_limits;
use sr_gpu::output::{OutputColor, OutputStage};
use sr_gpu::three::{self, Light3, LightKind, ThreeEngine};
use sr_media::encode::InputFormat;
use sr_model::model::{ColorSpace, Transfer};

/// A desktop adapter: 4 GiB buffers, 2 GiB storage bindings, 2048 layers.
fn desktop() -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: 4 << 30,
        max_storage_buffer_binding_size: 2 << 30,
        max_uniform_buffer_binding_size: 64 << 10,
        max_texture_array_layers: 2048,
        max_texture_dimension_2d: 16384,
        ..wgpu::Limits::default()
    }
}

/// What the device was capped at whatever the hardware.
fn downlevel() -> wgpu::Limits {
    wgpu::Limits { max_storage_buffers_per_shader_stage: 8, ..wgpu::Limits::downlevel_defaults() }
}

#[test]
fn the_device_asks_for_the_adapters_buffer_binding_and_layer_limits() {
    let a = desktop();
    let l = device_limits(&a);
    assert_eq!(l.max_buffer_size, 4 << 30);
    assert_eq!(l.max_storage_buffer_binding_size, 2 << 30);
    assert_eq!(l.max_uniform_buffer_binding_size, 64 << 10);
    assert_eq!(l.max_texture_array_layers, 2048);
    assert_eq!(l.max_texture_dimension_2d, 16384);
    assert!(l.check_limits(&a), "nothing is requested beyond the adapter");
    // an adapter with no more than the downlevel set (OpenGL) still gets a device
    let gl = downlevel();
    let l = device_limits(&gl);
    assert!(l.check_limits(&gl));
    assert_eq!(l.max_storage_buffer_binding_size, gl.max_storage_buffer_binding_size);
    assert_eq!(l.max_storage_buffers_per_shader_stage, 8);
}

#[test]
fn a_device_has_at_least_the_downlevel_limits() {
    let Some(g) = gpu() else { return };
    let l = g.device.limits();
    assert!(l.max_buffer_size >= 256 << 20 && l.max_storage_buffer_binding_size >= 128 << 20, "{l:?}");
}

#[test]
fn materialx_files_and_graphs_render_from_relative_document_bases() {
    let dir = std::path::PathBuf::from(format!("target/materialx-render-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])).save(dir.join("red.png")).unwrap();
    for (name, graph, source) in [
        ("direct", "", "im"),
        (
            "graph",
            r#"<multiply name="tint" type="color3"><input name="in1" nodename="im"/><input name="in2" value="0.5"/></multiply>"#,
            "tint",
        ),
    ] {
        std::fs::write(dir.join(format!("{name}.mtlx")), format!(r#"<materialx><image name="im" type="color3"><input name="file" type="filename" value="red.png"/></image>{graph}<standard_surface name="s"><input name="base_color" nodename="{source}"/></standard_surface></materialx>"#)).unwrap();
        let xml = format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="1" duration="1"/><materials><material id="m" materialX="{name}.mtlx"/></materials><composition><object3D id="o" primitive="plane" width="50" height="50" x="32" y="32" material="m"/></composition></scene>"#
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { base_dir: Some(dir.clone()), verify_assets: true })
            .unwrap();
        let Some(frame) = render(&doc) else { return };
        assert!(frame.stats.errors.is_empty(), "{name}: {:?}", frame.stats.errors);
        let center = frame.at(32, 32);
        assert!(center[0] > 0.1 && center[0] > center[1] * 3.0, "{name}: {center:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_oversized_or_nonfinite_frames_report_errors_without_panicking() {
    let Some(gpu) = gpu() else { return };
    let d = doc(r#"width="16" height="8""#, "", "");
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    for width in [gpu.device.limits().max_texture_dimension_2d as f64 + 1.0, f64::INFINITY, f64::NAN, 0.0] {
        let mut graph = ev.evaluate(0.0);
        graph.size = [width, 1.0];
        let frame = renderer.render(&graph, ev.program());
        assert!(!frame.stats.errors.is_empty(), "accepted {width}");
        assert_eq!(frame.texture.size, [1, 1]);
    }
    assert!(renderer.render(&ev.evaluate(0.0), ev.program()).stats.errors.is_empty());
}

#[test]
fn output_frames_larger_than_a_buffer_are_packed_in_pieces() {
    let d = doc(
        r##"width="16" height="8" background="#00000000""##,
        "",
        r#"<layer id="a" asset="quad" x="0" y="0" scaleX="8" scaleY="4"/>"#,
    );
    let Some(g) = gpu() else { return };
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(g.clone(), ev.program());
    let f = r.render(&ev.evaluate(0.0), ev.program());
    let color = OutputColor::new(ColorSpace::Srgb, Transfer::Auto, true);
    for format in [InputFormat::Gbrapf32, InputFormat::Rgba16, InputFormat::Rgba8, InputFormat::Nv12, InputFormat::P010]
    {
        let mut whole = OutputStage::new(g.device.clone(), g.queue.clone());
        let p = whole.submit(&f.texture, &r.working(), &color, format, [16, 8], true);
        let want = whole.wait(p);
        assert_eq!(want.len(), format.frame_bytes(16, 8));
        // 25 words a piece; the frame is up to 2048 bytes
        let mut pieces = OutputStage::new(g.device.clone(), g.queue.clone());
        pieces.limit_buffers(100);
        for _ in 0..2 {
            pieces.seek(0);
            let p = pieces.submit(&f.texture, &r.working(), &color, format, [16, 8], true);
            let got = pieces.wait(p);
            assert!(got == want, "{format:?}: pieces differ from the whole frame");
        }
    }
}

#[test]
fn path_tracing_tiles_large_frames_but_reports_geometry_beyond_the_device() {
    use sr_gpu::pathtrace::fits;
    // UHD guides need 265,420,800 bytes. Image buffers must be tiled instead of
    // silently changing the requested renderer on a 128 MiB binding device.
    assert!(fits([3840, 2160], 1000, &downlevel()).is_ok(), "UHD must remain path traced");
    assert!(fits([1920, 1080], 1000, &downlevel()).is_ok());
    assert!(fits([3840, 2160], 1000, &desktop()).is_ok());
    // Geometry remains a separately bounded binding (384 bytes per triangle).
    assert!(fits([64, 64], 3_000_000, &downlevel()).is_err());
    assert!(fits([u32::MAX, u32::MAX], u64::MAX, &desktop()).is_err(), "no overflow");
}

#[test]
fn splats_beyond_a_storage_binding_are_cut_and_counted() {
    // 64 bytes a splat, 192 of spherical harmonics
    assert_eq!(three::splat_capacity(&downlevel(), false), (128 << 20) / 64);
    assert_eq!(three::splat_capacity(&downlevel(), true), (128 << 20) / 192);
    assert!(three::splat_capacity(&desktop(), false) > 30_000_000);
    let Some(g) = gpu() else { return };
    let eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let mut s = sr_3d::Splats::default();
    for i in 0..5 {
        s.pos.push([i as f32, 0.0, 0.0]);
        s.scale.push([1.0; 3]);
        s.rot.push([0.0, 0.0, 0.0, 1.0]);
        s.color.push([1.0; 4]);
        s.sh.push([0.0; 48]);
    }
    s.sh_degree = 1;
    let cut = eng.upload_splats_within(&s, 2);
    assert_eq!((cut.n, cut.buf.size(), cut.sh.size()), (2, 2 * 64, 2 * 192));
    assert_eq!(cut.hi.x, 1.0, "bounds cover the splats kept");
    assert_eq!(eng.upload_splats(&s).n, 5);
    let note = three::splat_note(5, 2).expect("a note when splats were dropped");
    assert!(note.contains('3') && note.contains('5'), "{note}");
    assert!(three::splat_note(5, 5).is_none());
}

fn point(shadow: bool) -> Light3 {
    Light3 {
        kind: LightKind::Point,
        pos: Vec3::new(0.0, -50.0, 0.0),
        dir: Vec3::Y,
        right: Vec3::X,
        color: Vec3::splat(1000.0),
        range: 0.0,
        falloff: 2.0,
        cos_outer: 0.0,
        cos_inner: 0.0,
        cast_shadow: shadow,
        softness: 1.0,
        bias: 0.0005,
        map_size: 16,
        size: [0.0; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.0,
    }
}

#[test]
fn shadow_views_stop_at_the_layer_limit() {
    // a point light takes six views: 42 fit 256 layers, the last two light without shadows
    let lights: Vec<Light3> = (0..44).map(|_| point(true)).collect();
    let cast = three::shadow_casters(&lights, true, 256);
    assert_eq!(cast.iter().filter(|c| **c).count(), 42);
    assert!(cast[..42].iter().all(|c| *c) && !cast[42] && !cast[43]);
    assert!(three::shadow_casters(&lights, true, 2048).iter().all(|c| *c));
    // directional lights take four cascades under a perspective camera, one view otherwise
    let sun = Light3 { kind: LightKind::Directional, ..point(true) };
    let mixed = [point(false), sun.clone(), Light3 { kind: LightKind::Spot, ..point(true) }, sun.clone(), point(true)];
    assert_eq!(three::shadow_casters(&mixed, true, 9), [false, true, true, true, false]);
    assert_eq!(three::shadow_casters(&mixed, false, 3), [false, true, true, true, false]);
    let note = three::shadow_note(&lights, true, 256).expect("a note when lights lose their shadows");
    assert!(note.contains('2') && note.contains("256"), "{note}");
    assert!(three::shadow_note(&lights, true, 2048).is_none());
}

#[test]
fn more_shadow_views_than_layers_render_without_a_validation_error() {
    let Some(g) = gpu() else { return };
    if g.info.backend == wgpu::Backend::Gl {
        return;
    }
    let layers = g.device.limits().max_texture_array_layers as usize;
    let mut eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let cube = sr_3d::prim::sphere(20.0, 8);
    let lights: Vec<Light3> = (0..layers / 6 + 2).map(|_| point(true)).collect();
    let scene = three::Scene3 {
        cam: sr_3d::camera::resolve(&Default::default(), 32.0, 32.0),
        clip_fix: glam::Mat4::IDENTITY,
        size: [32, 32],
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: vec![three::Draw3 {
            mesh: three::MeshSrc::Cached(eng.upload_mesh(&cube.vertices, &cube.indices)),
            model: glam::Mat4::IDENTITY,
            material: Default::default(),
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: true,
            receive_shadow: true,
        }],
        lights,
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
    };
    let px = eng.render_now(&scene, None);
    assert_eq!(px.len(), 32 * 32);
}

#[test]
fn material_textures_are_reduced_to_the_device_limit() {
    assert_eq!(three::fit_texture(4096, 2048, 8192), [4096, 2048]);
    assert_eq!(three::fit_texture(16384, 8192, 8192), [8192, 4096]);
    assert_eq!(three::fit_texture(20000, 1, 8192), [8192, 1]);
    assert_eq!(three::fit_texture(3, 30000, 2048), [1, 2048]);
    let Some(g) = gpu() else { return };
    let mut eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let limit = g.device.limits().max_texture_dimension_2d;
    let t = eng.upload_rgba8(limit + 1, 1, &vec![255u8; (limit as usize + 1) * 4], true);
    assert_eq!(t.size, [limit.min(8192), 1]);
}

#[test]
fn shader_uniforms_and_samplers_beyond_the_device_fall_back() {
    use sr_gpu::glsl::{build_effect, check_limits};
    let main = "void main() { fragColor = table[1]; }";
    let p = build_effect(&format!("uniform vec4 table[2000];\n{main}")).unwrap();
    let e = check_limits(&p, &downlevel()).expect_err("32000 bytes of uniforms against 16384");
    assert!(e.contains("32000") && e.contains("16384"), "{e}");
    assert!(check_limits(&p, &desktop()).is_ok());
    let many: String = (0..17).map(|k| format!("uniform sampler2D s{k};\n")).collect();
    let p = build_effect(&format!("{many}void main() {{ fragColor = texture(s16, uv); }}")).unwrap();
    let e = check_limits(&p, &desktop()).expect_err("17 textures against 16");
    assert!(e.contains("17") && e.contains("16"), "{e}");
    // sizes that overflow 32 bits are errors, not wrapped offsets
    assert!(build_effect(&format!("uniform vec4 table[300000000];\n{main}")).is_err());
    assert!(build_effect(&format!("uniform mat4 table[4294967295];\n{main}")).is_err());
    // on a device: the effect passes its input through and the problem is reported
    let Some(g) = gpu() else { return };
    let l = g.device.limits();
    let elements = (l.max_uniform_buffer_binding_size / 16 + 1).min(u32::MAX as u64);
    let textures = l.max_sampled_textures_per_shader_stage + 1;
    let dir = fixtures();
    for (name, decl, read) in [
        ("big.glsl", format!("uniform vec4 table[{elements}];"), "table[1]"),
        ("many.glsl", (0..textures).map(|k| format!("uniform sampler2D s{k};\n")).collect(), "texture(s7, uv)"),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, format!("{decl}\nvoid main() {{ fragColor = {read}; }}")).unwrap();
        let d = doc_with(
            r##"background="#00000000""##,
            "",
            r#"<layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="e"/>"#,
            &format!(r#"<effects><effect id="e" type="shader" src="{}"/></effects>"#, path.display()),
        );
        let Some(r) = render(&d) else { return };
        assert_px(&r, 16, 16, [1.0, 0.0, 0.0, 1.0], 1e-2);
        let p: Vec<&String> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
        assert!(!p.is_empty(), "{name}: the fallback is reported");
    }
}

#[test]
fn percent_escapes_before_multibyte_characters_do_not_split_them() {
    let base = std::path::Path::new("/tmp");
    assert_eq!(sr_gpu::glsl::load_source("data:,%a\u{e9}%41%", base).unwrap().0, "%a\u{e9}A%");
    assert_eq!(sr_gpu::glsl::load_source("data:,%\u{e9}", base).unwrap().0, "%\u{e9}");
}

#[test]
fn large_images_are_reduced_before_they_are_converted_to_float() {
    use image::DynamicImage;
    let img = DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 32, |x, _| {
        image::Rgb(if x < 32 { [255, 0, 0] } else { [0, 0, 255] })
    }));
    let small = sr_gpu::resources::fit(img.clone(), 16);
    assert!(matches!(small, DynamicImage::ImageRgb8(_)), "the reduction keeps the source's sample format");
    assert_eq!((small.width(), small.height()), (16, 8));
    let p = small.to_rgb8();
    assert_eq!((p.get_pixel(2, 4).0, p.get_pixel(13, 4).0), ([255, 0, 0], [0, 0, 255]));
    let same = sr_gpu::resources::fit(img, 64);
    assert_eq!((same.width(), same.height()), (64, 32));
    let tall = sr_gpu::resources::fit(DynamicImage::ImageLuma16(image::ImageBuffer::new(3, 4000)), 100);
    assert!(matches!(tall, DynamicImage::ImageLuma16(_)));
    assert_eq!((tall.width(), tall.height()), (1, 100));
}

/// A 1×1 texture of one colour.
fn solid(g: &sr_gpu::Gpu, c: [u8; 4]) -> wgpu::TextureView {
    let t = g.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    g.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &t,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &c,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    t.create_view(&Default::default())
}

#[test]
fn emitters_sharing_a_sprite_id_keep_their_own_textures() {
    use sr_gpu::particles::{Inst, ParticleEngine, ParticleJob};
    let Some(g) = gpu() else { return };
    let mut eng = ParticleEngine::new(g.device.clone(), &g.queue);
    let layout = sr_gpu::resources::source_layout(&g.device);
    // two documents' assets with the same id: a red sprite and a blue one
    let mut centre = Vec::new();
    for c in [[255, 0, 0, 255], [0, 0, 255, 255]] {
        let target = sr_gpu::resources::create(&g.device, &layout, [8, 8], 1, "particles");
        let job = ParticleJob {
            insts: vec![Inst {
                centre_half: [4.0, 4.0, 4.0, 4.0],
                rot_shape: [0.0, 2.0, 0.0, 0.0],
                color: [1.0; 4],
                cell: [0.0, 0.0, 1.0, 1.0],
            }],
            sprite: Some(("spark".into(), solid(&g, c))),
        };
        let mut enc = g.device.create_command_encoder(&Default::default());
        eng.record(&mut enc, &job, &target.view, [8, 8]);
        let buf = g.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(8) },
            },
            wgpu::Extent3d { width: 8, height: 8, depth_or_array_layers: 1 },
        );
        g.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        g.wait();
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let at = 4 * 256 + 4 * 8;
        let px: Vec<f32> =
            (0..4).map(|k| half::f16::from_le_bytes([data[at + 2 * k], data[at + 2 * k + 1]]).to_f32()).collect();
        centre.push(px);
    }
    assert!(centre[0][0] > 0.9 && centre[0][2] < 0.1, "the first emitter's sprite is red: {:?}", centre[0]);
    assert!(centre[1][2] > 0.9 && centre[1][0] < 0.1, "the second emitter's sprite is blue: {:?}", centre[1]);
}

#[test]
fn a_3d_document_on_opengl_reports_how_to_render_it() {
    use sr_gpu::gpu::GpuOptions;
    let Ok(g) = sr_gpu::Gpu::with_options(&GpuOptions { backends: wgpu::Backends::GL, adapter: None }) else { return };
    let d = doc_with(
        r##"background="#00000000""##,
        "",
        r#"<object3D id="ball" primitive="sphere" radius="10" x="32" y="16"/><object3D id="b2" primitive="sphere" radius="4" x="8" y="8"/>"#,
        "",
    );
    let r = render_times_on(g, &d, &[0.0]).expect("OpenGL renders the rest");
    let notes: Vec<&String> = r.stats.unsupported.iter().filter(|m| m.contains("3D objects are not drawn")).collect();
    assert_eq!(notes.len(), 1, "one warning for the 3D pass: {:?}", r.stats.unsupported);
    assert!(notes[0].starts_with("ball: ") && notes[0].contains("SR_GPU_BACKEND=vulkan"), "{}", notes[0]);
}

#[test]
fn every_kind_the_3d_pass_draws_reaches_the_3d_pass() {
    // the list the adapter choice reads (sr_eval::THREE_D_DRAWN) and the renderer's 3D pass agree: on an OpenGL
    // device each kind is turned back by the 3D pass's own note, so none is drawn by the pass yet missed by the choice
    use sr_gpu::gpu::GpuOptions;
    let cases = [
        ("object3D", r#"<object3D id="n" primitive="sphere" radius="10" x="32" y="16"/>"#),
        ("particles3D", r#"<particles3D id="n" rate="50" lifetime="1" dt="0.05" x="32" y="16"/>"#),
        ("ocean", r#"<ocean id="n" width="8" depth="8" bottomDepth="2" x="32" y="16"/>"#),
    ];
    let mut covered: Vec<&str> = cases.iter().map(|c| c.0).collect();
    let mut kinds: Vec<&str> = sr_eval::THREE_D_DRAWN.to_vec();
    covered.sort_unstable();
    kinds.sort_unstable();
    assert_eq!(covered, kinds, "a kind was added to THREE_D_DRAWN without a case here");
    for (kind, xml) in cases {
        let Ok(g) = sr_gpu::Gpu::with_options(&GpuOptions { backends: wgpu::Backends::GL, adapter: None }) else {
            return;
        };
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="32" fps="10" duration="4" background="#00000000"/><composition>{xml}</composition></scene>"##
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let r = render_times_on(g, &d, &[0.5]).expect("OpenGL renders the rest");
        assert!(
            r.stats.unsupported.iter().any(|m| m.starts_with("n: ") && m.contains("3D objects are not drawn")),
            "{kind}: {:?}",
            r.stats.unsupported
        );
    }
}

#[test]
fn a_second_device_opens_on_the_same_adapter() {
    let Some(g) = gpu() else { return };
    let twin = g.open_like().expect("the same adapter opens again");
    assert_eq!((twin.info.name, twin.info.backend, twin.info.device), (g.info.name, g.info.backend, g.info.device));
}
