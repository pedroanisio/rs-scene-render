//! PMTiles and MVT against the reference Python readers (`pmtiles`,
//! `mapbox-vector-tile`): `tools/fixtures/make_tile_expected.py`.

use std::path::PathBuf;

use serde_json::Value;
use sr_geo::mvt::{self, GeomType};
use sr_geo::pmtiles::{self, Archive, TileType};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn expected() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture("tiles.json")).unwrap()).unwrap()
}

#[test]
fn tile_ids_match_the_reference() {
    for v in expected()["tile_ids"].as_array().unwrap() {
        let (z, x, y, id) = (
            v[0].as_u64().unwrap() as u8,
            v[1].as_u64().unwrap() as u32,
            v[2].as_u64().unwrap() as u32,
            v[3].as_u64().unwrap(),
        );
        assert_eq!(pmtiles::tile_id(z, x, y), id, "{z}/{x}/{y}");
        assert_eq!(pmtiles::tile_zxy(id), (z, x, y));
    }
}

#[test]
fn a_protomaps_extract_reads_like_the_reference() {
    let e = expected();
    let a = Archive::open(&fixture("baixa.pmtiles")).unwrap();
    let h = &e["header"];
    assert_eq!(a.header.tile_type, TileType::Mvt);
    assert_eq!(a.header.max_zoom as u64, h["max_zoom"].as_u64().unwrap());
    assert_eq!(a.header.root_length, h["root_length"].as_u64().unwrap());
    assert!((a.header.bounds[0] - h["min_lon_e7"].as_f64().unwrap() / 1e7).abs() < 1e-7);
    assert_eq!(a.metadata["type"], "baselayer");
    for (key, layers) in e["tiles"].as_object().unwrap() {
        let zxy: Vec<u32> = key.split('/').map(|s| s.parse().unwrap()).collect();
        let bytes = a.tile(zxy[0] as u8, zxy[1], zxy[2]).unwrap().expect("tile present");
        let got = mvt::decode(&bytes).unwrap();
        let want = layers.as_object().unwrap();
        assert_eq!(got.len(), want.len(), "{key}");
        for l in &got {
            let w = &want[&l.name];
            assert_eq!(l.extent as u64, w["extent"].as_u64().unwrap());
            assert_eq!(l.features.len() as u64, w["features"].as_u64().unwrap(), "{key} {}", l.name);
            // the reference closes polygon rings by repeating their first point
            let (mut verts, mut sx, mut sy) = (0u64, 0f64, 0f64);
            for f in &l.features {
                for p in &f.parts {
                    let close = if f.kind == GeomType::Polygon { Some(p[0]) } else { None };
                    for q in p.iter().chain(close.iter()) {
                        verts += 1;
                        sx += q[0];
                        sy += q[1];
                    }
                }
            }
            assert_eq!(verts, w["vertices"].as_u64().unwrap(), "{key} {}", l.name);
            assert_eq!((sx, sy), (w["sx"].as_f64().unwrap(), w["sy"].as_f64().unwrap()), "{key} {}", l.name);
            for (f, wf) in l.features.iter().zip(w["first"].as_array().unwrap()) {
                assert_eq!(f.id, wf["id"].as_u64(), "{key} {}", l.name);
                let kind = match (f.kind, f.parts.len(), f.polygons().len()) {
                    (GeomType::Point, 1, _) => "Point",
                    (GeomType::Point, _, _) => "MultiPoint",
                    (GeomType::LineString, 1, _) => "LineString",
                    (GeomType::LineString, _, _) => "MultiLineString",
                    (GeomType::Polygon, _, 1) => "Polygon",
                    _ => "MultiPolygon",
                };
                assert_eq!(kind, wf["type"].as_str().unwrap(), "{key} {}", l.name);
                for (k, v) in wf["properties"].as_object().unwrap() {
                    let g = &f.properties[k];
                    match (g.as_f64(), v.as_f64()) {
                        (Some(a), Some(b)) => assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "{k}: {g} {v}"),
                        _ => assert_eq!(g, v, "{key} {} {k}", l.name),
                    }
                }
            }
        }
    }
}

#[test]
fn raster_archives_from_the_reference_writer() {
    let a = Archive::open(&fixture("raster.pmtiles")).unwrap();
    assert_eq!(a.header.tile_type, TileType::Png);
    for z in 0..3u8 {
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                let png = a.tile(z, x, y).unwrap().unwrap();
                let img = image_rgb(&png);
                assert_eq!(img, [60 * z, 40 * x as u8, 40 * y as u8], "{z}/{x}/{y}");
            }
        }
    }
    assert_eq!(a.tile(3, 0, 0).unwrap(), None);
}

/// The first pixel of a PNG (8-bit RGB, no filter needed for a solid colour: every row repeats).
fn image_rgb(png: &[u8]) -> [u8; 3] {
    // decode IDAT with flate2 through the crate's gzip path is not available; parse minimally
    let mut i = 8;
    let mut idat = Vec::new();
    while i + 8 <= png.len() {
        let n = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        if &png[i + 4..i + 8] == b"IDAT" {
            idat.extend_from_slice(&png[i + 8..i + 8 + n]);
        }
        i += 12 + n;
    }
    let mut raw = Vec::new();
    use std::io::Read;
    flate2::read::ZlibDecoder::new(&idat[..]).read_to_end(&mut raw).unwrap();
    // row 0: filter byte, then RGB (filters Sub/Up/Paeth leave the first pixel as-is)
    [raw[1], raw[2], raw[3]]
}
