//! SREP 67, Semantics 5 to 7: `error-diffusion` and `segmented-sort`, on the documents of sr-core's kit
//! (conformance/srep_cases/srep-0067.json, written by conformance/tools/srep67_cases.py). Each region's mean 8-bit
//! display colour is within 3 code values of the kit's.

use super::common;
use common::*;

fn doc(body: &str, effects: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/><composition>{body}</composition><effects>{effects}</effects></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// The frame at 0 as 8-bit display sRGB.
fn frame(d: &sr_model::Document) -> Option<(Vec<u8>, Vec<String>)> {
    let gpu = gpu()?;
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let g = ev.evaluate(0.0);
    let mut sub = |t: f64| ev.evaluate(t);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    let problems = f.stats.unsupported.iter().chain(&f.stats.errors).cloned().collect();
    Some((r.to_srgb8(&r.read(&f.texture)), problems))
}

/// Mean 8-bit R, G, B over `[x0, y0, x1, y1)`.
fn mean(px: &[u8], [x0, y0, x1, y1]: [usize; 4]) -> [f64; 3] {
    let mut s = [0.0; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            for c in 0..3 {
                s[c] += px[(y * 640 + x) * 4 + c] as f64;
            }
        }
    }
    let n = ((x1 - x0) * (y1 - y0)) as f64;
    s.map(|v| v / n)
}

fn close(got: [f64; 3], want: f64, what: &str) {
    assert!(got.iter().all(|v| (v - want).abs() <= 3.0), "{what}: {got:?}, kit {want}");
}

#[test]
fn error_diffusion_of_mid_grey_matches_the_kit() {
    let grey = r##"<shape id="g" shape="rect" width="64" height="64" x="100" y="100" fill="#808080FF" effects="ed"/>"##;
    for (kernel, m, first) in
        [("floyd-steinberg", 127.7, [255.0, 0.0, 255.0, 0.0]), ("atkinson", 127.5, [255.0, 0.0, 0.0, 255.0])]
    {
        let d = doc(
            grey,
            &format!(r##"<effect id="ed" type="error-diffusion" kernel="{kernel}" palette="#000000FF #FFFFFFFF"/>"##),
        );
        let Some((px, problems)) = frame(&d) else { return };
        assert!(problems.is_empty(), "{problems:?}");
        close(mean(&px, [100, 100, 164, 164]), m, kernel);
        for (k, want) in first.iter().enumerate() {
            close(mean(&px, [100 + k, 100, 101 + k, 101]), *want, &format!("{kernel} pixel {k}"));
        }
        // every pixel is black or white
        for y in 100..164 {
            for x in 100..164 {
                let v = px[(y * 640 + x) * 4];
                assert!(v == 0 || v == 255, "{kernel} ({x}, {y}): {v}");
            }
        }
    }
}

#[test]
fn segmented_sort_matches_the_kit() {
    let strip = r##"<group id="g" effects="ps"><shape id="w" shape="rect" width="32" height="8" x="100" y="100" fill="#FFFFFFFF"/><shape id="b" shape="rect" width="32" height="8" x="132" y="100" fill="#000000FF"/></group>"##;
    for (attrs, left, right) in [
        (r#"direction="horizontal" order="ascending""#, 0.0, 255.0),
        (r#"direction="horizontal" order="descending""#, 255.0, 0.0),
        (r#"direction="horizontal" low="0" high="0.5""#, 255.0, 0.0),
    ] {
        let d = doc(strip, &format!(r#"<effect id="ps" type="segmented-sort" {attrs}/>"#));
        let Some((px, problems)) = frame(&d) else { return };
        assert!(problems.is_empty(), "{problems:?}");
        close(mean(&px, [100, 100, 132, 108]), left, attrs);
        close(mean(&px, [132, 100, 164, 108]), right, attrs);
    }
}
