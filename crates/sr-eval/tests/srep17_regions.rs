//! SREP 17 §3: shapes on regions of a pdf asset follow the layer that shows the page. The geometry of the
//! conformance cases (pdf-region, pdf-region-fit, pdf-region-rotate) on the evaluated frame.

use sr_eval::{EvalOptions, Evaluator, FrameGraph};

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn load(layer: &str, shape: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="2"/><assets><pdf id="paper" src="p.pdf" sha256="{ZERO}" cache="p.png" cacheSha256="{ZERO}" width="200" height="100"><region id="g" x="20" y="10" width="60" height="20"/></pdf></assets><composition>{layer}<group id="holder" x="1000" y="1000" scaleX="9">{shape}</group></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn frame(d: &sr_model::Document, t: f64) -> FrameGraph {
    Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t)
}

/// Axis-aligned box (x0, y0, x1, y1) of node `id`'s box in frame pixels.
fn bbox(g: &FrameGraph, id: &str) -> [f64; 4] {
    let n = g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"));
    let [w, h] = n.size.expect("a size");
    n.world.bbox([0.0, 0.0, w, h])
}

#[track_caller]
fn near(got: [f64; 4], want: [f64; 4]) {
    for k in 0..4 {
        assert!((got[k] - want[k]).abs() < 1e-9, "box {got:?}, want {want:?}");
    }
}

const MARK: &str = r##"<shape id="s" shape="rect" region="g" regionLayer="L" fill="#FF0000FF"/>"##;

#[test]
fn pdf_region() {
    // layer at (100, 50) scaled 2: the region grown by 5 is (15, 5, 70, 30) in page pixels
    let d = load(
        r#"<layer id="L" asset="paper" x="100" y="50" scaleX="2" scaleY="2"/>"#,
        r##"<shape id="s" shape="rect" region="g" regionLayer="L" regionPadding="5" fill="#FF0000FF"/>"##,
    );
    // centre (200, 90), 140 x 60; the group's transform (it holds the shape) is not applied
    near(bbox(&frame(&d, 0.0), "s"), [130.0, 60.0, 270.0, 120.0]);
}

#[test]
fn pdf_region_fit() {
    // a 100 x 100 box, contain: scale 0.5, the page 25 px down
    let d = load(r#"<layer id="L" asset="paper" boxWidth="100" boxHeight="100" fit="contain"/>"#, MARK);
    near(bbox(&frame(&d, 0.0), "s"), [10.0, 30.0, 40.0, 40.0]);
}

#[test]
fn pdf_region_rotate() {
    let d = load(r#"<layer id="L" asset="paper" x="300" y="50" rotation="90"/>"#, MARK);
    near(bbox(&frame(&d, 0.0), "s"), [270.0, 70.0, 290.0, 130.0]);
}

#[test]
fn crop_and_flip_carry_the_mark() {
    // the left half of the page is cropped away and the rest flipped: the region (20..80) is now outside the
    // kept half and drawn where it falls, mirrored
    let d = load(r#"<layer id="L" asset="paper" cropLeft="0.5" flipX="true"/>"#, MARK);
    let b = bbox(&frame(&d, 0.0), "s");
    // kept u in [0.5, 1] is drawn over x in [100, 200] mirrored: x = 100 + (1 - u) * 200
    near(b, [100.0 + (1.0 - 0.4) * 200.0, 10.0, 100.0 + (1.0 - 0.1) * 200.0, 30.0]);
}

#[test]
fn the_shapes_own_transform_comes_last() {
    let d = load(
        r#"<layer id="L" asset="paper" x="100" y="50"/>"#,
        r##"<shape id="s" shape="rect" region="g" regionLayer="L" x="3" y="4" fill="#FF0000FF"/>"##,
    );
    near(bbox(&frame(&d, 0.0), "s"), [123.0, 64.0, 183.0, 84.0]);
}

#[test]
fn the_mark_follows_the_layer_and_its_padding_animates() {
    let d = load(
        r#"<layer id="L" asset="paper"><animate property="x"><key time="0" value="0"/><key time="1" value="100"/></animate></layer>"#,
        r##"<shape id="s" shape="rect" region="g" regionLayer="L" fill="#FF0000FF"><animate property="regionPadding"><key time="0" value="0"/><key time="1" value="10"/></animate></shape>"##,
    );
    near(bbox(&frame(&d, 0.0), "s"), [20.0, 10.0, 80.0, 30.0]);
    near(bbox(&frame(&d, 1.0), "s"), [110.0, 0.0, 190.0, 40.0]);
}

#[test]
fn the_mark_is_not_drawn_while_its_page_is_not() {
    let d = load(r#"<layer id="L" asset="paper" start="1"/>"#, MARK);
    let g = frame(&d, 0.0);
    assert!(!g.nodes.iter().find(|n| &*n.id == "s").unwrap().draw);
    let g = frame(&d, 1.5);
    assert!(g.nodes.iter().find(|n| &*n.id == "s").unwrap().draw);
}

#[test]
fn a_connector_can_point_at_a_mark() {
    let d = load(
        r#"<layer id="L" asset="paper" x="100" y="50"/><shape id="from" shape="rect" x="400" y="40" width="20" height="20"/><connector id="c" from="from" to="s" toAnchor="right"/>"#,
        MARK,
    );
    let g = frame(&d, 0.0);
    let c = g.nodes.iter().find(|n| &*n.id == "c").unwrap();
    let path = &c.connector.as_ref().expect("drawn").path;
    let end = c.world.apply(path[path.len() - 1]);
    assert!((end[0] - 180.0).abs() < 1e-9 && (end[1] - 70.0).abs() < 1e-9, "{end:?}");
}
