use sr_eval::{
    geo::DemEncoding,
    terrain::{Dem, Missing},
};
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "sr-terrain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn archive(&self, tiles: Vec<((u8, u32, u32), image::RgbImage)>) -> std::path::PathBuf {
        let tiles: Vec<_> = tiles
            .into_iter()
            .map(|(key, img)| {
                let mut b = std::io::Cursor::new(Vec::new());
                img.write_to(&mut b, image::ImageFormat::Png).unwrap();
                (key, b.into_inner())
            })
            .collect();
        let p = self.0.join("dem.pmtiles");
        std::fs::write(&p, sr_geo::pmtiles::write(&tiles, sr_geo::pmtiles::TileType::Png, 1, &serde_json::json!({})))
            .unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn dem_encodings_use_metres_and_polar_caps_fade_to_the_datum() {
    for (enc, rgb) in [(DemEncoding::Terrarium, [128, 100, 0]), (DemEncoding::Mapbox, [1, 138, 136])] {
        let f = Fixture::new();
        let path = f.archive(vec![((0, 0, 0), image::RgbImage::from_pixel(2, 2, image::Rgb(rgb)))]);
        let mut dem = Dem::open(&path, 0, enc, 2, 1 << 20).unwrap();
        assert!((dem.sample(0., 0., Missing::Error).unwrap() - 100.).abs() < 1e-9);
        assert_eq!(dem.sample(0., 90., Missing::Error).unwrap(), 0.);
        let cap = dem.sample(0., 88., Missing::Error).unwrap();
        assert!(cap > 0. && cap < 100.);
    }
}

#[test]
fn bilinear_elevation_crosses_tile_edges_and_wraps_the_date_line() {
    let f = Fixture::new();
    let mut tiles = Vec::new();
    for x in 0..2 {
        for y in 0..2 {
            tiles.push(((1, x, y), image::RgbImage::from_pixel(2, 2, image::Rgb([128, (x * 100) as u8, 0]))));
        }
    }
    let path = f.archive(tiles);
    let mut dem = Dem::open(&path, 1, DemEncoding::Terrarium, 2, 1 << 20).unwrap();
    assert!((dem.sample(0., 0., Missing::Error).unwrap() - 50.).abs() < 1e-9);
    assert_eq!(dem.sample(-180., 0., Missing::Error).unwrap(), dem.sample(180., 0., Missing::Error).unwrap());
    assert!(
        (dem.sample(179.999, 0., Missing::Error).unwrap() - dem.sample(-179.999, 0., Missing::Error).unwrap()).abs()
            < 0.01
    );
    assert!(dem.sample(f64::INFINITY, 0., Missing::Error).is_err());
}

#[test]
fn missing_tiles_and_decoded_memory_limits_are_explicit() {
    let f = Fixture::new();
    let path = f.archive(vec![((1, 0, 0), image::RgbImage::from_pixel(2, 2, image::Rgb([128, 0, 0])))]);
    let mut dem = Dem::open(&path, 1, DemEncoding::Terrarium, 2, 1 << 20).unwrap();
    assert!(dem.sample(90., -60., Missing::Error).unwrap_err().contains("missing"));
    assert_eq!(dem.sample(90., -60., Missing::Zero).unwrap(), 0.);
    let large = Fixture::new();
    let path = large.archive(vec![((0, 0, 0), image::RgbImage::from_pixel(1024, 1024, image::Rgb([128, 0, 0])))]);
    let mut dem = Dem::open(&path, 0, DemEncoding::Terrarium, 1024, 1 << 20).unwrap();
    assert!(dem.sample(0., 0., Missing::Error).unwrap_err().contains("budget"));
    let mut mismatch = Dem::open(&path, 0, DemEncoding::Terrarium, 256, 64 << 20).unwrap();
    assert!(mismatch.sample(0., 0., Missing::Error).unwrap_err().contains("dimensions"));
}

#[test]
fn terrain_tile_formats_are_checked_before_numeric_interpretation() {
    let f = Fixture::new();
    let rgb = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([128, 100, 0])));
    for (img, format, declared, error) in [
        (rgb.clone(), image::ImageFormat::WebP, sr_geo::pmtiles::TileType::Webp, None),
        (rgb, image::ImageFormat::WebP, sr_geo::pmtiles::TileType::Png, Some("disagrees")),
        (
            image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(2, 2, image::Luma([128]))),
            image::ImageFormat::Png,
            sr_geo::pmtiles::TileType::Png,
            Some("RGB8"),
        ),
        (
            image::DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(2, 2, image::Rgb([32768u16, 100, 0]))),
            image::ImageFormat::Png,
            sr_geo::pmtiles::TileType::Png,
            Some("RGB8"),
        ),
    ] {
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, format).unwrap();
        let path = f.0.join("formats.pmtiles");
        std::fs::write(
            &path,
            sr_geo::pmtiles::write(&[((0, 0, 0), bytes.into_inner())], declared, 1, &serde_json::json!({})),
        )
        .unwrap();
        let mut dem = Dem::open(&path, 0, DemEncoding::Terrarium, 2, 1 << 20).unwrap();
        let result = dem.sample(0., 0., Missing::Error);
        match error {
            Some(error) => assert!(result.unwrap_err().contains(error)),
            None => assert_eq!(result.unwrap(), 100.),
        }
    }
}

#[test]
fn scene_globe_geometry_uses_explicit_planet_scale_and_animated_exaggeration() {
    let f = Fixture::new();
    f.archive(vec![((0, 0, 0), image::RgbImage::from_pixel(2, 2, image::Rgb([128, 100, 0])))]);
    let xml = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/><assets><tiles id="dem" src="dem.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" planetRadius="1000" radius="10" segments="32"><animate property="exaggeration"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D></composition></scene>"##;
    let d =
        sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(f.0.clone()) }).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut surfaces = Vec::new();
    for t in [0., 1., 0.] {
        let frame = ev.evaluate(t);
        surfaces.push(sr_eval::terrain::globe(ev.program(), &frame.nodes[0]).unwrap());
    }
    for (i, radius) in [11., 12., 11.].into_iter().enumerate() {
        let v = surfaces[i].vertices.iter().find(|v| v.uv == [0.5, 0.5]).unwrap();
        assert!((v.pos[2] + radius).abs() < 1e-6);
    }
    assert!(std::sync::Arc::ptr_eq(&surfaces[0], &surfaces[2]));
    let points: Vec<_> = surfaces[0].vertices.iter().map(|v| v.pos.map(f64::from)).collect();
    // The exact render surface must also be a valid closed volumetric collider.
    assert!(sr_sim::pyro::mesh::Mesh::new(&points, surfaces[0].indices.as_chunks::<3>().0, 64 << 20).is_ok());
}

#[test]
fn included_globe_relief_uses_its_scoped_asset_and_document_directory() {
    let main = Fixture::new();
    let library = Fixture(main.0.join("library"));
    std::fs::create_dir(&library.0).unwrap();
    library.archive(vec![((0, 0, 0), image::RgbImage::from_pixel(2, 2, image::Rgb([128, 100, 0])))]);
    let project = r#"<project width="64" height="64" fps="10" duration="1"/>"#;
    std::fs::write(library.0.join("earth.xml"), format!(r##"<scene version="1.3">{project}<assets><tiles id="dem" src="dem.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><symbols><symbol id="world" width="64" height="64"><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" planetRadius="1000" radius="10" segments="32"/></symbol></symbols><composition/></scene>"##)).unwrap();
    let xml = format!(
        r#"<scene version="1.3">{project}<assets><tiles id="dem" src="deliberately-missing.pmtiles"/></assets><composition><include id="inc" src="library/earth.xml" symbol="world"/></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(main.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let node = frame.nodes.iter().find(|n| &*n.id == "inc/earth").unwrap();
    let surface = sr_eval::terrain::globe(ev.program(), node).unwrap();
    let v = surface.vertices.iter().find(|v| v.uv == [0.5, 0.5]).unwrap();
    assert!((v.pos[2] + 11.).abs() < 1e-6);
}

#[test]
fn globe_relief_is_shared_by_rigid_particle_and_pyro_colliders() {
    let f = Fixture::new();
    f.archive(vec![((0, 0, 0), image::RgbImage::from_pixel(2, 2, image::Rgb([128, 250, 0])))]);
    let xml = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/><assets><tiles id="dem" src="dem.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" terrainZoom="0" radius="2" planetRadius="1000" segments="32"><rigidBody type="static"/></object3D><object3D id="ball" primitive="sphere" radius="0.1" z="-3"><rigidBody velocityZ="2" linearDamping="0" restitution="0"/></object3D><particles3D id="dust" z="-3" rate="0" velocityZ="2" dt="0.02" lifetime="2" collisionRadius="0.05" bounce="0" colliders="earth"><burst time="0" count="1"/></particles3D><object3D id="smoke" primitive="volume"><pyro width="6" height="6" depth="6" voxelSize="0.5" dt="0.1" boundary="open" colliders="earth"><pyroSource shape="box" width="8" height="8" depth="8" densityRate="10"/></pyro></object3D></composition><physics gravityY="0" bounds="none" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##;
    let d =
        sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(f.0.clone()) }).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let early = ev.evaluate(0.1);
    assert!(early.problems.is_empty(), "{:?}", early.problems);
    let smoke = early.nodes.iter().find(|n| &*n.id == "smoke").unwrap().sim_volume.as_ref().unwrap();
    let density = smoke.data.grid("density").unwrap();
    assert_eq!(density.sample_world([0.25, 0.25, -2.25]), 0., "relief shell must carve smoke");
    assert!(density.sample_world([0.25, 0.25, -2.75]) > 0.5);
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let ball = frame.nodes.iter().find(|n| &*n.id == "ball").unwrap();
    assert!(ball.pose3.unwrap()[14] < -2.45, "rigid collision ignored relief: {:?}", ball.pose3);
    let dust = frame.nodes.iter().find(|n| &*n.id == "dust").unwrap().particles3d.as_ref().unwrap();
    assert!(dust.frame.particles[0].position[2] < -2.45, "particle collision ignored relief");
}
