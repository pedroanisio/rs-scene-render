//! A map with no `zoom` and no `fit` draws its layers, and a layer that draws nothing says so.
//!
//! The repro is the G2 lab case `lambert-default-empty`: a default `lambert-conformal` map fitted the
//! whole sphere, a conformal cone sends the opposite pole to infinity, and the 20-degree box was
//! sub-pixel with no diagnostic.

mod common;
use common::*;

use std::path::PathBuf;

fn geo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geo")
}

fn map_doc(geo: &str, map_attrs: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="480" height="360" fps="10" duration="1" background="#202020"/>
           <assets><geo id="g" src="{geo}"/>
           <map id="m" width="480" height="360" {map_attrs} background="#000000">
             <geoLayer geo="g" fill="#FF0000" stroke="#FF0000" strokeWidth="0"/></map></assets>
           <composition><layer id="l" asset="m"/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(geo_dir()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn red_pixels(r: &Rendered, w: u32, h: u32) -> usize {
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            let p = r.at(x, y);
            if p[0] > 0.5 && p[1] < 0.2 && p[2] < 0.2 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_default_lambert_conformal_map_draws_its_polygon() {
    let d = map_doc("lambert-box.geojson", r#"projection="lambert-conformal" centerLon="0" centerLat="50""#);
    let Some(r) = render(&d) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    let n = red_pixels(&r, 480, 360);
    assert!(n > 100, "the box drew {n} red pixels");
}

#[test]
fn a_geo_layer_that_draws_nothing_is_reported() {
    // a 0.001-degree speck at the default view of a world map is far below a pixel
    let d = map_doc("speck.geojson", r#"projection="equal-earth""#);
    let Some(r) = render(&d) else { return };
    assert_eq!(red_pixels(&r, 480, 360), 0);
    assert!(
        r.stats.unsupported.iter().any(|m| m.contains("geoLayer g") && m.contains("nothing is drawn")),
        "{:?}",
        r.stats.unsupported
    );
}

#[test]
fn a_geo_layer_that_draws_is_not_reported() {
    let d = map_doc("lambert-box.geojson", r#"projection="equal-earth""#);
    let Some(r) = render(&d) else { return };
    assert!(red_pixels(&r, 480, 360) > 0);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
}
