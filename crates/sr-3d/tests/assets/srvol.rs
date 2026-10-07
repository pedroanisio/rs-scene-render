//! A voxel model as an SRVOL cache: the palette indices as exact values of a grid, the cells of a model back out of it as
//! they went in, and a label that is not an index is an error and not a wrapped number.

use sr_3d::occupancy::{Limits, Occupancy};
use sr_3d::voxel::{srvol, vox};
use sr_volume::{CacheLimits, SparseGrid, Transform, Volume};

fn occupancy(name: &str) -> Occupancy {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vox");
    let bytes = std::fs::read(dir.join(format!("{name}.vox"))).unwrap();
    vox::import(&bytes, None, Limits::default(), &vox::Bounds::default()).unwrap().occupancy
}

fn grid_of(values: &[([i32; 3], f32)]) -> Vec<u8> {
    let mut grid = SparseGrid::new(Transform::identity(), 0.0, 1000).unwrap();
    for (index, v) in values {
        grid.set(*index, *v).unwrap();
    }
    let mut volume = Volume::new();
    volume.insert("voxels", grid).unwrap();
    let mut out = Vec::new();
    volume.write(&mut out).unwrap();
    out
}

#[test]
fn a_model_goes_out_as_a_cache_and_comes_back_the_same_cells_and_the_same_revision() {
    for name in ["one-model", "two-models", "nested", "rotations"] {
        let original = occupancy(name);
        let bytes = srvol::write(&original, 2.5).unwrap();
        let back = srvol::import(&bytes, srvol::GRID, Limits::default(), CacheLimits::default()).unwrap();
        assert_eq!(back.occupancy.fingerprint(), original.fingerprint(), "{name}");
        assert_eq!(back.occupancy.revision(), original.revision(), "{name}");
        assert_eq!(back.occupancy.count(), original.count(), "{name}");
        assert_eq!(back.cell_size, Some(2.5), "{name}: the size of a cell is the scale of the grid");
        // the same bytes every time
        assert_eq!(bytes, srvol::write(&original, 2.5).unwrap(), "{name}");
    }
}

#[test]
fn an_empty_model_and_a_model_below_the_origin_go_round_too() {
    let empty = Occupancy::new();
    let back =
        srvol::import(&srvol::write(&empty, 1.0).unwrap(), srvol::GRID, Limits::default(), CacheLimits::default())
            .unwrap();
    assert_eq!(back.occupancy.count(), 0);
    let below = Occupancy::from_cells([([-9, -1, -17], 3u8), ([0, 0, 0], 255), ([7, -8, 9], 1)]).unwrap();
    let back =
        srvol::import(&srvol::write(&below, 1.0).unwrap(), srvol::GRID, Limits::default(), CacheLimits::default())
            .unwrap();
    assert_eq!(back.occupancy.fingerprint(), below.fingerprint());
}

#[test]
fn a_label_that_is_not_an_index_is_an_error_that_names_it() {
    let read = |values: &[([i32; 3], f32)]| {
        srvol::import(&grid_of(values), srvol::GRID, Limits::default(), CacheLimits::default())
    };
    assert!(read(&[([0, 0, 0], 255.0), ([1, 0, 0], 1.0)]).is_ok());
    for (value, word) in [(256.0, "256"), (300.0, "300"), (-1.0, "-1"), (1.5, "1.5"), (0.25, "0.25")] {
        let error = read(&[([2, 3, 4], value)]).unwrap_err();
        assert!(error.contains(word) && error.contains("[2, 3, 4]"), "{value}: {error}");
    }
}

#[test]
fn a_grid_with_a_background_but_zero_or_the_wrong_name_is_refused() {
    let mut grid = SparseGrid::new(Transform::identity(), 1.0, 10).unwrap();
    grid.set([0, 0, 0], 2.0).unwrap();
    let mut volume = Volume::new();
    volume.insert("voxels", grid).unwrap();
    let mut bytes = Vec::new();
    volume.write(&mut bytes).unwrap();
    assert!(srvol::import(&bytes, srvol::GRID, Limits::default(), CacheLimits::default())
        .unwrap_err()
        .contains("background"));
    let named = grid_of(&[([0, 0, 0], 1.0)]);
    assert!(srvol::import(&named, "density", Limits::default(), CacheLimits::default())
        .unwrap_err()
        .contains("density"));
    assert!(srvol::import(b"not a cache", srvol::GRID, Limits::default(), CacheLimits::default()).is_err());
}

#[test]
fn the_size_of_a_cell_is_the_scale_of_the_grid_if_it_is_one() {
    let with = |columns: [f64; 16]| {
        let mut grid = SparseGrid::new(Transform::new(columns).unwrap(), 0.0, 10).unwrap();
        grid.set([0, 0, 0], 1.0).unwrap();
        let mut volume = Volume::new();
        volume.insert("voxels", grid).unwrap();
        let mut bytes = Vec::new();
        volume.write(&mut bytes).unwrap();
        srvol::import(&bytes, srvol::GRID, Limits::default(), CacheLimits::default()).unwrap().cell_size
    };
    let scale = |s: [f64; 3], t: [f64; 3]| {
        [s[0], 0.0, 0.0, 0.0, 0.0, s[1], 0.0, 0.0, 0.0, 0.0, s[2], 0.0, t[0], t[1], t[2], 1.0]
    };
    assert_eq!(with(scale([0.5; 3], [0.0; 3])), Some(0.5));
    // a grid that is not a cubic lattice from the origin has no size of cell of its own
    assert_eq!(with(scale([0.5, 0.5, 1.0], [0.0; 3])), None);
    assert_eq!(with(scale([0.5; 3], [3.0, 0.0, 0.0])), None);
}

#[test]
fn the_limits_of_the_grid_and_of_the_cache_are_checked_before_anything_is_built() {
    let original = occupancy("rotations");
    let bytes = srvol::write(&original, 1.0).unwrap();
    let limits = Limits { max_cells: original.count() - 1, ..Limits::default() };
    assert!(srvol::import(&bytes, srvol::GRID, limits, CacheLimits::default()).unwrap_err().contains("cells"));
    let small = CacheLimits { max_bytes: 100, ..CacheLimits::default() };
    assert!(srvol::import(&bytes, srvol::GRID, Limits::default(), small).is_err());
}

#[test]
fn a_cell_outside_the_keys_of_an_occupancy_is_an_error_that_names_the_brick_and_the_edge_is_in() {
    let key_limit = sr_3d::occupancy::KEY_LIMIT;
    let read = |bytes: Vec<u8>| srvol::import(&bytes, srvol::GRID, Limits::default(), CacheLimits::default());
    let inside = read(grid_of(&[([key_limit - 1, -key_limit, 0], 7.0)])).unwrap();
    assert_eq!(inside.occupancy.count(), 1);
    assert_eq!(inside.occupancy.get([key_limit - 1, -key_limit, 0]), 7);
    for index in [[key_limit, 0, 0], [0, -key_limit - 1, 0], [i32::MAX, 0, i32::MAX], [0, i32::MIN, 0]] {
        let error = read(grid_of(&[(index, 1.0)])).unwrap_err();
        assert!(error.contains("outside"), "{index:?}: {error}");
    }
    // and the index 0 of the palette has no colour, whatever the model is
    let model = read(grid_of(&[([0, 0, 0], 1.0)])).unwrap();
    assert_eq!(model.occupancy.palette().color(0), [0, 0, 0, 0]);
}

#[test]
fn every_palette_index_goes_out_as_itself_and_comes_back_as_itself() {
    // 255 cells, the cell i with the index i: a writer or reader that moved an index would show in the cell that has it
    let mut original = Occupancy::new();
    for i in 1..=255u8 {
        original.set([i32::from(i) - 128, 0, i32::from(i % 7)], i).unwrap();
    }
    let bytes = srvol::write(&original, 0.5).unwrap();
    let back = srvol::import(&bytes, srvol::GRID, Limits::default(), CacheLimits::default()).unwrap();
    assert_eq!(back.cell_size, Some(0.5));
    assert_eq!(back.occupancy.count(), 255);
    for i in 1..=255u8 {
        assert_eq!(back.occupancy.get([i32::from(i) - 128, 0, i32::from(i % 7)]), i, "index {i}");
    }
    assert_eq!(back.occupancy.fingerprint(), original.fingerprint());
}
