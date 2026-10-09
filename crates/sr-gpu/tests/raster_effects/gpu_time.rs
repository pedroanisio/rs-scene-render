//! GPU time, measured with timestamp queries where the adapter has them: per frame and per
//! effect pass, apart from the CPU time spent planning and submitting.

use super::common;

use common::*;
use sr_gpu::Renderer;

fn scene() -> sr_model::Document {
    doc_with(
        r##"background="#203040""##,
        "",
        r#"<layer id="img" asset="wide" x="4" y="4" scaleX="6" scaleY="6" effects="soft ink"/><adjustment id="finish" effects="grade"/>"#,
        r##"<effects><effect id="soft" type="blur" radius="0.5"/><effect id="ink" type="stroke" size="0.5" color="#FF0000"/><effect id="grade" type="color-grade" saturation="1.2"/></effects>"##,
    )
}

#[test]
fn gpu_time_is_reported_per_frame_and_per_effect_pass() {
    let Some(gpu) = gpu() else { return };
    let d = scene();
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu.clone(), ev.program());
    r.time_gpu = true;
    let f = r.render(&ev.evaluate(0.0), ev.program());
    gpu.wait();
    let Some(t) = r.gpu_times() else {
        assert!(!gpu.timestamps, "the adapter has timestamp queries but nothing was timed");
        eprintln!("skipping: {} has no timestamp queries", gpu.info.name);
        return;
    };
    assert_eq!(t.passes.len(), f.stats.fx_passes, "one time per effect pass: {:?}", t.passes);
    assert!(t.passes.iter().all(|p| p.ms >= 0.0 && p.ms.is_finite()), "{:?}", t.passes);
    assert!(t.passes.iter().any(|p| p.label.contains("img")), "passes are labelled by their node: {:?}", t.passes);
    assert!(t.passes.iter().any(|p| p.label.contains("finish")), "{:?}", t.passes);
    if let Some(frame) = t.frame_ms {
        let sum: f64 = t.passes.iter().map(|p| p.ms).sum();
        assert!(frame > 0.0 && sum <= frame * 1.05 + 0.05, "passes {sum} ms within the frame's {frame} ms");
    }
}

#[test]
fn nothing_is_timed_unless_asked() {
    let Some(gpu) = gpu() else { return };
    let d = scene();
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu.clone(), ev.program());
    r.render(&ev.evaluate(0.0), ev.program());
    gpu.wait();
    assert!(r.gpu_times().is_none());
}

#[test]
fn raster_3d_timings_separate_shadows_geometry_and_depth_of_field_without_changing_pixels() {
    let Some(gpu) = gpu() else { return };
    let d = doc_with(
        "",
        "",
        r#"<camera id="cam" fov="60" x="32" y="16" z="-60" depthOfField="true" fStop="2" focusTarget="box"/>
        <object3D id="box" primitive="box" width="20" height="20" depth="20" x="32" y="16" z="10"/>"#,
        r#"<lights><light id="sun" type="directional" intensity="2" pitch="-35" yaw="70" castShadow="true" shadowMapSize="64"/></lights>"#,
    );
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let g = ev.evaluate(0.0);
    let mut plain = Renderer::new(gpu.clone(), ev.program());
    let f = plain.render(&g, ev.program());
    let expected = plain.read(&f.texture);
    assert!(plain.gpu_times().is_none());
    let mut timed = Renderer::new(gpu.clone(), ev.program());
    timed.time_gpu = true;
    let f = timed.render(&g, ev.program());
    assert!(f.stats.objects3d > 0);
    assert!(f.stats.errors.is_empty(), "{:?}", f.stats.errors);
    assert_eq!(timed.read(&f.texture), expected);
    let Some(t) = timed.gpu_times() else {
        assert!(!gpu.timestamps);
        return;
    };
    for label in
        ["three-shadow", "three-opaque", "three-depth-resolve", "three-coc-tiles", "three-coc-dilate", "three-dof"]
    {
        assert!(t.passes.iter().any(|p| p.label == label), "missing {label}: {:?}", t.passes);
    }
    assert!(t.passes.iter().all(|p| p.ms.is_finite() && p.ms >= 0.0));
    if let Some(frame) = t.frame_ms {
        let sum: f64 = t.passes.iter().map(|p| p.ms).sum();
        assert!(sum <= frame * 1.05 + 0.05, "passes {sum} ms within frame {frame} ms");
    }
}
