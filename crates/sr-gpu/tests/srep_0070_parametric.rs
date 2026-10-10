//! SREP 70: the pixel cases of the SREP's kit (`conformance/srep_cases/srep-0070.json`), with its documents and its
//! measure: per colour class, the centre of its pixels and the extent of its bounding box, within 2 px; a region's
//! mean colour within 3 code values. Also: a parametric shape draws exactly as the literal path of its samples.

mod common;
use common::*;

/// The kit's colour classes.
fn class(name: &str, p: [f32; 4]) -> bool {
    let [r, g, b, _] = p;
    match name {
        "red" => r > 0.5 && g < 0.3 && b < 0.3,
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

const PATH_ELLIPSE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="e" shape="parametric" fill="#FF0000FF"><parametricPath x="320 + 100 * Math.cos(t)" y="180 + 50 * Math.sin(t)" t0="0" t1="6.283185307179586" samples="256" closed="true"/></shape>
</composition>
</scene>
"##;

const PATH_LISSAJOUS: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="l" shape="parametric" fill="#00000000" stroke="#FF0000FF" strokeWidth="4"><parametricPath x="320 + 100 * Math.sin(3 * t)" y="180 + 80 * Math.sin(2 * t)" t0="0" t1="6.283185307179586" samples="2000"/></shape>
</composition>
</scene>
"##;

const PATH_BREAK: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<composition>
<shape id="b" shape="parametric" fill="#00000000" stroke="#FF0000FF" strokeWidth="10"><parametricPath x="120 + 400 * t" y="(t &gt; 0.455 &amp;&amp; t &lt; 0.545) ? NaN : 180" t0="0" t1="1" samples="101"/></shape>
</composition>
</scene>
"##;

const SURFACE_PLANE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="s" primitive="parametric" material="m-red" x="320" y="180"><parametricSurface x="u" y="v" z="0" u0="-50" u1="50" v0="-25" v1="25" uSamples="8" vSamples="8"/></object3D>
</composition>
</scene>
"##;

const SURFACE_PLANE_MIRRORED_CULLED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="s" primitive="parametric" material="m-red" x="320" y="180"><parametricSurface x="-u" y="v" z="0" u0="-50" u1="50" v0="-25" v1="25" uSamples="8" vSamples="8"/></object3D>
</composition>
</scene>
"##;

const SURFACE_PLANE_MIRRORED_DOUBLE_SIDED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="s" primitive="parametric" material="m-red2" x="320" y="180"><parametricSurface x="-u" y="v" z="0" u0="-50" u1="50" v0="-25" v1="25" uSamples="8" vSamples="8"/></object3D>
</composition>
</scene>
"##;

const HEIGHTFIELD_FLAT: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="h" primitive="heightfield" material="m-red" x="320" y="180" rotationX="90"><heightfield height="0" width="100" depth="50" xSamples="4" zSamples="4"/></object3D>
</composition>
</scene>
"##;

const HEIGHTFIELD_UNDERSIDE_CULLED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true"/><material id="m-red2" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
<composition>
<object3D id="h" primitive="heightfield" material="m-red" x="320" y="180" rotationX="-90"><heightfield height="0" width="100" depth="50" xSamples="4" zSamples="4"/></object3D>
</composition>
</scene>
"##;

#[test]
fn srep_0070_path_ellipse() {
    let Some(r) = render_xml(PATH_ELLIPSE) else { return };
    expect(&r, "red", [320.0, 180.0, 200.0, 100.0]);
}

#[test]
fn srep_0070_path_lissajous() {
    let Some(r) = render_xml(PATH_LISSAJOUS) else { return };
    expect(&r, "red", [320.0, 180.0, 204.0, 164.0]);
}

#[test]
fn srep_0070_path_break() {
    let Some(r) = render_xml(PATH_BREAK) else { return };
    region(&r, [315, 176, 325, 184], [0.0, 0.0, 0.0]);
    region(&r, [196, 177, 204, 183], [255.0, 0.0, 0.0]);
    region(&r, [436, 177, 444, 183], [255.0, 0.0, 0.0]);
}

#[test]
fn srep_0070_surface_plane() {
    let Some(r) = render_xml(SURFACE_PLANE) else { return };
    expect(&r, "red", [320.0, 180.0, 100.0, 50.0]);
}

#[test]
fn srep_0070_surface_plane_mirrored_culled() {
    let Some(r) = render_xml(SURFACE_PLANE_MIRRORED_CULLED) else { return };
    assert!(measure(&r, "red").is_none(), "red must not appear: {:?}", measure(&r, "red"));
}

#[test]
fn srep_0070_surface_plane_mirrored_double_sided() {
    let Some(r) = render_xml(SURFACE_PLANE_MIRRORED_DOUBLE_SIDED) else { return };
    expect(&r, "red", [320.0, 180.0, 100.0, 50.0]);
}

#[test]
fn srep_0070_heightfield_flat() {
    let Some(r) = render_xml(HEIGHTFIELD_FLAT) else { return };
    expect(&r, "red", [320.0, 180.0, 100.0, 50.0]);
}

#[test]
fn srep_0070_heightfield_underside_culled() {
    let Some(r) = render_xml(HEIGHTFIELD_UNDERSIDE_CULLED) else { return };
    assert!(measure(&r, "red").is_none(), "red must not appear: {:?}", measure(&r, "red"));
}

#[test]
fn a_parametric_shape_draws_as_the_literal_path_of_its_samples() {
    // SREP 70 §2, equivalence: the same fill, stroke and fill rule on the path data of the samples
    let pts: Vec<[f64; 2]> = (0..7)
        .map(|i| {
            let t = i as f64 * std::f64::consts::TAU / 7.0;
            [320.0 + 120.0 * (2.0 * t).cos(), 180.0 + 120.0 * (3.0 * t).sin()]
        })
        .collect();
    let (d, _) = sr_eval::parametric::path_data(&pts, true);
    let paint = r##"fill="#FF0000FF" stroke="#00FF00FF" strokeWidth="6" fillRule="evenodd""##;
    let scene = |shape: &str| {
        format!(
            r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF"/><composition>{shape}</composition></scene>"##
        )
    };
    let Some(param) = render_xml(&scene(&format!(
        r#"<shape id="s" shape="parametric" {paint}><parametricPath x="320 + 120 * Math.cos(2 * t)" y="180 + 120 * Math.sin(3 * t)" t1="6.283185307179586" samples="7" closed="true"/></shape>"#
    ))) else {
        return;
    };
    let literal =
        render_xml(&scene(&format!(r#"<shape id="s" shape="path" path="{d}" width="640" height="360" {paint}/>"#)))
            .unwrap();
    assert_eq!(param.px, literal.px);
}
