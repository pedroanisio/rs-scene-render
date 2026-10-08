//! A body of cells in a document, end to end through the evaluator: its mass is its cells', and a document with no cell bodies is what it was.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::Evaluator;
use std::path::PathBuf;

/// A directory of its own for one test, removed with it.
pub(crate) struct Dir(pub PathBuf);

/// The names that the directories of this process were made with: the tests of a process run at once and a name is one directory, which `new` empties and
/// `drop` removes, so two tests that take the same name take each other's files away.
static NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

impl Dir {
    pub(crate) fn new(name: &str) -> Dir {
        {
            let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                !names.iter().any(|n| n == name),
                "the directory name {name:?} is taken twice in this process: the tests that run at once would share it"
            );
            names.push(name.to_string());
        }
        let path = std::env::temp_dir().join(format!("voxel-scene-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A block of `n` cells on a side, all the same material.
pub(crate) fn cube(n: i32) -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                cells.push(([i, j, k], 1u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

/// An evaluator for `xml` with its files in `dir`.
pub(crate) fn evaluator(dir: &Dir, xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir.0.clone()) })
        .unwrap_or_else(|e| panic!("{e:?}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Every frame of the document, to the last, has no failure and no problem: an end-to-end test that looks at chosen instants misses what happens after the
/// impact of its own document (a crater of a body no larger than its projectile failed in the last frame of a corpus document that was only looked at in the
/// first and the middle ones).
pub(crate) fn assert_every_frame_is_clean(ev: &Evaluator, what: &str) {
    for n in 0..ev.frame_count() {
        let frame = ev.evaluate_frame(n);
        assert!(
            frame.failures.is_empty() && frame.problems.is_empty(),
            "{what}, frame {n}: {:?} {:?}",
            frame.failures,
            frame.problems
        );
    }
}

/// Where the body `id` is at `t` (the translation of its world matrix, scene units).
pub(crate) fn position(ev: &Evaluator, id: &str, t: f64) -> [f64; 3] {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    let m = frame
        .nodes
        .iter()
        .find(|n| &*n.id == id)
        .unwrap_or_else(|| panic!("no node {id}"))
        .pose3
        .expect("a body has a pose");
    [m[12], m[13], m[14]]
}

fn block(extra: &str) -> String {
    format!(
        r#"<object3D id="block" primitive="voxels" voxels="model" material="stone" {extra}><rigidBody density="2400" restitution="0" friction="0" linearDamping="0" angularDamping="0"/></object3D>"#
    )
}

fn hit_document(extra: &str) -> String {
    let block = block(extra);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="block.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="ball" primitive="sphere" radius="0.5" x="-3" y="1" z="1">
            <rigidBody shape="sphere" mass="1000" velocityX="10" restitution="0" friction="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          {block}
        </composition>
        <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##
    )
}

#[test]
fn a_block_of_cells_has_the_mass_of_its_cells_as_a_ball_that_hits_it_finds() {
    // a ball of 1000 kg at 10 m/s hits, head on and with no restitution or friction, a block of 8 by 8 by 8 cells of a quarter of a metre at
    // 2400 kg/m3 (19 200 kg), in a world with no gravity: they go on together at m v / (m + M)
    let dir = Dir::new("mass");
    std::fs::write(dir.0.join("block.srvol"), srvol::write(&cube(8), 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &hit_document(""));
    let (t0, t1) = (1.0, 1.4);
    let (b0, b1) = (position(&ev, "ball", t0), position(&ev, "ball", t1));
    let (c0, c1) = (position(&ev, "block", t0), position(&ev, "block", t1));
    let ball = (b1[0] - b0[0]) / (t1 - t0);
    let block = (c1[0] - c0[0]) / (t1 - t0);
    let wanted = 1000.0 * 10.0 / (1000.0 + 19_200.0);
    assert!((ball - wanted).abs() < 1e-4 * wanted, "the ball goes at {ball}, together they should go at {wanted}");
    assert!((block - wanted).abs() < 1e-4 * wanted, "the block goes at {block}, together they should go at {wanted}");
    // the mass that the exchange says
    let mass = 1000.0 * (10.0 - block) / block;
    println!("VOXEL BODY ball {ball}, block {block}, together at {wanted}; the exchange says {mass} kg for 19 200");
    assert!((mass - 19_200.0).abs() < 1e-4 * 19_200.0, "the block weighs {mass} kg for 19 200");
}

#[test]
fn the_cells_of_a_body_are_scaled_by_each_axis_and_a_mirrored_body_of_cells_is_refused() {
    // twice as wide along x (the axis the ball comes along): the block is 4 m by 2 m by 2 m, 38 400 kg, and the cells are not cubes
    let dir = Dir::new("scale");
    std::fs::write(dir.0.join("block.srvol"), srvol::write(&cube(8), 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &hit_document(r#"scaleX="2""#));
    let (t0, t1) = (1.0, 1.4);
    let block = (position(&ev, "block", t1)[0] - position(&ev, "block", t0)[0]) / (t1 - t0);
    let wanted = 1000.0 * 10.0 / (1000.0 + 38_400.0);
    assert!((block - wanted).abs() < 1e-4 * wanted, "the block goes at {block}, together they should go at {wanted}");
    // a negative scale would turn the lattice inside out
    let ev = evaluator(&dir, &hit_document(r#"scaleX="-1""#));
    let frame = ev.evaluate(0.5);
    assert!(
        frame.failures.iter().any(|f| f.contains("block") && f.contains("mirrored")),
        "{:?} {:?}",
        frame.failures,
        frame.problems
    );
}
