//! SREP 16 conformance cases on the rendered frame: 640 × 360 on black, stroke #00FF00, strokeWidth 6, butt caps.
//! R is a red 80 × 60 rect at (100, 150), B a blue one at (460, 150). Boxes within 2 px.

mod common;
use common::*;

const R: &str = r##"<shape id="R" shape="rect" x="100" y="150" width="80" height="60" fill="#FF0000FF"/>"##;
const B: &str = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" fill="#0000FFFF"/>"##;

fn scene(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="2" background="#000000FF"/><assets><text id="t" text="Hi" width="40" height="20" size="12" color="#FFFFFFFF"/></assets><composition>{body}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn conn(attrs: &str) -> String {
    format!(r##"<connector id="c" from="R" to="B" stroke="#00FF00FF" strokeWidth="6" {attrs}/>"##)
}

fn green(r: &Rendered, x: u32, y: u32) -> bool {
    let p = r.at(x, y);
    p[1] > 0.5 && p[0] < 0.5 && p[2] < 0.5
}

/// Bounding box (x0, y0, x1, y1) of the green pixels.
fn green_box(r: &Rendered) -> Option<[u32; 4]> {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            if green(r, x, y) {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    (b[0] != u32::MAX).then_some(b)
}

#[track_caller]
fn near(got: Option<[u32; 4]>, want: [f64; 4]) {
    let g = got.expect("green drawn");
    for k in 0..4 {
        assert!((g[k] as f64 - want[k]).abs() <= 2.0, "box {g:?}, want {want:?} (±2)");
    }
}

#[test]
fn straight() {
    let Some(r) = render(&scene(&format!("{R}{B}{}", conn("")))) else { return };
    near(green_box(&r), [180.0, 177.0, 460.0, 183.0]);
}

#[test]
fn rotated_gap() {
    let sq = r##"<shape id="B" shape="rect" x="500" y="180" width="80" height="80" anchorX="40" anchorY="40" rotation="45" fill="#0000FFFF"/>"##;
    let Some(r) = render(&scene(&format!("{R}{sq}{}", conn(r#"fromGap="10" toGap="10""#)))) else { return };
    near(green_box(&r), [190.0, 177.0, 433.43, 183.0]);
}

#[test]
fn anchor() {
    let r0 = r##"<shape id="R" shape="rect" x="280" y="40" width="80" height="60" fill="#FF0000FF"/>"##;
    let b0 = r##"<shape id="B" shape="rect" x="440" y="260" width="80" height="60" fill="#0000FFFF"/>"##;
    let Some(r) = render(&scene(&format!("{r0}{b0}{}", conn(r#"fromAnchor="bottom" toAnchor="top""#)))) else {
        return;
    };
    // line (320, 100) -> (480, 260), 6 px wide with butt caps: the box of a 45° band
    let half = 3.0 / std::f64::consts::SQRT_2;
    near(green_box(&r), [320.0 - half, 100.0 - half, 480.0 + half, 260.0 + half]);
    assert!(green(&r, 400, 180));
}

#[test]
fn orthogonal() {
    let r0 = r##"<shape id="R" shape="rect" x="100" y="60" width="80" height="60" fill="#FF0000FF"/>"##;
    let b0 = r##"<shape id="B" shape="rect" x="460" y="240" width="80" height="60" fill="#0000FFFF"/>"##;
    let Some(r) = render(&scene(&format!("{r0}{b0}{}", conn(r#"route="orthogonal""#)))) else { return };
    assert!(green(&r, 320, 180), "the vertical leg at x = 320");
    assert!(!green(&r, 250, 180), "black off the route");
    let b = green_box(&r).unwrap();
    assert!((b[0] as f64 - 180.0).abs() <= 2.0 && (b[2] as f64 - 460.0).abs() <= 2.0, "{b:?}");
}

#[test]
fn curved() {
    let Some(r) = render(&scene(&format!("{R}{B}{}", conn(r#"route="curved" bend="30""#)))) else { return };
    assert!(green(&r, 320, 128), "apex at (320, 127.15)");
    assert!(!green(&r, 320, 180), "nothing on the chord");
}

#[test]
fn arrow() {
    let Some(r) = render(&scene(&format!("{R}{B}{}", conn(r#"markerEnd="arrow""#)))) else { return };
    near(green_box(&r), [180.0, 168.0, 460.0, 192.0]);
}

#[test]
fn trim_arrow() {
    let Some(r) = render(&scene(&format!("{R}{B}{}", conn(r#"markerEnd="arrow" trimEnd="0.5""#)))) else { return };
    near(green_box(&r), [180.0, 168.0, 320.0, 192.0]);
    for x in 323..640 {
        assert!(!green(&r, x, 180), "green right of 322 at x = {x}");
    }
}

#[test]
fn follows() {
    let b0 = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" fill="#0000FFFF"><animate property="x"><key time="0" value="400"/></animate></shape>"##;
    let Some(r) = render(&scene(&format!("{R}{b0}{}", conn("")))) else { return };
    let b = green_box(&r).unwrap();
    assert!((b[0] as f64 - 180.0).abs() <= 2.0 && (b[2] as f64 - 400.0).abs() <= 2.0, "{b:?}");
}

#[test]
fn follows_after_a_cached_frame() {
    // the node cache must not keep the path of a frame whose ends were elsewhere
    let b0 = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" fill="#0000FFFF"><animate property="x"><key time="0" value="400"/><key time="1" value="460"/></animate></shape>"##;
    let d = scene(&format!("{R}{b0}{}", conn("")));
    let Some(r) = render_times(&d, &[0.0, 1.0]) else { return };
    let b = green_box(&r).unwrap();
    assert!((b[2] as f64 - 460.0).abs() <= 2.0, "{b:?}");
}

#[test]
fn absent() {
    let b0 = r##"<shape id="B" shape="rect" x="460" y="150" width="80" height="60" fill="#0000FFFF" start="0.5"/>"##;
    let Some(r) = render(&scene(&format!("{R}{b0}{}", conn("")))) else { return };
    assert!(green_box(&r).is_none());
}

#[test]
fn label_offset_moves_the_label_along_the_normal() {
    // the relative measurement of SREP 16 Conformance: two identical labels at different offsets
    let white = |r: &Rendered| {
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for y in 0..r.size[1] {
            for x in 0..r.size[0] {
                let p = r.at(x, y);
                if p[0] > 0.5 && p[1] > 0.5 && p[2] > 0.5 {
                    sx += x as f64;
                    sy += y as f64;
                    n += 1.0;
                }
            }
        }
        assert!(n > 0.0, "no label drawn");
        [sx / n, sy / n]
    };
    let at = |off: f64| render(&scene(&format!("{R}{B}{}", conn(&format!(r#"label="t" labelOffset="{off}""#)))));
    let (Some(a), Some(b)) = (at(0.0), at(30.0)) else { return };
    let (ca, cb) = (white(&a), white(&b));
    assert!((cb[0] - ca[0]).abs() <= 1.0 && (cb[1] - ca[1] - 30.0).abs() <= 1.0, "{ca:?} -> {cb:?}");
}
