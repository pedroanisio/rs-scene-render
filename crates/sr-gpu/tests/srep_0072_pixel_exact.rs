//! SREP 72: pixel-exact options. The pixel cases of the SREP's kit (`conformance/srep_cases/srep-0072.json`), with
//! its documents and shaders (`tests/shaders/srep72-*.fs`, byte-equal to the kit's) and its measure: per colour
//! class, the centre of its pixels and the extent of its bounding box within 2 px; a region's mean colour within 3
//! code values.

mod common;
use common::*;

fn class(name: &str, p: [f32; 4]) -> bool {
    let [r, g, b, _] = p;
    match name {
        "red" => r > 0.5 && g < 0.3 && b < 0.3,
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
            for (k, s) in sum.iter_mut().enumerate() {
                *s += (p[k].clamp(0.0, 1.0) * 255.0).round() as f64;
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

fn shader(name: &str) -> String {
    format!("{}/tests/shaders/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The kit's 640 × 360 scene on black, with extra project attributes, a composition and effects.
fn scene(project: &str, composition: &str, effects: &str) -> String {
    format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1" {project}/>
  <composition>{composition}</composition>{effects}</scene>"##
    )
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

/// The kit's square for the shader cases: 100 × 100, white, centred on (x, 180), with the shader `file`.
fn shaded_square(x: &str, file: &str) -> String {
    scene(
        "",
        &format!(
            r##"<shape id="s" shape="rect" width="100" height="100" anchorX="50" anchorY="50" x="{x}" y="180" fill="#FFFFFFFF" effects="fx"/>"##
        ),
        &format!(r#"<effects><effect id="fx" type="shader" src="{}"/></effects>"#, shader(file)),
    )
}

#[test]
fn srep_0072_content_rect_stripe() {
    let Some(r) = render_xml(&shaded_square("320", "srep72-stripe.fs")) else { return };
    expect(&r, "red", [275.0, 180.0, 10.0, 100.0]);
}

#[test]
fn srep_0072_content_rect_stripe_fractional() {
    let Some(r) = render_xml(&shaded_square("320.4", "srep72-stripe.fs")) else { return };
    expect(&r, "red", [275.4, 180.0, 10.0, 100.0]);
}

#[test]
fn the_content_rect_reaches_every_program_form() {
    // the scene-render convention: the first 10 columns of the content
    let Some(r) = render_xml(&shaded_square("320", "srep72-stripe-plain.glsl")) else { return };
    expect(&r, "red", [275.0, 180.0, 10.0, 100.0]);
    // Shadertoy, whose gl_FragCoord counts rows from the bottom: the first 10 rows from the content's top
    let r = render_xml(&shaded_square("320", "srep72-stripe-shadertoy.glsl")).unwrap();
    expect(&r, "red", [320.0, 135.0, 100.0, 10.0]);
}

/// The kit's half-covered column: a white 100 × 50 rect at x = 100.5 on black, so pixel column 100 is half covered.
fn half_covered(project: &str, edge: &str) -> String {
    scene(
        project,
        &format!(r##"<shape id="e" shape="rect" width="100" height="50" x="100.5" y="100" fill="#FFFFFFFF" {edge}/>"##),
        "",
    )
}

#[test]
fn srep_0072_edge_blend_linear() {
    // linear overrides linearLight="false": half of white in linear light is 187.5 encoded
    let Some(r) = render_xml(&half_covered(r#"linearLight="false""#, r#"edgeBlend="linear""#)) else { return };
    region(&r, [100, 110, 101, 140], [187.5; 3]);
}

#[test]
fn srep_0072_edge_blend_encoded() {
    // in a linear-light document, the node mixes on encoded values: half of white is 127.5
    let Some(r) = render_xml(&half_covered("", r#"edgeBlend="encoded""#)) else { return };
    region(&r, [100, 110, 101, 140], [127.5; 3]);
}

#[test]
fn srep_0072_edge_blend_inherit() {
    let Some(r) = render_xml(&half_covered("", "")) else { return };
    region(&r, [100, 110, 101, 140], [187.5; 3]);
    // inherit follows linearLight="false"
    let r = render_xml(&half_covered(r#"linearLight="false""#, "")).unwrap();
    region(&r, [100, 110, 101, 140], [127.5; 3]);
}

#[test]
fn an_edge_blend_equal_to_the_frame_space_draws_as_today() {
    // edgeBlend="linear" in a linear document and "encoded" in an encoded one take the normal path: the same pixels
    let Some(a) = render_xml(&half_covered("", r#"edgeBlend="linear""#)) else { return };
    let b = render_xml(&half_covered("", "")).unwrap();
    assert_eq!(a.px, b.px);
    let a = render_xml(&half_covered(r#"linearLight="false""#, r#"edgeBlend="encoded""#)).unwrap();
    let b = render_xml(&half_covered(r#"linearLight="false""#, "")).unwrap();
    assert_eq!(a.px, b.px);
}

/// The kit's step edge: a white 200 × 50 rect at x = 100, rendered at n = 2.
fn step(filter: &str) -> String {
    scene(
        &format!(r#"supersample="2" supersampleFilter="{filter}""#),
        r##"<shape id="e" shape="rect" width="200" height="50" x="100" y="100" fill="#FFFFFFFF"/>"##,
        "",
    )
}

#[track_caller]
fn columns(r: &Rendered, want: [f64; 8]) {
    for (k, v) in want.iter().enumerate() {
        let x = 96 + k as u32;
        region(r, [x, 110, x + 1, 140], [*v; 3]);
    }
}

#[test]
fn srep_0072_supersample_box() {
    let Some(r) = render_xml(&step("box")) else { return };
    assert_eq!(r.size, [640, 360], "the frame keeps its size");
    columns(&r, [0.0, 0.0, 0.0, 0.0, 255.0, 255.0, 255.0, 255.0]);
}

#[test]
fn srep_0072_supersample_lanczos3() {
    let Some(r) = render_xml(&step("lanczos3")) else { return };
    assert_eq!(r.size, [640, 360]);
    columns(&r, [0.0, 12.0, 0.0, 65.5, 248.9, 255.0, 254.6, 255.0]);
}

#[test]
fn supersample_taps_follow_the_kernels() {
    // box: the n samples inside the pixel
    assert_eq!(sr_gpu::shader::supersample_taps(2, false), (0, vec![1.0, 1.0]));
    assert_eq!(sr_gpu::shader::supersample_taps(3, false), (0, vec![1.0; 3]));
    // Lanczos-3 at n = 2: the samples with |(d + 0.5)/2 − 0.5| < 3, d = −5 … 6, symmetric about the pixel centre
    let (first, w) = sr_gpu::shader::supersample_taps(2, true);
    assert_eq!((first, w.len()), (-5, 12));
    for k in 0..w.len() {
        assert!((w[k] - w[w.len() - 1 - k]).abs() < 1e-12, "{w:?}");
    }
    let sinc = |x: f64| (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x);
    assert!((w[5] - sinc(0.25) * sinc(0.25 / 3.0)).abs() < 1e-12, "the sample at x = −0.25");
}

#[test]
fn supersample_one_is_todays_rendering() {
    let plain = scene(
        "",
        r##"<shape id="e" shape="ellipse" width="201" height="77" x="100.3" y="100.6" fill="#FF8040FF"/>"##,
        "",
    );
    let one = plain.replace(r#"seed="1" "#, r#"seed="1" supersample="1" supersampleFilter="lanczos3" "#);
    let Some(a) = render_xml(&plain) else { return };
    assert_eq!(a.px, render_xml(&one).unwrap().px);
}
