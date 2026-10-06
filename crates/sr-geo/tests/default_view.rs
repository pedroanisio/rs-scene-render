//! The default view of a map (no `fit`, zoom 0) draws what the document gives it.
//!
//! A conformal cone sends the pole opposite its parallels to infinity, so fitting the whole
//! sphere collapsed the scale and a default `lambert-conformal` map drew nothing. The fix is a floor on the
//! effective scale: a view that already drew something sensible (a zoom of 11 or more) keeps its meaning.

use sr_geo::data::Geometry;
use sr_geo::view::{Kind, Map, View};

/// The 20 x 20 degree box at 40-60 N, read as a document reads it (rings are wound the way the loader expects).
fn box_20() -> Geometry {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lambert-box.geojson");
    let features = sr_geo::data::load(&path, None, None).expect("the fixture loads");
    features[0].geometry.clone()
}

/// The pixel size of the 20 x 20 degree box at 40-60 N through a default (no `fit`) map at `zoom`.
fn box_at(kind: Kind, zoom: f64) -> [f64; 2] {
    let (map, c) = Map::new(kind, None, [480.0, 360.0], &[], 0.0, Some([0.0, 50.0]));
    let proj = map.projection(&View { lon: c[0], lat: c[1], zoom, rotation: 0.0 });
    let b = proj.project(&box_20()).bounds().expect("the box is inside the frame");
    [b[1][0] - b[0][0], b[1][1] - b[0][1]]
}

fn near(got: [f64; 2], want: [f64; 2], what: &str) {
    assert!((got[0] - want[0]).abs() < 0.01 && (got[1] - want[1]).abs() < 0.01, "{what}: {got:?}, want {want:?}");
}

#[test]
fn a_default_lambert_conformal_map_shows_the_box_at_a_visible_size() {
    let [w, h] = box_at(Kind::LambertConformal, 0.0);
    assert!(w > 10.0 && h > 10.0, "the box is {w:.2} x {h:.2} px");
}

/// Measured on main 32c8ac1 (uncapped) with the same box and frame: zoom 0 to 10 drew a box of 0.015 to 15.5 px,
/// zoom 11 one of 31.0 x 42.9 px.
#[test]
fn the_floor_lifts_only_the_views_that_drew_less_than_it_and_leaves_the_rest() {
    let floor = [15.546, 21.498];
    for zoom in [0.0, 4.0, 6.0, 8.0, 9.0, 10.0] {
        near(box_at(Kind::LambertConformal, zoom), floor, &format!("zoom {zoom}"));
    }
    near(box_at(Kind::LambertConformal, 11.0), [31.041, 42.924], "zoom 11 keeps its meaning");
}

/// The other projections fit the whole sphere finitely; their default view is what it was (measured on main).
#[test]
fn bounded_projections_keep_their_default_fit() {
    near(box_at(Kind::EqualEarth, 0.0), [21.240, 24.995], "equal-earth");
    near(box_at(Kind::Albers, 0.0), [23.943, 30.607], "albers");
    near(box_at(Kind::Mercator, 0.0), [15.132, 24.018], "mercator");
}

#[test]
fn a_map_with_a_fit_is_not_floored() {
    // the fit target decides the scale; the floor belongs to the whole-sphere default only: with a fit the floor is
    // never above the fitted scale, so removing it changes nothing (the decision rests on this equality)
    let fit = box_20();
    let view = View { lon: 0.0, lat: 50.0, zoom: 0.0, rotation: 0.0 };
    let (map, _) = Map::new(Kind::LambertConformal, None, [480.0, 360.0], &[&fit], 0.0, None);
    let mut without = map.clone();
    without.min_scale = 0.0;
    let bounds = |m: &Map| m.projection(&view).project(&fit).bounds().unwrap();
    assert_eq!(bounds(&map), bounds(&without), "the floor changed a fitted map");
    assert!(map.min_scale <= map.base_scale, "{} above {}", map.min_scale, map.base_scale);
    // the fitted box (measured with the floor code in place): 244.5 x 338.1 px
    let b = bounds(&map);
    near([b[1][0] - b[0][0], b[1][1] - b[0][1]], [244.499, 338.100], "the fitted box");
}
