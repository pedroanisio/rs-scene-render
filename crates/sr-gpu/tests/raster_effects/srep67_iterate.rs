//! SREP 67, Semantics 4: `iterate` restarts the stateful shaders inside it from their zero state at every frame and
//! advances them `steps` steps (fewer with `until="converged"`). The counter follows sr-core's kit shader
//! `srep67-counter.fs`; here it draws the count / 64 in red.

use super::common;
use common::*;

const COUNTER: &str = r#"/*{ "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}],
    "PASSES": [{"TARGET": "acc", "PERSISTENT": true, "FLOAT": true}, {}] }*/
void main() {
    if (PASSINDEX == 0) {
        vec4 prev = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
        gl_FragColor = vec4(prev.r + 1.0, float(FRAMEINDEX), 0.0, 1.0);
    } else {
        vec4 v = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
        gl_FragColor = vec4(v.r / 64.0, v.g / 64.0, 0.0, 1.0);
    }
}
"#;

/// Approaches 1 by halving the distance each step: converges to within any tolerance in a few dozen steps.
const HALVING: &str = r#"/*{ "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}],
    "PASSES": [{"TARGET": "acc", "PERSISTENT": true, "FLOAT": true}, {}] }*/
void main() {
    if (PASSINDEX == 0) {
        vec4 prev = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
        gl_FragColor = vec4(prev.r + 0.5 * (1.0 - prev.r), 0.0, 0.0, 1.0);
    } else {
        gl_FragColor = vec4(IMG_NORM_PIXEL(acc, isf_FragNormCoord).r, 0.0, 0.0, 1.0);
    }
}
"#;

fn doc(iterate: &str, shader: &str, name: &str) -> sr_model::Document {
    std::fs::write(fixtures().join(name), shader).unwrap();
    let xml = format!(
        r##"<scene version="1.6"><project width="64" height="32" fps="10" duration="4" background="#00000000"/>{ASSETS}<composition><iterate id="it" {iterate}><layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="f"/></iterate></composition><effects><effect id="f" type="shader" src="{name}" space="raw" stepsPerFrame="7" prewarm="3"/></effects></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn render_seq(d: &sr_model::Document, ts: &[f64]) -> Option<Rendered> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let mut last = None;
    for &t in ts {
        let g = ev.evaluate(t);
        let mut sub = |t: f64| ev.evaluate(t);
        let f = r.render_with(&g, ev.program(), Some(&mut sub));
        last = Some((r.read(&f.texture), f.stats, f.texture.size));
    }
    let (px, stats, size) = last?;
    Some(Rendered { px, size, stats, renderer: r })
}

#[test]
fn iterate_takes_its_steps_from_zero_at_every_frame() {
    // kit srep-0067-iterate-steps (10) and -short (9); stepsPerFrame and prewarm are ignored inside an iterate
    for steps in [9u64, 10, 40] {
        let d = doc(&format!(r#"steps="{steps}""#), COUNTER, "srep67-counter.fs");
        let Some(one) = render_seq(&d, &[0.0]) else { return };
        let [count, last_index, ..] = one.at(16, 16);
        assert!((count * 64.0 - steps as f32).abs() < 0.01, "{steps}: {}", count * 64.0);
        // FRAMEINDEX counts the steps from 0
        assert!((last_index * 64.0 - (steps - 1) as f32).abs() < 0.01, "{steps}: FRAMEINDEX {}", last_index * 64.0);
        assert_eq!(one.stats.iterate_steps, vec![("f|a".to_string(), steps)]);
        // the state does not carry over: frames in order, and a seek, give the same picture without replaying
        let three = render_seq(&d, &[0.0, 0.1, 0.2]).unwrap();
        assert_eq!(three.px, one.px);
        let seek = render_seq(&d, &[0.3]).unwrap();
        assert_eq!(seek.px, one.px);
        assert_eq!(seek.stats.replayed_frames, 0);
    }
}

#[test]
fn until_converged_stops_early_and_never_passes_the_ceiling() {
    let d = doc(r#"steps="1000" until="converged" tolerance="0.001" checkEvery="4""#, HALVING, "srep67-halving.fs");
    let Some(r) = render_seq(&d, &[0.0]) else { return };
    let taken = r.stats.iterate_steps[0].1;
    // the change over 4 steps falls under 0.001 after a dozen or so steps; it stops on a multiple of checkEvery
    assert!(taken < 100 && taken % 4 == 0, "{taken} steps");
    assert!((r.at(16, 16)[0] - 1.0).abs() < 0.002, "{:?}", r.at(16, 16));
    // the ceiling holds when it does not converge in time
    let capped =
        doc(r#"steps="8" until="converged" tolerance="0.000001" checkEvery="4""#, HALVING, "srep67-halving.fs");
    let r = render_seq(&capped, &[0.0]).unwrap();
    assert_eq!(r.stats.iterate_steps[0].1, 8);
    // until="steps" (the default) runs every step
    let all = doc(r#"steps="100""#, HALVING, "srep67-halving.fs");
    assert_eq!(render_seq(&all, &[0.0]).unwrap().stats.iterate_steps[0].1, 100);
}

#[test]
fn a_node_outside_the_iterate_keeps_its_state_from_frame_to_frame() {
    std::fs::write(fixtures().join("srep67-counter.fs"), COUNTER).unwrap();
    let xml = format!(
        r##"<scene version="1.6"><project width="64" height="32" fps="10" duration="4" background="#00000000"/>{ASSETS}<composition><iterate id="it" steps="5"><layer id="a" asset="red" x="0" y="0" scaleX="2" scaleY="2" effects="f"/></iterate><layer id="b" asset="red" x="40" y="8" scaleX="4" scaleY="4" effects="f"/></composition><effects><effect id="f" type="shader" src="srep67-counter.fs" space="raw"/></effects></scene>"##
    );
    let d =
        sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) }).unwrap();
    let Some(r) = render_seq(&d, &[0.0, 0.1, 0.2]) else { return };
    assert!((r.at(4, 4)[0] * 64.0 - 5.0).abs() < 0.01, "inside: {}", r.at(4, 4)[0] * 64.0);
    assert!((r.at(48, 16)[0] * 64.0 - 3.0).abs() < 0.01, "outside: {}", r.at(48, 16)[0] * 64.0);
}
