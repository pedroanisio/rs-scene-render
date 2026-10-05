//! Transform constraints, skeletons, tracking data and stabilisation.

use sr_eval::{EvalOptions, Evaluator, FrameGraph, FrameNode};

fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sr-eval-rig-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn doc(comp: &str, after: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.1"><project width="200" height="100" fps="10" duration="10"/><assets><image id="img" src="img.png" width="100" height="50"/></assets>{after}<composition>{comp}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn eval(d: &sr_model::Document, t: f64) -> FrameGraph {
    Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t)
}

fn node<'f>(f: &'f FrameGraph, id: &str) -> &'f FrameNode {
    f.nodes.iter().find(|n| &*n.id == id).unwrap()
}

fn pivot(n: &FrameNode) -> [f64; 2] {
    n.world.apply(n.anchor)
}

fn rot(n: &FrameNode) -> f64 {
    libm::atan2(n.world.0[1], n.world.0[0]).to_degrees()
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

const T: &str = r##"<shape id="t" shape="rect" x="50" y="30" width="10" height="10" rotation="90"/>"##;

#[test]
fn position_rotation_and_scale_copies() {
    let d = doc(
        &format!(
            r##"{T}
            <shape id="cp" shape="rect" x="0" y="0" width="4" height="4"><transformConstraint type="copy-position" target="t" offsetX="5"/></shape>
            <shape id="half" shape="rect" x="0" y="0" width="4" height="4"><transformConstraint type="copy-position" target="t" influence="0.5"/></shape>
            <shape id="cr" shape="rect" x="10" y="10" width="4" height="4"><transformConstraint type="copy-rotation" target="t" offsetRotation="10"/></shape>
            <shape id="cs" shape="rect" x="10" y="10" width="4" height="4" scaleX="2"><transformConstraint type="copy-scale" target="t"/></shape>
            <shape id="ct" shape="rect" x="0" y="0" width="4" height="4"><transformConstraint type="copy-transform" target="t"/></shape>
            <group id="g" x="0" y="0"><shape id="kid" shape="rect" x="3" y="0" width="1" height="1"/><transformConstraint type="copy-position" target="t"/></group>"##
        ),
        "",
    );
    let f = eval(&d, 0.0);
    assert!(near(pivot(node(&f, "cp")), [55.0, 30.0], 1e-9));
    assert!(near(pivot(node(&f, "half")), [25.0, 15.0], 1e-9));
    assert!((rot(node(&f, "cr")) - 100.0).abs() < 1e-9);
    assert!(near(pivot(node(&f, "cr")), [10.0, 10.0], 1e-9), "rotation keeps the pivot");
    let w = node(&f, "cs").world.0;
    assert!((libm::hypot(w[0], w[1]) - 1.0).abs() < 1e-9);
    assert_eq!(node(&f, "ct").world, node(&f, "t").world);
    // descendants follow their constrained container
    assert!(near(pivot(node(&f, "kid")), [53.0, 30.0], 1e-9), "{:?}", pivot(node(&f, "kid")));
}

#[test]
fn look_at_distance_parent_and_follow_path() {
    let d = doc(
        &format!(
            r##"{T}
            <shape id="la" shape="rect" x="40" y="20" width="4" height="4"><transformConstraint type="look-at" target="t"/></shape>
            <shape id="dist" shape="rect" x="0" y="30" width="4" height="4"><transformConstraint type="distance" target="t" maxDistance="20"/></shape>
            <shape id="near" shape="rect" x="48" y="30" width="4" height="4"><transformConstraint type="distance" target="t" minDistance="5"/></shape>
            <shape id="par" shape="rect" x="10" y="0" width="4" height="4"><transformConstraint type="parent" target="t"/></shape>
            <shape id="fp" shape="rect" x="0" y="0" width="4" height="4"><transformConstraint type="follow-path" path="M0 0 L0 100" progress="0.25" autoOrient="true"/></shape>"##
        ),
        "",
    );
    let f = eval(&d, 0.0);
    assert!((rot(node(&f, "la")) - 45.0).abs() < 1e-9);
    assert!(near(pivot(node(&f, "dist")), [30.0, 30.0], 1e-9));
    assert!(near(pivot(node(&f, "near")), [45.0, 30.0], 1e-9));
    // parent: t's world (at 50, 30, rotated 90°) applied to the local offset (10, 0)
    assert!(near(pivot(node(&f, "par")), [50.0, 40.0], 1e-9), "{:?}", pivot(node(&f, "par")));
    assert!(near(pivot(node(&f, "fp")), [0.0, 25.0], 1e-9));
    assert!((rot(node(&f, "fp")) - 90.0).abs() < 1e-9);
}

#[test]
fn node_ik_reaches_the_target() {
    let d = doc(
        r##"<shape id="goal" shape="rect" x="60" y="40" width="1" height="1"/>
           <group id="upper" x="20" y="20">
             <shape id="lower" shape="rect" x="30" y="0" width="1" height="1"><transformConstraint type="ik" target="goal" offsetX="30"/></shape>
           </group>"##,
        "",
    );
    let f = eval(&d, 0.0);
    let n = node(&f, "lower");
    let eff = n.world.apply([n.anchor[0] + 30.0, n.anchor[1]]);
    assert!(near(eff, [60.0, 40.0], 1e-6), "{eff:?}");
    assert!(near(pivot(node(&f, "upper")), [20.0, 20.0], 1e-9), "the chain root stays put");
}

#[test]
fn skeleton_bones_with_ik_and_animation() {
    let d = doc(
        r##"<shape id="goal" shape="rect" x="150" y="60" width="1" height="1"/>
           <skeleton id="rig">
             <bone id="root" x="100" y="50" length="0"/>
             <bone id="arm" parent="root" length="40"/>
             <bone id="hand" parent="arm" x="40" length="40"/>
             <transformConstraint type="ik" target="goal"/>
             <bone id="tail" parent="root" rotation="90" length="10"><animate property="rotation"><key time="0" value="90"/><key time="1" value="180"/></animate></bone>
           </skeleton>"##,
        "",
    );
    let f = eval(&d, 0.5);
    let sk = node(&f, "rig");
    assert_eq!(sk.bones.len(), 4);
    let hand = sk.bones.iter().find(|b| &*b.id == "hand").unwrap();
    let tip = hand.world.apply([hand.length, 0.0]);
    assert!(near(tip, [150.0, 60.0], 1e-6), "{tip:?}");
    let tail = sk.bones.iter().find(|b| &*b.id == "tail").unwrap();
    assert!((libm::atan2(tail.world.0[1], tail.world.0[0]).to_degrees() - 135.0).abs() < 1e-9);
    assert!(
        (libm::atan2(tail.rest.0[1], tail.rest.0[0]).to_degrees() - 90.0).abs() < 1e-9,
        "rest uses document values"
    );
}

#[test]
fn track_constraint_and_stabilisation() {
    std::fs::write(dir().join("walk.csv"), "frame,track,x,y\n0,eye,10,10\n10,eye,30,20\n").unwrap();
    std::fs::write(
        dir().join("shake.json"),
        r#"{"fps":10,"points":{"p":[[0,50,25],[1,52,25],[2,50,25],[3,52,25],[4,50,25]]}}"#,
    )
    .unwrap();
    let d = doc(
        r##"<shape id="follower" shape="rect" x="0" y="0" width="2" height="2"><transformConstraint type="track" target="walk" point="eye" offsetX="1"/></shape>
           <layer id="shaky" asset="img" x="0" y="0" stabilize="true" stabilizeSmoothness="1"/>"##,
        r##"<tracking><trackData id="walk" src="walk.csv" kind="point" format="csv"/><trackData id="shake" src="shake.json" kind="point" footage="img"/></tracking>"##,
    );
    // CSV frames are at 24 fps: frame 5 is 5/24 s
    let f = eval(&d, 5.0 / 24.0);
    assert!(near(pivot(node(&f, "follower")), [21.0, 15.0], 1e-9), "{:?}", pivot(node(&f, "follower")));
    // locked stabilisation: at 0.1 s the tracked point moved +2 px; the layer moves −2 px
    let g = eval(&d, 0.1);
    let l = node(&g, "shaky");
    assert!(near(l.world.apply([50.0, 25.0]), [48.0, 25.0], 1e-6), "{:?}", l.world);
}

#[test]
fn unreadable_tracking_data_is_e17() {
    let d = doc("", r##"<tracking><trackData id="gone" src="missing.csv" kind="point" format="csv"/></tracking>"##);
    let Err(e) = Evaluator::new(&d, &EvalOptions::default()) else { panic!("expected E17") };
    assert!(e.diagnostics.iter().any(|x| x.code == "E17"), "{e}");
}

#[test]
fn a_bone_is_a_source_for_links_and_expressions_in_either_order() {
    // a bone is an element with an id and animatable properties: `b0.rotation` must read its animated
    // value, not its static one, wherever the reading node sits in the document
    let skeleton = r##"<skeleton id="rig"><bone id="b0" length="10"><animate property="rotation"><key time="0" value="0"/><key time="1" value="90"/></animate></bone></skeleton>"##;
    let linked = r##"<group id="g" width="10" height="10"><link property="rotation" source="b0.rotation"/></group>"##;
    let scripted = r##"<group id="h" width="10" height="10"><expression property="rotation">prop("b0.rotation") * 2</expression></group>"##;
    for body in [format!("{skeleton}{linked}{scripted}"), format!("{linked}{scripted}{skeleton}")] {
        let d = doc(&body, "");
        let (half, full) = (eval(&d, 0.5), eval(&d, 1.0));
        assert!((rot(node(&half, "g")) - 45.0).abs() < 1e-6, "link at 0.5 s: {}", rot(node(&half, "g")));
        assert!((rot(node(&full, "g")) - 90.0).abs() < 1e-6, "link at 1 s: {}", rot(node(&full, "g")));
        assert!((rot(node(&full, "h")) - 180.0).abs() < 1e-6, "expression at 1 s: {}", rot(node(&full, "h")));
    }
}
