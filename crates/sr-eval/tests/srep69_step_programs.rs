//! SREP 69: a stepping program advances p + s(f)·k steps by frame f, where s(f) counts the frames at which it is
//! drawn (SREP 68, Semantics 1). Its picture is the frame export's RGBA. The modules are the sr-core kit's
//! (`crates/sr-wasm/tests/modules`): srep69-counter draws red once its step count reaches the parameter "target",
//! and blue before.

use std::path::PathBuf;

use sr_eval::Evaluator;
use sr_model::{load_str, LoadOptions};

fn modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-wasm/tests/modules")
}

fn evaluator(target: u64, attrs: &str) -> Evaluator {
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(std::fs::read(modules().join("srep69-counter.wasm")).unwrap())
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    let xml = format!(
        r#"<scene version="1.6"><project width="640" height="360" fps="10" duration="2" seed="1"/><composition>
        <program id="p" mode="step" width="100" height="100" x="270" y="130" src="srep69-counter.wasm" sha256="{sha}"{attrs}><param name="target" value="{target}"/></program>
        </composition></scene>"#
    );
    let doc = load_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(modules()) }).expect("valid");
    Evaluator::new(&doc, &Default::default()).expect("compiles")
}

/// "red", "blue", or what the node shows at frame `f`.
fn colour(e: &Evaluator, f: u64) -> String {
    let g = e.evaluate_frame(f);
    assert!(g.failures.is_empty(), "{:?}", g.failures);
    let n = g.nodes.iter().find(|n| &*n.id == "p").expect("the program node");
    match &n.sim_image {
        None => "none".into(),
        Some(img) => {
            assert_eq!((img.width, img.height, img.rgba.len()), (100, 100, 10_000));
            let px = img.rgba[0];
            if px == [1.0, 0.0, 0.0, 1.0] {
                "red".into()
            } else if px == [0.0, 0.0, 1.0, 1.0] {
                "blue".into()
            } else {
                format!("{px:?}")
            }
        }
    }
}

#[test]
fn one_step_per_frame_by_default() {
    assert_eq!(colour(&evaluator(1, ""), 0), "red");
    assert_eq!(colour(&evaluator(2, ""), 0), "blue");
}

#[test]
fn steps_per_frame_and_prewarm() {
    assert_eq!(colour(&evaluator(5, r#" stepsPerFrame="5""#), 0), "red");
    assert_eq!(colour(&evaluator(6, r#" stepsPerFrame="5""#), 0), "blue");
    assert_eq!(colour(&evaluator(5, r#" prewarm="4""#), 0), "red");
    assert_eq!(colour(&evaluator(17, r#" stepsPerFrame="3" prewarm="2""#), 4), "red");
    assert_eq!(colour(&evaluator(18, r#" stepsPerFrame="3" prewarm="2""#), 4), "blue");
}

#[test]
fn a_seek_shows_the_steps_of_the_frames_before_it() {
    assert_eq!(colour(&evaluator(5, ""), 4), "red");
    assert_eq!(colour(&evaluator(6, ""), 4), "blue");
}

#[test]
fn a_frame_at_which_the_program_is_hidden_takes_no_step() {
    // frames 0, 1, 3 and 4 are drawn: 4 steps at frame 4
    assert_eq!(colour(&evaluator(4, r#" condition="frame != 2""#), 4), "red");
    assert_eq!(colour(&evaluator(5, r#" condition="frame != 2""#), 4), "blue");
}

#[test]
fn the_picture_does_not_depend_on_the_order_of_frames() {
    let (a, b) = (evaluator(3, ""), evaluator(3, ""));
    let seq: Vec<String> = (0..6).map(|f| colour(&a, f)).collect();
    let mut back = vec![String::new(); 6];
    for f in [5u64, 1, 4, 0, 3, 2] {
        back[f as usize] = colour(&b, f);
    }
    assert_eq!(seq, ["blue", "blue", "red", "red", "red", "red"]);
    assert_eq!(seq, back);
}

#[test]
fn running_out_of_fuel_fails_the_frame_with_prg12() {
    let e = evaluator(1, r#" fuel="1000""#);
    let g = e.evaluate_frame(0);
    assert!(g.failures.iter().any(|f| f.starts_with("p: PRG12")), "{:?}", g.failures);
}
