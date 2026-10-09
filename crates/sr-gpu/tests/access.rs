//! Batched contrast probes preserve independent counterfactual measurements.
mod common;

use sr_gpu::{ContrastTarget, Renderer};

#[test]
fn contrast_counterfactual_reuses_identical_3d_shutter_results() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.1">
      <project width="128" height="96" fps="30" duration="2" background="#000000"
        motionBlur="true" motionBlurSamples="4" adaptiveMotionBlur="false"/>
      <assets><text id="text" text="AB" width="48" height="32" size="24" color="#ffffff" font="DejaVu Sans"/></assets>
      <materials><material id="gray" baseColor="#444444" roughness="0.8"/></materials>
      <composition>
        <camera id="cam" x="64" y="48" z="-120" fov="60" depthOfField="true" fStop="2" focusTarget="box"/>
        <object3D id="box" primitive="box" width="110" height="70" depth="20" x="64" y="48" material="gray">
          <animate property="rotationY"><key time="0" value="0"/><key time="2" value="20"/></animate>
        </object3D>
        <group id="label" isolate="true"><layer id="ink" asset="text" x="40" y="32"/></group>
      </composition>
      <lights><light id="sun" type="directional" intensity="2" yaw="35" pitch="-48" castShadow="true" shadowMapSize="64"/></lights>
    </scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let g = ev.evaluate(0.5);
    let hide = |mut graph: sr_eval::FrameGraph| {
        graph.nodes.iter_mut().find(|n| &*n.id == "ink").unwrap().draw = false;
        graph
    };
    // Independent full renders establish the ratio without the contrast API's reuse.
    let mut reference = Renderer::new(gpu.clone(), ev.program());
    reference.time_gpu = true;
    let with = reference.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
    let after = reference.read(&with.texture);
    let without = reference.render_with(&hide(g.clone()), ev.program(), Some(&mut |t| hide(ev.evaluate(t))));
    let before = reference.read(&without.texture);
    let expected = reference.contrast_of(&before, &after);
    assert!(expected.is_some_and(|ratio| ratio > 1.0));
    if gpu.timestamps {
        assert!(reference.gpu_times().unwrap().passes.iter().any(|p| p.label == "three-opaque"));
    }
    let mut probe = Renderer::new(gpu.clone(), ev.program());
    probe.time_gpu = true;
    let actual =
        probe.contrast_with_without(&g, ev.program(), &ContrastTarget::Node("ink".into()), &mut |t| ev.evaluate(t));
    assert_eq!(actual, expected);
    if gpu.timestamps {
        let times = probe.gpu_times().unwrap();
        assert!(
            times.passes.iter().all(|p| !p.label.starts_with("three-")),
            "counterfactual rerendered identical 3D: {:?}",
            times.passes
        );
    }
    // Reuse is scoped to the probe. Ordinary renders retain their normal lifecycle.
    let restored = probe.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
    assert_eq!(probe.read(&restored.texture), after);
    if gpu.timestamps {
        assert!(probe.gpu_times().unwrap().passes.iter().any(|p| p.label == "three-opaque"));
    }
}

#[test]
fn contrast_reuse_tracks_changed_geometry_at_identical_sample_times() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.1">
      <project width="96" height="64" fps="30" duration="2" background="#000000"
        motionBlur="true" motionBlurSamples="4" adaptiveMotionBlur="false"/>
      <assets><text id="text" text="AB" width="48" height="32" size="24" color="#ffffff" font="DejaVu Sans"/></assets>
      <materials><material id="gray" baseColor="#444444" unlit="true"/></materials>
      <composition>
        <object3D id="solid" primitive="box" width="28" height="36" depth="10" x="48" y="32" material="gray"/>
        <group id="label" isolate="true"><layer id="ink" asset="text" x="24" y="16"/></group>
      </composition>
    </scene>"##;
    let original = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let changed =
        sr_model::load_str(&xml.replace("width=\"28\"", "width=\"80\""), &sr_model::LoadOptions::without_assets())
            .unwrap();
    let a = sr_eval::Evaluator::new(&original, &Default::default()).unwrap();
    let b = sr_eval::Evaluator::new(&changed, &Default::default()).unwrap();
    let g = a.evaluate(0.5);
    let hide = |mut graph: sr_eval::FrameGraph| {
        graph.nodes.iter_mut().find(|n| &*n.id == "ink").unwrap().draw = false;
        graph
    };
    let mut reference = Renderer::new(gpu.clone(), a.program());
    let mut original_calls = 0;
    let with = reference.render_with(
        &g,
        a.program(),
        Some(&mut |t| {
            original_calls += 1;
            a.evaluate(t)
        }),
    );
    let after = reference.read(&with.texture);
    assert_eq!(original_calls, 4);
    let without = reference.render_with(&hide(g.clone()), a.program(), Some(&mut |t| hide(b.evaluate(t))));
    let expected = reference.contrast_of(&reference.read(&without.texture), &after);
    assert!(expected.is_some());
    let mut probe = Renderer::new(gpu.clone(), a.program());
    probe.time_gpu = true;
    let mut calls = 0;
    let actual = probe.contrast_with_without(&g, a.program(), &ContrastTarget::Node("ink".into()), &mut |t| {
        calls += 1;
        if calls <= original_calls {
            a.evaluate(t)
        } else {
            b.evaluate(t)
        }
    });
    assert_eq!(calls, 8);
    assert_eq!(actual, expected, "time and node identity alone cannot key a rendered sample");
    if gpu.timestamps {
        assert!(
            probe.gpu_times().unwrap().passes.iter().any(|p| p.label == "three-opaque"),
            "changed geometry must render again"
        );
    }
}

#[test]
fn batched_contrast_preserves_ratios_and_reuses_the_original_frame() {
    let Some(gpu) = common::gpu() else { return };
    let project = r##"width="128" height="96" fps="30" duration="2" background="#000000"
            motionBlur="true" motionBlurSamples="4" adaptiveMotionBlur="false""##;
    let assets = r##"<assets>
          <text id="white" text="AB" width="48" height="32" size="24" color="#ffffff" font="DejaVu Sans"/>
          <text id="black" text="AB" width="48" height="32" size="24" color="#000000" font="DejaVu Sans"/>
        </assets>"##;
    let body = r##"<group id="group" isolate="true">
          <layer id="bright" asset="white" x="2" y="2"/>
          <layer id="equal" asset="black" x="64" y="2"/>
          <layer id="covered" asset="white" x="2" y="48"/>
          <shape id="cover" shape="rect" x="0" y="46" width="56" height="40" fill="#000000"/>
          <layer id="moving" asset="white" x="64" y="48">
            <animate property="x"><key time="0" value="64"/><key time="2" value="80"/></animate>
          </layer>
        </group>
        <shape id="inline_bg" shape="rect" x="108" y="80" width="20" height="16" fill="#818181" opacity="0.37"/>
        <layer id="inline_text" asset="white" x="108.3" y="80.7" opacity="0.37"/>"##;
    let xml = format!(r#"<scene version="1.1"><project {project}/>{assets}<composition>{body}</composition></scene>"#);
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let g = ev.evaluate(0.5);
    let targets: Vec<_> = ["bright", "equal", "covered", "moving", "missing"]
        .into_iter()
        .map(|id| ContrastTarget::Node(id.into()))
        .chain([ContrastTarget::Captions])
        .collect();
    let mut reference = Renderer::new(gpu.clone(), ev.program());
    let mut reference_calls = 0;
    let expected: Vec<_> = targets
        .iter()
        .map(|target| {
            reference.contrast_with_without(&g, ev.program(), target, &mut |t| {
                reference_calls += 1;
                ev.evaluate(t)
            })
        })
        .collect();
    assert!(expected[0].unwrap() > 10.0);
    assert_eq!(expected[1], Some(1.0), "equal ink still requires the coverage probes");
    assert_eq!(expected[2], None, "later occlusion remains part of the counterfactual");
    assert!(expected[3].unwrap() > 10.0);
    assert_eq!(expected[4], None);
    assert_eq!(expected[5], None);

    let mut batch = Renderer::new(gpu.clone(), ev.program());
    batch.contrast_probe = true;
    let mut batch_calls = 0;
    let actual = batch.contrasts_with_without(&g, ev.program(), &targets, &mut |t| {
        batch_calls += 1;
        ev.evaluate(t)
    });
    assert_eq!(actual, expected);
    assert!(batch.contrast_probe, "restore the caller's probe setting");
    assert!(batch_calls < reference_calls, "batch {batch_calls}, independent {reference_calls}");
    let mut plain = Renderer::new(gpu.clone(), ev.program());
    plain.contrast_probe = true;
    let mut streaming = Renderer::new(gpu, ev.program());
    for (k, time) in [0.5, 0.5, 0.8, 0.8].into_iter().enumerate() {
        let g = ev.evaluate(time);
        let ordinary = plain.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
        let pixels = plain.read(&ordinary.texture);
        let (main, prefetched) =
            streaming.render_with_deferred_contrasts(&g, ev.program(), &targets, &mut |t| ev.evaluate(t));
        assert_eq!(streaming.read(&main.texture), pixels);
        assert_eq!(
            main.stats.contrast_unprobed, ordinary.stats.contrast_unprobed,
            "prefetch must preserve subsequent cache-dependent observations, frame {k}"
        );
        assert_eq!(main.stats.contrast, ordinary.stats.contrast);
        if k == 0 {
            let selected: Vec<_> = targets
                .iter()
                .cloned()
                .zip(expected.iter().copied())
                .filter(|(id, _)| main.stats.contrast_unprobed.contains(id))
                .collect();
            assert_eq!(prefetched, selected, "prefetch preserves coverage probes and occluded text");
            assert_eq!(prefetched.len(), 4, "missing nodes and absent captions are not prefetched");
        }
    }
    let reversed: Vec<_> = targets.into_iter().rev().collect();
    assert_eq!(
        batch.contrasts_with_without(&g, ev.program(), &reversed, &mut |t| ev.evaluate(t)),
        expected.into_iter().rev().collect::<Vec<_>>()
    );
    assert!(batch.contrasts_with_without(&g, ev.program(), &[], &mut |_| panic!("empty batch evaluated")).is_empty());
}

#[test]
fn deferred_contrast_reuses_the_main_frame_and_returns_stable_pixels() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.1">
      <project width="128" height="96" fps="30" duration="2" background="#000000"
        motionBlur="true" motionBlurSamples="4" adaptiveMotionBlur="false"/>
      <assets><text id="text" text="AB" width="48" height="32" size="24" color="#ffffff" font="DejaVu Sans"/></assets>
      <materials><material id="gray" baseColor="#444444" roughness="0.8"/></materials>
      <composition>
        <camera id="cam" x="64" y="48" z="-120" fov="60" depthOfField="true" fStop="2" focusTarget="box"/>
        <object3D id="box" primitive="box" width="110" height="70" depth="20" x="64" y="48" material="gray">
          <animate property="rotationY"><key time="0" value="0"/><key time="2" value="20"/></animate>
        </object3D>
        <group id="label" isolate="true"><layer id="ink" asset="text" x="40" y="32"/></group>
      </composition>
      <lights><light id="sun" type="directional" intensity="2" yaw="35" pitch="-48" castShadow="true" shadowMapSize="64"/></lights>
    </scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let g = ev.evaluate(0.5);
    let target = ContrastTarget::Node("ink".into());
    let mut reference = Renderer::new(gpu.clone(), ev.program());
    reference.contrast_probe = true;
    let frame = reference.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
    let expected_pixels = reference.read(&frame.texture);
    let expected_ratio = reference.contrast_with_without(&g, ev.program(), &target, &mut |t| ev.evaluate(t));
    assert!(expected_ratio.is_some_and(|r| r > 1.0));
    let mut renderer = Renderer::new(gpu.clone(), ev.program());
    renderer.time_gpu = true;
    assert_eq!(renderer.contrast_candidates(&g, ev.program()), vec![(target.clone(), 1.0)]);
    let mut calls = 0;
    let (frame, measured) =
        renderer.render_with_deferred_contrasts(&g, ev.program(), std::slice::from_ref(&target), &mut |t| {
            calls += 1;
            ev.evaluate(t)
        });
    assert_eq!(renderer.read(&frame.texture), expected_pixels);
    assert_eq!(measured, vec![(target, expected_ratio)]);
    assert!(!renderer.contrast_probe, "restore caller's setting");
    assert!(frame.stats.errors.is_empty());
    assert_eq!(calls, 8, "deferred probes must not render the original frame twice");
    if gpu.timestamps {
        assert!(
            renderer.gpu_times().unwrap().passes.iter().all(|p| !p.label.starts_with("three-")),
            "counterfactuals reuse the main frame's 3D shutter samples"
        );
    }
    let next = renderer.render_with(&ev.evaluate(0.8), ev.program(), Some(&mut |t| ev.evaluate(t)));
    let mut fresh = Renderer::new(gpu, ev.program());
    let expected = fresh.render_with(&ev.evaluate(0.8), ev.program(), Some(&mut |t| ev.evaluate(t)));
    assert_eq!(renderer.read(&next.texture), fresh.read(&expected.texture));
    assert_eq!(renderer.read(&frame.texture), expected_pixels, "the returned main image survives subsequent renders");
}
