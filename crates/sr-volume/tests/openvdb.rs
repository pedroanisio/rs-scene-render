use sr_volume::{openvdb, CacheLimits};
use std::{fs::File, io::Cursor, path::PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/openvdb").join(format!("{name}.vdb"))
}

#[test]
fn official_scalar_files_preserve_active_and_inactive_values_across_codecs() {
    for name in ["density-none", "density-zip", "density-blosc", "density-half"] {
        let volume = openvdb::read(File::open(fixture(name)).unwrap(), CacheLimits::default()).unwrap();
        let grid = volume.grid("density").unwrap();
        assert_eq!(grid.value([1, 2, 3]), 1.25, "{name}");
        assert_eq!(grid.value([-1, -2, -3]), 2.5, "{name}");
        assert_eq!(grid.value([4, 5, 6]), 0.75, "{name}: inactive non-background value");
        assert_eq!(grid.value([7, 7, 7]), 0.0);
        assert_eq!(grid.background(), 0.0);
    }
}

#[test]
fn official_affine_background_and_grid_instances_preserve_field_coordinates() {
    let volume = openvdb::read(File::open(fixture("affine-background")).unwrap(), CacheLimits::default()).unwrap();
    let grid = volume.grid("density").unwrap();
    assert_eq!(grid.background(), 0.5);
    assert_eq!(grid.transform().index_to_world([1., 2., 3.]), [12., 26.25, 18.]);
    assert_eq!(grid.sample_world([12., 26.25, 18.]), 3.0);
    assert_eq!(grid.value([4, 5, 6]), -0.5);
    assert_eq!(grid.value([400, 500, 600]), 0.5);
    let volume = openvdb::read(File::open(fixture("instanced")).unwrap(), CacheLimits::default()).unwrap();
    assert_eq!(volume.grid("density").unwrap().sample_world([1., 2., 3.]), 1.25);
    assert_eq!(volume.grid("temperature").unwrap().sample_world([2., 4., 6.]), 1.25);
}

#[test]
fn official_vector_grids_split_into_named_scalar_components() {
    let volume = openvdb::read(File::open(fixture("vector")).unwrap(), CacheLimits::default()).unwrap();
    for (name, background, value) in [("velocity.x", 0.25, 4.0), ("velocity.y", 0.5, -5.0), ("velocity.z", 0.75, 6.0)] {
        let grid = volume.grid(name).unwrap();
        assert_eq!(grid.background(), background);
        assert_eq!(grid.value([1, 2, 3]), value);
    }
    assert!(volume.grid("density").is_some());
}

#[test]
fn official_tiles_are_preserved_and_expansion_is_admitted_before_allocation() {
    let volume = openvdb::read(File::open(fixture("tiles")).unwrap(), CacheLimits::default()).unwrap();
    let grid = volume.grid("density").unwrap();
    assert_eq!(grid.value([-8, 0, 0]), 2.0);
    assert_eq!(grid.value([-1, 7, 7]), 2.0);
    assert_eq!(grid.value([8, 0, 0]), 0.75);
    assert_eq!(grid.value([15, 7, 7]), 0.75);
    assert_eq!(grid.value([128, 0, 0]), 1.0);
    assert_eq!(grid.value([255, 127, 127]), 1.0);
    assert_eq!(grid.value([256, 0, 0]), 0.0);
    assert!(openvdb::read(
        File::open(fixture("tiles")).unwrap(),
        CacheLimits { max_bricks: 4097, ..CacheLimits::default() }
    )
    .is_err());
    assert!(openvdb::read(File::open(fixture("large-tile")).unwrap(), CacheLimits::default()).is_err());
}

#[test]
fn corrupt_or_unsupported_files_return_errors_without_panics() {
    for name in ["unsupported-bool", "frustum"] {
        assert!(openvdb::read(File::open(fixture(name)).unwrap(), CacheLimits::default()).is_err());
    }
    let data = std::fs::read(fixture("density-blosc")).unwrap();
    for cut in (0..200.min(data.len())).chain([data.len() / 2, data.len() - 1]) {
        assert!(openvdb::read(Cursor::new(&data[..cut]), CacheLimits::default()).is_err(), "cut={cut}");
    }
    let mut bad = data.clone();
    bad[0] ^= 1;
    assert!(openvdb::read(Cursor::new(bad), CacheLimits::default()).is_err());
    assert!(openvdb::read(Cursor::new(data), CacheLimits { max_bytes: 64, ..CacheLimits::default() }).is_err());
}

#[test]
fn root_bookkeeping_is_charged_even_when_all_tiles_equal_background() {
    // Start with an official uncompressed root-only grid, replacing its one
    // 17-byte tile with many valid, distinct background-valued tiles.
    let mut data = std::fs::read(fixture("large-tile")).unwrap();
    let old_len = data.len() as u64;
    let offsets: Vec<_> =
        data.windows(8).enumerate().filter_map(|(i, v)| (v == old_len.to_le_bytes()).then_some(i)).collect();
    assert_eq!(offsets.len(), 2);
    data.truncate(data.len() - 33);
    data.extend(1u32.to_le_bytes());
    data.extend(0f32.to_le_bytes());
    let count = 20_000u32;
    data.extend(count.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    for i in 0..count {
        data.extend(((i as i32) * 4096).to_le_bytes());
        data.extend([0; 8]);
        data.extend(0f32.to_le_bytes());
        data.push(1);
    }
    let end = data.len() as u64;
    for offset in offsets {
        data[offset..offset + 8].copy_from_slice(&end.to_le_bytes());
    }
    assert!(openvdb::read(Cursor::new(data), CacheLimits { max_bytes: (4 << 20) + 500_000, ..CacheLimits::default() })
        .is_err());
}

#[test]
fn instance_payload_cannot_hide_trailing_bytes() {
    let mut data = std::fs::read(fixture("instanced")).unwrap();
    let old_len = data.len() as u64;
    let offset = data.windows(8).position(|v| v == old_len.to_le_bytes()).unwrap();
    data.push(0);
    data[offset..offset + 8].copy_from_slice(&(old_len + 1).to_le_bytes());
    assert!(openvdb::read(Cursor::new(data), CacheLimits::default()).is_err());
}

#[test]
fn official_dense_compression_and_double_fields_match_reference_values() {
    for name in ["dense-zip", "dense-blosc"] {
        let volume = openvdb::read(File::open(fixture(name)).unwrap(), CacheLimits::default()).unwrap();
        let grid = volume.grid("density").unwrap();
        for x in 0..8 {
            for y in 0..8 {
                for z in 0..8 {
                    assert_eq!(
                        grid.value([x, y, z]),
                        0.25 + x as f32 + 0.5 * y as f32 + 0.125 * z as f32,
                        "{name}: {x},{y},{z}"
                    );
                }
            }
        }
    }
    let volume = openvdb::read(File::open(fixture("double")).unwrap(), CacheLimits::default()).unwrap();
    let density = volume.grid("density").unwrap();
    assert_eq!(density.background(), 0.25);
    assert_eq!(density.value([-1, 2, -3]), 1.125);
    let volume = openvdb::read(File::open(fixture("vector-double")).unwrap(), CacheLimits::default()).unwrap();
    for (channel, value) in [("x", 4.), ("y", -5.), ("z", 6.)] {
        assert_eq!(volume.grid(&format!("velocity.{channel}")).unwrap().value([-1, 2, -3]), value);
    }
}
