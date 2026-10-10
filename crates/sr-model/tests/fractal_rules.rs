//! SREP 75: validation of fractal assets. V16 (version 1.6) and FRC1 (a Julia set has its constant, a Mandelbrot set
//! has none), with the documents of the kit (`conformance/srep_cases/srep-0075.json`): its findings cases fail with
//! their code and its pixel cases are valid. The centre is an `xs:decimal`, read at any length and kept as its text.

use sr_model::model::{AssetsChild, FractalAssetKind};
use sr_model::{validate_str, LoadOptions};

fn codes(xml: &str) -> Vec<String> {
    let r = validate_str(xml, &LoadOptions::without_assets());
    let mut c: Vec<String> = r.diagnostics.iter().map(|d| d.code.clone()).collect();
    c.sort();
    c.dedup();
    c
}

const CX: &str = "-0.743643887037158704752191506114774";
const CY: &str = "0.131825904205311970493132056385139";

/// A kit document: `fractal` attributes after the kit's centre, at `version`.
fn kit(version: &str, attrs: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="{version}">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<assets><fractal id="f" width="640" height="360" centerX="{CX}" centerY="{CY}" {attrs} colorMode="bands" palette="#FF0000FF #0000FFFF" insideColor="#000000FF"/></assets>
<composition>
<layer id="l" asset="f"/>
</composition>
</scene>
"##
    )
}

#[test]
fn the_kits_pixel_documents_are_valid_and_keep_the_centre_exactly() {
    for (zoom, max) in [(6, 3000), (12, 12000), (20, 20000)] {
        let xml = kit("1.6", &format!(r#"kind="mandelbrot" zoom="{zoom}" maxIterations="{max}""#));
        assert_eq!(codes(&xml), Vec::<String>::new(), "srep-0075-mandelbrot-zoom-{zoom}");
        let doc = sr_model::load_str(&xml, &LoadOptions::without_assets()).unwrap();
        let Some(AssetsChild::Fractal(f)) = doc.asset("f") else { panic!("no fractal") };
        assert_eq!((f.center_x.as_str(), f.center_y.as_str()), (CX, CY));
        assert_eq!((f.kind, f.zoom, f.max_iterations), (FractalAssetKind::Mandelbrot, zoom as f64, max));
        // the defaults of the schema
        assert_eq!((f.span.get(), f.bailout.get(), f.rotation, f.palette_offset), (4.0, 2.0, 0.0, 0.0));
        assert!(f.julia_x.is_none() && f.julia_y.is_none());
    }
}

#[test]
fn srep_0075_version_gate() {
    assert_eq!(codes(&kit("1.5", r#"kind="mandelbrot" zoom="6" maxIterations="100""#)), ["V16"]);
    for v in ["1.0", "1.1", "1.2", "1.3", "1.4"] {
        assert!(codes(&kit(v, r#"kind="mandelbrot""#)).contains(&"V16".to_string()), "{v}");
    }
}

#[test]
fn srep_0075_julia_needs_c() {
    assert_eq!(codes(&kit("1.6", r#"kind="julia" zoom="6" maxIterations="100""#)), ["FRC1"]);
    assert_eq!(codes(&kit("1.6", r#"kind="julia" juliaX="-0.8""#)), ["FRC1"]);
    assert_eq!(codes(&kit("1.6", r#"kind="julia" juliaX="-0.8" juliaY="0.156""#)), Vec::<String>::new());
    // a Mandelbrot set has no constant
    assert_eq!(codes(&kit("1.6", r#"kind="mandelbrot" juliaX="0" juliaY="0""#)), ["FRC1"]);
}

#[test]
fn a_decimal_has_no_exponent_and_any_number_of_digits() {
    for bad in ["1e-3", "INF", "NaN", "0x1", ".", "1.2.3", ""] {
        let xml =
            kit("1.6", r#"kind="mandelbrot""#).replace(&format!(r#"centerX="{CX}""#), &format!(r#"centerX="{bad}""#));
        assert!(codes(&xml).contains(&"S06".to_string()), "{bad:?}");
    }
    let long = format!("-0.{}", "7".repeat(400));
    for good in ["+.5", "-1.", "007", long.as_str()] {
        let xml =
            kit("1.6", r#"kind="mandelbrot""#).replace(&format!(r#"centerX="{CX}""#), &format!(r#"centerX="{good}""#));
        assert_eq!(codes(&xml), Vec::<String>::new(), "{good:?}");
    }
}
