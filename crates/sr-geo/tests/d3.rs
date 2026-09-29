//! sr-geo against d3-geo 3.1 and topojson-client 3 on the Natural Earth
//! 110m countries (`tools/fixtures/make_geo_expected.mjs`).

use std::path::PathBuf;

use serde_json::Value;
use sr_geo::data::{self, Feature, Format, Geometry};
use sr_geo::project::{Projection, Raw};
use sr_geo::sphere::RAD;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn expected() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture("d3.json")).unwrap()).unwrap()
}

fn countries() -> Vec<Feature> {
    data::load(&fixture("countries-110m.json"), Some(Format::TopoJson), Some("countries")).unwrap()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()).unwrap_or_default()
}

fn build(s: &Value) -> Projection {
    let par = s.get("parallels").map(nums);
    let raw = match s["projection"].as_str().unwrap() {
        "equirectangular" => Raw::Equirectangular,
        "mercator" => Raw::Mercator,
        "equal-earth" => Raw::EqualEarth,
        "natural-earth" => Raw::NaturalEarth,
        "albers" => {
            let p = par.clone().unwrap_or(vec![0.0, 60.0]);
            Raw::conic_equal_area(p[0] * RAD, p[1] * RAD)
        }
        "lambert-conformal" => {
            let p = par.clone().unwrap_or(vec![30.0, 30.0]);
            Raw::conic_conformal(p[0] * RAD, p[1] * RAD)
        }
        "orthographic" => Raw::Orthographic,
        "stereographic" => Raw::Stereographic,
        "azimuthal-equal-area" => Raw::AzimuthalEqualArea,
        "azimuthal-equidistant" => Raw::AzimuthalEquidistant,
        other => panic!("{other}"),
    };
    let mut p = Projection::new(raw);
    // d3's default scales per projection factory
    let default_scale = match s["projection"].as_str().unwrap() {
        "equirectangular" => 152.63,
        "mercator" => 961.0 / std::f64::consts::TAU,
        "equal-earth" => 177.158,
        "natural-earth" => 175.295,
        "albers" => 155.424,
        "lambert-conformal" => 109.5,
        "orthographic" => 249.5,
        "stereographic" => 250.0,
        "azimuthal-equal-area" => 124.75,
        "azimuthal-equidistant" => 79.4188,
        _ => 150.0,
    };
    p.set_scale(s.get("scale").and_then(Value::as_f64).unwrap_or(default_scale));
    if s["projection"] == "albers" && s.get("center").is_none() {
        p.set_center(0.0, 33.6442);
    }
    if let Some(r) = s.get("rotate") {
        let r = nums(r);
        p.set_rotate([r[0], r[1], r[2]]);
    }
    if let Some(c) = s.get("center") {
        let c = nums(c);
        p.set_center(c[0], c[1]);
    }
    if let Some(a) = s.get("angle").and_then(Value::as_f64) {
        p.set_angle(a);
    }
    if let Some(pr) = s.get("precision").and_then(Value::as_f64) {
        p.set_precision(pr);
    }
    if let Some(e) = s.get("extent") {
        let (a, b) = (nums(&e[0]), nums(&e[1]));
        p.set_extent(Some([[a[0], a[1]], [b[0], b[1]]]));
    }
    p
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn topojson_decodes_like_topojson_client() {
    let e = expected();
    let cs = countries();
    assert_eq!(cs.len(), 177);
    for d in e["decoded"].as_array().unwrap() {
        let name = d["name"].as_str().unwrap();
        let f = cs.iter().find(|f| f.text("name").as_deref() == Some(name)).unwrap();
        assert_eq!(f.id.as_deref(), d["id"].as_str(), "{name}");
        let first = match &f.geometry {
            Geometry::Polygons(p) => &p[0][0],
            g => panic!("{name}: {g:?}"),
        };
        let want = d["first"].as_array().unwrap();
        assert_eq!(first.len(), want.len(), "{name}");
        for (a, b) in first.iter().zip(want) {
            let b = nums(b);
            assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{name}: {a:?} {b:?}");
        }
    }
}

#[test]
fn projected_countries_match_d3() {
    let e = expected();
    let cs = countries();
    for case in e["cases"].as_array().unwrap() {
        let s = &case["setup"];
        let p = build(s);
        assert!(close(p.scale(), case["scale"].as_f64().unwrap(), 1e-12), "{s}");
        let sphere = p.project(&Geometry::Sphere);
        let want = case["sphere"]["area"].as_f64().unwrap();
        assert!(close(sphere.area(), want, 1e-6), "{s} sphere {} vs {want}", sphere.area());
        let mut failures = Vec::new();
        for (f, want) in cs.iter().zip(case["countries"].as_array().unwrap()) {
            let name = want["name"].as_str().unwrap();
            let got = p.project(&f.geometry);
            let area = want["area"].as_f64().unwrap();
            let rings: usize = got.polygons.iter().map(Vec::len).sum();
            let want_rings = want["rings"].as_u64().unwrap() as usize;
            if !close(got.area(), area, 1e-6) || rings != want_rings {
                failures.push(format!("{name}: area {} vs {area}, rings {rings} vs {want_rings}", got.area()));
                continue;
            }
            if let Some(b) = got.bounds() {
                let wb = &want["bounds"];
                for (i, j) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let w = wb[i][j].as_f64().unwrap();
                    if !close(b[i][j], w, 1e-6) {
                        failures.push(format!("{name}: bounds[{i}][{j}] {} vs {w}", b[i][j]));
                    }
                }
            }
            if let Some(coords) = want.get("coords").and_then(Value::as_array) {
                let flat: Vec<&Vec<[f64; 2]>> = got.polygons.iter().flatten().collect();
                for (r, (a, b)) in flat.iter().zip(coords).enumerate() {
                    let b = b.as_array().unwrap();
                    if a.len() != b.len() {
                        failures.push(format!("{name}: ring {r} has {} points, d3 {}", a.len(), b.len()));
                        continue;
                    }
                    for (pa, pb) in a.iter().zip(b) {
                        let pb = nums(pb);
                        if (pa[0] - pb[0]).abs() > 1e-6 || (pa[1] - pb[1]).abs() > 1e-6 {
                            failures.push(format!("{name}: ring {r} point {pa:?} vs {pb:?}"));
                            break;
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{s}: {} of 177 differ:\n{}", failures.len(), failures.join("\n"));
        for pt in case["points"].as_array().unwrap() {
            let ll = nums(&pt["ll"]);
            let xy = nums(&pt["xy"]);
            let q = p.point_unclipped(ll[0], ll[1]);
            assert!((q[0] - xy[0]).abs() < 1e-6 && (q[1] - xy[1]).abs() < 1e-6, "{s} {ll:?}: {q:?} vs {xy:?}");
            assert_eq!(p.point(ll[0], ll[1]).is_some(), pt["visible"].as_bool().unwrap(), "{s} {ll:?}");
        }
    }
}

#[test]
fn fits_match_d3() {
    let e = expected();
    let cs = countries();
    for f in e["fits"].as_array().unwrap() {
        let mut p = build(f);
        let brazil = cs.iter().find(|c| c.text("name").as_deref() == Some("Brazil")).unwrap();
        let g = if f["object"] == "sphere" { &Geometry::Sphere } else { &brazil.geometry };
        let (a, b) = (nums(&f["extent"][0]), nums(&f["extent"][1]));
        p.fit_extent([[a[0], a[1]], [b[0], b[1]]], &[g]);
        assert!(close(p.scale(), f["scale"].as_f64().unwrap(), 1e-9), "{f}: {}", p.scale());
        let t = nums(&f["translate"]);
        let q = p.point_unclipped(0.0, 0.0);
        let _ = q;
        let tr = p.translate();
        assert!((tr[0] - t[0]).abs() < 1e-6 && (tr[1] - t[1]).abs() < 1e-6, "{f}: {tr:?}");
    }
}

#[test]
fn fly_to_paths_match_d3_interpolate_zoom() {
    let e = expected();
    for z in e["zooms"].as_array().unwrap() {
        let (a, b) = (nums(&z["a"]), nums(&z["b"]));
        let path = sr_geo::view::ZoomPath::new([a[0], a[1], a[2]], [b[0], b[1], b[2]], z["rho"].as_f64().unwrap());
        let d = z["duration"].as_f64().unwrap() / 1000.0;
        assert!(close(path.duration(), d, 1e-9), "{z}: {}", path.duration());
        for at in z["at"].as_array().unwrap() {
            let t = at[0].as_f64().unwrap();
            let want = nums(&at[1]);
            let got = path.at(t);
            for k in 0..3 {
                assert!(close(got[k], want[k], 1e-9), "{z} t={t}: {got:?} vs {want:?}");
            }
        }
    }
}
