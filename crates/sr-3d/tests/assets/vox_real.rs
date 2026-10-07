//! The reader against real MagicaVoxel files: three by the author of the program (`ephtracy_*`, the characters of its repository) and four
//! from the `dot_vox` crate's test resources (`dotvox_*`, written by MagicaVoxel 0.99), with the licences next to them
//! (`fixtures/vox/real`, sources in SOURCES.md). What they prove, and what they do not:
//!
//! * the palette offset by one, from the description of the format ("color [0-254] are mapped to palette index [1-255]"): the colours of the
//!   knight are the file's entries, one place down;
//! * a `MATL` id is the palette index, by the files themselves: in `metal-material` the voxels have the colour index 85 and the metal that
//!   is not the default is the `MATL` 85; in `single-voxel-with-material` the voxel is 249 and the odd material is the `MATL` 249;
//! * the centre of a model under a scene graph, `floor(size / 2)`, is the rule of `ogt_vox.h` and the layout of `axes.vox` fits it (the
//!   cube sits on the plane z = 0 and about x = y = 0), which is evidence and not a proof: the file has no rotation;
//! * the rotation byte is not proved by any real file here (none has a rotation), only by the example of the description of the format, in
//!   the fixture `spec-rotation`.

use sr_3d::occupancy::Limits;
use sr_3d::voxel::default_palette::DEFAULT_PALETTE;
use sr_3d::voxel::{vox, Colours};

fn real(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vox/real").join(name)).unwrap()
}

fn import(name: &str) -> sr_3d::voxel::Imported {
    vox::import(&real(name), None, Limits::default(), &vox::Bounds::default()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn the_files_of_the_author_and_of_dot_vox_are_what_their_sources_say_they_are() {
    // (file, version, models as (size, voxels), colours)
    type Model = ([u32; 3], usize);
    let table: [(&str, i32, Vec<Model>, Colours); 6] = [
        ("ephtracy_chr_cat.vox", 150, vec![([20, 20, 20], 563)], Colours::Default),
        ("ephtracy_chr_sol.vox", 150, vec![([20, 21, 20], 294)], Colours::Default),
        ("ephtracy_chr_knight.vox", 150, vec![([20, 21, 20], 398)], Colours::File),
        ("dotvox_single-voxel-with-material.vox", 150, vec![([1, 1, 1], 1)], Colours::File),
        ("dotvox_metal-material.vox", 150, vec![([3, 3, 3], 20)], Colours::File),
        (
            "dotvox_axes.vox",
            200,
            vec![([40, 40, 40], 920), ([40, 40, 40], 1640), ([40, 40, 40], 1480), ([32, 32, 32], 332)],
            Colours::File,
        ),
    ];
    for (name, version, models, colours) in table {
        let bytes = real(name);
        let file = vox::parse(&bytes, &vox::Bounds::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.version, version, "{name}");
        assert_eq!(file.models.len(), models.len(), "{name}");
        for (got, (size, count)) in file.models.iter().zip(&models) {
            assert_eq!(&got.size, size, "{name}");
            assert_eq!(got.voxels.len(), *count, "{name}");
            assert!(!got.voxels.is_empty(), "{name}");
        }
        assert_eq!(import(name).colours, colours, "{name}");
    }
    let error = vox::import(&real("dotvox_not_a.vox"), None, Limits::default(), &vox::Bounds::default()).unwrap_err();
    assert!(error.contains("VOX"), "{error}");
}

#[test]
fn a_file_with_no_rgba_chunk_has_the_default_palette_and_one_with_it_has_the_files_colours_one_place_down() {
    let cat = import("ephtracy_chr_cat.vox");
    let palette = cat.occupancy.palette();
    let used: std::collections::BTreeSet<u8> = cat.occupancy.cells().map(|c| cat.occupancy.get(c)).collect();
    assert!(used.len() > 3, "the cat uses several colours: {used:?}");
    for index in &used {
        assert_eq!(palette.color(*index), DEFAULT_PALETTE[usize::from(*index)]);
    }
    // the knight: the RGBA chunk is the 1024 bytes after its header, and the colour of index c is its entry c - 1
    let bytes = real("ephtracy_chr_knight.vox");
    let at = bytes.windows(4).position(|w| w == b"RGBA").unwrap();
    let rgba = &bytes[at + 12..at + 12 + 1024];
    let knight = import("ephtracy_chr_knight.vox");
    for c in 1..=255usize {
        assert_eq!(
            knight.occupancy.palette().color(c as u8),
            [rgba[4 * (c - 1)], rgba[4 * (c - 1) + 1], rgba[4 * (c - 1) + 2], rgba[4 * (c - 1) + 3]],
            "index {c}"
        );
    }
    assert!(knight.occupancy.cells().all(|c| knight.occupancy.get(c) != 0));
}

#[test]
fn the_material_of_a_palette_index_is_the_matl_with_that_id() {
    // metal-material: every voxel has the colour index 85 or another, and the one non-default metal of the file is the MATL 85
    let metal = import("dotvox_metal-material.vox");
    let used: std::collections::BTreeSet<u8> = metal.occupancy.cells().map(|c| metal.occupancy.get(c)).collect();
    assert!(used.contains(&85), "{used:?}");
    assert_eq!(metal.materials[&85].get("_type").map(String::as_str), Some("_metal"));
    assert_eq!(metal.materials[&85].get("_weight").map(String::as_str), Some("0.526316"));
    assert_eq!(metal.materials[&79].get("_weight").map(String::as_str), Some("1"));
    assert_eq!(metal.materials[&249].get("_type").map(String::as_str), Some("_glass"));
    // single-voxel-with-material: the voxel is 249, and the material that is not the default one is the MATL 249
    let single = import("dotvox_single-voxel-with-material.vox");
    let cells: Vec<_> = single.occupancy.cells().collect();
    assert_eq!(cells.len(), 1);
    assert_eq!(single.occupancy.get(cells[0]), 249);
    assert_eq!(single.materials[&249].get("_flux").map(String::as_str), Some("4"));
    assert_eq!(single.materials[&249].get("_plastic").map(String::as_str), Some("1"));
    // every other material of the file is the plain default one
    assert!(single
        .materials
        .iter()
        .filter(|(i, _)| **i != 249)
        .all(|(_, m)| m.get("_type").map(String::as_str) == Some("_diffuse")));
}

/// The sum, wrapping, over cells of a hash of the cell and its index, which does not depend on the order of the cells.
fn checksum(o: &sr_3d::occupancy::Occupancy) -> u64 {
    o.cells()
        .map(|c| {
            let h = |v: i32, p: u64| ((i64::from(v) + 1000) as u64).wrapping_mul(p);
            h(c[0], 73856093) ^ h(c[1], 19349663) ^ h(c[2], 83492791) ^ u64::from(o.get(c)).wrapping_mul(1315423911)
        })
        .fold(0u64, u64::wrapping_add)
}

#[test]
fn the_scene_of_axes_is_what_an_independent_decoder_places_and_the_cube_sits_on_the_ground() {
    let expected: serde_json::Value = serde_json::from_slice(&real("dotvox_axes.expected.json")).unwrap();
    let axes = import("dotvox_axes.vox");
    assert_eq!(axes.occupancy.count(), expected["cells"].as_u64().unwrap());
    let (min, max) = axes.occupancy.bounds().unwrap();
    let want = |k: &str| -> [i64; 3] { std::array::from_fn(|a| expected[k][a].as_i64().unwrap()) };
    assert_eq!(min.map(i64::from), want("min"));
    assert_eq!(max.map(i64::from), want("max"));
    assert_eq!(checksum(&axes.occupancy), expected["checksum"].as_u64().unwrap());
    // the cube (colour 254, the model of 32 cells to a side placed by a translation of 16 up): its cells, in the axes of MagicaVoxel (x, y, z)
    // = (x, z, -y - 1) of the scene, reach z = 0 and are centred on x = y = 0 about the cell boundary: the pivot is floor(size / 2)
    let cube: Vec<[i32; 3]> =
        axes.occupancy.cells().filter(|c| axes.occupancy.get(*c) == 254).map(|c| [c[0], c[2], -c[1] - 1]).collect();
    assert_eq!(cube.len() as u64, expected["cube_cells"].as_u64().unwrap());
    let low: [i64; 3] = std::array::from_fn(|a| i64::from(cube.iter().map(|c| c[a]).min().unwrap()));
    let high: [i64; 3] = std::array::from_fn(|a| i64::from(cube.iter().map(|c| c[a]).max().unwrap()));
    assert_eq!(low, want("cube_min_voxel_axes"));
    assert_eq!(high, want("cube_max_voxel_axes"));
    assert_eq!(low, [-16, -16, 0], "the base of the cube is on the ground and it is centred");
}

#[test]
fn every_real_file_that_is_in_the_samples_folder_is_read_or_refused_by_name() {
    // the larger files of the same sources, which are not in the repository: read where they are
    let Ok(dir) = std::env::var("VOX_SAMPLES") else { return };
    let mut read = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("vox") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        match vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()) {
            Ok(model) => {
                assert!(model.occupancy.count() > 0, "{}", path.display());
                read += 1;
            }
            Err(error) => {
                assert!(path.ends_with("dotvox_not_a.vox") && error.contains("VOX"), "{}: {error}", path.display())
            }
        }
    }
    assert!(read >= 10, "{read} files read");
}
