//! SREP 75: the pixel cases of the SREP's kit (`conformance/srep_cases/srep-0075.json`), with its documents and its
//! measure: nine one-pixel regions, each within 3 code values of red (an even escape count), blue (odd) or black
//! (inside). Also the colours of `smooth` and of a Julia set at the renderer.

mod common;
use common::*;

/// The colour of the box [x0, x1) × [y0, y1), in 8-bit code values, within 3 of `rgb`.
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

fn kit(zoom: u32, max: u32) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.6">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<assets><fractal id="f" kind="mandelbrot" width="640" height="360" centerX="-0.743643887037158704752191506114774" centerY="0.131825904205311970493132056385139" zoom="{zoom}" maxIterations="{max}" colorMode="bands" palette="#FF0000FF #0000FFFF" insideColor="#000000FF"/></assets>
<composition>
<layer id="l" asset="f"/>
</composition>
</scene>
"##
    )
}

const AT: [(u32, u32); 9] =
    [(40, 30), (600, 30), (320, 180), (40, 330), (600, 330), (160, 90), (480, 270), (100, 200), (520, 120)];
const RED: [f64; 3] = [255.0, 0.0, 0.0];
const BLUE: [f64; 3] = [0.0, 0.0, 255.0];
const BLACK: [f64; 3] = [0.0, 0.0, 0.0];

#[track_caller]
fn kit_case(zoom: u32, max: u32, want: [[f64; 3]; 9]) {
    let Some(r) = render_xml(&kit(zoom, max)) else {
        return;
    };
    for ((x, y), rgb) in AT.into_iter().zip(want) {
        region(&r, [x, y, x + 1, y + 1], rgb);
    }
}

#[test]
fn srep_0075_mandelbrot_zoom_6() {
    // 223, inside, 1005, 275, 1308, 486, inside, 307, inside
    kit_case(6, 3000, [BLUE, BLACK, BLUE, BLUE, RED, RED, BLACK, BLUE, BLACK]);
}

#[test]
fn srep_0075_mandelbrot_zoom_12() {
    // 2054, 1986, 4047, 1985, 2221, 2052, 2064, 1972, 2086
    kit_case(12, 12000, [RED, RED, BLUE, BLUE, BLUE, RED, RED, RED, RED]);
}

#[test]
fn srep_0075_mandelbrot_zoom_20() {
    // 8510, 9061, 10056, 9059, 8485, 9943, 9760, 10082, 9228
    kit_case(20, 20000, [RED, BLUE, RED, BLUE, BLUE, BLUE, RED, RED, RED]);
}

#[test]
fn a_julia_set_and_smooth_colours_render() {
    // c = -1: the basilica. Its centre 0 is inside (black); far corners escape at once, n = 1, in palette[1] (bands)
    let xml = |mode: &str| {
        format!(
            r##"<scene version="1.6"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/>
<assets><fractal id="f" kind="julia" juliaX="-1" juliaY="0" width="64" height="64" span="8" maxIterations="200" colorMode="{mode}" palette="#FF0000FF #0000FFFF" insideColor="#00FF00FF"/></assets>
<composition><layer id="l" asset="f"/></composition></scene>"##
        )
    };
    let Some(r) = render_xml(&xml("bands")) else {
        return;
    };
    region(&r, [31, 31, 33, 33], [0.0, 255.0, 0.0]);
    region(&r, [0, 0, 1, 1], BLUE);
    let s = render_xml(&xml("smooth")).expect("a GPU");
    region(&s, [31, 31, 33, 33], [0.0, 255.0, 0.0]);
    // smooth: a mix of the palette's two colours, never green
    let p = s.at(0, 0);
    assert!(p[1] < 0.02 && p[0] + p[2] > 0.3, "{p:?}");
}
