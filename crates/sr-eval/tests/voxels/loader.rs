//! The loader of a `voxelAsset`: the file or the mesh it names, within its limits, at the origin, with its materials, read once and read
//! again when its bytes are not the ones that were read.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::{Limits, Occupancy};
use sr_3d::voxel::material::{fingerprint, parse_all};
use sr_3d::voxel::{srvol, vox, Colours};
use sr_eval::voxel_asset::{load, DEFAULT_MAX_CELLS};
use sr_eval::Evaluator;
use sr_sim::physics3d::shape_mass_properties;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A directory of its own for one test, removed with it.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!("voxel-loader-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A program with the assets `assets` and an object of primitive voxels for each voxelAsset among them (a program knows the assets that
/// its objects name).
fn evaluator(dir: &Dir, assets: &str) -> Evaluator {
    let objects: String = assets
        .split("<voxelAsset id=\"")
        .skip(1)
        .map(|rest| {
            let id = rest.split('"').next().unwrap();
            format!(r#"<object3D id="o-{id}" primitive="voxels" voxels="{id}" material="stone"/>"#)
        })
        .collect();
    let xml = format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets>{assets}</assets><materials><material id="stone" baseColor="#808080"/></materials><composition>{objects}</composition></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir.0.clone()) })
        .unwrap_or_else(|e| panic!("{e:?}"));
    Evaluator::new(&doc, &Default::default()).unwrap()
}

fn chunk(name: &str, content: &[u8]) -> Vec<u8> {
    let mut out = name.as_bytes().to_vec();
    out.extend((content.len() as i32).to_le_bytes());
    out.extend(0i32.to_le_bytes());
    out.extend(content);
    out
}

fn dictionary(pairs: &[(&str, &str)]) -> Vec<u8> {
    let string = |s: &str| [(s.len() as i32).to_le_bytes().to_vec(), s.as_bytes().to_vec()].concat();
    let mut out = (pairs.len() as i32).to_le_bytes().to_vec();
    for (k, v) in pairs {
        out.extend(string(k));
        out.extend(string(v));
    }
    out
}

fn file(children: Vec<u8>) -> Vec<u8> {
    let mut out = b"VOX ".to_vec();
    out.extend(150i32.to_le_bytes());
    out.extend(b"MAIN");
    out.extend(0i32.to_le_bytes());
    out.extend((children.len() as i32).to_le_bytes());
    out.extend(children);
    out
}

fn model(size: [i32; 3], cells: &[[u8; 4]]) -> Vec<u8> {
    let mut xyzi = (cells.len() as i32).to_le_bytes().to_vec();
    cells.iter().for_each(|c| xyzi.extend(c));
    [chunk("SIZE", &size.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()), chunk("XYZI", &xyzi)].concat()
}

/// The model placed by one `nTRN` with the translation `t` (MagicaVoxel's axes) over one `nSHP`.
fn placed(size: [i32; 3], cells: &[[u8; 4]], t: [i32; 3]) -> Vec<u8> {
    let mut trn = 0i32.to_le_bytes().to_vec();
    trn.extend(dictionary(&[]));
    for v in [1i32, -1, -1, 1] {
        trn.extend(v.to_le_bytes());
    }
    trn.extend(dictionary(&[("_t", &format!("{} {} {}", t[0], t[1], t[2]))]));
    let mut shp = 1i32.to_le_bytes().to_vec();
    shp.extend(dictionary(&[]));
    shp.extend(1i32.to_le_bytes());
    shp.extend(0i32.to_le_bytes());
    shp.extend(dictionary(&[]));
    file([model(size, cells), chunk("nTRN", &trn), chunk("nSHP", &shp)].concat())
}

fn cube_cells() -> Vec<[u8; 4]> {
    (0..8u8).map(|i| [i & 1, i >> 1 & 1, i >> 2 & 1, 3]).collect()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_cube_of_two_cells_a_side_has_the_mass_and_inertia_of_its_cells_from_the_file_to_the_world() {
    let dir = Dir::new("mass");
    dir.write("cube.vox", &file(model([2, 2, 2], &cube_cells())));
    let ev = evaluator(&dir, r#"<voxelAsset id="cube" src="cube.vox"/>"#);
    let loaded = load(ev.program(), "cube").unwrap();
    assert_eq!(loaded.occupancy.count(), 8);
    // cells of 3 scene units, 1000 kg to the cubic metre, 100 scene units to the metre: 8 x 1000 x 27 / 1e6 kg
    let (size, density, ppm) = ([3.0; 3], 1000.0, 100.0);
    let body = sr_eval::voxels::body(&loaded.occupancy, size, density, ppm).unwrap();
    let mass = 8.0 * density * 27.0 / 1e6;
    assert!((body.mass - mass).abs() < 1e-12 * mass, "{} against {mass}", body.mass);
    let world = shape_mass_properties(&body.shape, body.mass, ppm).unwrap();
    // a cube of side L = 6 scene units: the inertia is m L^2 / 6 about each axis through its centre, with no product, and the centre is
    // the middle of the box that has its corner at the origin
    let l = 6.0f64;
    let want = mass * l * l / 6.0;
    for k in 0..3 {
        assert!((world.centre[k] - 3.0).abs() < 1e-12, "centre {k}: {}", world.centre[k]);
        for j in 0..3 {
            let expected = if j == k { want } else { 0.0 };
            assert!(
                (world.inertia[k][j] - expected).abs() < 1e-12 * want,
                "inertia [{k}][{j}]: {} against {expected}",
                world.inertia[k][j]
            );
        }
    }
    assert!((world.mass - mass).abs() < 1e-12 * mass);
}

#[test]
fn the_cells_are_at_the_origin_and_the_origin_says_how_far_they_were_from_it() {
    let dir = Dir::new("origin");
    let cells = [[0u8, 0, 0, 1], [2, 1, 0, 2], [1, 2, 1, 3]];
    // the same model under two scene graphs that translate it differently, in MagicaVoxel's axes (x right, y forward, z up)
    let (a, b) = ([5, 0, 0], [-7, 3, 2]);
    dir.write("a.vox", &placed([3, 3, 3], &cells, a));
    dir.write("b.vox", &placed([3, 3, 3], &cells, b));
    let ev = evaluator(&dir, r#"<voxelAsset id="a" src="a.vox"/><voxelAsset id="b" src="b.vox"/>"#);
    let (first, second) = (load(ev.program(), "a").unwrap(), load(ev.program(), "b").unwrap());
    // the same cells, and so the same fingerprint
    assert_eq!(first.occupancy.cells().collect::<Vec<_>>(), second.occupancy.cells().collect::<Vec<_>>());
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(first.occupancy.bounds().unwrap().0, [0, 0, 0]);
    // the origins differ by the translations in the scene's axes: [x, y, z] of MagicaVoxel is [x, -z, y]
    let scene = |t: [i32; 3]| [i64::from(t[0]), -i64::from(t[2]), i64::from(t[1])];
    let (sa, sb) = (scene(a), scene(b));
    for k in 0..3 {
        assert_eq!(first.origin_cells[k] - second.origin_cells[k], sa[k] - sb[k], "axis {k}");
    }
    // and the origin is the minimum key of what the file holds, worked out by hand: the cells of the model about its pivot (1, 1, 1) are
    // (-1, -1, -1), (1, 0, -1), (0, 1, 0), moved by the translation, then [x, y, z] -> [x, -z - 1, y]
    let key = |q: [i64; 3], t: [i32; 3]| {
        let p = [q[0] + i64::from(t[0]), q[1] + i64::from(t[1]), q[2] + i64::from(t[2])];
        [p[0], -p[2] - 1, p[1]]
    };
    let all = [[-1, -1, -1], [1, 0, -1], [0, 1, 0]].map(|q| key(q, a));
    let min: [i64; 3] = std::array::from_fn(|k| all.iter().map(|c| c[k]).min().unwrap());
    assert_eq!(first.origin_cells, min);
    // the cells of the file are the cells of the model plus the origin
    for cell in first.occupancy.cells() {
        let at: [i64; 3] = std::array::from_fn(|k| i64::from(cell[k]) + first.origin_cells[k]);
        assert!(all.contains(&at), "{at:?}");
    }
}

#[test]
fn the_declared_sha256_is_checked_on_the_bytes_read_before_anything_is_parsed() {
    let dir = Dir::new("sha");
    let good = file(model([2, 2, 2], &cube_cells()));
    dir.write("good.vox", &good);
    dir.write("not-a-model.vox", b"this is not a MagicaVoxel file at all");
    let right = sha256(&good);
    let wrong = sha256(b"something else");
    let ev = evaluator(
        &dir,
        &format!(
            r#"<voxelAsset id="ok" src="good.vox" sha256="{right}"/><voxelAsset id="bad" src="good.vox" sha256="{wrong}"/><voxelAsset id="junk" src="not-a-model.vox" sha256="{wrong}"/><voxelAsset id="junk-ok" src="not-a-model.vox"/>"#
        ),
    );
    assert_eq!(load(ev.program(), "ok").unwrap().occupancy.count(), 8);
    let error = load(ev.program(), "bad").unwrap_err();
    assert!(error.contains("sha256") && error.contains(&right) && error.contains(&wrong), "{error}");
    // a file that is not a model, with a digest that is not its own: refused as not the file the document names, not as a malformed one
    let error = load(ev.program(), "junk").unwrap_err();
    assert!(error.contains("sha256") && !error.contains("VOX"), "{error}");
    // with no digest the same file is refused by its format
    assert!(load(ev.program(), "junk-ok").unwrap_err().contains("VOX"));
}

#[test]
fn the_limits_of_the_asset_are_checked_and_say_what_was_over() {
    let dir = Dir::new("limits");
    dir.write("cube.vox", &file(model([2, 2, 2], &cube_cells())));
    dir.write("big.vox", &vec![0u8; 2 << 20]);
    let ev = evaluator(
        &dir,
        r#"<voxelAsset id="few" src="cube.vox" maxCells="7"/><voxelAsset id="exact" src="cube.vox" maxCells="8"/><voxelAsset id="big" src="big.vox" maxMemoryMiB="1"/><voxelAsset id="remote" src="https://example.com/cube.vox"/><voxelAsset id="gone" src="missing.vox"/><voxelAsset id="odd" src="cube.bin"/>"#,
    );
    let error = load(ev.program(), "few").unwrap_err();
    assert!(error.contains('8') && error.contains('7') && error.contains("cells"), "{error}");
    assert_eq!(load(ev.program(), "exact").unwrap().occupancy.count(), 8);
    let error = load(ev.program(), "big").unwrap_err();
    assert!(error.contains("2097152") && error.contains("1048576") && error.contains("maxMemoryMiB"), "{error}");
    assert!(load(ev.program(), "remote").unwrap_err().contains("remote"));
    assert!(load(ev.program(), "gone").unwrap_err().contains("missing.vox"));
    dir.write("cube.bin", &file(model([2, 2, 2], &cube_cells())));
    assert!(load(ev.program(), "odd").unwrap_err().contains("format"));
    assert_eq!(DEFAULT_MAX_CELLS, 4_194_304);
}

#[test]
fn an_asset_is_read_once_and_read_again_when_its_bytes_are_not_the_ones_that_were_read() {
    let dir = Dir::new("cache");
    let a = file(model([2, 2, 2], &cube_cells()));
    let path = dir.write("m.vox", &a);
    let ev = evaluator(&dir, r#"<voxelAsset id="m" src="m.vox"/>"#);
    let first = load(ev.program(), "m").unwrap();
    // the same bytes: the same model, not read again
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "m").unwrap()));
    // the file touched and not changed: still the same model (the decision is of the bytes, not of the time)
    std::fs::write(&path, &a).unwrap();
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "m").unwrap()));
    // the same length, one voxel moved: the model is read again and its fingerprint is another
    let mut cells = cube_cells();
    cells[7] = [1, 1, 1, 9];
    let b = file(model([2, 2, 2], &cells));
    assert_eq!(a.len(), b.len());
    std::fs::write(&path, &b).unwrap();
    let second = load(ev.program(), "m").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_ne!(first.source.sha256, second.source.sha256);
    assert_eq!(second.source.bytes, b.len() as u64);
    // and back
    std::fs::write(&path, &a).unwrap();
    assert_eq!(load(ev.program(), "m").unwrap().fingerprint, first.fingerprint);
    // another program has its own cache
    let other = evaluator(&dir, r#"<voxelAsset id="m" src="m.vox"/>"#);
    assert!(!Arc::ptr_eq(&first, &load(other.program(), "m").unwrap()));
}

#[test]
fn the_materials_of_the_file_are_numbers_by_index_with_a_fingerprint_of_their_own() {
    let dir = Dir::new("materials");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sr-3d/tests/fixtures/vox");
    dir.write("materials.vox", &std::fs::read(fixtures.join("materials.vox")).unwrap());
    dir.write("one-model.vox", &std::fs::read(fixtures.join("one-model.vox")).unwrap());
    let ev = evaluator(&dir, r#"<voxelAsset id="m" src="materials.vox"/><voxelAsset id="p" src="one-model.vox"/>"#);
    let m = load(ev.program(), "m").unwrap();
    assert_eq!(m.colours, Some(Colours::File));
    assert_eq!(m.materials.keys().copied().collect::<Vec<_>>(), vec![1, 2, 7]);
    assert_eq!(m.materials[&1].rough, Some(0.25));
    let imported = vox::import(
        &std::fs::read(fixtures.join("materials.vox")).unwrap(),
        None,
        Limits::default(),
        &vox::Bounds::default(),
    )
    .unwrap();
    assert_eq!(m.materials_fingerprint, fingerprint(&parse_all(&imported.materials).unwrap()));
    // a file with no material has the fingerprint of none, and it is not that of this one
    let plain = load(ev.program(), "p").unwrap();
    assert!(plain.materials.is_empty());
    assert_eq!(plain.materials_fingerprint, fingerprint(&Default::default()));
    assert_ne!(plain.materials_fingerprint, m.materials_fingerprint);
}

#[test]
fn a_mesh_asset_cut_into_cells_is_cut_in_the_frame_it_is_drawn_in_and_a_cache_has_the_scale_of_its_grid() {
    let dir = Dir::new("sources");
    dir.write(
        "cube.glb",
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cube-1m.glb")).unwrap(),
    );
    let mut original = Occupancy::new();
    for i in 0..5 {
        original.set([i, 2 * i, -i], (i + 1) as u8).unwrap();
    }
    dir.write("grid.srvol", &srvol::write(&original, 0.5).unwrap());
    let ev = evaluator(
        &dir,
        r#"<mesh id="rock" src="cube.glb"/><voxelAsset id="cut" fromMesh="rock" cellSize="10"/><voxelAsset id="tiny" fromMesh="rock" cellSize="1000"/><voxelAsset id="cache" src="grid.srvol"/>"#,
    );
    // a cube of a metre, at 100 scene units a metre, with y and z turned: x 0 to 100, y -100 to 0, z -100 to 0, in cells of 10
    let cut = load(ev.program(), "cut").unwrap();
    assert_eq!(cut.occupancy.count(), 1000);
    assert_eq!(cut.origin_cells, [0, -10, -10]);
    assert_eq!(cut.occupancy.bounds().unwrap(), ([0, 0, 0], [9, 9, 9]));
    assert_eq!(cut.cell_size, Some(10.0));
    assert_eq!(cut.colours, None);
    assert!(cut.materials.is_empty());
    // a cell that no centre of is inside the mesh: no cell
    assert!(load(ev.program(), "tiny").unwrap_err().contains("no cell"));
    // a cache: the scale of the grid is the size of a cell, the cells are the cells at the origin, no palette
    let cache = load(ev.program(), "cache").unwrap();
    assert_eq!(cache.cell_size, Some(0.5));
    assert_eq!(cache.occupancy.count(), 5);
    assert_eq!(cache.origin_cells, [0, 0, -4]);
    assert_eq!(cache.colours, None);
    for i in 0..5 {
        assert_eq!(cache.occupancy.get([i, 2 * i, 4 - i]), (i + 1) as u8, "cell {i}");
    }
}
