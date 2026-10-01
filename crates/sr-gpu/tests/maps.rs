//! `<map>` assets: projection, choropleths, feature styles, globes, fly-to
//! moves, routes and every geo input format, checked at pixels whose
//! geography is known.

mod common;
use common::*;

use std::path::PathBuf;

fn geo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/geo")
}

fn world() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-geo/tests/fixtures/countries-110m.json").display().to_string()
}

/// A document of `w`×`h` with `assets` whose composition shows the map asset `m` unscaled.
fn map_doc(w: u32, h: u32, assets: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="{w}" height="{h}" fps="10" duration="6" background="#000000"/>
           <assets>{assets}</assets><composition><layer id="l" asset="m"/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(geo_dir()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rgb(hex: u32) -> [f32; 4] {
    [lin8((hex >> 16) as u8), lin8((hex >> 8) as u8), lin8(hex as u8), 1.0]
}

/// Pixel of longitude/latitude on a 360×180 equirectangular world (one pixel per degree).
fn eq(lon: f64, lat: f64) -> (u32, u32) {
    ((lon + 180.0) as u32, (90.0 - lat) as u32)
}

#[test]
fn equirectangular_world_places_land_and_sea() {
    let d = map_doc(
        360,
        180,
        &format!(
            r##"<geo id="w" src="{}" object="countries"/>
                <map id="m" width="360" height="180" projection="equirectangular" background="#0000FF">
                  <geoLayer geo="w" fill="#FFFFFF"/></map>"##,
            world()
        ),
    );
    let Some(r) = render(&d) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    // DR Congo, Algeria, Brazil, Australia (inside, away from borders, which neighbouring
    // countries share without a seam); the Atlantic, the Pacific, the Indian Ocean.
    for (lon, lat) in [(23.0, -2.0), (20.0, 5.0), (3.0, 28.0), (-50.0, -10.0), (135.0, -25.0)] {
        let (x, y) = eq(lon, lat);
        assert_px(&r, x, y, rgb(0xFFFFFF), 1e-3);
    }
    for (lon, lat) in [(-30.0, 0.0), (-140.0, 10.0), (80.0, -20.0)] {
        let (x, y) = eq(lon, lat);
        assert_px(&r, x, y, rgb(0x0000FF), 1e-3);
    }
}

#[test]
fn choropleths_map_values_to_the_palette() {
    let assets = |scale: &str| {
        format!(
            r##"<geo id="sq" src="squares.geojson"/>
                <map id="m" width="360" height="180" projection="equirectangular">
                  <geoLayer geo="sq" fillBy="v" palette="#000000 #FFFFFF" scale="{scale}" noData="#FF0000"/></map>"##
        )
    };
    let Some(r) = render(&map_doc(360, 180, &assets("linear"))) else { return };
    let at = |r: &Rendered, lon: f64| {
        let (x, y) = eq(lon, 10.0);
        r.at(x, y)
    };
    assert_px(&r, eq(-150.0, 10.0).0, eq(-150.0, 10.0).1, rgb(0x000000), 1e-3);
    // 50 of 0‥100: halfway in sRGB, like d3's interpolateRgb.
    let mid = at(&r, -90.0);
    assert!((mid[0] - srgb_to_linear(0.5)).abs() < 2e-3, "{mid:?}");
    assert_px(&r, eq(-30.0, 10.0).0, eq(-30.0, 10.0).1, rgb(0xFFFFFF), 1e-3);
    assert_px(&r, eq(30.0, 10.0).0, eq(30.0, 10.0).1, rgb(0xFF0000), 1e-3);
    // quantize: two colours, 50 falls in the upper half
    let Some(q) = render(&map_doc(360, 180, &assets("quantize"))) else { return };
    assert_px(&q, eq(-90.0, 10.0).0, eq(-90.0, 10.0).1, rgb(0xFFFFFF), 1e-3);
}

#[test]
fn feature_styles_animate_single_features() {
    let d = map_doc(
        360,
        180,
        r##"<geo id="sq" src="squares.geojson"/>
            <map id="m" width="360" height="180" projection="equirectangular">
              <geoLayer geo="sq" fill="#808080">
                <featureStyle key="b" fill="#FF0000">
                  <animate property="fill"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate>
                </featureStyle>
              </geoLayer></map>"##,
    );
    let (bx, by) = eq(-90.0, 10.0);
    let (ax, ay) = eq(-150.0, 10.0);
    let Some(r0) = render_times(&d, &[0.0]) else { return };
    assert_px(&r0, bx, by, rgb(0xFF0000), 1e-3);
    assert_px(&r0, ax, ay, rgb(0x808080), 1e-3);
    let Some(r1) = render_times(&d, &[1.0]) else { return };
    assert_px(&r1, bx, by, rgb(0x0000FF), 1e-3);
}

#[test]
fn globes_hide_the_far_side_and_fly_to_their_targets() {
    let globe = |center: f64, extra: &str| {
        format!(
            r##"<map id="m" width="200" height="200" projection="orthographic" centerLon="{center}" centerLat="0">
                  <pin lon="0" lat="0" radius="5" fill="#00FF00" strokeWidth="0"/>{extra}</map>"##
        )
    };
    let Some(front) = render(&map_doc(200, 200, &globe(0.0, ""))) else { return };
    assert_px(&front, 100, 100, rgb(0x00FF00), 1e-3);
    let Some(back) = render(&map_doc(200, 200, &globe(180.0, ""))) else { return };
    assert_px(&back, 100, 100, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // A move from the far side to (0, 0) ends with the pin in the middle.
    let d = map_doc(200, 200, &globe(180.0, r#"<flyTo begin="0.5" duration="2" lon="0" lat="0" zoom="1"/>"#));
    let Some(before) = render_times(&d, &[0.2]) else { return };
    assert_px(&before, 100, 100, [0.0, 0.0, 0.0, 1.0], 1e-3);
    let Some(after) = render_times(&d, &[3.0]) else { return };
    assert_px(&after, 100, 100, rgb(0x00FF00), 1e-3);
}

#[test]
fn routes_advance_by_ground_distance() {
    let d = map_doc(
        360,
        180,
        r##"<map id="m" width="360" height="180" projection="equirectangular">
              <route points="0,0 90,0" progress="0.5" stroke="#FFFFFF" strokeWidth="3" headRadius="4" headFill="#FF0000"/></map>"##,
    );
    let Some(r) = render(&d) else { return };
    let (x, y) = eq(20.0, 0.0);
    assert_px(&r, x, y, rgb(0xFFFFFF), 1e-3);
    let (x, y) = eq(45.0, 0.0);
    assert_px(&r, x, y, rgb(0xFF0000), 1e-3);
    let (x, y) = eq(70.0, 0.0);
    assert_px(&r, x, y, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn kml_and_gpx_sources_draw() {
    let d = map_doc(
        360,
        180,
        r##"<geo id="k" src="place.kml"/><geo id="g" src="track.gpx"/>
            <map id="m" width="360" height="180" projection="equirectangular">
              <geoLayer geo="k" fill="#00FF00"/>
              <route geo="g" stroke="#FF00FF" strokeWidth="3"/></map>"##,
    );
    let Some(r) = render(&d) else { return };
    let (x, y) = eq(80.0, 0.0);
    assert_px(&r, x, y, rgb(0x00FF00), 1e-3);
    let (x, y) = eq(-149.0, -50.0);
    assert_px(&r, x, y, rgb(0xFF00FF), 1e-3);
}

#[test]
fn web_mercator_uses_tile_zoom_levels() {
    // Zoom 0: the world is 512 pixels square, clipped at ±85.05° like map tiles.
    let d = map_doc(
        512,
        512,
        &format!(
            r##"<geo id="w" src="{}" object="countries"/>
                <map id="m" width="512" height="512" projection="web-mercator" centerLon="0" centerLat="0" background="#0000FF">
                  <geoLayer geo="w" fill="#FFFFFF"/></map>"##,
            world()
        ),
    );
    let Some(r) = render(&d) else { return };
    // The square world fills the frame: its corners are ocean (Arctic, Southern Ocean edges
    // at ±85°), its bottom row is Antarctica, and 0°, 0° (Gulf of Guinea) is in the middle.
    assert_px(&r, 2, 2, rgb(0x0000FF), 1e-3);
    assert_px(&r, 256, 510, rgb(0xFFFFFF), 1e-3);
    assert_px(&r, 256, 256, rgb(0x0000FF), 1e-3);
    // Central Algeria (3°E, 28°N): x = 256 + 512·3/360, y = 256 − 512/2π·ln tan(45° + 14°).
    let x: f64 = 256.0 + 512.0 * 3.0 / 360.0;
    let y = 256.0 - 512.0 / std::f64::consts::TAU * (std::f64::consts::FRAC_PI_4 + 28f64.to_radians() / 2.0).tan().ln();
    assert_px(&r, x.round() as u32, y.round() as u32, rgb(0xFFFFFF), 1e-3);
}

#[test]
fn bad_geo_references_fail_validation() {
    let xml = r#"<scene version="1.2"><project width="64" height="32" fps="10" duration="1"/>
        <assets><map id="m" width="64" height="32"><geoLayer geo="m"/></map></assets>
        <composition><layer id="l" asset="m"/></composition></scene>"#;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(geo_dir()) };
    let err = format!("{:?}", sr_model::load_str(xml, &opts).unwrap_err());
    assert!(err.contains("R36"), "{err}");
}

#[test]
fn expressions_place_layers_on_the_map() {
    // A globe spinning a quarter turn east over two seconds; the marker layer follows
    // 0°N 0°E with geo() and fades out with geoVisible() once it turns away.
    let xml = r##"<scene version="1.2"><project width="200" height="200" fps="10" duration="4" background="#000000"/>
        <assets><map id="m" width="200" height="200" projection="orthographic" centerLon="0" centerLat="0">
          <animate property="centerLon"><key time="0" value="0"/><key time="2" value="180"/></animate></map>
          <image id="dot" src="red.png" width="4" height="4"/></assets>
        <composition><layer id="map" asset="m"/>
          <layer id="mark" asset="dot" anchorX="0.5" anchorY="0.5">
            <expression property="x">geo('m', 0, 0)[0]</expression>
            <expression property="y">geo('m', 0, 0)[1]</expression>
            <expression property="opacity">geoVisible('m', 0, 0)</expression>
          </layer></composition></scene>"##;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let d = sr_model::load_str(xml, &opts).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mark = |t: f64| {
        let g = ev.evaluate(t);
        let n = g.nodes.iter().find(|n| &*n.id == "mark").unwrap().clone();
        let num = |k: &str| n.props.get(k).and_then(sr_eval::Value::as_num).unwrap();
        (num("x"), num("y"), num("opacity"))
    };
    let (x, y, o) = mark(0.0);
    assert!((x - 100.0).abs() < 1e-6 && (y - 100.0).abs() < 1e-6 && o == 1.0, "{x} {y} {o}");
    // At t = 1 the centre is 90°E: 0°E sits on the western limb, still visible (just).
    let (x, _, _) = mark(0.5);
    let r = 100.0; // zoom 0 fits the globe: radius = half the frame
    assert!((x - (100.0 - r * 45f64.to_radians().sin())).abs() < 1e-3, "{x}");
    let (_, _, o) = mark(1.5);
    assert_eq!(o, 0.0, "the far side is hidden");
}

fn geo_fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-geo/tests/fixtures").join(name).display().to_string()
}

/// Straight sRGB 0‥1 → the renderer's linear values.
fn lin(c: [f64; 4]) -> [f32; 4] {
    [srgb_to_linear(c[0] as f32), srgb_to_linear(c[1] as f32), srgb_to_linear(c[2] as f32), 1.0]
}

#[test]
fn vector_basemaps_draw_their_style() {
    // Praça do Comércio and the Tagus at zoom 15.5 (z15 tiles, overzoomed from the archive's z14)
    let doc = |labels: &str| {
        map_doc(
            400,
            300,
            &format!(
                r##"<tiles id="t" src="{}"/>
                    <map id="m" width="400" height="300" projection="web-mercator" centerLon="-9.1365" centerLat="38.7060" zoom="15.5">
                      <basemap tiles="t" labels="{labels}" attribution="false"/></map>"##,
                geo_fixture("baixa.pmtiles")
            ),
        )
    };
    let Some(r) = render(&doc("false")) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    // the river, south of the square: the style's water colour at this zoom
    let style = sr_geo::style::Style::parse(sr_geo::style::builtin("protomaps-light").unwrap()).unwrap();
    let water = style.layers.iter().find(|l| l.id == "water").unwrap();
    let props = serde_json::Map::new();
    let cx = sr_geo::style::Ctx { zoom: 15.5, properties: &props, geometry: "Polygon", id: None };
    let c = water.paint("fill-color", &cx).color().unwrap();
    let [x, y] = {
        let (m, _) = sr_geo::view::Map::new(
            sr_geo::view::Kind::WebMercator,
            None,
            [400.0, 300.0],
            &[],
            0.0,
            Some([-9.1365, 38.706]),
        );
        let p = m.projection(&sr_geo::view::View { lon: -9.1365, lat: 38.706, zoom: 15.5, rotation: 0.0 });
        p.point(-9.1365, 38.7045).unwrap()
    };
    assert_px(&r, x as u32, y as u32, lin(c), 2e-3);
    // labels add text
    let Some(l) = render(&doc("true")) else { return };
    let differ = (0..300)
        .flat_map(|y| (0..400).map(move |x| (x, y)))
        .filter(|&(x, y)| !close(r.at(x, y), l.at(x, y), 1e-3))
        .count();
    assert!(differ > 200, "labels changed only {differ} pixels");
}

#[test]
fn raster_basemaps_warp_into_the_projection() {
    // z0-2 tiles of solid colours (60·z, 40·x, 40·y); a 512-pixel Web Mercator world at zoom 0
    // shows 256-pixel tiles of z1
    let rgb8 = |r: u8, g: u8, b: u8| [lin8(r), lin8(g), lin8(b), 1.0];
    let d = map_doc(
        512,
        512,
        &format!(
            r##"<tiles id="t" src="{}" attribution="Test tiles"/>
                <map id="m" width="512" height="512" projection="web-mercator" centerLon="0" centerLat="0">
                  <basemap tiles="t"/></map>"##,
            geo_fixture("raster.pmtiles")
        ),
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 128, 128, rgb8(60, 0, 0), 1e-3);
    assert_px(&r, 384, 128, rgb8(60, 40, 0), 1e-3);
    assert_px(&r, 128, 384, rgb8(60, 0, 40), 1e-3);
    assert_px(&r, 300, 300, rgb8(60, 40, 40), 1e-3);
    // the credit plate in the corner
    assert!(!close(r.at(505, 505), rgb8(60, 40, 40), 1e-2), "{:?}", r.at(505, 505));
    // on a globe: tiles on the near side, nothing beyond the limb
    let d = map_doc(
        256,
        256,
        &format!(
            r##"<tiles id="t" src="{}"/>
                <map id="m" width="256" height="256" projection="orthographic" centerLon="20" centerLat="20">
                  <basemap tiles="t" attribution="false"/></map>"##,
            geo_fixture("raster.pmtiles")
        ),
    );
    let Some(g) = render(&d) else { return };
    // a 256-pixel globe spans 2π·128 ≈ 804 pixels of world: z2 tiles; its centre (20°E, 20°N) is in
    // tile (2, 1)
    assert_px(&g, 128, 128, rgb8(120, 80, 40), 1e-3);
    assert_px(&g, 2, 2, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn maps_stop_at_their_frame() {
    // a 200 × 200 map of a 512-pixel world (zoom 0: 256-pixel z1 tiles reach well past it) in a
    // 400 × 200 frame: nothing may show right of x = 200
    let d = map_doc(
        400,
        200,
        &format!(
            r##"<tiles id="t" src="{}"/><geo id="w" src="{}" object="countries"/>
                <map id="m" width="200" height="200" projection="web-mercator" centerLon="30" centerLat="10">
                  <basemap tiles="t" attribution="false"/><geoLayer geo="w" fill="#FFFFFF" stroke="#FF0000" strokeWidth="8"/></map>"##,
            geo_fixture("raster.pmtiles"),
            world()
        ),
    );
    let Some(r) = render(&d) else { return };
    for (x, y) in [(205, 100), (260, 20), (390, 190)] {
        assert_px(&r, x, y, [0.0, 0.0, 0.0, 1.0], 1e-3);
    }
    assert!(!close(r.at(195, 100), [0.0, 0.0, 0.0, 1.0], 1e-3));
}

#[test]
fn a_basemap_asked_for_more_tiles_than_a_frame_can_use_is_an_error() {
    // the whole world twenty zoom levels finer than the view: 4^20 tiles
    let doc = map_doc(
        400,
        300,
        &format!(
            r##"<tiles id="t" src="{}"/>
                <map id="m" width="400" height="300" projection="web-mercator" centerLon="0" centerLat="0" zoom="1">
                  <basemap tiles="t" detail="20" attribution="false"/></map>"##,
            geo_fixture("baixa.pmtiles")
        ),
    );
    let Some(r) = render(&doc) else { return };
    assert!(r.stats.errors.iter().any(|e| e.contains("tiles")), "{:?}", r.stats.errors);
}
