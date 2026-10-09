//! `motionPath/@additive`: the path is an offset from the node's own position.

use sr_eval::{EvalOptions, Evaluator};

fn doc(path: &str, additive: &str, x_keys: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="4"/><composition>
  <shape id="s" shape="rect" x="100" y="50" width="10" height="10">{x_keys}<motionPath path="{path}" start="0" end="2" {additive}/></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn at(d: &sr_model::Document, t: f64) -> (f64, f64) {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    let p = f.nodes.iter().find(|n| &*n.id == "s").unwrap().world.apply([0.0, 0.0]);
    (p[0], p[1])
}

#[test]
fn without_additive_the_path_is_the_position_as_before() {
    let d = doc("M 20 20 L 120 20", "", "");
    let (x, y) = at(&d, 1.0);
    assert!((x - 70.0).abs() < 1e-9 && (y - 20.0).abs() < 1e-9, "({x}, {y})");
    assert_eq!(at(&doc("M 20 20 L 120 20", r#"additive="false""#, ""), 1.0), at(&d, 1.0));
}

#[test]
fn additive_offsets_the_node_by_the_distance_along_the_path() {
    for path in ["M 0 0 L 100 0", "M 20 20 L 120 20", "M -300 400 L -200 400"] {
        let d = doc(path, r#"additive="true""#, "");
        let (x0, y0) = at(&d, 0.0);
        let (x, y) = at(&d, 1.0);
        let (x2, y2) = at(&d, 2.0);
        assert!((x0 - 100.0).abs() < 1e-9 && (y0 - 50.0).abs() < 1e-9, "{path}: starts where it would: ({x0}, {y0})");
        assert!((x - 150.0).abs() < 1e-9 && (y - 50.0).abs() < 1e-9, "{path}: halfway: ({x}, {y})");
        assert!((x2 - 200.0).abs() < 1e-9 && (y2 - 50.0).abs() < 1e-9, "{path}: at the end: ({x2}, {y2})");
    }
}

#[test]
fn additive_adds_to_the_nodes_own_animation() {
    // the node's own x runs 100 -> 300 over 2 s; the path adds up to 100 more
    let keys = r#"<animate property="x"><key time="0" value="100"/><key time="2" value="300"/></animate>"#;
    let d = doc("M 0 0 L 100 0", r#"additive="true""#, keys);
    let (x, _) = at(&d, 1.0);
    assert!((x - (200.0 + 50.0)).abs() < 1e-9, "{x}");
    // replaced (not additive), the keys are ignored
    let (x, _) = at(&doc("M 0 0 L 100 0", "", keys), 1.0);
    assert!((x - 50.0).abs() < 1e-9, "{x}");
}

/// The path's point is also the node's `x` and `y` properties: the 3D renderer reads a camera's eye and an object3D's
/// position from them, and ignored a motion path while it was only in the 2D transform (Inova probe A, v0.1.4).
#[test]
fn the_path_point_is_the_nodes_x_and_y_properties() {
    for (additive, want) in [("", [70.0, 20.0]), (r#"additive="true""#, [150.0, 50.0])] {
        let d = doc("M 20 20 L 120 20", additive, "");
        let f = Evaluator::new(&d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(1.0);
        let n = f.nodes.iter().find(|n| &*n.id == "s").unwrap();
        let got = ["x", "y"].map(|k| n.props.get(k).and_then(|v| v.as_num()).unwrap_or(f64::NAN));
        assert!((got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9, "{additive}: {got:?}");
    }
}

/// A camera turns by yaw, pitch and roll, so `autoOrient` on its motion path has nothing to turn: the document is
/// refused (and `validate` reports it) instead of the camera silently not turning.
#[test]
fn a_camera_that_would_turn_along_its_path_is_refused() {
    let xml = |orient: &str| {
        format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="2"/><composition>
  <camera id="cam" z="-500"><motionPath path="M0 0 L100 0" {orient}/></camera></composition></scene>"#
        )
    };
    let build = |orient: &str| {
        let d = sr_model::load_str(&xml(orient), &sr_model::LoadOptions::without_assets()).unwrap();
        Evaluator::new(&d, &EvalOptions::default()).map(|_| ())
    };
    assert!(build("").is_ok());
    let refused = build(r#"autoOrient="true""#).expect_err("a camera does not turn along its path");
    assert!(refused.to_string().contains("autoOrient"), "{refused}");
}
