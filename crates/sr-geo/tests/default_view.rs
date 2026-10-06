//! The default view of a map (no `fit`, zoom 0) draws what the document gives it.
//!
//! A conformal cone sends the pole opposite its parallels to infinity, so fitting the whole
//! sphere collapsed the scale and a default `lambert-conformal` map drew nothing.

use sr_geo::data::Geometry;
use sr_geo::view::{Kind, Map, View};

/// The 20 x 20 degree box at 40-60 N, read as a document reads it (rings are wound the way the loader expects).
fn box_20() -> Geometry {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lambert-box.geojson");
    let features = sr_geo::data::load(&path, None, None).expect("the fixture loads");
    features[0].geometry.clone()
}

/// The pixel size of a 20 x 20 degree box at 40-60 N seen through the default view.
fn box_pixels(kind: Kind) -> [f64; 2] {
    let (map, c) = Map::new(kind, None, [480.0, 360.0], &[], 0.0, Some([0.0, 50.0]));
    let proj = map.projection(&View { lon: c[0], lat: c[1], zoom: 0.0, rotation: 0.0 });
    let b = proj.project(&box_20()).bounds().expect("the box is inside the frame");
    [b[1][0] - b[0][0], b[1][1] - b[0][1]]
}

#[test]
fn a_default_lambert_conformal_map_shows_a_twenty_degree_box_at_a_visible_size() {
    let [w, h] = box_pixels(Kind::LambertConformal);
    assert!(w > 10.0 && h > 10.0, "the box is {w:.2} x {h:.2} px");
}

#[test]
fn measure_default_sizes() {
    for kind in [Kind::EqualEarth, Kind::Albers, Kind::LambertConformal, Kind::Mercator] {
        let [w, h] = box_pixels(kind);
        eprintln!("SIZE {kind:?} {w:.3} x {h:.3}");
    }
}
