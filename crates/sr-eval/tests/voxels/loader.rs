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

/// The file's modification time set to `base` plus `seconds` (so that a rewrite is a change of time whatever the clock's grain).
fn stamp(path: &Path, base: std::time::SystemTime, seconds: u64) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(base + std::time::Duration::from_secs(seconds)).unwrap();
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
    // all three cells of the model, in both
    assert_eq!((first.occupancy.count(), second.occupancy.count()), (3, 3));
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
        r#"<voxelAsset id="few" src="cube.vox" maxCells="7"/><voxelAsset id="exact" src="cube.vox" maxCells="8"/><voxelAsset id="big" src="big.vox" maxMemoryMiB="1"/><voxelAsset id="web" src="https://example.com/cube.vox"/><voxelAsset id="gone" src="missing.vox"/><voxelAsset id="odd" src="cube.bin"/>"#,
    );
    let error = load(ev.program(), "few").unwrap_err();
    assert!(error.contains('8') && error.contains('7') && error.contains("cells"), "{error}");
    assert_eq!(load(ev.program(), "exact").unwrap().occupancy.count(), 8);
    let error = load(ev.program(), "big").unwrap_err();
    assert!(error.contains("2097152") && error.contains("1048576") && error.contains("maxMemoryMiB"), "{error}");
    let error = load(ev.program(), "web").unwrap_err();
    assert!(error.contains("is remote (https)"), "{error}");
    assert!(load(ev.program(), "gone").unwrap_err().contains("missing.vox"));
    dir.write("cube.bin", &file(model([2, 2, 2], &cube_cells())));
    assert!(load(ev.program(), "odd").unwrap_err().contains("format"));
    assert_eq!(DEFAULT_MAX_CELLS, 4_194_304);
}

#[test]
fn an_asset_is_read_once_and_read_again_when_its_length_or_its_time_changes_and_the_bytes_with_them() {
    let dir = Dir::new("cache");
    let a = file(model([2, 2, 2], &cube_cells()));
    let path = dir.write("m.vox", &a);
    let base = std::time::SystemTime::now() - std::time::Duration::from_secs(1000);
    stamp(&path, base, 0);
    let ev = evaluator(&dir, r#"<voxelAsset id="m" src="m.vox"/>"#);
    let first = load(ev.program(), "m").unwrap();
    // the same file: the same model, not read again (a file of the same length and time is taken for the same: the cache is "parse once")
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "m").unwrap()));
    std::fs::write(&path, vec![0u8; a.len()]).unwrap();
    stamp(&path, base, 0);
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "m").unwrap()));
    // the time changed and the bytes did not: the file is read and hashed, and it is the same model
    std::fs::write(&path, &a).unwrap();
    stamp(&path, base, 10);
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "m").unwrap()));
    // the time and the bytes changed at the same length (the colour of one voxel, 3 to 9): the model is read again and is another
    let mut cells = cube_cells();
    cells[7][3] = 9;
    let b = file(model([2, 2, 2], &cells));
    assert_eq!(a.len(), b.len());
    std::fs::write(&path, &b).unwrap();
    stamp(&path, base, 20);
    let second = load(ev.program(), "m").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_ne!(first.source.sha256, second.source.sha256);
    assert_eq!(second.source.bytes, b.len() as u64);
    // and back
    std::fs::write(&path, &a).unwrap();
    stamp(&path, base, 30);
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
    // and the value is the one that tools/vox_materials_hash.py works out from the description of the hash, with none of this code
    assert_eq!(m.materials_fingerprint, 10385904548290088009);
    // a file with no material has the fingerprint of none, and it is not that of this one
    let plain = load(ev.program(), "p").unwrap();
    assert!(plain.materials.is_empty());
    assert_eq!(plain.materials_fingerprint, fingerprint(&Default::default()));
    assert_eq!(plain.materials_fingerprint, 3530818670739341123);
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

#[test]
fn a_from_mesh_asset_of_an_included_document_reads_the_mesh_of_its_own_document() {
    let dir = Dir::new("include");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // the included document's `rock` is a cube of a metre; the main document has a `rock` of its own, a cube of two metres
    dir.write("rock-1m.glb", &std::fs::read(fixtures.join("cube-1m.glb")).unwrap());
    dir.write("rock-2m.glb", &std::fs::read(fixtures.join("cube-2m.glb")).unwrap());
    dir.write(
        "lib.scene.xml",
        br##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/>
<assets><mesh id="rock" src="rock-1m.glb"/><voxelAsset id="v" fromMesh="rock" cellSize="10"/></assets>
<materials><material id="stone" baseColor="#808080"/></materials>
<symbols><symbol id="s" width="32" height="32"><object3D id="o" primitive="voxels" voxels="v" material="stone"/></symbol></symbols><composition/></scene>"##,
    );
    let main = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/>
<assets><mesh id="rock" src="rock-2m.glb"/></assets>
<composition><object3D id="big" primitive="mesh" mesh="rock"/><include id="inc" src="lib.scene.xml" symbol="s"/></composition></scene>"##;
    let doc = sr_model::load_str(main, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir.0.clone()) })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    // the key of an asset of an included document is its namespace and its id; the mesh it names is in the same document
    let cut = load(ev.program(), "inc/v").unwrap();
    assert_eq!(
        cut.occupancy.count(),
        1000,
        "a cube of a metre at cells of 10 is 10 a side, not the 20 of the main document's rock"
    );
    assert_eq!(cut.occupancy.bounds().unwrap(), ([0, 0, 0], [9, 9, 9]));
}

#[test]
fn the_digest_of_a_mesh_is_checked_on_the_bytes_and_includes_the_files_that_it_reads() {
    let dir = Dir::new("mesh-digest");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let cube = std::fs::read(fixtures.join("cube-1m.glb")).unwrap();
    dir.write("cube.glb", &cube);
    let wrong = sha256(b"something else");
    let right = sha256(&cube);
    let ev = evaluator(
        &dir,
        &format!(
            r#"<mesh id="bad" src="cube.glb" sha256="{wrong}"/><mesh id="good" src="cube.glb" sha256="{right}"/><voxelAsset id="from-bad" fromMesh="bad" cellSize="10"/><voxelAsset id="from-good" fromMesh="good" cellSize="10"/>"#
        ),
    );
    // the digest the document declares for the mesh is the mesh file's, checked before the mesh is cut
    let error = load(ev.program(), "from-bad").unwrap_err();
    assert!(error.contains("sha256") && error.contains(&wrong) && error.contains(&right), "{error}");
    let good = load(ev.program(), "from-good").unwrap();
    assert_eq!(good.occupancy.count(), 1000);
    assert_eq!(good.source.sha256.iter().map(|b| format!("{b:02x}")).collect::<String>(), right);

    // a .gltf with its buffer in another file: the digest of the source covers both, so a change of the buffer is a change of the model
    let positions = dir.0.join("cube.bin");
    let corners: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2 & 1) as f32]).collect();
    let quads: [[u16; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let triangles: Vec<u16> = quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect();
    let buffer = |scale: f32| -> Vec<u8> {
        let mut bin: Vec<u8> = corners.iter().flatten().flat_map(|v| (v * scale).to_le_bytes()).collect();
        bin.extend(triangles.iter().flat_map(|i| i.to_le_bytes()));
        bin
    };
    let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"accessors":[{"bufferView":0,"componentType":5126,"count":8,"type":"VEC3","min":[0,0,0],"max":[2,2,2]},{"bufferView":1,"componentType":5123,"count":36,"type":"SCALAR"}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":96},{"buffer":0,"byteOffset":96,"byteLength":72}],"buffers":[{"byteLength":168,"uri":"cube.bin"}]}"#;
    dir.write("split.gltf", json.as_bytes());
    let base = std::time::SystemTime::now() - std::time::Duration::from_secs(1000);
    std::fs::write(&positions, buffer(1.0)).unwrap();
    stamp(&positions, base, 0);
    let ev =
        evaluator(&dir, r#"<mesh id="split" src="split.gltf"/><voxelAsset id="v" fromMesh="split" cellSize="10"/>"#);
    let first = load(ev.program(), "v").unwrap();
    assert_eq!(first.occupancy.count(), 1000);
    // the buffer changes (the cube is two metres now) and the .gltf does not: the model is another, and so is the digest of its source
    std::fs::write(&positions, buffer(2.0)).unwrap();
    stamp(&positions, base, 10);
    let second = load(ev.program(), "v").unwrap();
    assert_eq!(second.occupancy.count(), 8000);
    assert_ne!(first.source.sha256, second.source.sha256);
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_eq!(second.source.bytes, (json.len() + 168) as u64);
}

#[test]
fn cells_that_are_too_far_apart_for_an_occupancy_are_an_error_that_names_the_span() {
    let dir = Dir::new("span");
    let key_limit = sr_3d::occupancy::KEY_LIMIT;
    let grid = |cells: &[[i32; 3]]| {
        let mut grid = sr_volume::SparseGrid::new(sr_volume::Transform::identity(), 0.0, 1000).unwrap();
        for c in cells {
            grid.set(*c, 1.0).unwrap();
        }
        let mut volume = sr_volume::Volume::new();
        volume.insert("voxels", grid).unwrap();
        let mut out = Vec::new();
        volume.write(&mut out).unwrap();
        out
    };
    // the first and the last key that an occupancy has: 2^31 cells apart, one more than the keys of an axis allow once the first is the origin
    dir.write("wide.srvol", &grid(&[[-key_limit, 0, 0], [key_limit - 1, 0, 0]]));
    // 2^30 cells wide, which is as wide as an occupancy can be
    dir.write("widest.srvol", &grid(&[[0, 0, 0], [key_limit - 1, 0, 0]]));
    let ev = evaluator(&dir, r#"<voxelAsset id="wide" src="wide.srvol"/><voxelAsset id="widest" src="widest.srvol"/>"#);
    let error = load(ev.program(), "wide").unwrap_err();
    assert!(error.contains("span") && error.contains("2147483648") && error.contains("axis 0"), "{error}");
    let widest = load(ev.program(), "widest").unwrap();
    assert_eq!(widest.occupancy.count(), 2);
    assert_eq!(widest.occupancy.bounds().unwrap(), ([0, 0, 0], [key_limit - 1, 0, 0]));
}

/// Sets the modification time of `path` to `time` exactly (so that a file that is rewritten can be made to look as it did).
fn restamp(path: &Path, time: std::time::SystemTime) {
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(time).unwrap();
}

#[test]
fn a_mesh_over_the_limit_is_refused_before_it_is_parsed_and_a_cached_model_is_returned_without_parsing_the_mesh() {
    let dir = Dir::new("order");
    // 2 MiB that are not a mesh, under a limit of 1 MiB: the size is what is refused, not the content (it would be "malformed glTF" if the
    // importer's dependency scan, which reads and parses the whole file, ran first)
    dir.write("big.glb", &vec![0u8; 2 << 20]);
    let ev = evaluator(
        &dir,
        r#"<mesh id="big" src="big.glb"/><voxelAsset id="v" fromMesh="big" cellSize="10" maxMemoryMiB="1"/>"#,
    );
    let error = load(ev.program(), "v").unwrap_err();
    assert!(error.contains("maxMemoryMiB") && error.contains("2097152"), "{error}");

    // a model that is cached is returned by the size and the time of its files, with no parse of the mesh at all: the .gltf is made into
    // something that no importer reads, of the same length and with the time that it had, and the call still gives the model
    let cube = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cube-1m.glb")).unwrap();
    let path = dir.write("cube.glb", &cube);
    let ev = evaluator(&dir, r#"<mesh id="m" src="cube.glb"/><voxelAsset id="v" fromMesh="m" cellSize="10"/>"#);
    let first = load(ev.program(), "v").unwrap();
    let time = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, vec![b'x'; cube.len()]).unwrap();
    restamp(&path, time);
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "v").unwrap()));
}

#[test]
fn a_file_that_a_mesh_names_and_that_is_not_there_yet_is_part_of_its_source() {
    let dir = Dir::new("missing-dependency");
    // a cube as an .obj that names a material library which does not exist: the importer goes on without it
    let corners: Vec<String> = (0..8).map(|i| format!("v {} {} {}", i & 1, i >> 1 & 1, i >> 2 & 1)).collect();
    let quads = [[1, 3, 4, 2], [5, 6, 8, 7], [1, 2, 6, 5], [3, 7, 8, 4], [1, 5, 7, 3], [2, 4, 8, 6]];
    let faces: Vec<String> = quads.iter().map(|q| format!("f {} {} {} {}", q[0], q[1], q[2], q[3])).collect();
    let obj = format!("mtllib stone.mtl\n{}\nusemtl stone\n{}\n", corners.join("\n"), faces.join("\n"));
    dir.write("cube.obj", obj.as_bytes());
    let ev = evaluator(&dir, r#"<mesh id="m" src="cube.obj"/><voxelAsset id="v" fromMesh="m" cellSize="10"/>"#);
    let first = load(ev.program(), "v").unwrap();
    assert_eq!(first.occupancy.count(), 1000);
    assert!(Arc::ptr_eq(&first, &load(ev.program(), "v").unwrap()));
    // the library is made: the source is not the one that was read, whatever the mesh file did, and the model is read again
    dir.write("stone.mtl", b"newmtl stone\nKd 0.5 0.5 0.5\n");
    let second = load(ev.program(), "v").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.source.sha256, second.source.sha256);
    assert_eq!(second.occupancy.count(), 1000);
}

#[test]
fn a_box_as_wide_as_an_occupancy_can_be_is_moved_to_the_origin_by_a_copy() {
    let dir = Dir::new("widest-copy");
    let key_limit = sr_3d::occupancy::KEY_LIMIT;
    let mut grid = sr_volume::SparseGrid::new(sr_volume::Transform::identity(), 0.0, 1000).unwrap();
    // 2^30 cells wide from the first key of an occupancy to the key before the origin, so the corner is not at the origin and the cells are moved
    for c in [[-key_limit, 0, 0], [-1, 0, 0]] {
        grid.set(c, 5.0).unwrap();
    }
    let mut volume = sr_volume::Volume::new();
    volume.insert("voxels", grid).unwrap();
    let mut bytes = Vec::new();
    volume.write(&mut bytes).unwrap();
    dir.write("widest.srvol", &bytes);
    let ev = evaluator(&dir, r#"<voxelAsset id="widest" src="widest.srvol"/>"#);
    let widest = load(ev.program(), "widest").unwrap();
    assert_eq!(widest.origin_cells, [-i64::from(key_limit), 0, 0]);
    assert_eq!(widest.occupancy.bounds().unwrap(), ([0, 0, 0], [key_limit - 1, 0, 0]));
    assert_eq!((widest.occupancy.get([0, 0, 0]), widest.occupancy.get([key_limit - 1, 0, 0])), (5, 5));
}
