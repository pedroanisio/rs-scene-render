//! SREP 26: a repeat with a `points` child places copy i on point i, before its step offsets. These tests run
//! the conformance arrangements of the SREP on the evaluated frame: copy centres, extents and counts, and the
//! five expression names. Coordinates are compared to 1e-9 px unless a case says otherwise.

use sr_eval::{Affine, EvalOptions, Evaluator};

const EPS: f64 = 1e-9;

fn load(body: &str) -> sr_model::Document {
    load_with(body, "")
}

fn load_with(body: &str, project: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="640" height="360" fps="24" duration="4"{project}/><composition>{body}</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Every node named `s` (the copies' content), in copy order: (id, world transform).
fn shapes(d: &sr_model::Document, t: f64) -> Vec<(String, Affine)> {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    let mut out: Vec<(String, Affine)> =
        f.nodes.iter().filter(|n| n.id.ends_with("/s") || &*n.id == "s").map(|n| (n.id.to_string(), n.world)).collect();
    out.sort_by_key(|(id, _)| copy_order(id));
    out
}

/// Sort key: the copy indices along the id (`r[2]/s` → [2], `o[1]/r[3]/s` → [1, 3]).
fn copy_order(id: &str) -> Vec<u64> {
    id.split('[').skip(1).filter_map(|p| p.split(']').next()?.parse().ok()).collect()
}

/// Centres of the rect shapes `s` of size w × h (anchor at their middle).
fn centres(d: &sr_model::Document, t: f64, w: f64, h: f64) -> Vec<[f64; 2]> {
    shapes(d, t).iter().map(|(_, x)| x.apply([w / 2.0, h / 2.0])).collect()
}

/// Axis-aligned extent (width, height) of each w × h rect `s`.
fn extents(d: &sr_model::Document, t: f64, w: f64, h: f64) -> Vec<[f64; 2]> {
    shapes(d, t)
        .iter()
        .map(|(_, x)| {
            let c = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| x.apply(p));
            let (x0, x1) = (
                c.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
                c.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
            );
            let (y0, y1) = (
                c.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
                c.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
            );
            [x1 - x0, y1 - y0]
        })
        .collect()
}

fn near(a: [f64; 2], b: [f64; 2], eps: f64) -> bool {
    (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
}

fn assert_centres(got: &[[f64; 2]], want: &[[f64; 2]], eps: f64) {
    assert_eq!(got.len(), want.len(), "copies: {got:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(near(*g, *w, eps), "copy {i}: {g:?}, want {w:?}");
    }
}

const SQUARE: &str =
    r##"<shape id="s" shape="rect" width="20" height="20" anchorX="10" anchorY="10" fill="#FF0000FF"/>"##;

#[test]
fn grid_case() {
    let d = load(
        r##"<repeat id="r" x="320" y="180"><points type="grid" columns="3" rows="2"/>
<shape id="s" shape="rect" width="40" height="40" anchorX="20" anchorY="20" fill="#FF0000FF"/></repeat>"##,
    );
    let want = [[220.0, 130.0], [320.0, 130.0], [420.0, 130.0], [220.0, 230.0], [320.0, 230.0], [420.0, 230.0]];
    assert_centres(&centres(&d, 0.0, 40.0, 40.0), &want, EPS);
    // the same squares at explicit positions land on the same centres
    let explicit: String = want
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                r##"<shape id="e{i}" shape="rect" x="{}" y="{}" width="40" height="40" anchorX="20" anchorY="20"/>"##,
                c[0], c[1]
            )
        })
        .collect();
    let e = load(&explicit);
    let f = Evaluator::new(&e, &EvalOptions::default()).unwrap().evaluate(0.0);
    for (i, c) in want.iter().enumerate() {
        let n = f.nodes.iter().find(|n| *n.id == *format!("e{i}")).unwrap();
        assert!(near(n.world.apply([20.0, 20.0]), *c, EPS));
    }
}

#[test]
fn count_is_the_number_of_points() {
    // opacity 1 / count: six copies give 1/6
    let d = load(&format!(
        r#"<repeat id="r"><points type="grid" columns="3" rows="2"/>{}</repeat>"#,
        SQUARE.replace("/>", r#"><expression property="opacity">1 / count</expression></shape>"#)
    ));
    let f = Evaluator::new(&d, &EvalOptions::default()).unwrap().evaluate(0.0);
    let ops: Vec<f64> = f.nodes.iter().filter(|n| n.id.ends_with("/s")).map(|n| n.world_opacity).collect();
    assert_eq!(ops.len(), 6);
    assert!(ops.iter().all(|o| (o - 1.0 / 6.0).abs() < EPS), "{ops:?}");
}

#[test]
fn grid_steps_apply_after_the_point() {
    // point (x, 0), then T(i·10, 0) · S(0.5^i)
    let d = load(&format!(
        r#"<repeat id="r" x="320" y="180" offsetX="10" scaleStep="0.5"><points type="grid" columns="3"/>{SQUARE}</repeat>"#
    ));
    assert_centres(&centres(&d, 0.0, 20.0, 20.0), &[[220.0, 180.0], [330.0, 180.0], [440.0, 180.0]], EPS);
    let ext = extents(&d, 0.0, 20.0, 20.0);
    for (i, e) in ext.iter().enumerate() {
        let w = 20.0 * 0.5f64.powi(i as i32);
        assert!(near(*e, [w, w], EPS), "copy {i}: {e:?}");
    }
}

#[test]
fn order_case_offsets_turn_with_the_point() {
    // srep-0026-order: point 1 is (100, 0) heading 90°, offsetX 20 is applied in the rotated frame
    let d = load(
        r##"<repeat id="r" x="320" y="180" offsetX="20"><points type="along-path" path="M100,-50 L100,0" count="2" orient="true"/>
<shape id="s" shape="rect" width="40" height="10" anchorX="20" anchorY="5" fill="#FF0000FF"/></repeat>"##,
    );
    let c = centres(&d, 0.0, 40.0, 10.0);
    assert!(near(c[1], [420.0, 200.0], EPS), "copy 1 at {:?}, not (440, 180)", c[1]);
    let e = extents(&d, 0.0, 40.0, 10.0);
    assert!(near(e[0], [10.0, 40.0], EPS) && near(e[1], [10.0, 40.0], EPS), "{e:?}");
}

#[test]
fn transform_order_maps_the_origin() {
    let d = load(
        r#"<repeat id="r" offsetX="20"><points type="along-path" path="M100,-50 L100,0" count="2" orient="true"/>
<group id="s"/></repeat>"#,
    );
    let s = shapes(&d, 0.0);
    assert!(near(s[1].1.apply([0.0, 0.0]), [100.0, 20.0], EPS), "{:?}", s[1].1.apply([0.0, 0.0]));
}

#[test]
fn along_open_and_closed_paths() {
    let open = load(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="along-path" path="M-100,0 L100,0" count="5"/>{SQUARE}</repeat>"#
    ));
    let want: Vec<[f64; 2]> = [-100.0, -50.0, 0.0, 50.0, 100.0].iter().map(|x| [320.0 + x, 180.0]).collect();
    assert_centres(&centres(&open, 0.0, 20.0, 20.0), &want, EPS);
    let closed = load(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="along-path" path="M-50,-50 H50 V50 H-50 Z" count="4"/>{SQUARE}</repeat>"#
    ));
    assert_centres(
        &centres(&closed, 0.0, 20.0, 20.0),
        &[[270.0, 130.0], [370.0, 130.0], [370.0, 230.0], [270.0, 230.0]],
        EPS,
    );
}

#[test]
fn bars_on_a_circle_follow_the_tangent_only_with_orient() {
    // a circle of radius 100 as two arcs; 8 bars 40 x 4
    let circle = "M100,0 A100,100 0 0 1 -100,0 A100,100 0 0 1 100,0 Z";
    let bar = r#"<shape id="s" shape="rect" width="40" height="4" anchorX="20" anchorY="2"/>"#;
    for orient in [true, false] {
        let d = load(&format!(
            r#"<repeat id="r" x="320" y="180"><points type="along-path" path="{circle}" count="8" orient="{orient}"/>{bar}</repeat>"#
        ));
        let pts = sr_eval::points::along_path(&sr_eval::points::FlatPath::parse(circle).unwrap(), 8, true);
        for (i, e) in extents(&d, 0.0, 40.0, 4.0).iter().enumerate() {
            let th = if orient { pts[i].direction.to_radians() } else { 0.0 };
            let (c, s) = (th.cos().abs(), th.sin().abs());
            let want = [40.0 * c + 4.0 * s, 40.0 * s + 4.0 * c];
            assert!(near(*e, want, 1e-6), "orient {orient}, bar {i}: {e:?} vs {want:?}");
        }
    }
}

#[test]
fn copies_sit_where_a_motion_path_node_is_at_the_same_fraction() {
    // copies at quarter lengths of a cubic and of an arc land where a motion-path node with constant speed is at
    // progress 0, 1/4, 1/2, 3/4 and 1: one measure of path data
    for path in ["M-150,50 C-150,-150 150,-150 150,50", "M-100,0 A100,60 0 0 1 100,0"] {
        let mut body = format!(
            r#"<repeat id="r" x="320" y="180"><points type="along-path" path="{path}" count="5"/>{SQUARE}</repeat>"#
        );
        for k in 0..5 {
            let at = k as f64 / 4.0;
            body.push_str(&format!(
                r#"<group id="m{k}" x="320" y="180"><group id="n{k}"><motionPath path="{path}"><animate property="progress"><key time="0" value="{at}"/></animate></motionPath></group></group>"#
            ));
        }
        let d = load(&body);
        let f = Evaluator::new(&d, &EvalOptions::default()).unwrap().evaluate(0.0);
        let c = centres(&d, 0.0, 20.0, 20.0);
        for (k, centre) in c.iter().enumerate() {
            let node = f.nodes.iter().find(|n| *n.id == *format!("n{k}")).unwrap();
            let want = node.world.apply([0.0, 0.0]);
            assert!(near(*centre, want, EPS), "{path}, copy {k}: {centre:?} vs motion path {want:?}");
        }
    }
}

#[test]
fn group_clocks_reach_the_points_on_both_paths() {
    // the repeat sits in a group whose children see (t - 0.5) * 2: at t = 1 the repeat's own time is 1, where
    // spacingX is 100. The copies' places (from the frame's values) and pointX (read by an expression through the
    // slot's own time) must both show that grid.
    let grid = r#"<points type="grid" columns="3"><animate property="spacingX"><key time="0" value="0"/><key time="2" value="200"/></animate></points>"#;
    let body = format!(
        r#"<group id="g" timeOffset="0.5" timeScale="2"><repeat id="r" x="320" y="180">{grid}<shape id="s" shape="rect" width="20" height="20" anchorX="10" anchorY="10"><expression property="scaleX">1 + (pointX + 100) / 100</expression></shape></repeat></group>"#
    );
    let d = load(&body);
    for (t, spacing) in [(1.0, 100.0), (0.75, 50.0), (1.25, 150.0)] {
        let c = centres(&d, t, 20.0, 20.0);
        let xs: Vec<f64> = c.iter().map(|p| p[0] - 320.0).collect();
        assert!(xs.iter().zip([-spacing, 0.0, spacing]).all(|(a, b)| (a - b).abs() < EPS), "t = {t}: {xs:?}");
        let scales: Vec<f64> = extents(&d, t, 20.0, 20.0).iter().map(|e| e[0] / 20.0).collect();
        for (x, sc) in xs.iter().zip(&scales) {
            assert!(
                (sc - (1.0 + (x + 100.0) / 100.0)).abs() < EPS,
                "t = {t}: pointX {sc} disagrees with the place {x}"
            );
        }
    }
}

#[test]
fn degenerate_paths_draw_nothing_or_one_place() {
    for points in [
        r#"<points type="along-path" path="" count="3"/>"#,
        r#"<points type="along-path" path="M50,50" count="3"/>"#,
        r#"<points type="along-path" path="M0,0 L10,0" count="0"/>"#,
        r#"<points type="vertices" path=""/>"#,
        r#"<points type="scatter" path="M0,0 L0,100" count="3"/>"#,
    ] {
        let d = load(&format!(r#"<repeat id="r">{points}{SQUARE}</repeat>"#));
        assert!(shapes(&d, 0.0).is_empty(), "{points}");
    }
    let d = load(&format!(
        r#"<repeat id="r"><points type="along-path" path="M50,50 L50,50" count="3" orient="true"/>{}</repeat>"#,
        r#"<shape id="s" shape="rect" width="40" height="4" anchorX="20" anchorY="2"/>"#
    ));
    assert_centres(&centres(&d, 0.0, 40.0, 4.0), &[[50.0, 50.0]; 3], EPS);
    assert!(extents(&d, 0.0, 40.0, 4.0).iter().all(|e| near(*e, [40.0, 4.0], EPS)), "unrotated");
}

#[test]
fn scatter_in_a_rectangle_uses_the_unit_draw() {
    for seed in [7u64, 61, 14325487974692486532] {
        let d = load(&format!(
            r#"<repeat id="r" x="320" y="180"><points type="scatter" count="1" width="200" height="100" seed="{seed}"/>{SQUARE}</repeat>"#
        ));
        let (u0, u1) = (sr_eval::points::unit(seed, 0, 0), sr_eval::points::unit(seed, 1, 0));
        assert_centres(&centres(&d, 0.0, 20.0, 20.0), &[[320.0 + (u0 - 0.5) * 200.0, 180.0 + (u1 - 0.5) * 100.0]], EPS);
        // seek-checked: the same at another time
        assert_eq!(centres(&d, 0.0, 20.0, 20.0), centres(&d, 3.0, 20.0, 20.0));
    }
}

#[test]
fn scatter_edge_sliver_and_fill_rule_cases() {
    let edge = load(&format!(
        r#"<repeat id="r"><points type="scatter" path="M0,0 H100 V100 H0 Z" seed="14325487974692486532" count="1"/>{SQUARE}</repeat>"#
    ));
    assert_centres(&centres(&edge, 0.0, 20.0, 20.0), &[[0.0, 60.38343622198615]], EPS);
    for (seed, n) in [(7, 7), (61, 6)] {
        let d = load(&format!(
            r#"<repeat id="r"><points type="scatter" path="M0,0 H100 V0.5 H0.5 V100 H0 Z" seed="{seed}" count="10"/>{SQUARE}</repeat>"#
        ));
        assert_eq!(shapes(&d, 0.0).len(), n, "seed {seed}");
    }
    let region = "M0,0 H100 V100 H0 Z M25,25 H75 V75 H25 Z";
    let inner = |c: &[f64; 2]| c[0] > 25.0 && c[0] < 75.0 && c[1] > 25.0 && c[1] < 75.0;
    let nonzero = load(&format!(
        r#"<repeat id="r"><points type="scatter" path="{region}" seed="7" count="5"/>{SQUARE}</repeat>"#
    ));
    let c = centres(&nonzero, 0.0, 20.0, 20.0);
    assert!(c.iter().any(|p| near(*p, [30.35, 66.18], 0.01)), "{c:?}");
    let evenodd = load(&format!(
        r#"<repeat id="r"><points type="scatter" path="{region}" fillRule="evenodd" seed="7" count="5"/>{SQUARE}</repeat>"#
    ));
    let c = centres(&evenodd, 0.0, 20.0, 20.0);
    assert!(!c.iter().any(inner), "{c:?}");
    assert!(c.iter().any(|p| near(*p, [88.77, 13.93], 0.01)), "{c:?}");
}

#[test]
fn vertices_and_list_cases() {
    let tri = load(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="vertices" path="M-100,-50 L100,-50 Q50,0 0,50 Z"/>{SQUARE}</repeat>"#
    ));
    assert_centres(&centres(&tri, 0.0, 20.0, 20.0), &[[220.0, 130.0], [420.0, 130.0], [320.0, 230.0]], EPS);
    let moves = load(&format!(r#"<repeat id="r"><points type="vertices" path="M10,10 M90,10"/>{SQUARE}</repeat>"#));
    assert_centres(&centres(&moves, 0.0, 20.0, 20.0), &[[10.0, 10.0], [90.0, 10.0]], EPS);
    let list = load(&format!(
        r#"<repeat id="r" x="320" y="180"><points type="list" at="-100,-50 0,0 100,50"/>{SQUARE}</repeat>"#
    ));
    assert_centres(&centres(&list, 0.0, 20.0, 20.0), &[[220.0, 130.0], [320.0, 180.0], [420.0, 230.0]], EPS);
}

/// The value an expression gives to scaleX of each copy's square, read back from its extent.
fn scale_x_from(points: &str, value: &str, project: &str, t: f64) -> Vec<f64> {
    let d = load_with(
        &format!(
            r#"<repeat id="r">{points}<shape id="s" shape="rect" width="10" height="10"><expression property="scaleX">{value}</expression></shape></repeat>"#
        ),
        project,
    );
    extents(&d, t, 10.0, 10.0).iter().map(|e| e[0] / 10.0).collect()
}

#[test]
fn expression_names() {
    // pointU: 0, 1/4, ..., 1
    let u = scale_x_from(r#"<points type="grid" columns="5"/>"#, "1 + pointU", "", 0.0);
    assert_eq!(u.len(), 5);
    for (i, s) in u.iter().enumerate() {
        assert!((s - (1.0 + i as f64 / 4.0)).abs() < EPS, "copy {i}: {s}");
    }
    // pointRandom: U(s, 2, i), with the points seed, else the project seed
    let r = scale_x_from(r#"<points type="grid" columns="6" seed="7"/>"#, "1 + pointRandom", "", 0.0);
    assert!((r[0] - 1.5407366383275906).abs() < EPS && (r[5] - 1.356758657594873).abs() < EPS, "{r:?}");
    let other = scale_x_from(r#"<points type="grid" columns="6" seed="8"/>"#, "1 + pointRandom", "", 0.0);
    assert_ne!(r, other);
    let project = scale_x_from(r#"<points type="grid" columns="6"/>"#, "1 + pointRandom", r#" seed="7""#, 0.0);
    assert_eq!(r, project);
    // constant in time
    assert_eq!(r, scale_x_from(r#"<points type="grid" columns="6" seed="7"/>"#, "1 + pointRandom", "", 2.5));
    // pointX, pointY
    let x = scale_x_from(r#"<points type="list" at="1,7 2,8 3,9"/>"#, "pointX + pointY / 100", "", 0.0);
    assert!(x.iter().zip([1.07, 2.08, 3.09]).all(|(a, b)| (a - b).abs() < EPS), "{x:?}");
    // pointAngle: the direction along the path whatever orient says, 0 elsewhere
    let a = scale_x_from(
        r#"<points type="along-path" path="M0,0 L10,0 L10,10" count="3"/>"#,
        "1 + pointAngle / 90",
        "",
        0.0,
    );
    assert!(a.iter().zip([1.0, 2.0, 2.0]).all(|(a, b)| (a - b).abs() < EPS), "{a:?}");
    let g = scale_x_from(r#"<points type="grid" columns="2"/>"#, "1 + pointAngle", "", 0.0);
    assert_eq!(g, vec![1.0, 1.0]);
}

#[test]
fn point_names_refer_to_the_nearest_repeat_with_points() {
    // a counted repeat inside a points repeat: the names are the outer copy's
    let d = load(
        r#"<repeat id="o"><points type="list" at="3,0 5,0"/><repeat id="r" count="2"><shape id="s" shape="rect" width="10" height="10"><expression property="scaleX">pointX</expression></shape></repeat></repeat>"#,
    );
    let e: Vec<f64> = extents(&d, 0.0, 10.0, 10.0).iter().map(|e| e[0] / 10.0).collect();
    assert_eq!(e, vec![3.0, 3.0, 5.0, 5.0]);
}

#[test]
fn point_names_are_unknown_outside_a_points_repeat() {
    for body in [
        r#"<shape id="s" shape="rect" width="10" height="10"><expression property="scaleX">pointU</expression></shape>"#,
        r#"<repeat id="r" count="2"><shape id="s" shape="rect" width="10" height="10"><expression property="scaleX">pointRandom</expression></shape></repeat>"#,
    ] {
        let d = load(body);
        match Evaluator::new(&d, &EvalOptions::default()) {
            Ok(_) => panic!("{body} compiled"),
            Err(r) => {
                assert!(r.diagnostics.iter().any(|x| x.code == "E01" && x.message.contains("unknown name")), "{r}")
            }
        }
    }
    // a local of the same name is still allowed anywhere
    let d = load(
        r#"<shape id="s" shape="rect" width="10" height="10"><expression property="scaleX">let pointU = 2; pointU</expression></shape>"#,
    );
    assert!(Evaluator::new(&d, &EvalOptions::default()).is_ok());
}

#[test]
fn animated_spacing_places_every_copy_on_one_grid() {
    // spacingX 50 → 150 over 2 s; timeStep delays the content, not the point
    let d = load(&format!(
        r#"<repeat id="r" x="320" y="180" timeStep="0.5"><points type="grid" columns="3"><animate property="spacingX"><key time="0" value="50"/><key time="2" value="150"/></animate></points>{SQUARE}</repeat>"#
    ));
    assert_centres(&centres(&d, 1.0, 20.0, 20.0), &[[220.0, 180.0], [320.0, 180.0], [420.0, 180.0]], EPS);
    // at 1.5 s the spacing is 125 for every copy, although copy 2's content is only 0.5 s in
    assert_centres(&centres(&d, 1.5, 20.0, 20.0), &[[195.0, 180.0], [320.0, 180.0], [445.0, 180.0]], EPS);
    // the expression names see the same grid
    let x = scale_x_from(
        r#"<points type="grid" columns="3"><animate property="spacingX"><key time="0" value="50"/><key time="2" value="150"/></animate></points>"#,
        "2 + pointX / 100",
        "",
        1.0,
    );
    assert!(x.iter().zip([1.0, 2.0, 3.0]).all(|(a, b)| (a - b).abs() < EPS), "{x:?}");
}

#[test]
fn animated_scatter_rectangle() {
    let d = load(&format!(
        r#"<repeat id="r"><points type="scatter" count="2" width="0" height="0" seed="7"><animate property="width"><key time="0" value="0"/><key time="2" value="100"/></animate></points>{SQUARE}</repeat>"#
    ));
    let c = centres(&d, 2.0, 20.0, 20.0);
    assert!((c[0][0] - (0.8272724219886994 - 0.5) * 100.0).abs() < EPS && c[0][1] == 0.0, "{c:?}");
    assert!(centres(&d, 0.0, 20.0, 20.0).iter().all(|p| p[0] == 0.0));
}

#[test]
fn nested_repeat_points_carry_the_outer_delay_in_any_frame_order() {
    // outer copies are delayed by 1 s each; the inner grid's spacing runs 0 → 100 over 2 s of its own time
    let body = format!(
        r#"<repeat id="o" count="2" timeStep="1" offsetY="100"><repeat id="r"><points type="grid" columns="2"><animate property="spacingX"><key time="0" value="0"/><key time="2" value="100"/></animate></points>{SQUARE}</repeat></repeat>"#
    );
    let d = load(&body);
    let at = |t: f64| centres(&d, t, 20.0, 20.0);
    // at t = 2: outer copy 0 reads spacing 100, outer copy 1 (1 s late) reads 50
    let c = at(2.0);
    assert_eq!(c.len(), 4);
    assert!((c[1][0] - c[0][0] - 100.0).abs() < EPS, "{c:?}");
    assert!((c[3][0] - c[2][0] - 50.0).abs() < EPS, "{c:?}");
    // no state between frames: one evaluator asked in order and out of order gives the same frames
    let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let frame = |t: f64| -> Vec<(String, [f64; 2])> {
        let mut v: Vec<(String, [f64; 2])> = ev
            .evaluate(t)
            .nodes
            .iter()
            .filter(|n| n.id.ends_with("/s"))
            .map(|n| (n.id.to_string(), n.world.apply([10.0, 10.0])))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let times = [0.5, 1.0, 1.5, 2.0, 3.0];
    let forward: Vec<_> = times.iter().map(|&t| frame(t)).collect();
    for order in [[4, 0, 3, 1, 2], [2, 4, 1, 3, 0]] {
        for k in order {
            assert_eq!(frame(times[k]), forward[k], "t = {}", times[k]);
        }
    }
}

#[test]
fn a_repeat_without_points_is_unchanged() {
    let d = load(&format!(r#"<repeat id="r" x="100" y="100" count="3" offsetX="30">{SQUARE}</repeat>"#));
    assert_centres(&centres(&d, 0.0, 20.0, 20.0), &[[100.0, 100.0], [130.0, 100.0], [160.0, 100.0]], EPS);
}

#[test]
fn bad_path_data_is_an_error() {
    let d = load(&format!(r#"<repeat id="r"><points type="vertices" path="M0,0 X1,1"/>{SQUARE}</repeat>"#));
    let Err(e) = Evaluator::new(&d, &EvalOptions::default()) else { panic!("bad path data compiled") };
    assert!(e.diagnostics.iter().any(|x| x.code == "E15"), "{e}");
}

#[test]
fn a_count_past_the_node_budget_is_refused_before_points_are_made() {
    let body = format!(
        r#"<repeat id="r"><points type="along-path" path="M0,0 L10,0" count="1000000000000"/>{SQUARE}</repeat>"#
    );
    let d = load(&body);
    let Err(e) = Evaluator::new(&d, &EvalOptions::default()) else { panic!("a trillion copies were accepted") };
    assert!(e.diagnostics.iter().any(|x| x.code == "E18"), "{e}");
}
