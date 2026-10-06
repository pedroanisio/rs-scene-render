//! SREP 16: connectors that follow the nodes they join. The geometry of the conformance cases, measured on the
//! evaluated frame: the visible path V (after clipping to the ends' boxes and the gaps), in connector space.

use sr_eval::{EvalOptions, Evaluator, FrameGraph};

const EPS: f64 = 1e-6;

/// R: a red 80 × 60 rect at (100, 150); B: a blue 80 × 60 rect at (460, 150).
const R: &str = r##"<shape id="R" shape="rect" x="100" y="150" width="80" height="60" fill="#FF0000FF"/>"##;
const B: &str = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" fill="#0000FFFF"/>"##;

fn load(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="2" background="#000000FF"/><assets><text id="t" text="Hi" width="40" height="20" size="12"/></assets><composition>{body}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn frame(d: &sr_model::Document, t: f64) -> FrameGraph {
    Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t)
}

/// The visible path of connector `c` in frame pixels, or None when it draws nothing.
fn path(g: &FrameGraph, c: &str) -> Option<Vec<[f64; 2]>> {
    let n = g.nodes.iter().find(|n| &*n.id == c).unwrap_or_else(|| panic!("no node {c}"));
    n.connector.as_ref().map(|k| k.path.iter().map(|p| n.world.apply(*p)).collect())
}

fn near(a: [f64; 2], b: [f64; 2], eps: f64) -> bool {
    (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
}

#[track_caller]
fn ends(p: &[[f64; 2]], a: [f64; 2], b: [f64; 2], eps: f64) {
    assert!(near(p[0], a, eps) && near(p[p.len() - 1], b, eps), "path {p:?}, want {a:?} -> {b:?}");
}

fn conn(attrs: &str) -> String {
    format!(r##"<connector id="c" from="R" to="B" stroke="#00FF00FF" strokeWidth="6" {attrs}/>"##)
}

#[test]
fn straight_stops_at_the_boxes() {
    let d = load(&format!("{R}{B}{}", conn("")));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    ends(&p, [180.0, 180.0], [460.0, 180.0], EPS);
    assert_eq!(p.len(), 2);
}

#[test]
fn rotated_target_and_gaps() {
    let square = r##"<shape id="B" shape="rect" x="500" y="180" width="80" height="80" anchorX="40" anchorY="40" rotation="45" fill="#0000FFFF"/>"##;
    let d = load(&format!("{R}{square}{}", conn(r#"fromGap="10" toGap="10""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    let vertex = 500.0 - 40.0 * std::f64::consts::SQRT_2;
    ends(&p, [190.0, 180.0], [vertex - 10.0, 180.0], 1e-6);
    assert!((p[1][0] - 433.43).abs() < 0.01);
}

#[test]
fn anchor_keywords_are_not_clipped() {
    let r = r##"<shape id="R" shape="rect" x="280" y="40" width="80" height="60"/>"##;
    let b = r##"<shape id="B" shape="rect" x="440" y="260" width="80" height="60"/>"##;
    let d = load(&format!("{r}{b}{}", conn(r#"fromAnchor="bottom" toAnchor="top""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    ends(&p, [320.0, 100.0], [480.0, 260.0], EPS);
    for (kw, pt) in [
        ("center", [320.0, 70.0]),
        ("top", [320.0, 40.0]),
        ("right", [360.0, 70.0]),
        ("left", [280.0, 70.0]),
        ("top-left", [280.0, 40.0]),
        ("top-right", [360.0, 40.0]),
        ("bottom-right", [360.0, 100.0]),
        ("bottom-left", [280.0, 100.0]),
    ] {
        let d = load(&format!("{r}{b}{}", conn(&format!(r#"fromAnchor="{kw}" toAnchor="top""#))));
        let p = path(&frame(&d, 0.0), "c").expect("drawn");
        assert!(near(p[0], pt, EPS), "{kw}: {:?}", p[0]);
    }
}

#[test]
fn explicit_anchor_points_use_percent_of_the_box() {
    let d = load(&format!("{R}{B}{}", conn(r#"fromX="100%" fromY="0" toX="0" toY="50%""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    ends(&p, [180.0, 150.0], [460.0, 180.0], EPS);
}

#[test]
fn point_ends_are_never_clipped() {
    let d = load(&format!(
        r##"{R}<connector id="c" from="R" toX="600" toY="20" stroke="#00FF00FF"/><connector id="k" fromX="10" fromY="10" toX="50%" toY="100%"/>"##
    ));
    let g = frame(&d, 0.0);
    let p = path(&g, "c").expect("drawn");
    assert!(near(p[p.len() - 1], [600.0, 20.0], EPS));
    let k = path(&g, "k").expect("drawn");
    ends(&k, [10.0, 10.0], [320.0, 360.0], EPS);
}

#[test]
fn orthogonal_route_and_its_clipping() {
    let r = r##"<shape id="R" shape="rect" x="100" y="60" width="80" height="60"/>"##;
    let b = r##"<shape id="B" shape="rect" x="460" y="240" width="80" height="60"/>"##;
    let d = load(&format!("{r}{b}{}", conn(r#"route="orthogonal""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    let want = [[180.0, 90.0], [320.0, 90.0], [320.0, 270.0], [460.0, 270.0]];
    assert_eq!(p.len(), 4, "{p:?}");
    for (a, b) in p.iter().zip(want) {
        assert!(near(*a, b, EPS), "{p:?}");
    }
    // vertical first when the ends are further apart in y
    let r2 = r##"<shape id="R" shape="rect" x="100" y="0" width="80" height="60"/>"##;
    let b2 = r##"<shape id="B" shape="rect" x="200" y="300" width="80" height="60"/>"##;
    let d = load(&format!("{r2}{b2}{}", conn(r#"route="orthogonal""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    let want = [[140.0, 60.0], [140.0, 180.0], [240.0, 180.0], [240.0, 300.0]];
    for (a, b) in p.iter().zip(want) {
        assert!(near(*a, b, EPS), "{p:?}");
    }
}

#[test]
fn orthogonal_through_waypoints() {
    // a = (140, 180), w1 = (300, 40), b = (500, 180): |dx| >= |dy| joins through (qx, py)
    let d = load(&format!("{R}{B}{}", conn(r#"route="orthogonal" points="300,40""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    // a -> (300, 180) -> w1 (300, 40) -> (500, 40) -> b; clipped at R's right edge and B's top edge
    let want = [[180.0, 180.0], [300.0, 180.0], [300.0, 40.0], [500.0, 40.0], [500.0, 150.0]];
    assert_eq!(p.len(), want.len(), "{p:?}");
    for (a, b) in p.iter().zip(want) {
        assert!(near(*a, b, EPS), "{p:?}");
    }
}

#[test]
fn straight_waypoints_and_coincident_points() {
    let d = load(&format!("{R}{B}{}", conn(r#"points="320,100 320,100""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    // the route a, w, w, b drops the repeated point; clipping cuts the first and last legs at the boxes
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(near(p[1], [320.0, 100.0], EPS));
}

#[test]
fn curved_apex_is_three_quarters_l_sin_beta() {
    let d = load(&format!("{R}{B}{}", conn(r#"route="curved" bend="30""#)));
    let g = frame(&d, 0.0);
    let p = path(&g, "c").expect("drawn");
    // the flattened cubic's vertex at t = 1/2 lies on the apex: (320, 180 - 0.75 * 0.3915 * 360 * sin 30°)
    let apex = p.iter().fold([0.0, f64::INFINITY], |m, q| if q[1] < m[1] { *q } else { m });
    assert!(near(apex, [320.0, 180.0 - 0.75 * 0.3915 * 360.0 * 0.5], 1e-6), "{apex:?}");
    assert!((apex[1] - 127.15).abs() < 0.01);
    // bend 0 is the straight segment
    let d = load(&format!("{R}{B}{}", conn(r#"route="curved" bend="0""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    assert!(p.iter().all(|q| (q[1] - 180.0).abs() < 1e-9));
    ends(&p, [180.0, 180.0], [460.0, 180.0], 1e-9);
    // negative bend bulges the other way
    let d = load(&format!("{R}{B}{}", conn(r#"route="curved" bend="-30""#)));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    assert!(p.iter().any(|q| q[1] > 230.0));
}

#[test]
fn the_connector_follows_a_moving_end() {
    let b = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60"><animate property="x"><key time="0" value="400"/><key time="1" value="460"/></animate></shape>"##;
    let d = load(&format!("{R}{b}{}", conn("")));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    ends(&p, [180.0, 180.0], [400.0, 180.0], EPS);
    let p = path(&frame(&d, 1.0), "c").expect("drawn");
    ends(&p, [180.0, 180.0], [460.0, 180.0], EPS);
}

#[test]
fn an_absent_end_draws_nothing() {
    let b = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" start="0.5"/>"##;
    let d = load(&format!("{R}{b}{}", conn("")));
    assert!(path(&frame(&d, 0.0), "c").is_none());
    assert!(path(&frame(&d, 1.0), "c").is_some());
    // invisible and transparent targets are still anchors
    let b = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" visible="false" opacity="0"/>"##;
    let d = load(&format!("{R}{b}{}", conn("")));
    assert!(path(&frame(&d, 0.0), "c").is_some());
    // a false condition makes the target absent
    let b = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" condition="false"/>"##;
    let d = load(&format!("{R}{b}{}", conn("")));
    assert!(path(&frame(&d, 0.0), "c").is_none());
    // an absent ancestor too
    let b = r##"<group id="gb" start="0.5"><shape id="B" shape="rect" x="460" y="150" width="80" height="60"/></group>"##;
    let d = load(&format!("{R}{b}{}", conn("")));
    assert!(path(&frame(&d, 0.0), "c").is_none());
}

#[test]
fn overlapping_ends_or_gaps_past_each_other_draw_nothing() {
    let d = load(&format!("{R}{B}{}", conn(r#"fromGap="200" toGap="100""#)));
    assert!(path(&frame(&d, 0.0), "c").is_none());
    let over = r##"<shape id="B" shape="rect" x="120" y="160" width="80" height="60"/>"##;
    let d = load(&format!("{R}{over}{}", conn("")));
    assert!(path(&frame(&d, 0.0), "c").is_none(), "B's centre is inside R: the route never leaves R before B");
}

#[test]
fn the_connector_lives_in_its_parents_space() {
    // the connector inside a moved, scaled group: its path is in the group's space, its ends where they are drawn
    let body = format!(
        r##"<group id="world" x="10" y="20" scaleX="0.5" scaleY="0.5">{R}{B}<connector id="c" from="R" to="B"/></group>"##
    );
    let d = load(&body);
    let g = frame(&d, 0.0);
    let n = g.nodes.iter().find(|n| &*n.id == "c").unwrap();
    let k = n.connector.as_ref().expect("drawn");
    assert!(near(k.path[0], [180.0, 180.0], EPS) && near(k.path[k.path.len() - 1], [460.0, 180.0], EPS));
    let p = path(&g, "c").unwrap();
    ends(&p, [100.0, 110.0], [240.0, 110.0], EPS);
    // an end in another branch of the tree, under its own transform
    let body = format!(
        r##"<group id="a" x="0" y="100">{R}</group><group id="b" rotation="0" x="0" y="-100">{B}</group><connector id="c" from="R" to="B"/>"##
    );
    let p = path(&frame(&load(&body), 0.0), "c").unwrap();
    // R centre (140, 280), B centre (500, 80): the line leaves R's top edge... compute against the boxes
    assert!(p[0][1] < 280.0 && p[p.len() - 1][1] > 80.0, "{p:?}");
}

#[test]
fn group_ends_use_their_geometric_box() {
    // a group with two sized children: its box is their union in the group's space
    let g = r##"<group id="G" x="400" y="100"><shape id="a" shape="rect" x="0" y="0" width="40" height="40"/><shape id="b" shape="rect" x="60" y="80" width="40" height="40"/></group>"##;
    let d = load(&format!("{R}{g}<connector id=\"c\" from=\"R\" to=\"G\" toAnchor=\"top-left\"/>"));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    assert!(near(p[p.len() - 1], [400.0, 100.0], EPS), "{p:?}");
    let d = load(&format!("{R}{g}<connector id=\"c\" from=\"R\" to=\"G\" toAnchor=\"bottom-right\"/>"));
    let p = path(&frame(&d, 0.0), "c").expect("drawn");
    assert!(near(p[p.len() - 1], [500.0, 220.0], EPS), "{p:?}");
}

#[test]
fn label_sits_at_label_at_with_its_offset_along_the_normal() {
    let d = load(&format!("{R}{B}{}", conn(r#"label="t" labelAt="0.25" labelOffset="10""#)));
    let g = frame(&d, 0.0);
    let n = g.nodes.iter().find(|n| &*n.id == "c").unwrap();
    let l = n.connector.as_ref().unwrap().label.expect("a label");
    // s0 = 0, s1 = 280: P = (180 + 70, 180); the normal turned 90° clockwise from +x is +y
    assert!(near(l.center, [250.0, 190.0], EPS), "{l:?}");
    assert_eq!(l.angle, 0.0);
    // the label is a layer of the text asset, centred on that point
    let lab = g.nodes.iter().find(|k| k.kind == "layer" && k.parent.is_some_and(|p| &*g.nodes[p as usize].id == "c"));
    let lab = lab.expect("label layer");
    assert!(near(lab.world.apply([20.0, 10.0]), [250.0, 190.0], EPS), "{:?}", lab.world.apply([20.0, 10.0]));
    // along: rotated with the direction, never upside down
    let d = load(&format!(
        "{R}{B}<connector id=\"c\" from=\"B\" to=\"R\" label=\"t\" labelOrient=\"along\"/>"
    ));
    let g = frame(&d, 0.0);
    let l = g.nodes.iter().find(|n| &*n.id == "c").unwrap().connector.as_ref().unwrap().label.unwrap();
    assert!((l.angle - 0.0).abs() < 1e-9 || (l.angle - 360.0).abs() < 1e-9, "travel 180° reads as 0°: {l:?}");
    let up = r##"<shape id="U" shape="rect" x="300" y="0" width="40" height="40"/><shape id="D" shape="rect" x="300" y="300" width="40" height="40"/>"##;
    let d = load(&format!("{up}<connector id=\"c\" from=\"U\" to=\"D\" label=\"t\" labelOrient=\"along\"/>"));
    let g = frame(&d, 0.0);
    let l = g.nodes.iter().find(|n| &*n.id == "c").unwrap().connector.as_ref().unwrap().label.unwrap();
    assert!((l.angle - 90.0).abs() < 1e-9, "{l:?}");
    let d = load(&format!("{up}<connector id=\"c\" from=\"D\" to=\"U\" label=\"t\" labelOrient=\"along\"/>"));
    let g = frame(&d, 0.0);
    let l = g.nodes.iter().find(|n| &*n.id == "c").unwrap().connector.as_ref().unwrap().label.unwrap();
    assert!((l.angle - 90.0).abs() < 1e-9, "-90° is outside (-90, 90], so it turns by 180: {l:?}");
}

#[test]
fn no_label_layer_is_drawn_when_the_connector_is_not() {
    let b = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" start="0.5"/>"##;
    let d = load(&format!("{R}{b}{}", conn(r#"label="t""#)));
    let g = frame(&d, 0.0);
    assert!(!g.nodes.iter().any(|k| k.kind == "layer" && k.draw && k.asset.as_deref() == Some("t")));
}

#[test]
fn bounds_are_those_of_v_before_trim() {
    let d = load(&format!("{R}{B}{}", conn(r#"trimEnd="0.5""#)));
    let g = frame(&d, 0.0);
    let k = g.nodes.iter().find(|n| &*n.id == "c").unwrap().connector.clone().unwrap();
    assert_eq!(k.bounds, [180.0, 180.0, 460.0, 180.0]);
}

#[test]
fn animated_gap_and_bend() {
    let d = load(&format!(
        "{R}{B}<connector id=\"c\" from=\"R\" to=\"B\"><animate property=\"fromGap\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"100\"/></animate></connector>"
    ));
    let p = path(&frame(&d, 0.5), "c").unwrap();
    ends(&p, [230.0, 180.0], [460.0, 180.0], EPS);
}
