//! A fracture in an object of cells, end to end through the evaluator: the object breaks at its time into the pieces of its partition, and what is too
//! small is dust.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{evaluator, Dir};
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::{Evaluator, FrameNode};
use std::collections::BTreeSet;

/// A block of 12 by 6 by 6 cells of a quarter of a metre; the cells of the left third have the palette index 1 and the rest 2.
fn block() -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..6 {
        for j in 0..6 {
            for i in 0..12 {
                cells.push(([i, j, k], if i < 4 { 1u8 } else { 2 }));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

fn document(rigid: &str, fracture: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="block.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="block" primitive="voxels" voxels="model" material="stone" x="0" y="0" z="0">
            <rigidBody density="2400" velocityX="1" velocityY="0.4" velocityZ="-0.3" angularVelocityY="30" restitution="0" friction="0" linearDamping="0" angularDamping="0" {rigid}/>
            <fracture {fracture}/>
          </object3D>
        </composition>
        <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##
    )
}

fn setup(name: &str, rigid: &str, fracture: &str) -> (Dir, Evaluator) {
    let dir = Dir::new(name);
    std::fs::write(dir.0.join("block.srvol"), srvol::write(&block(), 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &document(rigid, fracture));
    (dir, ev)
}

fn block_node(ev: &Evaluator, t: f64) -> FrameNode {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "block").expect("the block").clone()
}

fn cells_of(grid: &Occupancy) -> BTreeSet<[i32; 3]> {
    grid.cells().collect()
}

#[test]
fn a_block_breaks_at_its_time_into_the_pieces_of_its_planes_and_the_small_ones_are_dust() {
    // the planes x >= 0.5 m and x >= 1.5 m cut the block of 3 m into columns of 2, 4 and 6 cells (72, 144 and 216 cells); under 100 cells is dust
    let (_dir, ev) =
        setup("planes", r#"fragmentMinCells="100""#, r#"at="0.2" partition="planes" planes="1 0 0 0.5 1 0 0 1.5""#);
    let before = block_node(&ev, 0.1);
    let state = before.voxels.as_ref().expect("an object of cells that breaks says what it is");
    assert!(state.enabled && state.pieces.is_empty(), "the block is whole until its time");
    assert_eq!(state.grid.count(), 432);
    let after = block_node(&ev, 0.3);
    let state = after.voxels.as_ref().unwrap();
    assert!(!state.enabled, "the block has given way to its pieces");
    let counts: Vec<u64> = state.pieces.iter().map(|p| p.grid.count()).collect();
    assert_eq!(counts, vec![144, 216], "the column of 72 cells is dust");
    assert!(state.pieces.iter().all(|p| p.enabled));
    // the pieces and the dust are the cells of the block, each once
    let mut all: BTreeSet<[i32; 3]> = BTreeSet::new();
    for piece in &state.pieces {
        for c in piece.grid.cells() {
            assert!(all.insert(c), "{c:?} is in two pieces");
        }
    }
    let dust: BTreeSet<[i32; 3]> = cells_of(&block()).difference(&all).copied().collect();
    assert_eq!(dust.len(), 72);
    assert!(dust.iter().all(|c| c[0] < 2), "the dust is the first column");
    // each cell keeps its palette index
    for piece in &state.pieces {
        for c in piece.grid.cells() {
            assert_eq!(piece.grid.get(c), block().get(c));
        }
    }
    // they move as the block did (it moves and turns): the first piece is not where the second is, and both are on their way
    let (a, b) = (&state.pieces[0].pose3, &state.pieces[1].pose3);
    assert_ne!(a.map(f64::to_bits), b.map(f64::to_bits));
}

#[test]
fn a_block_breaks_by_seeds_into_pieces_that_are_all_of_it_and_by_material_along_its_materials() {
    let (_dir, ev) = setup("seeds", "", r#"at="0.2" pieces="5" seed="7""#);
    let state = block_node(&ev, 0.3).voxels.unwrap();
    assert!(state.pieces.len() >= 2 && state.pieces.len() <= 64, "{} pieces", state.pieces.len());
    let total: u64 = state.pieces.iter().map(|p| p.grid.count()).sum();
    assert_eq!(total, 432, "no dust with the least of one cell: every cell is in a piece");
    let (_dir, ev) = setup("labels", "", r#"at="0.2" partition="labels" labels="material""#);
    let state = block_node(&ev, 0.3).voxels.unwrap();
    let counts: Vec<u64> = state.pieces.iter().map(|p| p.grid.count()).collect();
    assert_eq!(
        counts,
        vec![144, 288],
        "the first four columns are of the first material and the other eight of the second"
    );
    for piece in &state.pieces {
        let indices: BTreeSet<u8> = piece.grid.cells().map(|c| piece.grid.get(c)).collect();
        assert_eq!(indices.len(), 1, "a piece is of one material");
    }
}

#[test]
fn more_pieces_than_maxfragments_is_a_failure_that_names_both_numbers_and_dust_if_the_document_says_so() {
    let (_dir, ev) =
        setup("overflow", r#"maxFragments="2""#, r#"at="0.2" partition="planes" planes="1 0 0 0.5 1 0 0 1.5""#);
    let frame = ev.evaluate(0.3);
    assert!(
        frame
            .failures
            .iter()
            .chain(&frame.problems)
            .any(|f| f.contains("3 pieces") && f.contains("2 slots") && f.contains("maxFragments")),
        "{:?} {:?}",
        frame.failures,
        frame.problems
    );
    let (_dir, ev) = setup(
        "overflow-dust",
        r#"maxFragments="2" fragmentOverflow="dust""#,
        r#"at="0.2" partition="planes" planes="1 0 0 0.5 1 0 0 1.5""#,
    );
    let state = block_node(&ev, 0.3).voxels.unwrap();
    let counts: Vec<u64> = state.pieces.iter().map(|p| p.grid.count()).collect();
    assert_eq!(counts, vec![144, 216], "the smallest is dust");
}

#[test]
fn the_pieces_of_a_block_that_breaks_carry_the_momentum_it_had_end_to_end() {
    // no dust: the pieces' centres of mass move so that the sum of their momenta is the block's, which was moving and turning
    let (_dir, ev) = setup("momentum", "", r#"at="0.2" pieces="5" seed="7""#);
    // frames are of the steps of 1/240 s: six steps apart, so that the quotient is the velocity of the step and not of a rounded time
    let h = 6.0 / 240.0;
    let of = |t: f64| -> Vec<[f64; 3]> {
        let state = block_node(&ev, t).voxels.unwrap();
        state
            .pieces
            .iter()
            .map(|piece| {
                let cells: Vec<[i32; 3]> = piece.grid.cells().collect();
                let mass = cells.len() as f64 * 2400.0 * 0.25f64.powi(3);
                let q = sr_sim::physics3d::shape_mass_properties(
                    &sr_sim::physics3d::Shape3::Voxels { size: [0.25; 3], cells },
                    mass,
                    1.0,
                )
                .unwrap();
                let m = piece.pose3;
                std::array::from_fn(|i| {
                    m[i] * q.centre[0] + m[4 + i] * q.centre[1] + m[8 + i] * q.centre[2] + m[12 + i]
                })
            })
            .collect()
    };
    let (a, b) = (of(0.3 + 1e-9), of(0.3 + h + 1e-9));
    let masses: Vec<f64> = block_node(&ev, 0.3)
        .voxels
        .unwrap()
        .pieces
        .iter()
        .map(|p| p.grid.count() as f64 * 2400.0 * 0.25f64.powi(3))
        .collect();
    let total: f64 = masses.iter().sum();
    assert!((total - 432.0 * 2400.0 * 0.25f64.powi(3)).abs() < 1e-6 * total);
    let mut momentum = [0.0; 3];
    for k in 0..a.len() {
        for i in 0..3 {
            momentum[i] += masses[k] * (b[k][i] - a[k][i]) / h;
        }
    }
    for (i, v) in [1.0, 0.4, -0.3].iter().enumerate() {
        assert!((momentum[i] - total * v).abs() < 1e-6 * total, "axis {i}: {} against {}", momentum[i], total * v);
    }
}
