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
    // a model larger than the format allows: the coordinates of a cell are bytes, so 256 to a side is the most
    assert!(read(file([size(256, 2, 2), xyzi(&[[255, 0, 0, 1]])].concat())).is_ok());
    assert!(read(file([size(257, 2, 2), xyzi(&[[0, 0, 0, 1]])].concat())).unwrap_err().contains("256"));
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

/// Bytes of a `.vox` file written by hand, chunk by chunk, so that a test says exactly what is in the file.
mod raw {
    pub fn chunk(name: &str, content: &[u8]) -> Vec<u8> {
        let mut out = name.as_bytes().to_vec();
        out.extend((content.len() as i32).to_le_bytes());
        out.extend(0i32.to_le_bytes());
        out.extend(content);
        out
    }
    fn string(s: &str) -> Vec<u8> {
        [(s.len() as i32).to_le_bytes().to_vec(), s.as_bytes().to_vec()].concat()
    }
    pub fn dictionary(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut out = (pairs.len() as i32).to_le_bytes().to_vec();
        for (k, v) in pairs {
            out.extend(string(k));
            out.extend(string(v));
        }
        out
    }
    /// A model of the size `size` with the cells `[x, y, z, index]`.
    pub fn model(size: [i32; 3], cells: &[[u8; 4]]) -> Vec<u8> {
        let mut xyzi = (cells.len() as i32).to_le_bytes().to_vec();
        for c in cells {
            xyzi.extend(c);
        }
        [
            chunk("SIZE", &[size[0].to_le_bytes(), size[1].to_le_bytes(), size[2].to_le_bytes()].concat()),
            chunk("XYZI", &xyzi),
        ]
        .concat()
    }
    /// An `nTRN` node: its child, and a translation and a rotation byte as the strings the file holds.
    pub fn transform(node: i32, child: i32, translation: Option<&str>, rotation: Option<&str>) -> Vec<u8> {
        let mut content = node.to_le_bytes().to_vec();
        content.extend(dictionary(&[]));
        content.extend(child.to_le_bytes());
        content.extend((-1i32).to_le_bytes());
        content.extend((-1i32).to_le_bytes());
        content.extend(1i32.to_le_bytes());
        let mut pairs = Vec::new();
        if let Some(r) = rotation {
            pairs.push(("_r", r));
        }
        if let Some(t) = translation {
            pairs.push(("_t", t));
        }
        content.extend(dictionary(&pairs));
        chunk("nTRN", &content)
    }
    pub fn group(node: i32, children: &[i32]) -> Vec<u8> {
        let mut content = node.to_le_bytes().to_vec();
        content.extend(dictionary(&[]));
        content.extend((children.len() as i32).to_le_bytes());
        for c in children {
            content.extend(c.to_le_bytes());
        }
        chunk("nGRP", &content)
    }
    pub fn shape(node: i32, models: &[i32]) -> Vec<u8> {
        let mut content = node.to_le_bytes().to_vec();
        content.extend(dictionary(&[]));
        content.extend((models.len() as i32).to_le_bytes());
        for m in models {
            content.extend(m.to_le_bytes());
            content.extend(dictionary(&[]));
        }
        chunk("nSHP", &content)
    }
    pub fn file(children: Vec<u8>) -> Vec<u8> {
        let mut out = b"VOX ".to_vec();
        out.extend(150i32.to_le_bytes());
        out.extend(b"MAIN");
        out.extend(0i32.to_le_bytes());
        out.extend((children.len() as i32).to_le_bytes());
        out.extend(children);
        out
    }
}

fn read_raw(children: Vec<u8>) -> Result<sr_3d::voxel::Imported, String> {
    vox::import(&raw::file(children), None, Limits::default(), &vox::Bounds::default())
}

#[test]
fn translations_that_add_up_past_the_keys_of_an_occupancy_are_an_error_and_never_a_wrapped_cell() {
    let read = |first: &str, second: &str| {
        read_raw(
            [
                raw::model([2, 2, 2], &[[0, 0, 0, 1]]),
                raw::transform(0, 1, Some(first), None),
                raw::transform(1, 2, Some(second), None),
                raw::shape(2, &[0]),
            ]
            .concat(),
        )
    };
    // in reach: the cell of the model (0, 0, 0) is the cell -1 about its pivot (1, 1, 1), moved by 3 + 4 = 7 in x
    let moved = read("3 0 0", "4 0 0").unwrap();
    assert_eq!(moved.occupancy.count(), 1);
    // MagicaVoxel's cell (6, -1, -1) is the scene's (6, 0, -1)
    assert_eq!(moved.occupancy.get([6, 0, -1]), 1);
    // every one of these adds up, in 64 bits, to a number that wraps or to one far outside the keys
    for (first, second) in [
        ("9223372036854775807 0 0", "9223372036854775807 0 0"),
        ("-9223372036854775808 0 0", "-9223372036854775808 0 0"),
        ("4611686018427387904 0 0", "4611686018427387904 0 0"),
        ("1073741824 0 0", "0 0 0"),
        ("0 0 0", "0 -2147483649 0"),
    ] {
        let error = read(first, second).err().unwrap_or_else(|| format!("{first} / {second} was read"));
        assert!(error.contains("translation") || error.contains("far"), "{first} / {second}: {error}");
    }
}

#[test]
fn the_empty_palette_index_has_no_colour_even_when_every_entry_of_the_file_has_one() {
    // every entry of the file is [9, 9, 9, 9]: with the offset by one the index 0 has no entry and stays empty, and a reader that put
    // the file's entry c at the index c would give the index 0 a colour (the per-index check of the fixtures is in the first test)
    let colours = raw::chunk("RGBA", &[9u8; 256 * 4]);
    let imported = read_raw([raw::model([2, 2, 2], &[[0, 0, 0, 1]]), colours].concat()).unwrap();
    assert_eq!(imported.colours, Colours::File);
    assert_eq!(imported.occupancy.palette().color(0), [0, 0, 0, 0]);
    for c in 1..=255u8 {
        assert_eq!(imported.occupancy.palette().color(c), [9, 9, 9, 9], "index {c}");
    }
}

#[test]
fn the_default_palette_is_the_table_of_the_description_of_the_format() {
    // The literals are the entries of `default_palette` of section 8 of MagicaVoxel-file-format-vox.txt, as written there (0xAABBGGRR),
    // read by hand from the text and not from the table that is tested.
    for (index, abgr) in [
        (0usize, 0x0000_0000u32),
        (1, 0xffff_ffff),
        (2, 0xffcc_ffff),
        (37, 0xffff_ffcc),
        (100, 0xff66_3399),
        (150, 0xff00_ff33),
        (216, 0xff00_00ee),
        (255, 0xff11_1111),
    ] {
        let [r, g, b, a] = abgr.to_le_bytes();
        assert_eq!(DEFAULT_PALETTE[index], [r, g, b, a], "index {index}");
    }
}

/// `levels` pairs of nodes, each an `nTRN` whose child is an `nGRP` that names the next pair twice, and a shape at the bottom: `2^levels`
/// places of one model, in a file of a few hundred bytes.
fn doubling(levels: i32) -> Vec<u8> {
    let mut children = raw::model([2, 2, 2], &[[0, 0, 0, 1]]);
    for k in 0..levels {
        children.extend(raw::transform(2 * k, 2 * k + 1, Some("1 0 0"), None));
        children.extend(raw::group(2 * k + 1, &[2 * k + 2, 2 * k + 2]));
    }
    children.extend(raw::shape(2 * levels, &[0]));
    children
}

#[test]
fn a_node_that_two_parents_share_is_counted_before_it_is_placed() {
    let start = std::time::Instant::now();
    // 2^30 places in about two kilobytes: the count says so before a single place is made, and nothing is allocated for them
    let error = read_raw(doubling(30)).unwrap_err();
    assert!(error.contains("1073741824") && error.contains("places"), "{error}");
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    // the cells that they would make are counted too, against the limit on cells
    let limits = Limits { max_cells: 1000, ..Limits::default() };
    let error = vox::import(&raw::file(doubling(20)), None, limits, &vox::Bounds::default()).unwrap_err();
    assert!(error.contains("1048576") && error.contains("cells"), "{error}");
    // within the bounds a shared node is placed as many times as it is used, the later place winning where two fall on one cell
    let few = read_raw(doubling(3)).unwrap();
    assert_eq!(few.occupancy.count(), 1);
    // and the bound on places is a bound of its own
    let bounds = vox::Bounds { max_placements: 7, ..vox::Bounds::default() };
    let error = vox::import(&raw::file(doubling(3)), None, Limits::default(), &bounds).unwrap_err();
    assert!(error.contains('8') && error.contains("places"), "{error}");
}

#[test]
fn a_scene_graph_that_has_no_root_or_two_or_a_node_outside_the_root_is_an_error_in_every_case() {
    let model = raw::model([2, 2, 2], &[[0, 0, 0, 1]]);
    // the nodes only name each other: nTRN 0 -> nGRP 1 -> [0]
    let cycle = read_raw([model.clone(), raw::transform(0, 1, None, None), raw::group(1, &[0])].concat());
    assert!(cycle.unwrap_err().contains("root"));
    // a good root and a second node that nobody names
    let two_roots = read_raw(
        [model.clone(), raw::transform(0, 1, None, None), raw::shape(1, &[0]), raw::transform(2, 1, None, None)]
            .concat(),
    );
    assert!(two_roots.unwrap_err().contains("roots"));
    // a good root and a cycle of two nodes off to the side
    let aside = read_raw(
        [
            model.clone(),
            raw::transform(0, 1, None, None),
            raw::shape(1, &[0]),
            raw::transform(2, 3, None, None),
            raw::group(3, &[2]),
        ]
        .concat(),
    );
    assert!(aside.unwrap_err().contains("does not reach"));
    // a node that names a model the file does not have, and a child that is not there
    let missing = read_raw([model.clone(), raw::transform(0, 5, None, None)].concat());
    assert!(missing.unwrap_err().contains("node 5"));
    let unknown = read_raw([model.clone(), raw::transform(0, 1, None, None), raw::shape(1, &[3])].concat());
    assert!(unknown.unwrap_err().contains("model 3"));
    // the good graph is a graph
    let good = read_raw([model, raw::transform(0, 1, None, None), raw::shape(1, &[0])].concat()).unwrap();
    assert_eq!(good.occupancy.count(), 1);
}

#[test]
fn a_negated_axis_sends_the_cell_q_to_minus_q_minus_one_as_the_unit_boxes_of_ogt_vox_do() {
    // The values are worked out by hand from the box convention (the cell x of a model with the pivot c fills [x - c, x - c + 1); a
    // placement is the affine map p = R q + t of space; the cell is the one that holds the centre of the box), not by tools/make_vox.py.
    let placed = |size: [i32; 3], cell: [u8; 3], rotation: &str, t: &str| {
        let imported = read_raw(
            [
                raw::model(size, &[[cell[0], cell[1], cell[2], 1]]),
                raw::transform(0, 1, Some(t), Some(rotation)),
                raw::shape(1, &[0]),
            ]
            .concat(),
        )
        .unwrap();
        let cells: Vec<[i32; 3]> = imported.occupancy.cells().collect();
        assert_eq!(cells.len(), 1);
        // back to MagicaVoxel's axes: the scene's [x, -z - 1, y] of the cell [x, y, z]
        let c = cells[0];
        [c[0], c[2], -c[1] - 1]
    };
    // the example of the description of the format, R = [[0, 1, 0], [0, 0, -1], [-1, 0, 0]] (byte 105), a model of 3 x 3 x 3 (pivot
    // 1, 1, 1), the cell (0, 0, 0), q = (-1, -1, -1), centre (-0.5, -0.5, -0.5):
    // R c = (c_y, -c_z, -c_x) = (-0.5, 0.5, 0.5), plus (5, 6, 7) = (4.5, 6.5, 7.5): the cell (4, 6, 7). Not (4, 7, 8).
    assert_eq!(placed([3, 3, 3], [0, 0, 0], "105", "5 6 7"), [4, 6, 7]);
    // a half turn about z, R = diag(-1, -1, 1) (byte 52 = 0 | 1 << 2 | 1 << 4 | 1 << 5), the cell (2, 1, 0) of 3 x 3 x 3:
    // q = (1, 0, -1), centre (1.5, 0.5, -0.5), R c = (-1.5, -0.5, -0.5): the cell (-2, -1, -1)
    assert_eq!(placed([3, 3, 3], [2, 1, 0], "52", "0 0 0"), [-2, -1, -1]);
    // a model of 2 x 2 x 2 (pivot 1, 1, 1), the cell (1, 0, 0): q = (0, -1, -1), centre (0.5, -0.5, -0.5), R c = (-0.5, 0.5, -0.5): (-1, 0, -1)
    assert_eq!(placed([2, 2, 2], [1, 0, 0], "52", "0 0 0"), [-1, 0, -1]);
    // no negation, no change of convention: the identity (byte 4: column 0 in the first row, column 1 in the second) moves the cell by the translation alone, (0, 0, 0) of 2 x 2 x 2 is -1 about the pivot
    assert_eq!(placed([2, 2, 2], [0, 0, 0], "4", "3 4 5"), [2, 3, 4]);
}
