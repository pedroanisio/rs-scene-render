//! SREP 68: `stepsPerFrame` and `prewarm` on shader effects with persistent ISF passes, and the state of a pair being
//! the same whatever order its frames are rendered in. The counter shader follows sr-core's kit shader
//! `conformance/srep_cases/assets/srep68-counter.fs`: it adds one to a float buffer per step; here it draws the count
//! / 64 in red so the count can be read back exactly.

use super::common;
use common::*;

const COUNTER: &str = r#"/*{ "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}],
    "PASSES": [{"TARGET": "acc", "PERSISTENT": true, "FLOAT": true}, {}] }*/
void main() {
    if (PASSINDEX == 0) {
        vec4 prev = IMG_NORM_PIXEL(acc, isf_FragNormCoord);
        gl_FragColor = vec4(prev.r + 1.0, 0.0, 0.0, 1.0);
    } else {
        gl_FragColor = vec4(IMG_NORM_PIXEL(acc, isf_FragNormCoord).r / 64.0, 0.0, 0.0, 1.0);
    }
}
"#;

/// Writes FRAMEINDEX / 64 and TIMEDELTA (in units of 1/fps / 8) as they were in the last step.
const UNIFORMS: &str = r#"/*{ "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}],
    "PASSES": [{"TARGET": "acc", "PERSISTENT": true, "FLOAT": true}, {}] }*/
void main() {
    gl_FragColor = vec4(float(FRAMEINDEX) / 64.0, TIMEDELTA * 10.0 / 8.0, 0.0, 1.0);
}
"#;

fn doc(effect_attrs: &str, node_attrs: &str, shader: &str, name: &str) -> sr_model::Document {
    std::fs::write(fixtures().join(name), shader).unwrap();
    doc_with(
        r##"background="#00000000""##,
        "",
        &format!(r#"<layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="f" {node_attrs}/>"#),
        &format!(r#"<effects><effect id="f" type="shader" src="{name}" space="raw" {effect_attrs}/></effects>"#),
    )
}

/// Renders `ts` in order with one renderer and a provider for other times, as the CLI does.
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

fn count(r: &Rendered) -> f32 {
    r.at(16, 16)[0] * 64.0
}

fn frames(n: usize) -> Vec<f64> {
    (0..n).map(|k| k as f64 / 10.0).collect()
}

#[test]
fn neutral_defaults_take_one_step_a_frame() {
    let d = doc("", "", COUNTER, "srep68-counter.fs");
    let Some(one) = render_seq(&d, &[0.0]) else { return };
    assert!((count(&one) - 1.0).abs() < 0.01, "{}", count(&one));
    let five = render_seq(&d, &frames(5)).unwrap();
    assert!((count(&five) - 5.0).abs() < 0.01, "{}", count(&five));
}

#[test]
fn steps_per_frame_takes_that_many_steps_each_frame() {
    let d = doc(r#"stepsPerFrame="10""#, "", COUNTER, "srep68-counter.fs");
    let Some(one) = render_seq(&d, &[0.0]) else { return };
    assert!((count(&one) - 10.0).abs() < 0.01, "{}", count(&one));
    let two = render_seq(&d, &frames(2)).unwrap();
    assert!((count(&two) - 20.0).abs() < 0.01, "{}", count(&two));
}

#[test]
fn prewarm_runs_before_the_first_frame() {
    let d = doc(r#"prewarm="9""#, "", COUNTER, "srep68-counter.fs");
    let Some(one) = render_seq(&d, &[0.0]) else { return };
    assert!((count(&one) - 10.0).abs() < 0.01, "{}", count(&one));
    let three = render_seq(&d, &frames(3)).unwrap();
    assert!((count(&three) - 12.0).abs() < 0.01, "{}", count(&three));
}

#[test]
fn a_seek_reaches_the_same_state_as_frames_in_order() {
    // kit srep-0068-seek-combined: k = 3, p = 2, frame 4 -> 2 + 5 * 3 = 17 steps
    let d = doc(r#"stepsPerFrame="3" prewarm="2""#, "", COUNTER, "srep68-counter.fs");
    let Some(direct) = render_seq(&d, &[0.4]) else { return };
    assert!((count(&direct) - 17.0).abs() < 0.01, "{}", count(&direct));
    let in_order = render_seq(&d, &frames(5)).unwrap();
    assert_eq!(direct.px, in_order.px);
    // seeking back and rendering a frame twice do not advance the state
    let back = render_seq(&d, &[0.6, 0.4]).unwrap();
    assert_eq!(back.px, direct.px);
    let twice = render_seq(&d, &[0.3, 0.4, 0.4]).unwrap();
    assert_eq!(twice.px, direct.px);
}

#[test]
fn a_node_that_starts_later_steps_from_its_first_frame() {
    // kit srep-0068-late-start: first active at frame 2, 3 steps at frame 4; prewarm runs at frame 2
    let d = doc("", r#"start="0.2""#, COUNTER, "srep68-counter.fs");
    let Some(r) = render_seq(&d, &[0.4]) else { return };
    assert!((count(&r) - 3.0).abs() < 0.01, "{}", count(&r));
    let p = doc(r#"prewarm="4""#, r#"start="0.2""#, COUNTER, "srep68-counter.fs");
    let r = render_seq(&p, &[0.4]).unwrap();
    assert!((count(&r) - 7.0).abs() < 0.01, "{}", count(&r));
}

/// The open question of SREP 68 ("Defaults"), settled on the reference: a frame at which the node is in its window but
/// not drawn (here at opacity 0) takes no step, as before SREP 68.
#[test]
fn a_frame_at_which_the_node_is_not_drawn_takes_no_step() {
    let keys = r#"<animate property="opacity"><key time="0" value="1" interpolation="hold"/><key time="0.1" value="0" interpolation="hold"/><key time="0.3" value="1" interpolation="hold"/></animate>"#;
    std::fs::write(fixtures().join("srep68-counter.fs"), COUNTER).unwrap();
    let d = doc_with(
        r##"background="#00000000""##,
        "",
        &format!(r#"<layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="f">{keys}</layer>"#),
        r#"<effects><effect id="f" type="shader" src="srep68-counter.fs" space="raw"/></effects>"#,
    );
    let Some(r) = render_seq(&d, &frames(5)) else { return };
    // drawn at frames 0, 3 and 4
    assert!((count(&r) - 3.0).abs() < 0.01, "{}", count(&r));
    let direct = render_seq(&d, &[0.4]).unwrap();
    assert_eq!(direct.px, r.px);
}

#[test]
fn the_defaults_keep_frameindex_and_timedelta_as_before() {
    // before SREP 68: FRAMEINDEX is the frame index (also for a node that starts later) and TIMEDELTA is 1/fps, 0 at time 0
    let d = doc("", r#"start="0.2""#, UNIFORMS, "srep68-uniforms.fs");
    let Some(r) = render_seq(&d, &[0.4]) else { return };
    let [fi, td, ..] = r.at(16, 16);
    assert!(
        (fi * 64.0 - 4.0).abs() < 0.01 && (td * 8.0 - 1.0).abs() < 0.01,
        "FRAMEINDEX {}, TIMEDELTA {}",
        fi * 64.0,
        td * 8.0
    );
    let at0 = render_seq(&doc("", "", UNIFORMS, "srep68-uniforms.fs"), &[0.0]).unwrap();
    assert!(at0.at(16, 16)[1].abs() < 1e-4, "TIMEDELTA at 0: {}", at0.at(16, 16)[1]);
    // with stepping: FRAMEINDEX counts the pair's steps (the last of frame 4: 2 + 3 * 3 - 1 = 10), TIMEDELTA is 1/(fps k)
    let s = doc(r#"stepsPerFrame="3" prewarm="2""#, r#"start="0.2""#, UNIFORMS, "srep68-uniforms.fs");
    let r = render_seq(&s, &[0.4]).unwrap();
    let [fi, td, ..] = r.at(16, 16);
    assert!((fi * 64.0 - 10.0).abs() < 0.01, "FRAMEINDEX {}", fi * 64.0);
    assert!((td * 8.0 - 1.0 / 3.0).abs() < 0.01, "TIMEDELTA {} of 1/fps", td * 8.0);
}

/// SREP 68, Semantics 2: to reach a frame after a seek, only the stateful node is drawn at the earlier frames, not the
/// rest of the scene, and the frame is the same as in order.
#[test]
fn a_seek_replays_the_stateful_node_alone() {
    std::fs::write(fixtures().join("srep68-counter.fs"), COUNTER).unwrap();
    let scene = |crowd: usize| {
        let crowd: String = (0..crowd)
            .map(|k| format!(r#"<layer id="c{k}" asset="red" x="{}" y="{}" effects="soft"/>"#, k % 8 * 8, k / 8 * 8))
            .collect();
        doc_with(
            r##"background="#00000000""##,
            "",
            &format!(r#"{crowd}<layer id="a" asset="red" x="8" y="8" scaleX="4" scaleY="4" effects="f"/>"#),
            r#"<effects><effect id="f" type="shader" src="srep68-counter.fs" space="raw" stepsPerFrame="2"/><effect id="soft" type="blur" radius="2"/></effects>"#,
        )
    };
    let d = scene(30);
    let Some(direct) = render_seq(&d, &[0.6]) else { return };
    let in_order = render_seq(&d, &frames(7)).unwrap();
    assert_eq!(direct.px, in_order.px);
    assert!((count(&direct) - 14.0).abs() < 0.01, "{}", count(&direct));
    assert_eq!(direct.stats.replayed_frames, 6);
    // the 30 blurred layers add nothing to the six earlier frames
    let alone = render_seq(&scene(0), &[0.6]).unwrap();
    assert_eq!(alone.stats.replayed_frames, 6);
    assert_eq!(
        (direct.stats.replay_fx_passes, direct.stats.replay_draws),
        (alone.stats.replay_fx_passes, alone.stats.replay_draws)
    );
}

/// Renders `ts` in order with a fresh renderer that saves and loads SREP 68 checkpoints in `dir` (another process).
fn render_with_checkpoints(d: &sr_model::Document, ts: &[f64], dir: &std::path::Path) -> Option<Rendered> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    r.set_checkpoint_dir(Some(dir.to_path_buf()));
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

/// SREP 68, Semantics 3: a segment rendered in another process resumes from the state saved on disk at the nearest
/// checkpoint frame, with the same picture; a document that differs does not load it.
#[test]
fn a_segment_in_another_process_resumes_from_a_saved_state() {
    let dir = std::env::temp_dir().join(format!("srep68-checkpoints-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = doc(r#"stepsPerFrame="3" prewarm="2""#, "", COUNTER, "srep68-counter.fs");
    // the first process renders frames 0 to 12 in order: states are saved after frames 0 and 10 (one a second)
    let Some(first) = render_with_checkpoints(&d, &frames(13), &dir) else { return };
    assert!((count(&first) - (2.0 + 13.0 * 3.0)).abs() < 0.01, "{}", count(&first));
    // a second process starts at frame 12: it loads frame 10 and replays frame 11 only
    let second = render_with_checkpoints(&d, &[1.2], &dir).unwrap();
    assert_eq!(second.px, first.px);
    assert_eq!((second.stats.checkpoint_hits, second.stats.replayed_frames), (1, 1));
    // a document that differs (another step count) has other keys: nothing is loaded, everything is replayed
    let other = doc(r#"stepsPerFrame="2" prewarm="2""#, "", COUNTER, "srep68-counter.fs");
    let third = render_with_checkpoints(&other, &[1.2], &dir).unwrap();
    assert_eq!((third.stats.checkpoint_hits, third.stats.checkpoint_misses, third.stats.replayed_frames), (0, 1, 12));
    assert!((count(&third) - (2.0 + 13.0 * 2.0)).abs() < 0.01, "{}", count(&third));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The neutral values written out render the same bytes as the attributes left out, which is the path every document
/// before SREP 68 takes.
#[test]
fn explicit_neutral_values_render_as_absent_ones() {
    for shader in [(COUNTER, "srep68-counter.fs"), (UNIFORMS, "srep68-uniforms.fs")] {
        let absent = doc("", r#"start="0.1""#, shader.0, shader.1);
        let explicit = doc(r#"stepsPerFrame="1" prewarm="0""#, r#"start="0.1""#, shader.0, shader.1);
        let Some(a) = render_seq(&absent, &frames(4)) else { return };
        let b = render_seq(&explicit, &frames(4)).unwrap();
        assert_eq!(a.px, b.px, "{}", shader.1);
    }
}
