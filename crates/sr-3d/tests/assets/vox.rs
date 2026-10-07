//! The reader of MagicaVoxel files, against fixtures that `tools/make_vox.py` writes and computes the answers for with
//! no code of the importer: the cells, the palette and the placed positions of each file, and every way a file can be cut or
//! corrupted. What no real file has proved (the palette offset by one, the centre of a model under the scene graph, the
//! rotation byte, the MATL id) is in the `unproved` list of each expected file, and the proposal says so.

use sr_3d::occupancy::Limits;
use sr_3d::voxel::default_palette::DEFAULT_PALETTE;
use sr_3d::voxel::{scene_cell, vox, Colours};

fn fixture(name: &str) -> (Vec<u8>, serde_json::Value) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vox");
    let bytes = std::fs::read(dir.join(format!("{name}.vox"))).unwrap();
    let expected = serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.expected.json"))).unwrap()).unwrap();
    (bytes, expected)
}

fn numbers<const N: usize>(v: &serde_json::Value) -> [i64; N] {
    let a = v.as_array().unwrap();
    assert_eq!(a.len(), N);
    std::array::from_fn(|i| a[i].as_i64().unwrap())
}

const NAMES: [&str; 7] =
    ["one-model", "one-model-v200", "unknown-chunks", "materials", "two-models", "nested", "rotations"];

/// The cells of an import, as [x, y, z, index] in the order of the scan.
fn cells(o: &sr_3d::occupancy::Occupancy) -> Vec<[i64; 4]> {
    o.cells().map(|c| [i64::from(c[0]), i64::from(c[1]), i64::from(c[2]), i64::from(o.get(c))]).collect()
}

#[test]
fn every_fixture_gives_the_cells_the_palette_and_the_materials_that_its_script_wrote() {
    for name in NAMES {
        let (bytes, expected) = fixture(name);
        let imported = vox::import(&bytes, None, Limits::default(), &vox::Bounds::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let want: Vec<[i64; 4]> = expected["cells"].as_array().unwrap().iter().map(numbers::<4>).collect();
        assert_eq!(cells(&imported.occupancy), want, "{name}: the cells");
        assert_eq!(imported.occupancy.count() as usize, want.len(), "{name}");
        // the colour of palette index c is the file's entry c - 1
        let palette = expected["palette"].as_array().unwrap();
        assert_eq!(imported.colours, Colours::File, "{name}");
        for c in 1..=255usize {
            let entry: [i64; 4] = numbers(&palette[c - 1]);
            let got = imported.occupancy.palette().color(c as u8).map(i64::from);
            assert_eq!(got, entry, "{name}: the colour of index {c}");
        }
        let materials = expected.get("materials").map(|m| m.as_object().unwrap().len()).unwrap_or(0);
        assert_eq!(imported.materials.len(), materials, "{name}");
        if let Some(m) = expected.get("materials") {
            for (index, props) in m.as_object().unwrap() {
                let got = &imported.materials[&index.parse::<u8>().unwrap()];
                for (k, v) in props.as_object().unwrap() {
                    assert_eq!(got.get(k).map(String::as_str), v.as_str(), "{name}: material {index} {k}");
                }
            }
        }
    }
}

#[test]
fn the_models_are_what_the_file_holds_and_a_single_model_is_unplaced() {
    let (bytes, expected) = fixture("two-models");
    let file = vox::parse(&bytes, &vox::Bounds::default()).unwrap();
    let models = expected["models"].as_array().unwrap();
    assert_eq!(file.models.len(), models.len());
    for (got, want) in file.models.iter().zip(models) {
        assert_eq!(got.size.map(i64::from), numbers::<3>(&want["size"]));
        let voxels: Vec<[i64; 4]> = want["voxels"].as_array().unwrap().iter().map(numbers::<4>).collect();
        assert_eq!(got.voxels.iter().map(|v| v.map(i64::from)).collect::<Vec<_>>(), voxels);
    }
    // `model` selects one and puts it with its corner at the origin, not placed by the scene graph
    for (k, want) in models.iter().enumerate() {
        let one = vox::import(&bytes, Some(k), Limits::default(), &vox::Bounds::default()).unwrap();
        let mut expect: Vec<[i64; 4]> = want["voxels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let v = numbers::<4>(v);
                let c = scene_cell([v[0] as i32, v[1] as i32, v[2] as i32]);
                [i64::from(c[0]), i64::from(c[1]), i64::from(c[2]), v[3]]
            })
            .collect();
        expect.sort_by_key(|c| (c[2], c[1], c[0]));
        assert_eq!(cells(&one.occupancy), expect, "model {k}");
    }
    assert!(vox::import(&bytes, Some(2), Limits::default(), &vox::Bounds::default()).unwrap_err().contains("model"));
}

#[test]
fn a_file_with_no_colours_has_the_default_palette_of_the_format_and_says_so() {
    let (bytes, expected) = fixture("no-rgba");
    let imported = vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()).unwrap();
    assert_eq!(imported.colours, Colours::Default);
    for (c, want) in DEFAULT_PALETTE.iter().enumerate().skip(1) {
        assert_eq!(imported.occupancy.palette().color(c as u8), *want, "index {c}");
    }
    let want: Vec<[i64; 4]> = expected["cells"].as_array().unwrap().iter().map(numbers::<4>).collect();
    assert_eq!(cells(&imported.occupancy), want);
}

#[test]
fn the_import_is_the_same_whatever_the_order_and_the_time_it_is_asked() {
    for name in NAMES {
        let (bytes, _) = fixture(name);
        let first = vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()).unwrap();
        for _ in 0..3 {
            let again = vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()).unwrap();
            assert_eq!(first.occupancy.fingerprint(), again.occupancy.fingerprint(), "{name}");
            assert_eq!(first.occupancy.appearance_fingerprint(), again.occupancy.appearance_fingerprint(), "{name}");
            assert_eq!(first.occupancy.revision(), again.occupancy.revision(), "{name}");
        }
    }
}

#[test]
fn every_prefix_of_a_file_and_every_flipped_byte_is_an_error_or_a_model_and_never_a_panic() {
    for name in NAMES {
        let (bytes, _) = fixture(name);
        for length in 0..bytes.len() {
            let result = vox::import(&bytes[..length], None, Limits::default(), &vox::Bounds::default());
            assert!(result.is_err(), "{name}: a file cut at {length} of {} bytes was read", bytes.len());
        }
        // a seeded walk over the bytes, flipping each one in three ways
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for position in 0..bytes.len() {
            for flip in [0xFFu8, 0x01, 0x80] {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let mut copy = bytes.clone();
                copy[position] ^= flip;
                if state >> 62 == 0 {
                    copy[position] = (state >> 24) as u8;
                }
                // the result does not matter, only that the reader comes back
                let _ = vox::import(&copy, None, Limits::default(), &vox::Bounds::default());
            }
        }
    }
}

#[test]
fn what_is_not_a_voxel_file_is_refused_by_name() {
    let (good, _) = fixture("one-model");
    let bad = |bytes: &[u8]| vox::import(bytes, None, Limits::default(), &vox::Bounds::default()).unwrap_err();
    assert!(bad(b"").contains("VOX"));
    let mut wrong = good.clone();
    wrong[..4].copy_from_slice(b"VXO ");
    assert!(bad(&wrong).contains("VOX"));
    let mut version = good.clone();
    version[4..8].copy_from_slice(&99i32.to_le_bytes());
    assert!(bad(&version).contains("version"));
    // a MAIN chunk that says it has more children than the file has bytes
    let mut long = good.clone();
    long[16..20].copy_from_slice(&(good.len() as i32).to_le_bytes());
    assert!(bad(&long).contains("MAIN"));
}

#[test]
fn a_voxel_outside_its_model_an_index_of_zero_a_voxel_twice_and_a_model_without_a_size_are_refused() {
    let chunk = |name: &str, content: &[u8]| {
        let mut out = name.as_bytes().to_vec();
        out.extend((content.len() as i32).to_le_bytes());
        out.extend(0i32.to_le_bytes());
        out.extend(content);
        out
    };
    let file = |children: Vec<u8>| {
        let mut out = b"VOX ".to_vec();
        out.extend(150i32.to_le_bytes());
        out.extend(b"MAIN");
        out.extend(0i32.to_le_bytes());
        out.extend((children.len() as i32).to_le_bytes());
        out.extend(children);
        out
    };
    let size = |x: i32, y: i32, z: i32| chunk("SIZE", &[x.to_le_bytes(), y.to_le_bytes(), z.to_le_bytes()].concat());
    let xyzi = |voxels: &[[u8; 4]]| {
        let mut content = (voxels.len() as i32).to_le_bytes().to_vec();
        for v in voxels {
            content.extend(v);
        }
        chunk("XYZI", &content)
    };
    let read = |bytes: Vec<u8>| vox::import(&bytes, None, Limits::default(), &vox::Bounds::default());
    assert!(read(file([size(2, 2, 2), xyzi(&[[0, 0, 0, 1]])].concat())).is_ok());
    assert!(read(file([size(2, 2, 2), xyzi(&[[2, 0, 0, 1]])].concat())).unwrap_err().contains("outside"));
    assert!(read(file([size(2, 2, 2), xyzi(&[[0, 0, 0, 0]])].concat())).unwrap_err().contains("index"));
    assert!(read(file([size(2, 2, 2), xyzi(&[[1, 1, 1, 3], [1, 1, 1, 4]])].concat())).unwrap_err().contains("twice"));
    assert!(read(file(xyzi(&[[0, 0, 0, 1]]))).unwrap_err().contains("SIZE"));
    // a count of voxels that the chunk does not hold
    let mut lie = xyzi(&[[0, 0, 0, 1]]);
    lie[12..16].copy_from_slice(&1000i32.to_le_bytes());
    assert!(read(file([size(2, 2, 2), lie].concat())).unwrap_err().contains("voxels"));
    // a model larger than the format allows
    assert!(read(file([size(100_000, 2, 2), xyzi(&[[0, 0, 0, 1]])].concat())).is_err());
    // a file with no model at all
    assert!(read(file(Vec::new())).unwrap_err().contains("model"));
}

#[test]
fn the_limits_are_checked_before_anything_is_built_and_say_what_was_over() {
    let (bytes, _) = fixture("rotations");
    // 24 uses of a model of 6 voxels place 144 cells
    let limited = |cells: u64| {
        vox::import(&bytes, None, Limits { max_cells: cells, ..Limits::default() }, &vox::Bounds::default())
    };
    assert!(limited(144).is_ok());
    let error = limited(143).unwrap_err();
    assert!(error.contains("144") && error.contains("cells"), "{error}");
    let bounds = vox::Bounds { max_file_bytes: bytes.len() - 1, ..vox::Bounds::default() };
    let error = vox::import(&bytes, None, Limits::default(), &bounds).unwrap_err();
    assert!(error.contains("bytes"), "{error}");
}
