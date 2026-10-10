//! SREP 70: parametric paths, parametric surfaces and heightfields at the evaluator. A curve's samples become the
//! shape's `path` property (§2), a surface's or heightfield's become a mesh (§3, §4); the expressions read their
//! sampling variables and, like any expression, `time` and `prop`.

use sr_eval::parametric::{path_data, sample_at, samples, GeomKind};
use sr_eval::{Evaluator, FrameNode};

fn evaluator(composition: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <materials><material id="m" baseColor="#FF0000FF" unlit="true"/></materials>
  <composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn node(ev: &Evaluator, id: &str, t: f64) -> FrameNode {
    let g = ev.evaluate(t);
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    g.nodes.iter().find(|n| &*n.id == id).cloned().unwrap_or_else(|| panic!("no node {id}"))
}

fn path_of(n: &FrameNode) -> String {
    match n.props.get("path") {
        Some(sr_eval::value::Value::Str(s)) => s.to_string(),
        other => panic!("no path property: {other:?}"),
    }
}

/// The points of path data made only of M, L and Z commands, one list per subpath, and whether each is closed.
fn subpaths(d: &str) -> Vec<(Vec<[f64; 2]>, bool)> {
    let mut out: Vec<(Vec<[f64; 2]>, bool)> = Vec::new();
    let toks: Vec<&str> = d.split_whitespace().collect();
    let mut k = 0;
    while k < toks.len() {
        match toks[k] {
            "M" => {
                out.push((vec![[toks[k + 1].parse().unwrap(), toks[k + 2].parse().unwrap()]], false));
                k += 3;
            }
            "L" => {
                out.last_mut().unwrap().0.push([toks[k + 1].parse().unwrap(), toks[k + 2].parse().unwrap()]);
                k += 3;
            }
            "Z" => {
                out.last_mut().unwrap().1 = true;
                k += 1;
            }
            other => panic!("unexpected {other:?} in {d}"),
        }
    }
    out
}

const ELLIPSE: &str = r##"<shape id="e" shape="parametric" fill="#FF0000FF"><parametricPath x="320 + 100 * Math.cos(t)" y="180 + 50 * Math.sin(t)" t0="0" t1="6.283185307179586" samples="256" closed="true"/></shape>"##;

#[test]
fn sampling_steps_over_the_range_and_a_closed_range_stops_a_step_short() {
    assert_eq!(sample_at(0.0, 1.0, 5, false, 4), 1.0);
    assert_eq!(sample_at(0.0, 1.0, 5, false, 1), 0.25);
    assert_eq!(sample_at(0.0, 1.0, 4, true, 3), 0.75);
    let s = samples(&GeomKind::Path { t0: 0.0, t1: 2.0, samples: 3, closed: false });
    assert_eq!(s.iter().map(|q| q[0]).collect::<Vec<_>>(), [0.0, 1.0, 2.0]);
    // a heightfield samples z along the first grid direction and x along the second: (x, z) per sample
    let h = samples(&GeomKind::Height { width: 10.0, depth: 4.0, nx: 3, nz: 2 });
    assert_eq!(h, [[-5.0, -2.0], [-5.0, 2.0], [0.0, -2.0], [0.0, 2.0], [5.0, -2.0], [5.0, 2.0]]);
}

#[test]
fn srep_0070_path_ellipse_is_the_closed_polyline_of_its_samples() {
    let n = node(&evaluator(ELLIPSE), "e", 0.0);
    let subs = subpaths(&path_of(&n));
    assert_eq!(subs.len(), 1);
    let (pts, closed) = &subs[0];
    assert!(*closed, "closed=\"true\" ends with Z");
    assert_eq!(pts.len(), 256, "t1 itself is not sampled on a closed curve");
    for (i, p) in pts.iter().enumerate() {
        let t = i as f64 * std::f64::consts::TAU / 256.0;
        assert!((p[0] - (320.0 + 100.0 * t.cos())).abs() < 1e-9 && (p[1] - (180.0 + 50.0 * t.sin())).abs() < 1e-9);
    }
    // the box is the bounding box of the points: the extremes are samples (i = 0, 64, 128, 192)
    let b = n.param_box.expect("a parametric shape has a box");
    for (got, want) in b.iter().zip([220.0, 130.0, 420.0, 230.0]) {
        assert!((got - want).abs() < 1e-9, "{b:?}");
    }
}

#[test]
fn srep_0070_path_break_starts_a_new_subpath_and_leaves_the_curve_open() {
    let xml = r##"<shape id="b" shape="parametric" fill="#00000000" stroke="#FF0000FF" strokeWidth="10"><parametricPath x="120 + 400 * t" y="(t &gt; 0.455 &amp;&amp; t &lt; 0.545) ? NaN : 180" t0="0" t1="1" samples="101"/></shape>"##;
    let subs = subpaths(&path_of(&node(&evaluator(xml), "b", 0.0)));
    assert_eq!(subs.len(), 2, "{subs:?}");
    // t = 0.46 … 0.54 (i = 46 … 54) are dropped
    assert_eq!(subs[0].0.len(), 46);
    assert_eq!(subs[1].0.len(), 46);
    assert!((subs[0].0.last().unwrap()[0] - (120.0 + 400.0 * 0.45)).abs() < 1e-9);
    assert!((subs[1].0[0][0] - (120.0 + 400.0 * 0.55)).abs() < 1e-9);
}

#[test]
fn path_data_breaks_and_closes() {
    let n = f64::NAN;
    let (d, b) = path_data(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], true);
    assert_eq!(d, "M 0 0 L 1 0 L 1 1 Z");
    assert_eq!(b, Some([0.0, 0.0, 1.0, 1.0]));
    // a break keeps a closed curve open
    assert_eq!(path_data(&[[0.0, 0.0], [n, 0.0], [1.0, 1.0], [2.0, 1.0]], true).0, "M 0 0 M 1 1 L 2 1");
    assert_eq!(path_data(&[[0.0, 0.0], [1.0, 1.0], [f64::INFINITY, 0.0]], true).0, "M 0 0 L 1 1");
    // leading non-finite points are dropped without breaking anything
    assert_eq!(path_data(&[[n, n], [0.0, 0.0], [1.0, 1.0]], true).0, "M 0 0 L 1 1 Z");
    assert_eq!(path_data(&[[n, n]], true), (String::new(), None));
}

#[test]
fn an_expression_that_reads_time_animates_the_curve() {
    let xml = r##"<shape id="s" shape="parametric" stroke="#FFFFFF" strokeWidth="2"><parametricPath x="100 * t + 50 * time" y="20" samples="2"/></shape>"##;
    let ev = evaluator(xml);
    assert_eq!(path_of(&node(&ev, "s", 0.0)), "M 0 20 L 100 20");
    assert_eq!(path_of(&node(&ev, "s", 1.0)), "M 50 20 L 150 20");
}

#[test]
fn an_expression_reads_other_properties() {
    let xml = r##"<shape id="a" shape="rect" x="7" y="0" width="1" height="1"/>
      <shape id="s" shape="parametric" stroke="#FFFFFF" strokeWidth="2"><parametricPath x="prop('a.x') * t" y="0" samples="2"/></shape>"##;
    assert_eq!(path_of(&node(&evaluator(xml), "s", 0.0)), "M 0 0 L 7 0");
}

#[test]
fn srep_0070_surface_plane_is_a_grid_of_its_samples() {
    let xml = r##"<object3D id="s" primitive="parametric" material="m" x="320" y="180"><parametricSurface x="u" y="v" z="0" u0="-50" u1="50" v0="-25" v1="25" uSamples="8" vSamples="8"/></object3D>"##;
    let n = node(&evaluator(xml), "s", 0.0);
    let pm = n.param_mesh.as_ref().expect("a mesh");
    assert_eq!(pm.mesh.vertices.len(), 64);
    assert_eq!(pm.mesh.indices.len(), 7 * 7 * 6);
    let (lo, hi) = pm.mesh.vertices.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| {
        (std::array::from_fn(|k| lo[k].min(v.pos[k])), std::array::from_fn(|k| hi[k].max(v.pos[k])))
    });
    assert_eq!((lo, hi), ([-50.0, -25.0, 0.0], [50.0, 25.0, 0.0]));
    assert!(pm.mesh.vertices.iter().all(|v| v.normal == [0.0, 0.0, -1.0]), "faces the camera");
    // texture coordinates span [0, 1]
    assert!(pm.mesh.vertices.iter().any(|v| v.uv == [1.0, 1.0]) && pm.mesh.vertices.iter().any(|v| v.uv == [0.0, 0.0]));
}

#[test]
fn srep_0070_heightfield_is_y_down_by_its_height() {
    let xml = r##"<object3D id="h" primitive="heightfield" material="m"><heightfield height="x + 2 * z" width="100" depth="50" xSamples="3" zSamples="3"/></object3D>"##;
    let n = node(&evaluator(xml), "h", 0.0);
    let pm = n.param_mesh.as_ref().expect("a mesh");
    assert_eq!(pm.mesh.vertices.len(), 9);
    for v in &pm.mesh.vertices {
        let [x, y, z] = v.pos;
        assert!((y + (x + 2.0 * z)).abs() < 1e-4, "vertex (x, −height(x, z), z): {:?}", v.pos);
        assert!([-50.0, 0.0, 50.0].contains(&x) && [-25.0, 0.0, 25.0].contains(&z), "{:?}", v.pos);
    }
    let flat = node(
        &evaluator(
            r##"<object3D id="h" primitive="heightfield" material="m"><heightfield height="0" width="100" depth="50" xSamples="4" zSamples="4"/></object3D>"##,
        ),
        "h",
        0.0,
    );
    assert!(flat.param_mesh.unwrap().mesh.vertices.iter().all(|v| v.normal == [0.0, -1.0, 0.0]), "a flat one faces up");
}

#[test]
fn equal_meshes_have_equal_keys_and_animated_ones_change() {
    let xml = r##"<object3D id="s" primitive="parametric" material="m"><parametricSurface x="u" y="v" z="10 * time" uSamples="2" vSamples="2"/></object3D>
      <object3D id="still" primitive="parametric" material="m"><parametricSurface x="u" y="v" z="0" uSamples="2" vSamples="2"/></object3D>"##;
    let ev = evaluator(xml);
    let key = |id: &str, t: f64| node(&ev, id, t).param_mesh.unwrap().key;
    assert_ne!(key("s", 0.0), key("s", 0.5));
    assert_eq!(key("still", 0.0), key("still", 1.5));
}

#[test]
fn an_unknown_name_is_an_expression_error_and_the_sampling_variables_stay_local() {
    let errors = |composition: &str| -> Vec<(String, String)> {
        let xml = format!(
            r##"<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/><composition>{composition}</composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        match Evaluator::new(&doc, &Default::default()) {
            Ok(_) => Vec::new(),
            Err(r) => r.diagnostics.iter().map(|d| (d.code.clone(), d.message.clone())).collect(),
        }
    };
    // a curve has t, not u
    let e = errors(r#"<shape id="s" shape="parametric"><parametricPath x="u" y="0"/></shape>"#);
    assert!(e.iter().any(|(c, m)| c == "E01" && m.contains("parametricPath@x") && m.contains("unknown name")), "{e:?}");
    // t is not a name of an ordinary expression
    let e =
        errors(r#"<shape id="r" shape="rect" width="4" height="4"><expression property="x">t</expression></shape>"#);
    assert!(e.iter().any(|(c, m)| c == "E01" && m.contains("unknown name")), "{e:?}");
}
