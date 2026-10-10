//! SREP 74: the pixel cases of the SREP's kit (`conformance/srep_cases/srep-0074.json`), with its documents and its
//! measure: per colour class, the centre of its pixels and the extent of its bounding box, within 2 px. Also: the
//! viewport's background, its choice of lights, its opacity, and a document 3D scene beside it.

mod common;
use common::*;

/// The kit's colour classes.
fn class(name: &str, p: [f32; 4]) -> bool {
    let [r, g, b, _] = p;
    match name {
        "red" => r > 0.5 && g < 0.3 && b < 0.3,
        "green" => g > 0.5 && r < 0.3 && b < 0.3,
        "blue" => b > 0.5 && r < 0.3 && g < 0.3,
        _ => unreachable!("{name}"),
    }
}

/// The kit's measure of one class: (cx, cy, w, h), or None under 12 pixels.
fn measure(r: &Rendered, name: &str) -> Option<[f64; 4]> {
    let [w, h] = r.size;
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if class(name, r.at(x, y)) {
                n += 1;
                sx += x as f64;
                sy += y as f64;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    (n >= 12).then(|| [sx / n as f64 + 0.5, sy / n as f64 + 0.5, (x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64])
}

#[track_caller]
fn expect(r: &Rendered, name: &str, want: [f64; 4]) {
    let got = measure(r, name).unwrap_or_else(|| panic!("no {name} drawn"));
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2.0), "{name}: measured {got:?}, expected {want:?}");
}

/// The mean colour of the box [x0, x1) × [y0, y1), in 8-bit code values, within 3 of `rgb`.
#[track_caller]
fn region(r: &Rendered, [x0, y0, x1, y1]: [u32; 4], rgb: [f64; 3]) {
    let mut sum = [0f64; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            let p = r.at(x, y);
            for k in 0..3 {
                sum[k] += (p[k].clamp(0.0, 1.0) * 255.0).round() as f64;
            }
        }
    }
    let n = ((x1 - x0) * (y1 - y0)) as f64;
    let got = sum.map(|s| s / n);
    assert!(
        got.iter().zip(rgb).all(|(g, w)| (g - w).abs() <= 3.0),
        "box {:?}: {got:?}, expected {rgb:?}",
        [x0, y0, x1, y1]
    );
}

fn render_xml(xml: &str) -> Option<Rendered> {
    let d =
        sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let r = render(&d)?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

/// A kit document: the scene of `srep-0074.json` with `body` as its composition.
fn kit(body: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true" doubleSided="true"/><material id="m-green" baseColor="#00FF00FF" unlit="true" doubleSided="true"/></materials>
<composition>
{body}
</composition>
</scene>
"##
    )
}

const PLANE: &str = r#"<object3D id="p" primitive="plane" width="40" height="20" x="100" y="50" material="m-red"/>"#;

#[test]
fn srep_0074_implicit_camera() {
    let Some(r) = render_xml(&kit(&format!(
        r#"<viewport3D id="v" width="200" height="100" x="100" y="100">{PLANE}</viewport3D>"#
    ))) else {
        return;
    };
    expect(&r, "red", [200.0, 150.0, 40.0, 20.0]);
}

#[test]
fn srep_0074_main_camera_does_not_reach() {
    let Some(r) = render_xml(&kit(&format!(
        r#"<camera id="c" x="1320" y="180" z="-554.256"/>
<viewport3D id="v" width="200" height="100" x="100" y="100">{PLANE}</viewport3D>"#
    ))) else {
        return;
    };
    expect(&r, "red", [200.0, 150.0, 40.0, 20.0]);
}

#[test]
fn srep_0074_own_camera() {
    let Some(r) = render_xml(&kit(&format!(
        r#"<viewport3D id="v" width="200" height="100" x="100" y="100" camera="vc"><camera id="vc" x="130.0" y="50.0" z="-173.205"/>{PLANE}</viewport3D>"#
    ))) else {
        return;
    };
    expect(&r, "red", [170.0, 150.0, 40.0, 20.0]);
}

#[test]
fn srep_0074_viewport_camera_stays_inside() {
    let Some(r) = render_xml(&kit(
        r#"<object3D id="p" primitive="plane" width="40" height="20" x="500" y="300" material="m-red"/>
<viewport3D id="v" width="200" height="100" x="100" y="100"><camera id="vc" x="900" y="900" z="-50"/><object3D id="q" primitive="plane" width="40" height="20" x="100" y="50" material="m-green"/></viewport3D>"#,
    )) else {
        return;
    };
    expect(&r, "red", [500.0, 300.0, 40.0, 20.0]);
    // and the document's camera does not see the viewport's object: the viewport's camera, far off, shows none of it
    assert!(measure(&r, "green").is_none(), "the viewport's plane is visible: {:?}", measure(&r, "green"));
}

#[test]
fn srep_0074_clipped() {
    let Some(r) = render_xml(&kit(
        r#"<viewport3D id="v" width="200" height="100" x="100" y="100"><object3D id="p" primitive="plane" width="100" height="20" x="0" y="50" material="m-red"/></viewport3D>"#,
    )) else {
        return;
    };
    expect(&r, "red", [125.0, 150.0, 50.0, 20.0]);
}

#[test]
fn the_document_scene_and_a_viewport_scene_do_not_see_each_other() {
    // a document plane at the viewport's place, behind it, and a viewport plane: each scene shows its own object only
    let Some(r) = render_xml(&kit(
        r#"<object3D id="d" primitive="plane" width="40" height="20" x="500" y="60" material="m-red"/>
<viewport3D id="v" width="200" height="100" x="100" y="100"><object3D id="q" primitive="plane" width="40" height="20" x="100" y="50" material="m-green"/></viewport3D>"#,
    )) else {
        return;
    };
    expect(&r, "red", [500.0, 60.0, 40.0, 20.0]);
    expect(&r, "green", [200.0, 150.0, 40.0, 20.0]);
}

#[test]
fn a_viewport_fills_its_picture_with_its_background_and_draws_with_its_opacity() {
    let Some(r) =
        render_xml(&kit(r##"<viewport3D id="v" width="200" height="100" x="100" y="100" background="#0000FFFF"/>
<viewport3D id="w" width="100" height="50" x="400" y="200" background="#0000FFFF" opacity="0.5"/>"##))
    else {
        return;
    };
    expect(&r, "blue", [200.0, 150.0, 200.0, 100.0]);
    region(&r, [110, 110, 290, 190], [0.0, 0.0, 255.0]);
    // outside the picture, the project's background
    region(&r, [0, 0, 90, 90], [0.0, 0.0, 0.0]);
    // half opacity over black: half the blue, in the working space the compositor blends in
    let p = r.at(450, 225);
    assert!(p[2] > 0.1 && p[2] < 0.9 && p[0] < 0.05, "{p:?}");
}

#[test]
fn a_viewports_lights_are_the_ones_it_names() {
    // a lit red plane: with every light of the document it is lit, with only the dark one it is black
    let xml = |lights: &str| {
        kit(&format!(
            r#"<viewport3D id="v" width="200" height="100" x="100" y="100"{lights}><object3D id="p" primitive="plane" width="40" height="20" x="100" y="50" material="m-lit"/></viewport3D>"#
        ))
        .replace(
            "</materials>",
            r##"<material id="m-lit" baseColor="#FF0000FF" doubleSided="true"/></materials>"##,
        )
        .replace(
            "</composition>",
            r#"</composition><lights><light id="bright" type="ambient" intensity="1"/><light id="dark" type="ambient" intensity="0"/></lights>"#,
        )
    };
    let Some(all) = render_xml(&xml("")) else {
        return;
    };
    expect(&all, "red", [200.0, 150.0, 40.0, 20.0]);
    let dark = render_xml(&xml(r#" lights="dark""#)).expect("a GPU");
    assert!(measure(&dark, "red").is_none(), "lit by a light it does not name");
    region(&dark, [190, 145, 210, 155], [0.0, 0.0, 0.0]);
    let bright = render_xml(&xml(r#" lights="bright""#)).expect("a GPU");
    expect(&bright, "red", [200.0, 150.0, 40.0, 20.0]);
}
