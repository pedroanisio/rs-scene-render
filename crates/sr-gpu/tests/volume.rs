mod common;

use std::sync::Arc;

use glam::{Mat4, Vec3};
use sr_3d::camera::{resolve, CameraParams};
use sr_gpu::pathtrace::{self, PathOpts, PtGpu, PtInputs};
use sr_gpu::three::{Scene3, ThreeEngine};
use sr_gpu::volume::VolumeDraw;
use sr_volume::medium::{Bounds, March, Medium, Optical};
use sr_volume::{SparseGrid, Transform};

fn slab(extinction: f64, emission: [f64; 3], lo: f64, hi: f64) -> VolumeDraw {
    let medium = Medium::new(
        Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()),
        Some(Bounds::new([-1000.0, -1000.0, lo], [1000.0, 1000.0, hi]).unwrap()),
        Transform::identity(),
        Optical { extinction, emission, ..Default::default() },
    )
    .unwrap();
    VolumeDraw::new(Arc::new(medium), March { step_size: 0.25, max_steps: 65536 }).unwrap()
}

#[test]
fn gpu_volume_transport_matches_analytic_slabs_and_overlapping_media() {
    let Some(g) = common::gpu() else { return };
    let mut eng = ThreeEngine::new(g.device.clone(), g.queue.clone());
    let size = [4, 4];
    let mut scene = Scene3 {
        cam: resolve(
            &CameraParams { orthographic: true, position: Some(Vec3::new(0.0, 0.0, -10.0)), ..Default::default() },
            4.0,
            4.0,
        ),
        clip_fix: Mat4::IDENTITY,
        size,
        exposure: 1.0,
        dof: None,
        lens_k1: 0.0,
        draws: Vec::new(),
        lights: Vec::new(),
        env: None,
        splats: Vec::new(),
        volumes: Vec::new(),
        encode_srgb: false,
        ao: None,
        ssr: false,
        path: None,
    };
    let black = eng.upload_f16([1, 1], &[[0.0; 4]]).create_view(&Default::default());
    let smp = g.device.create_sampler(&Default::default());
    let input = PtInputs { env: &black, backdrop: None, black: &black, sampler: &smp };
    let pt = PtGpu::new(&g.device, wgpu::TextureFormat::Rgba16Float);
    let mut sparse = SparseGrid::new(Transform::identity(), 0.0, 16).unwrap();
    for y in -4..=4 {
        for x in -4..=4 {
            sparse.set([x, y, 0], 1.0).unwrap();
        }
    }
    let sparse = Medium::new(
        Arc::new(sparse),
        None,
        Transform::new(glam::DMat4::from_scale(glam::DVec3::new(1.0, 1.0, 3.0)).to_cols_array()).unwrap(),
        Optical { emission: [0.2, 0.1, 0.0], ..Default::default() },
    )
    .unwrap();
    let thermal = slab(0.7, [0.01, 0.02, 0.0], 0.0, 3.0);
    let thermal = thermal
        .medium()
        .as_ref()
        .clone()
        .with_temperature(Arc::new(SparseGrid::new(Transform::identity(), 2500.0, 0).unwrap()), 2.0, 0.3)
        .unwrap();
    let mixed = thermal
        .clone()
        .with_next_frame(
            Arc::new(SparseGrid::new(Transform::identity(), 3.0, 0).unwrap()),
            Some(Arc::new(SparseGrid::new(Transform::identity(), 1000.0, 0).unwrap())),
            0.5,
        )
        .unwrap();
    let mut gradient = SparseGrid::new(
        Transform::new(glam::DMat4::from_scale(glam::DVec3::splat(2.0)).to_cols_array()).unwrap(),
        0.0,
        32,
    )
    .unwrap();
    for z in 0..=3 {
        for y in -4..=4 {
            for x in -4..=4 {
                gradient.set([x, y, z], 4000.0 + 1000.0 * z as f32).unwrap();
            }
        }
    }
    let gradient =
        slab(0.7, [0.0; 3], 0.0, 3.0).medium().as_ref().clone().with_temperature(Arc::new(gradient), 1.0, 0.3).unwrap();
    let shifted =
        Transform::new(glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, 1.0)).to_cols_array()).unwrap();
    let stretched = Transform::new(glam::DMat4::from_scale(glam::DVec3::new(1.0, 1.0, 1.5)).to_cols_array()).unwrap();
    let mut next_density = SparseGrid::new(shifted, 0.0, 16).unwrap();
    let mut next_temperature = SparseGrid::new(stretched, 300.0, 16).unwrap();
    for y in -4..=4 {
        for x in -4..=4 {
            for z in 0..=3 {
                next_density.set([x, y, z], 0.5 + z as f32).unwrap();
                next_temperature.set([x, y, z], 2000.0 + 500.0 * z as f32).unwrap();
            }
        }
    }
    let transformed_pair =
        gradient.clone().with_next_frame(Arc::new(next_density), Some(Arc::new(next_temperature)), 0.4).unwrap();
    // Two separated slabs must meet on the camera ray after velocity tracing.
    // Scalar cross-fading leaves that ray in vacuum, so this catches ignored motion.
    let moving_fields = |center: i32, kelvin: f32| {
        let mut density = SparseGrid::new(Transform::identity(), 0., 16).unwrap();
        let mut temperature = SparseGrid::new(Transform::identity(), 0., 16).unwrap();
        for x in center - 3..=center + 3 {
            for y in -4..=4 {
                for z in 0..=3 {
                    density.set([x, y, z], 1.).unwrap();
                    temperature.set([x, y, z], kelvin).unwrap();
                }
            }
        }
        (Arc::new(density), Arc::new(temperature))
    };
    let (d0, t0) = moving_fields(-5, 2000.);
    let (d1, t1) = moving_fields(5, 8000.);
    let velocity = [10., 0., 0.].map(|v| Arc::new(SparseGrid::new(Transform::identity(), v, 0).unwrap()));
    let advected = Medium::new(d0, None, Transform::identity(), Optical { extinction: 0.7, ..Default::default() })
        .unwrap()
        .with_temperature(t0, 1., 0.3)
        .unwrap()
        .with_next_frame(d1, Some(t1), 0.5)
        .unwrap()
        .with_advection(
            sr_volume::advection::Advection::new(velocity.clone(), 0.5).unwrap(),
            sr_volume::advection::Advection::new(velocity, -0.5).unwrap(),
        )
        .unwrap();
    for volumes in [
        vec![VolumeDraw::new(Arc::new(advected), March { step_size: 0.125, max_steps: 65536 }).unwrap()],
        vec![VolumeDraw::new(Arc::new(transformed_pair), March { step_size: 0.25, max_steps: 65536 }).unwrap()],
        vec![VolumeDraw::new(Arc::new(mixed), March { step_size: 0.25, max_steps: 65536 }).unwrap()],
        vec![VolumeDraw::new(Arc::new(thermal), March { step_size: 0.25, max_steps: 65536 }).unwrap()],
        vec![VolumeDraw::new(Arc::new(gradient), March { step_size: 0.25, max_steps: 65536 }).unwrap()],
        vec![slab(0.7, [0.2, 0.05, 0.0], 0.0, 3.0)],
        vec![slab(1.0, [0.2, 0.0, 0.0], 0.0, 3.0), slab(2.0, [0.0, 0.0, 0.3], 1.0, 2.0)],
        vec![VolumeDraw::new(Arc::new(sparse), March { step_size: 0.125, max_steps: 2048 }).unwrap()],
    ] {
        let expected = sr_volume::medium::integrate(
            &volumes.iter().map(|v| v.medium().as_ref().clone()).collect::<Vec<_>>(),
            sr_volume::medium::Ray::new([0.0, 0.0, -10.0], [0.0, 0.0, 1.0], 100.0).unwrap(),
            // Match GPU quadrature: thermal emission is nonlinear in the
            // interpolated temperature at the sparse slab's boundary.
            March {
                step_size: volumes.iter().map(|v| v.march().step_size).fold(f64::INFINITY, f64::min),
                max_steps: volumes.iter().map(|v| v.march().max_steps).min().unwrap(),
            },
            |_, _, _| [0.0; 3],
        )
        .unwrap();
        scene.volumes = volumes;
        assert!(pathtrace::limit_note(&scene, &g.device.limits()).is_none());
        let data = pathtrace::build(&scene);
        assert_eq!(
            data.pos.len() % 3,
            0,
            "even an empty BVH needs complete sentinel records before packed volume data"
        );
        let target = eng.target(size);
        let mut enc = g.device.create_command_encoder(&Default::default());
        pathtrace::render(
            &pt,
            &g.device,
            &mut enc,
            &scene,
            &data,
            PathOpts { samples: 1, bounces: 1, denoise: false },
            &input,
            &target.create_view(&Default::default()),
        );
        g.queue.submit([enc.finish()]);
        for pixel in eng.read(&target) {
            for c in 0..3 {
                assert!((f64::from(pixel[c]) - expected.radiance[c]).abs() < 0.001, "{pixel:?} vs {expected:?}");
            }
            assert!((f64::from(pixel[3]) - (1.0 - expected.transmittance)).abs() < 0.001, "{pixel:?} vs {expected:?}");
        }
    }
    // An opaque green plane clips a red-emitting medium at z=1, rather than
    // compositing the complete volume in front of the already rendered surface.
    let plane = sr_3d::prim::plane(20.0, 20.0, 1);
    scene.draws.push(sr_gpu::three::Draw3 {
        mesh: sr_gpu::three::MeshSrc::Cached(eng.upload_mesh(&plane.vertices, &plane.indices)),
        model: Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)),
        material: sr_3d::MaterialParams { base_color: [0.0, 1.0, 0.0, 1.0], unlit: true, ..Default::default() },
        maps: Default::default(),
        opacity: 1.0,
        cast_shadow: true,
        receive_shadow: true,
    });
    scene.volumes = vec![slab(0.7, [0.2, 0.0, 0.0], 0.0, 3.0)];
    let data = pathtrace::build(&scene);
    let target = eng.target(size);
    let mut enc = g.device.create_command_encoder(&Default::default());
    pathtrace::render(
        &pt,
        &g.device,
        &mut enc,
        &scene,
        &data,
        PathOpts { samples: 1, bounces: 1, denoise: false },
        &input,
        &target.create_view(&Default::default()),
    );
    g.queue.submit([enc.finish()]);
    for p in eng.read(&target) {
        assert!((p[0] - 0.2 / 0.7 * (1.0 - (-0.7_f32).exp())).abs() < 0.001, "{p:?}");
        assert!((p[1] - (-0.7_f32).exp()).abs() < 0.001, "{p:?}");
        assert_eq!(p[3], 1.0);
    }
    scene.draws.clear();
    scene.lights.push(sr_gpu::three::Light3 {
        kind: sr_gpu::three::LightKind::Directional,
        pos: Vec3::ZERO,
        dir: Vec3::Z,
        right: Vec3::X,
        color: Vec3::splat(4.0 * std::f32::consts::PI),
        range: 0.0,
        falloff: 2.0,
        cos_outer: 0.0,
        cos_inner: 0.0,
        cast_shadow: true,
        softness: 0.0,
        bias: 0.0005,
        map_size: 128,
        size: [0.0; 3],
        ies: None,
        affects_diffuse: true,
        affects_specular: true,
        contact: 0.0,
    });
    let scattering = Arc::new(
        Medium::new(
            Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()),
            Some(Bounds::new([-10.0, -10.0, 0.0], [10.0, 10.0, 2.0]).unwrap()),
            Transform::identity(),
            Optical { extinction: 0.5, albedo: [1.0; 3], ..Default::default() },
        )
        .unwrap(),
    );
    for (cast, receive) in [(true, true), (false, true), (true, false)] {
        scene.volumes = vec![VolumeDraw::new(scattering.clone(), March { step_size: 0.125, max_steps: 2048 })
            .unwrap()
            .with_shadows(cast, receive)];
        let data = pathtrace::build(&scene);
        let target = eng.target(size);
        let mut enc = g.device.create_command_encoder(&Default::default());
        pathtrace::render(
            &pt,
            &g.device,
            &mut enc,
            &scene,
            &data,
            PathOpts { samples: 1, bounces: 1, denoise: false },
            &input,
            &target.create_view(&Default::default()),
        );
        g.queue.submit([enc.finish()]);
        let want = if cast && receive { 0.5 * (1.0 - (-2.0_f32).exp()) } else { 1.0 - (-1.0_f32).exp() };
        for p in eng.read(&target) {
            assert!(
                (p[0] - want).abs() < 0.001,
                "single scattering, cast={cast}, receive={receive}: {p:?}, expected {want}"
            );
        }
    }
    // The shared engine API must include volumes even with no explicit path options.
    for p in eng.render_now(&scene, None) {
        assert!((p[0] - (1.0 - (-1.0_f32).exp())).abs() < 0.002, "default 3D pass lost its volume: {p:?}");
    }
    let limited = VolumeDraw::new(scattering, March { step_size: 10.0, max_steps: 5 }).unwrap();
    scene.volumes = vec![limited.clone(), limited];
    let target = eng.target(size);
    let mut enc = g.device.create_command_encoder(&Default::default());
    assert!(
        eng.render(&mut enc, &scene, None, &target.create_view(&Default::default())).is_err(),
        "over-budget volumes must fail through the public engine API instead of disappearing"
    );
}

#[test]
fn gpu_volume_rejects_unrepresentable_data_and_excessive_work_before_upload() {
    let density = Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap());
    let medium = Arc::new(
        Medium::new(
            density,
            Some(Bounds::new([0.0; 3], [10.0; 3]).unwrap()),
            Transform::identity(),
            Optical::default(),
        )
        .unwrap(),
    );
    assert!(VolumeDraw::new(medium.clone(), March { step_size: 1e-100, max_steps: 10 }).is_err());
    assert!(VolumeDraw::new(medium, March { step_size: 0.001, max_steps: 10 }).is_err());
}

#[test]
fn volume_document_loads_a_cache_and_respects_start_time() {
    let Some(gpu) = common::gpu() else { return };
    let mut cache = sr_volume::Volume::new();
    cache.insert("density", SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()).unwrap();
    let path = common::fixtures().join("uniform.srvol");
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    let xml = format!(
        r##"<scene version="1.3"><project width="8" height="8" fps="30" duration="3" background="#00000000"/>
      <assets><volume id="smoke" src="{}" boundsMinX="-10" boundsMinY="-10" boundsMinZ="0" boundsMaxX="10" boundsMaxY="10" boundsMaxZ="2"/></assets>
      <composition><camera id="camera" x="0" y="0" z="-10" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
      <object3D id="cloud" primitive="volume" volume="smoke" start="1"><medium extinction="0.5" albedo="#000000" emissionColor="#FF0000" emissionScale="0.1"/></object3D></composition></scene>"##,
        path.display()
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let early = common::render_times_on(gpu.clone(), &doc, &[0.5]).unwrap();
    assert_eq!(early.at(4, 4)[3], 0.0);
    let active = common::render_times_on(gpu.clone(), &doc, &[1.0, 0.0, 2.0]).unwrap();
    assert!(active.stats.errors.is_empty(), "{:?}", active.stats.errors);
    assert!(active.stats.unsupported.is_empty(), "{:?}", active.stats.unsupported);
    let pixel = active.at(4, 4);
    assert!((pixel[3] - (1.0 - (-1.0_f32).exp())).abs() < 0.002, "{pixel:?}");
    assert!(pixel[0] > 0.1 && pixel[1] == 0.0 && pixel[2] == 0.0, "{pixel:?}");
    let raster_xml = xml.replace(r#"renderer="pathtrace""#, r#"renderer="raster""#);
    let raster_doc =
        sr_model::load_str(&raster_xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let raster = common::render_times_on(gpu.clone(), &raster_doc, &[2.0]).unwrap();
    assert!(raster.stats.errors.is_empty() && raster.stats.unsupported.is_empty(), "{:?}", raster.stats);
    assert!(common::close(raster.at(4, 4), pixel, 0.002));

    for (xml, expected) in [
        (xml.replace("<medium ", r#"<medium maxSteps="1" "#), "maxSteps"),
        (xml.replace(r#"id="smoke" src=""#, r#"id="smoke" densityGrid="missing" src=""#), "density channel missing"),
    ] {
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
        let r = common::render_times_on(gpu.clone(), &doc, &[2.0]).unwrap();
        assert!(r.stats.errors.iter().any(|e| e.contains(expected)), "{:?}", r.stats.errors);
    }
}

#[test]
fn thermal_document_reads_temperature_and_animates_its_scale() {
    let Some(gpu) = common::gpu() else { return };
    let mut cache = sr_volume::Volume::new();
    cache.insert("density", SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()).unwrap();
    cache.insert("heat", SparseGrid::new(Transform::identity(), 2500.0, 0).unwrap()).unwrap();
    let path = common::fixtures().join("thermal.srvol");
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    let xml = format!(
        r##"<scene version="1.3"><project width="4" height="4" fps="24" duration="3" background="#00000000"/>
      <assets><volume id="smoke" src="{}" temperatureGrid="heat" boundsMinX="-10" boundsMinY="-10" boundsMinZ="0" boundsMaxX="10" boundsMaxY="10" boundsMaxZ="2"/></assets>
      <composition><camera id="camera" x="0" y="0" z="-10" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
      <object3D id="cloud" primitive="volume" volume="smoke"><medium extinction="0.5" albedo="#000000" blackbody="true" temperatureScale="2" emissionScale="0.3"/></object3D></composition></scene>"##,
        path.display()
    );
    let render = |xml: &str| {
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
        common::render_times_on(gpu.clone(), &doc, &[0.0]).unwrap()
    };
    let hot = render(&xml);
    assert!(hot.stats.errors.is_empty() && hot.stats.unsupported.is_empty(), "{:?}", hot.stats);
    let p = hot.at(2, 2);
    assert!(p[0] > 0.1 && p[1] > 0.1 && p[2] > 0.05, "thermal channel was ignored: {p:?}");
    let cold = render(&xml.replace("temperatureScale=\"2\"", "temperatureScale=\"0.12\""));
    assert!(cold.at(2, 2)[0] < 1e-5, "room-temperature matter must not glow visibly");
    let disabled = render(&xml.replace("blackbody=\"true\"", "blackbody=\"false\""));
    assert_eq!(&disabled.at(2, 2)[..3], &[0.0; 3]);
    let missing = render(&xml.replace("temperatureGrid=\"heat\"", "temperatureGrid=\"missing\""));
    assert!(missing.stats.errors.iter().any(|e| e.contains("temperature channel missing")), "{:?}", missing.stats);
    let invalid = render(&xml.replace("temperatureScale=\"2\"", "temperatureScale=\"21\""));
    assert!(invalid.stats.errors.iter().any(|e| e.contains("temperature")), "{:?}", invalid.stats);
    // The thermal color is linear sRGB and must be converted to the working primaries.
    let aces = render(&xml.replace("fps=\"24\"", "fps=\"24\" workingColorSpace=\"acescg\""));
    assert!(aces.stats.errors.is_empty(), "{:?}", aces.stats);
    let display = aces.renderer.to_srgb8(&[aces.at(2, 2)]);
    let expected_display = hot.renderer.to_srgb8(&[p]);
    assert!(
        display.iter().zip(&expected_display).all(|(a, b)| a.abs_diff(*b) <= 1),
        "{display:?} vs {expected_display:?}"
    );
    let animated = xml.replace(r#"emissionScale="0.3"/>"#, r#"emissionScale="0.3"><animate property="temperatureScale"><key time="0" value="0.12"/><key time="1" value="2"/></animate></medium>"#);
    let doc = sr_model::load_str(&animated, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let frames = common::render_sub_frames(&doc, &[0.0, 1.0, 0.0, 1.0]).unwrap();
    assert!(
        frames.iter().all(|f| f.stats.errors.is_empty()),
        "{:?}",
        frames.iter().map(|f| &f.stats.errors).collect::<Vec<_>>()
    );
    assert!(frames[0].at(2, 2)[0] < 1e-5);
    assert!(common::close(frames[1].at(2, 2), p, 0.002));
    assert_eq!(frames[0].px, frames[2].px);
    assert_eq!(frames[1].px, frames[3].px);
}

#[test]
fn volume_sequences_blend_fields_and_seek_in_object_local_time() {
    let Some(gpu) = common::gpu() else { return };
    let dir = common::fixtures();
    for (frame, density, temp) in [(0, 1.0, 2000.0), (1, 3.0, 8000.0)] {
        let mut cache = sr_volume::Volume::new();
        cache.insert("density", SparseGrid::new(Transform::identity(), density, 0).unwrap()).unwrap();
        cache.insert("heat", SparseGrid::new(Transform::identity(), temp, 0).unwrap()).unwrap();
        cache.write(std::fs::File::create(dir.join(format!("sequence-{frame:02}.srvol"))).unwrap()).unwrap();
    }
    let xml = format!(
        r##"<scene version="1.3"><project width="4" height="4" fps="1" duration="4" background="#00000000"/>
      <assets><volume id="smoke" src="{}/sequence-%02d.srvol" first="0" last="1" interpolation="linear" temperatureGrid="heat" boundsMinX="-10" boundsMinY="-10" boundsMinZ="0" boundsMaxX="10" boundsMaxY="10" boundsMaxZ="2"/></assets>
      <composition><camera id="camera" x="0" y="0" z="-10" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
      <object3D id="cloud" primitive="volume" volume="smoke" start="1"><medium extinction="0.5" albedo="#000000" blackbody="true" emissionScale="0.1"/></object3D></composition></scene>"##,
        dir.display()
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let frames = common::render_sub_frames(&doc, &[0.0, 1.0, 1.5, 2.0, 1.5]).unwrap();
    assert!(
        frames.iter().all(|f| f.stats.errors.is_empty() && f.stats.unsupported.is_empty()),
        "{:?}",
        frames.iter().map(|f| &f.stats).collect::<Vec<_>>()
    );
    assert_eq!(frames[0].at(2, 2), [0.0; 4]);
    assert_eq!(frames[2].px, frames[4].px);
    for (frame, density, temp) in [(1, 1.0, 2000.0), (2, 2.0, 5000.0), (3, 3.0, 8000.0)] {
        let sigma = density * 0.5_f64;
        let trans = (-sigma * 2.0).exp();
        let expected =
            sr_volume::thermal::blackbody_rgb(temp).unwrap().map(|v| v * 0.1 * density * (1.0 - trans) / sigma);
        let p = frames[frame].at(2, 2);
        for c in 0..3 {
            assert!((f64::from(p[c]) - expected[c]).abs() < 0.003, "frame {frame}: {p:?} vs {expected:?}");
        }
        assert!((f64::from(p[3]) - (1.0 - trans)).abs() < 0.001);
    }
    // Missing policy does not hide malformed data, and a transparent frame blends to zero fields.
    let missing_xml = xml.replace("last=\"1\"", "last=\"2\" missingFrame=\"transparent\"");
    let doc =
        sr_model::load_str(&missing_xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    let result = common::render_times_on(gpu.clone(), &doc, &[3.0]).unwrap();
    assert!(result.stats.errors.is_empty(), "{:?}", result.stats);
    assert_eq!(result.at(2, 2), [0.0; 4]);
    let hold_xml = missing_xml.replace("missingFrame=\"transparent\"", "missingFrame=\"hold\"");
    let doc = sr_model::load_str(&hold_xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    let result = common::render_times_on(gpu.clone(), &doc, &[3.0, 1.0, 3.0]).unwrap();
    assert_eq!(result.px, frames[3].px);
    let speed_xml = xml.replace("start=\"1\"", "start=\"1\" animationSpeed=\"2\" animationOffset=\"0.25\"");
    let doc = sr_model::load_str(&speed_xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let result = common::render_times_on(gpu.clone(), &doc, &[1.125]).unwrap();
    assert_eq!(result.px, frames[2].px);
    for isolated in [false, true] {
        let nested = xml
            .replace(
                "<object3D",
                &format!(r#"<group id="parent" start="0.5" timeScale="2" isolate="{isolated}"><object3D"#),
            )
            .replace("</object3D>", "</object3D></group>");
        let doc = sr_model::load_str(&nested, &sr_model::LoadOptions::without_assets()).unwrap();
        // Group clocks preserve their origin: child = .5 + (t - .5) * 2.
        // Subtracting the object's start (1) gives 0, 1 and .5 seconds.
        let evaluator = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let evaluated = evaluator.evaluate(1.0);
        assert_eq!(evaluated.nodes.iter().find(|n| &*n.id == "cloud").unwrap().local_time, 0.5);
        let result = common::render_times_on(gpu.clone(), &doc, &[0.75, 1.25, 1.0]).unwrap();
        assert!(result.stats.errors.is_empty(), "{:?}", result.stats);
        assert_eq!(
            result.px, frames[2].px,
            "normal rendering must invalidate volume playback under isolate={isolated}"
        );
    }
    let doc = sr_model::load_str(&missing_xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let result = common::render_times_on(gpu.clone(), &doc, &[2.5]).unwrap();
    let expected = sr_volume::thermal::blackbody_rgb(4000.0).unwrap().map(|v| v * 0.1 * (1.0 - (-1.5_f64).exp()) / 0.5);
    for (got, expected) in result.at(2, 2)[..3].iter().zip(expected) {
        assert!((f64::from(*got) - expected).abs() < 0.003, "transparent frames must blend fields before emission");
    }
    std::fs::write(dir.join("sequence-02.srvol"), b"corrupt cache").unwrap();
    let doc =
        sr_model::load_str(&missing_xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    let result = common::render_times_on(gpu, &doc, &[3.0]).unwrap();
    assert!(!result.stats.errors.is_empty(), "a corrupt existing frame cannot become transparent");
}

#[test]
fn volume_rejects_missing_selected_temperature_even_without_blackbody() {
    let Some(gpu) = common::gpu() else { return };
    let file = common::fixtures().join("missing-selected-temperature.srvol");
    let mut volume = sr_volume::Volume::new();
    volume.insert("density", SparseGrid::new(Transform::identity(), 0.0, 0).unwrap()).unwrap();
    volume.write(std::fs::File::create(&file).unwrap()).unwrap();
    let xml = format!(
        r#"<scene version="1.3"><project width="4" height="4" fps="1" duration="1"/>
      <assets><volume id="smoke" src="{}" temperatureGrid="missing"/></assets>
      <composition><object3D id="cloud" primitive="volume" volume="smoke"><medium blackbody="false"/></object3D></composition></scene>"#,
        file.display()
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let result = common::render_times_on(gpu, &doc, &[0.0]).unwrap();
    assert!(result.stats.errors.iter().any(|e| e.contains("temperature channel missing")), "{:?}", result.stats);
}

#[test]
fn native_pyro_renders_and_invalidates_isolated_volume_content() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.3"><project width="4" height="4" fps="10" duration="2" background="#00000000"/>
      <composition><camera id="camera" projection="orthographic" x="2" y="2" z="-20" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/>
      <group id="isolate" isolate="true"><object3D id="cloud" primitive="volume" x="2" y="2">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1">
          <pyroSource shape="box" width="8" height="8" depth="8" densityRate="1" temperatureRate="3000" end="1"/>
        </pyro><medium blackbody="true" emissionScale="0.1" extinction="0.1" albedo="#000000"/>
      </object3D></group></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let mut frames = Vec::new();
    for t in [0.0, 0.5, 1.0, 0.5] {
        let graph = ev.evaluate(t);
        assert!(graph.problems.is_empty(), "{:?}", graph.problems);
        let f = renderer.render(&graph, ev.program());
        assert!(f.stats.errors.is_empty(), "{:?}", f.stats);
        frames.push(renderer.read(&f.texture));
    }
    assert!(frames[0].iter().all(|p| p[3] == 0.0));
    assert!(frames[1][10][3] > 0.1);
    assert!(frames[2][10][0] > frames[1][10][0] + 0.001);
    assert_eq!(frames[1], frames[3]);
}

#[test]
fn native_pyro_collider_carves_the_rendered_volume() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.3"><project width="4" height="4" fps="10" duration="1" background="#00000000"/>
      <composition><camera id="camera" projection="orthographic" x="2" y="2" z="-20" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/>
      <object3D id="solid" primitive="box" x="1" y="2" width="2" height="8" depth="8" visible="false"/>
      <object3D id="cloud" primitive="volume" x="2" y="2"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" colliders="solid">
      <pyroSource shape="box" width="8" height="8" depth="8" densityRate="1"/>
      </pyro><medium extinction="0.1" emissionColor="#FF4000" emissionScale="1" albedo="#000000"/></object3D>
      </composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let graph = ev.evaluate(0.1);
    assert!(graph.problems.is_empty(), "{:?}", graph.problems);
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let frame = renderer.render(&graph, ev.program());
    assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
    let pixels = renderer.read(&frame.texture);
    assert!(pixels[4][3] < 1e-5, "collider column: {:?}", pixels[4]);
    assert!(pixels[7][3] > 0.04 && pixels[7][0] > 0.01, "smoke column: {:?}", pixels[7]);
}

#[test]
fn baked_pyro_matches_native_pixels_with_parent_and_source_retiming() {
    let Some(gpu) = common::gpu() else { return };
    let pyro = r#"<pyro width="4" height="4" depth="4" voxelSize="1" dt="0.05"><pyroSource shape="box" width="4" height="4" depth="4" densityRate="1" temperatureRate="1000"/></pyro>"#;
    let xml = format!(
        r##"<scene version="1.3"><project width="4" height="4" fps="10" duration="1" background="#00000000"/><composition><camera id="camera" projection="orthographic" x="2" y="2" z="-20" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/><group id="clock" start="0.2" timeScale="2" isolate="true"><object3D id="cloud" primitive="volume" start="0.2" animationSpeed="2" x="2" y="2">{pyro}<medium blackbody="true" extinction="0.1" emissionScale="0.1" albedo="#000000"/></object3D></group></composition></scene>"##
    );
    let native = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&native, &Default::default()).unwrap();
    // A bake intentionally refuses to overwrite. Common image fixtures persist
    // across processes, so give this transaction its own exclusively owned parent.
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temp = (0..1024)
        .find_map(|i| {
            let path = std::env::temp_dir().join(format!("sr-gpu-bake-{}-{i}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => Some(Temp(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => panic!("create temporary bake directory: {e}"),
            }
        })
        .expect("temporary directory available");
    let root = temp.0.join("take");
    let receipt = ev.bake_pyro("cloud", &root, 0, 10, Default::default()).unwrap();
    let asset = format!(
        r#"<assets><volume id="take" src="{}" format="srvseq" sha256="{}" temperatureGrid="temperature"/></assets><composition>"#,
        receipt.manifest.display(),
        receipt.sha256
    );
    let baked_xml = xml
        .replace("<composition>", &asset)
        .replace(pyro, "")
        .replace("primitive=\"volume\"", "primitive=\"volume\" volume=\"take\"");
    let baked = sr_model::load_str(&baked_xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let times = [0.2, 0.5, 0.8, 0.5];
    let render_all = |doc: &sr_model::Document| {
        let ev = sr_eval::Evaluator::new(doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        times
            .iter()
            .map(|&time| {
                let frame = renderer.render(&ev.evaluate(time), ev.program());
                assert!(frame.stats.errors.is_empty(), "{:?}", frame.stats);
                renderer.read(&frame.texture)
            })
            .collect::<Vec<_>>()
    };
    let a = render_all(&native);
    let b = render_all(&baked);
    assert!(a[1][10][3] > 0.01);
    assert_eq!(a, b);
    let corrupt = baked_xml.replace(&receipt.sha256, &"0".repeat(64));
    let doc = sr_model::load_str(&corrupt, &sr_model::LoadOptions::without_assets()).unwrap();
    let bad = common::render_times_on(gpu, &doc, &[0.5]).unwrap();
    assert!(bad.stats.errors.iter().any(|e| e.contains("SHA-256")), "{:?}", bad.stats);
}

#[test]
fn openvdb_sequence_renders_thermal_fields_and_replays_local_time() {
    let Some(gpu) = common::gpu() else { return };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sr-volume/tests/data/openvdb");
    let xml = format!(
        r##"<scene version="1.3"><project width="4" height="4" fps="1" duration="4" background="#00000000"/>
        <assets><volume id="smoke" src="{}/render-%d.vdb" format="openvdb" first="0" last="1" interpolation="linear" temperatureGrid="temperature" boundsMinX="-10" boundsMinY="-10" boundsMinZ="0" boundsMaxX="10" boundsMaxY="10" boundsMaxZ="2"/></assets>
        <composition><camera id="camera" x="0" y="0" z="-10" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
        <group id="parent" isolate="true"><object3D id="cloud" primitive="volume" volume="smoke" start="1"><medium extinction="0.5" albedo="#000000" blackbody="true" emissionScale="0.1"/></object3D></group></composition></scene>"##,
        dir.display()
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: None }).unwrap();
    let frames = common::render_sub_frames(&doc, &[0.0, 1.0, 1.5, 2.0, 1.5]).unwrap();
    for frame in &frames {
        assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
    }
    assert_eq!(frames[0].at(2, 2), [0.0; 4]);
    assert_eq!(frames[2].px, frames[4].px);
    for (frame, density, kelvin) in [(1, 1., 2000.), (2, 2., 5000.), (3, 3., 8000.)] {
        let sigma = density * 0.5_f64;
        let alpha = 1. - (-sigma * 2.).exp();
        let expected = sr_volume::thermal::blackbody_rgb(kelvin).unwrap().map(|v| v * 0.1 * density * alpha / sigma);
        let p = frames[frame].at(2, 2);
        for c in 0..3 {
            assert!((f64::from(p[c]) - expected[c]).abs() < 0.003, "{p:?} vs {expected:?}");
        }
        assert!((f64::from(p[3]) - alpha).abs() < 0.001);
    }
    let retimed = xml.replace("start=\"1\"", "start=\"1\" animationSpeed=\"2\" animationOffset=\"0.25\"");
    let doc = sr_model::load_str(&retimed, &sr_model::LoadOptions::without_assets()).unwrap();
    let result = common::render_times_on(gpu, &doc, &[2., 1.125]).unwrap();
    assert!(result.stats.errors.is_empty(), "{:?}", result.stats);
    assert_eq!(result.px, frames[2].px);
}

#[test]
fn openvdb_sparse_pixels_match_independent_grid_and_cache_respects_format() {
    let Some(gpu) = common::gpu() else { return };
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sr-volume/tests/data/openvdb/density-blosc.vdb");
    let reference = common::fixtures().join("openvdb-reference.srvol");
    let mut grid = SparseGrid::new(Transform::identity(), 0., 8).unwrap();
    for (point, value) in [([1, 2, 3], 1.25), ([-1, -2, -3], 2.5), ([4, 5, 6], 0.75)] {
        grid.set(point, value).unwrap();
    }
    let mut volume = sr_volume::Volume::new();
    volume.insert("density", grid).unwrap();
    volume.write(std::fs::File::create(&reference).unwrap()).unwrap();
    let xml = format!(
        r##"<scene version="1.3"><project width="8" height="8" fps="1" duration="3" background="#00000000"/>
        <assets><volume id="smoke" src="{}" format="openvdb"/></assets>
        <composition><camera id="camera" x="0" y="0" z="-20" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
        <object3D id="cloud" primitive="volume" volume="smoke"><medium extinction="0.5" stepSize="0.25" albedo="#000000" emissionColor="#FF0000" emissionScale="0.1"/></object3D></composition></scene>"##,
        path.display()
    );
    let load = |xml: &str| sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    for renderer in ["raster", "pathtrace"] {
        let scene = xml.replace("renderer=\"pathtrace\"", &format!("renderer=\"{renderer}\""));
        let vdb = common::render_times_on(gpu.clone(), &load(&scene), &[0.]).unwrap();
        let expected = common::render_times_on(
            gpu.clone(),
            &load(
                &scene
                    .replace(&path.display().to_string(), &reference.display().to_string())
                    .replace("format=\"openvdb\"", "format=\"srvol\""),
            ),
            &[0.],
        )
        .unwrap();
        assert!(vdb.stats.errors.is_empty() && vdb.stats.unsupported.is_empty(), "{:?}", vdb.stats);
        assert!(vdb.px.iter().any(|p| p[3] > 0.01), "fixture must produce visible pixels");
        assert_eq!(vdb.px, expected.px);
    }
    // Cache admission must include decoder identity. A prior failed SRVOL read
    // cannot poison a valid VDB read, nor may a cached VDB hide a wrong format.
    let asset = format!(r#"<volume id="wrong" src="{}" format="srvol"/>"#, path.display());
    let object = r##"<object3D id="bad" primitive="volume" volume="wrong"><medium/></object3D>"##;
    let mixed = xml
        .replace("</assets>", &format!("{asset}</assets>"))
        .replace("</composition>", &format!("{object}</composition>"));
    for wrong_first in [false, true] {
        let (good_start, bad_start) = if wrong_first { (1, 0) } else { (0, 1) };
        let scene = mixed
            .replace("id=\"cloud\"", &format!(r#"id="cloud" start="{good_start}" end="{}""#, good_start + 1))
            .replace("id=\"bad\"", &format!(r#"id="bad" start="{bad_start}" end="{}""#, bad_start + 1));
        let result = common::render_times_on(gpu.clone(), &load(&scene), &[0.5, 1.5]).unwrap();
        assert_eq!(result.stats.errors.is_empty(), wrong_first, "{:?}", result.stats);
        if wrong_first {
            assert!(result.px.iter().any(|p| p[3] > 0.01));
        }
    }
}

#[test]
fn advected_cache_documents_move_fields_replay_and_validate_channels() {
    let Some(gpu) = common::gpu() else { return };
    let dir = common::fixtures().join("advection");
    std::fs::create_dir_all(&dir).unwrap();
    let make = |center: i32, velocity: bool| {
        let mut cache = sr_volume::Volume::new();
        for (name, value) in [("density", 1.), ("heat", 5000.)] {
            let mut grid = SparseGrid::new(Transform::identity(), 0., 32).unwrap();
            for x in center - 2..=center + 2 {
                for y in -4..=4 {
                    for z in 0..=3 {
                        grid.set([x, y, z], value).unwrap();
                    }
                }
            }
            cache.insert(name, grid).unwrap();
        }
        if velocity {
            for (name, value) in [("vx", 10.), ("vy", 0.), ("vz", 0.)] {
                cache.insert(name, SparseGrid::new(Transform::identity(), value, 0).unwrap()).unwrap();
            }
        }
        cache
    };
    let first = make(-5, true);
    let last = make(5, true);
    for (name, cache) in [("0", &first), ("1", &last), ("reference", &make(0, false))] {
        cache.write(std::fs::File::create(dir.join(format!("{name}.srvol"))).unwrap()).unwrap();
    }
    let xml = format!(
        r##"<scene version="1.3"><project width="4" height="4" fps="1" duration="4" background="#00000000"/>
        <assets><volume id="smoke" src="{}/%d.srvol" first="0" last="1" interpolation="advect" velocityGridX="vx" velocityGridY="vy" velocityGridZ="vz" temperatureGrid="heat"/></assets>
        <composition><camera id="camera" x="0" y="0" z="-10" projection="orthographic" renderer="pathtrace" pathSamples="1" denoise="false"/>
        <object3D id="cloud" primitive="volume" volume="smoke" start="1"><medium extinction="0.5" albedo="#000000" blackbody="true" emissionScale="0.1" stepSize="0.125" maxSteps="65536"/></object3D></composition></scene>"##,
        dir.display()
    );
    let render = |xml: &str, times: &[f64]| {
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        common::render_times_on(gpu.clone(), &doc, times).unwrap()
    };
    let controls =
        r#" first="0" last="1" interpolation="advect" velocityGridX="vx" velocityGridY="vy" velocityGridZ="vz""#;
    for renderer in ["pathtrace", "raster"] {
        let xml = xml.replace("pathtrace", renderer);
        let actual = render(&xml, &[1., 2., 1.5]);
        assert!(actual.stats.errors.is_empty() && actual.stats.unsupported.is_empty(), "{:?}", actual.stats);
        let expected = render(&xml.replace("%d.srvol", "reference.srvol").replace(controls, ""), &[1.5]);
        assert!(expected.at(2, 2)[3] > 0.5, "reference must contain visible smoke");
        for (a, b) in actual.px.iter().flatten().zip(expected.px.iter().flatten()) {
            assert!((a - b).abs() < 0.003, "{renderer}: advected {a}, reference {b}");
        }
        assert_eq!(actual.px, render(&xml, &[1.5]).px);
        let retimed = xml.replace("start=\"1\"", "start=\"1\" animationSpeed=\"2\" animationOffset=\"0.25\"");
        assert_eq!(actual.px, render(&retimed, &[1.125]).px);
        let linear = render(&xml.replace(controls, r#" first="0" last="1" interpolation="linear""#), &[1.5]);
        assert!(linear.at(2, 2)[3] < 0.01, "linear must leave the gap empty");
        let missing = render(&xml.replace("velocityGridZ=\"vz\"", "velocityGridZ=\"missing\""), &[1.]);
        assert!(missing.stats.errors.iter().any(|e| e.contains("velocity channel missing")), "{:?}", missing.stats);
        let hold = xml.replace("last=\"1\"", "last=\"3\" missingFrame=\"hold\"");
        assert_eq!(render(&hold, &[3.5]).px, render(&xml, &[2.]).px, "held endpoints freeze");
    }
    // Bakes use absolute composition time; object-local retiming must not alter it.
    let mut writer = sr_volume::bake::BakeWriter::new(&dir.join("bake"), 1., 1., Default::default()).unwrap();
    writer.push(Some(&first)).unwrap();
    writer.push(Some(&last)).unwrap();
    let receipt = writer.finish().unwrap();
    let baked = xml
        .replace("%d.srvol", "bake/manifest.srvseq")
        .replace("first=\"0\" last=\"1\"", &format!(r#"format="srvseq" sha256="{}""#, receipt.sha256))
        .replace("start=\"1\"", "start=\"1\" animationSpeed=\"9\"");
    assert_eq!(render(&baked, &[1., 2., 1.5]).px, render(&xml, &[1.5]).px);
    // Equal content hashes share resident storage, not their sample timestamps.
    // Numbered files and digest-deduplicated bakes must follow the same traces.
    let mut repeated = sr_volume::bake::BakeWriter::new(&dir.join("repeated"), 1., 1., Default::default()).unwrap();
    for frame in 0..2 {
        repeated.push(Some(&first)).unwrap();
        first.write(std::fs::File::create(dir.join(format!("repeated-{frame}.srvol"))).unwrap()).unwrap();
    }
    let receipt = repeated.finish().unwrap();
    let repeated_bake = xml
        .replace("%d.srvol", "repeated/manifest.srvseq")
        .replace("first=\"0\" last=\"1\"", &format!(r#"format="srvseq" sha256="{}""#, receipt.sha256));
    for renderer in ["pathtrace", "raster"] {
        let numbered_xml = xml.replace("%d.srvol", "repeated-%d.srvol").replace("pathtrace", renderer);
        let baked_xml = repeated_bake.replace("pathtrace", renderer);
        for time in [1.5, 1.25, 1.75, 1.5] {
            let numbered = render(&numbered_xml, &[time]);
            assert!(numbered.stats.errors.is_empty(), "{:?}", numbered.stats);
            let baked = render(&baked_xml, &[1., 1.75, time]);
            assert!(baked.stats.errors.is_empty(), "{:?}", baked.stats);
            assert_eq!(baked.px, numbered.px, "{renderer}: shared frame storage at {time}");
            if time == 1.5 {
                assert!(numbered.at(2, 2)[3] > 0.1);
            }
        }
    }
}

/// A ball of smoke hovering over a mirror, written to a cache file; the domain spans
/// y in [-16, 6], across the mirror's plane (y = 0, up is negative y).
fn mirror_ball(name: &str) -> std::path::PathBuf {
    let mut ball = SparseGrid::new(Transform::identity(), 0.0, 16).unwrap();
    for z in -6..=6 {
        for y in -15..=-1 {
            for x in -6..=6 {
                let (dx, dy, dz) = (x as f32, (y + 8) as f32, z as f32);
                let r = (dx * dx + dy * dy + dz * dz).sqrt();
                if r <= 6.5 {
                    ball.set([x, y, z], (1.0 - r / 6.5).max(0.05)).unwrap();
                }
            }
        }
    }
    let mut cache = sr_volume::Volume::new();
    cache.insert("density", ball).unwrap();
    let path = common::fixtures().join(name);
    cache.write(std::fs::File::create(&path).unwrap()).unwrap();
    path
}

/// The ball seen by a camera `camera_y` above the plane of a mirror (or none), through a
/// medium and light rig given as markup.
fn mirror_view(
    gpu: &sr_gpu::Gpu,
    cache: &std::path::Path,
    camera_y: f64,
    mirror: bool,
    half_depth: u32,
    medium: &str,
    lights: &str,
) -> common::Rendered {
    let xml = mirror_xml(cache, camera_y, mirror, half_depth, medium, lights, 32);
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(common::fixtures()) })
            .unwrap();
    let shot = common::render_times_on(gpu.clone(), &doc, &[0.0]).unwrap();
    assert!(shot.stats.errors.is_empty(), "{:?}", shot.stats.errors);
    shot
}

fn mirror_xml(
    cache: &std::path::Path,
    camera_y: f64,
    mirror: bool,
    half_depth: u32,
    medium: &str,
    lights: &str,
    samples: u32,
) -> String {
    let plane = if mirror {
        r##"<object3D id="water" primitive="plane" width="400" height="400" rotationX="-90" material="mirror"/>"##
    } else {
        ""
    };
    format!(
        r##"<scene version="1.3"><project width="48" height="48" fps="10" duration="1" background="#00000000"/>
      <assets><volume id="smoke" src="{}" boundsMinX="-9" boundsMinY="-16" boundsMinZ="-{half_depth}" boundsMaxX="9" boundsMaxY="6" boundsMaxZ="{half_depth}"/></assets>
      <materials><material id="mirror" baseColor="#FFFFFF" metallic="1" roughness="0.02"/></materials>
      <composition><camera id="camera" x="0" y="{camera_y}" z="-18" fov="90" renderer="pathtrace" pathSamples="{samples}" maxBounces="2" denoise="false"/>
      {plane}
      <object3D id="cloud" primitive="volume" volume="smoke">{medium}</object3D>
      </composition><lights>{lights}</lights></scene>"##,
        cache.display()
    )
}

/// How far the mirror's lower half is from the mirrored camera's view of the same scene (the
/// mirror fills the frame's lower half; the mirrored camera sees that image upside down), as a
/// fraction of how much the ball changes that view away from what is behind it, and where the
/// worst pixel is.
fn mirror_error(reflected: &common::Rendered, mirrored: &common::Rendered) -> (f32, f32, (u32, u32)) {
    let h = reflected.size[1];
    let (mut worst, mut at, mut total, mut effect) = (0.0f32, (0, 0), 0.0f32, 0.0f32);
    for y in h / 2 + 4..h {
        // the column at the frame's edge sees only what lies behind the ball
        let behind = mirrored.at(0, h - 1 - y)[0];
        for x in 0..reflected.size[0] {
            let (a, b) = (reflected.at(x, y)[0], mirrored.at(x, h - 1 - y)[0]);
            total += (a - b).abs();
            effect += (b - behind).abs();
            if (a - b).abs() > worst {
                (worst, at) = ((a - b).abs(), (x, y));
            }
        }
    }
    assert!(effect > 1.0, "the ball changes the mirrored view: {effect}");
    (total / effect, worst, at)
}

/// A smoke ball hovering over a mirror must reflect exactly as it looks from the mirrored
/// camera, whether the water under it lies in front of the domain (the reflected ray enters
/// it, half depth 8) or inside it (the reflected ray starts in the medium, half depth 15).
#[test]
fn emissive_volume_seen_in_a_mirror_matches_the_mirrored_camera_view() {
    let Some(gpu) = common::gpu() else { return };
    let cache = mirror_ball("mirror_ball.srvol");
    let medium = r##"<medium extinction="0.4" albedo="#000000" emissionColor="#FFA040" emissionScale="0.3" stepSize="0.5" maxSteps="512"/>"##;
    let lights = r##"<light id="dark" type="ambient" color="#000000" intensity="0"/>"##;
    for half_depth in [8, 15] {
        let reflected = mirror_view(&gpu, &cache, -3.0, true, half_depth, medium, lights);
        let mirrored = mirror_view(&gpu, &cache, 3.0, false, half_depth, medium, lights);
        let (error, worst, at) = mirror_error(&reflected, &mirrored);
        assert!(
            error < 0.05,
            "half depth {half_depth}: mirror differs from the mirrored view by {error} (worst {worst} at {at:?})"
        );
    }
}

/// The same for a medium that only absorbs, against a visible sky: the mirror shows the sky
/// dimmed as the mirrored camera sees it.
#[test]
fn absorbing_volume_seen_in_a_mirror_matches_the_mirrored_camera_view() {
    let Some(gpu) = common::gpu() else { return };
    let cache = mirror_ball("mirror_ball.srvol");
    let medium = r##"<medium extinction="0.4" albedo="#000000" stepSize="0.5" maxSteps="512"/>"##;
    let lights = r##"<light id="sky" type="dome" environment="gray.png" environmentVisible="true" intensity="1"/>"##;
    for half_depth in [8, 15] {
        let reflected = mirror_view(&gpu, &cache, -3.0, true, half_depth, medium, lights);
        let mirrored = mirror_view(&gpu, &cache, 3.0, false, half_depth, medium, lights);
        let (error, worst, at) = mirror_error(&reflected, &mirrored);
        assert!(
            error < 0.05,
            "half depth {half_depth}: mirror differs from the mirrored view by {error} (worst {worst} at {at:?})"
        );
    }
}
