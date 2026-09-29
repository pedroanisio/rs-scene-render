//! Maps in 3D: a globe wearing its map, terrain raised from elevation tiles,
//! and buildings extruded from a vector basemap.

mod common;
use common::*;

use std::path::PathBuf;

fn geo_fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-geo/tests/fixtures").join(name).display().to_string()
}

fn geo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geo")
}

fn load(dir: &std::path::Path, xml: &str) -> sr_model::Document {
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.to_path_buf()) };
    sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Whether a pixel is (lit) white rather than blue or background.
fn whiteish(c: [f32; 4]) -> bool {
    c[0] > 0.2 && (c[0] - c[2]).abs() < 0.15 * c[2].max(0.2)
}

#[test]
fn globes_turn_their_map_to_the_camera() {
    // blue ocean, a white square over 50°W‥10°W, 10°S‥30°N; turning −30° about y brings 30°W to
    // face the camera (0° does at rest)
    let xml = |turn: f64| {
        format!(
            r##"<scene version="1.1"><project width="200" height="200" fps="10" duration="1" background="#000000"/>
              <assets><geo id="sq" src="squares.geojson"/>
                <map id="m" width="100" height="100" background="#0000FF"><geoLayer geo="sq" filter="v=100" fill="#FFFFFF"/></map></assets>
              <composition><object3D id="g" primitive="globe" map="m" radius="90" textureSize="512" x="100" y="100" rotationY="{turn}"/></composition>
              <lights><light id="a" type="ambient" intensity="1"/></lights></scene>"##
        )
    };
    let Some(front) = render(&load(&geo_dir(), &xml(-30.0))) else { return };
    let c = front.at(100, 110);
    assert!(whiteish(c), "30°W faces the camera: {c:?}");
    let Some(back) = render(&load(&geo_dir(), &xml(90.0))) else { return };
    let c = back.at(100, 110);
    assert!(c[2] > 2.0 * c[0].max(0.01), "90°E is ocean: {c:?}");
    // outside the sphere: background
    assert_px(&front, 3, 3, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

/// An archive of Terrarium elevation tiles (z10, around 0°, 0°): a 2000-metre hill at 0°, 0°.
fn hill_archive(dir: &std::path::Path) -> PathBuf {
    let mut tiles = Vec::new();
    for x in 510..514u32 {
        for y in 510..514u32 {
            let img = image::RgbImage::from_fn(256, 256, |px, py| {
                let [lon, lat] = sr_geo::tiles::lonlat(
                    10,
                    x as f64 + (px as f64 + 0.5) / 256.0,
                    y as f64 + (py as f64 + 0.5) / 256.0,
                );
                let e = 2000.0 * (-(lon * lon + lat * lat) / (0.05f64 * 0.05)).exp();
                let v = e + 32768.0;
                let r = (v / 256.0).floor();
                let g = (v - r * 256.0).floor();
                let b = ((v - r * 256.0 - g) * 256.0).floor();
                image::Rgb([r as u8, g as u8, b as u8])
            });
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            tiles.push(((10u8, x, y), png));
        }
    }
    let path = dir.join("hill.pmtiles");
    std::fs::write(&path, sr_geo::pmtiles::write(&tiles, sr_geo::pmtiles::TileType::Png, 1, &serde_json::json!({})))
        .unwrap();
    path
}

#[test]
fn terrain_raises_the_ground_toward_the_camera() {
    let dir = std::env::temp_dir().join(format!("sr-maps3d-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    hill_archive(&dir);
    // the map seen edge-on (turned 90° about x): flat ground is a line through the middle; the
    // hill at its centre rises above it
    let xml = |terrain: &str| {
        format!(
            r##"<scene version="1.1"><project width="300" height="200" fps="10" duration="1" background="#000000"/>
              <assets><tiles id="dem" src="hill.pmtiles"/>
                <map id="m" width="240" height="240" projection="web-mercator" centerLon="0" centerLat="0" zoom="12" background="#FFFFFF"/></assets>
              <composition><object3D id="g" primitive="map" map="m" {terrain} exaggeration="3" resolution="96" textureSize="256" x="150" y="100" rotationX="90"/></composition>
              <lights><light id="a" type="ambient" intensity="1"/></lights></scene>"##
        )
    };
    let Some(flat) = render(&load(&dir, &xml(""))) else { return };
    let Some(hill) = render(&load(&dir, &xml(r#"terrain="dem""#))) else { return };
    // above the ground line at the centre
    let lit = |r: &Rendered, y: u32| r.at(150, y)[0] > 0.1;
    let top = |r: &Rendered| (0..200).find(|&y| lit(r, y)).unwrap_or(200);
    let (tf, th) = (top(&flat), top(&hill));
    // 2000 m × 3 at zoom 12 on the equator is about 314 scene units: well above the flat line
    assert!(tf >= 95, "flat ground reaches y = {tf}");
    assert!(th + 40 < tf, "the hill rises to y = {th}, the flat ground to {tf}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn buildings_rise_from_the_basemap() {
    let xml = |b: bool| {
        format!(
            r##"<scene version="1.1"><project width="300" height="200" fps="10" duration="1" background="#000000"/>
              <assets><tiles id="t" src="{}"/>
                <map id="m" width="300" height="300" projection="web-mercator" centerLon="-9.1375" centerLat="38.7110" zoom="16">
                  <basemap tiles="t" labels="false" attribution="false"/></map></assets>
              <materials><material id="stone" baseColor="#FF0000" roughness="1"/></materials>
              <composition><object3D id="g" primitive="map" map="m" buildings="{b}" material="stone" textureSize="256" x="150" y="120" rotationX="90"/></composition>
              <lights><light id="a" type="ambient" intensity="1"/></lights></scene>"##,
            geo_fixture("baixa.pmtiles")
        )
    };
    let Some(flat) = render(&load(&geo_dir(), &xml(false))) else { return };
    let Some(city) = render(&load(&geo_dir(), &xml(true))) else { return };
    let red = |r: &Rendered| {
        (0..200)
            .flat_map(|y| (0..300).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let c = r.at(x, y);
                c[0] > 0.2 && c[1] < 0.1 && c[2] < 0.1
            })
            .count()
    };
    assert_eq!(red(&flat), 0);
    // Baixa's buildings (around 20 m) stand up in the object's material
    assert!(red(&city) > 500, "{} red pixels", red(&city));
}
