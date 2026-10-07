//! What a body of cells becomes when cells are taken out of it: the body for the world, and the cut that says what stays, what
//! separates (into the slots) and what is too small to be a body.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_eval::voxels::{body, cut, Overflow, Policy, Stay};
use sr_sim::physics3d::Shape3;
use std::collections::BTreeSet;

const SIZE: [f64; 3] = [0.25, 0.25, 0.25];
const DENSITY: f64 = 2400.0;
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

fn cells_of(xs: std::ops::Range<i32>, ys: std::ops::Range<i32>, zs: std::ops::Range<i32>) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for z in zs {
        for y in ys.clone() {
            for x in xs.clone() {
                cells.push([x, y, z]);
            }
        }
    }
    cells
}

fn grid(cells: &[[i32; 3]]) -> Occupancy {
    Occupancy::from_cells(cells.iter().map(|c| (*c, 1u8))).unwrap()
}

fn policy() -> Policy {
    Policy { stay: Stay::Largest, min_cells: 1, max_fragments: 8, overflow: Overflow::Error }
}

fn nobody(_: &[i32; 3]) -> bool {
    false
}

#[test]
fn a_body_from_an_occupancy_has_the_mass_of_its_cells_and_its_cells_in_the_order_of_the_scan() {
    let cells = cells_of(-2..3, 0..2, 4..7);
    let b = body(&grid(&cells), SIZE, DENSITY, 1.0).unwrap();
    assert!((b.mass - 30.0 * CELL_MASS).abs() < 1e-12 * b.mass);
    let Shape3::Voxels { size, cells: listed } = &b.shape else { panic!("not cells") };
    assert_eq!(size, &SIZE);
    let mut scan = cells.clone();
    scan.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(listed, &scan);
    // a metre is `ppm` scene units: the same cells of 25 scene units at 100 a metre are the same mass
    let scaled = body(&grid(&cells), SIZE.map(|s| s * 100.0), DENSITY, 100.0).unwrap();
    assert!((scaled.mass - b.mass).abs() < 1e-9 * b.mass);
    assert!(body(&Occupancy::new(), SIZE, DENSITY, 1.0).is_err());
    assert!(body(&grid(&cells), SIZE, -1.0, 1.0).is_err());
    assert!(body(&grid(&cells), [0.0, 1.0, 1.0], DENSITY, 1.0).is_err());
}

fn parts(c: &sr_eval::voxels::Cut, before: &[[i32; 3]]) -> BTreeSet<[i32; 3]> {
    // everything that the cut accounts for, once: the destroyed cells (the dust among them), the pieces and what stays
    let mut accounted: Vec<[i32; 3]> = c.cut.destroyed.clone();
    accounted.extend(c.cut.pieces.iter().flat_map(|p| p.cells.iter().copied()));
    accounted.extend(c.stays.iter().copied());
    let set: BTreeSet<[i32; 3]> = accounted.iter().copied().collect();
    assert_eq!(set.len(), accounted.len(), "no cell is in two parts");
    assert_eq!(set, before.iter().copied().collect::<BTreeSet<_>>(), "no cell is lost");
    set
}

#[test]
fn a_bar_cut_in_two_keeps_the_larger_part_and_gives_the_other_to_a_slot_and_every_cell_and_kilogram_is_accounted_for() {
    let bar = cells_of(0..12, 0..2, 0..2);
    let destroyed = cells_of(5..6, 0..2, 0..2);
    let c = cut(&grid(&bar), &destroyed, 7, SIZE, DENSITY, 1.0, &policy(), &nobody).unwrap();
    assert_eq!(c.cut.revision, 7);
    assert_eq!(c.cut.pieces.len(), 1);
    assert_eq!(c.cut.pieces[0].cells.len(), 20, "the part with x under 5 separates");
    assert_eq!(c.stays.len(), 24, "the larger part stays");
    assert!(c.dust.is_empty());
    parts(&c, &bar);
    // the masses add up: what stays, what separates and what is destroyed are the body
    let total =
        c.cut.parent_mass + c.cut.pieces.iter().map(|p| p.mass).sum::<f64>() + destroyed.len() as f64 * CELL_MASS;
    assert!((total - 48.0 * CELL_MASS).abs() < 1e-12 * total, "{total} against {}", 48.0 * CELL_MASS);
    assert!((c.cut.parent_mass - 24.0 * CELL_MASS).abs() < 1e-12 * total);
    // cells given that are not in the body are ignored
    let c = cut(
        &grid(&bar),
        &[[5, 0, 0], [5, 1, 0], [5, 0, 1], [5, 1, 1], [40, 0, 0]],
        8,
        SIZE,
        DENSITY,
        1.0,
        &policy(),
        &nobody,
    )
    .unwrap();
    assert_eq!(c.cut.destroyed.len(), 4);
}

#[test]
fn what_is_too_small_to_be_a_body_is_dust_and_nothing_is_lost() {
    // a bar with a two-cell island that the cut leaves alone, and a 3-cell piece
    let mut cells = cells_of(0..12, 0..2, 0..2);
    cells.extend([[20, 0, 0], [21, 0, 0]]);
    cells.extend([[30, 0, 0], [31, 0, 0], [32, 0, 0]]);
    let destroyed = cells_of(5..6, 0..2, 0..2);
    let p = Policy { min_cells: 4, ..policy() };
    let c = cut(&grid(&cells), &destroyed, 1, SIZE, DENSITY, 1.0, &p, &nobody).unwrap();
    assert_eq!(c.cut.pieces.len(), 1, "the part of 20 cells, and not the islands of 2 and of 3");
    assert_eq!(c.dust.len(), 5);
    parts(&c, &cells);
    // with a lower floor the island of three is a body
    let p = Policy { min_cells: 3, ..policy() };
    let c = cut(&grid(&cells), &destroyed, 1, SIZE, DENSITY, 1.0, &p, &nobody).unwrap();
    assert_eq!(c.cut.pieces.len(), 2);
    assert_eq!(c.dust.len(), 2);
    parts(&c, &cells);
}

#[test]
fn more_pieces_than_slots_is_an_error_or_the_smallest_become_dust_and_the_choice_is_deterministic() {
    // seven separate parts of 1, 2, 3, ... cells in a row, a cut that takes nothing of them
    let mut cells = Vec::new();
    for k in 0..7 {
        for x in 0..=k {
            cells.push([x, 3 * k, 0]);
        }
    }
    let nothing: [[i32; 3]; 0] = [];
    let policy = |overflow, max_fragments| Policy { max_fragments, overflow, ..policy() };
    let err = cut(&grid(&cells), &nothing, 1, SIZE, DENSITY, 1.0, &policy(Overflow::Error, 3), &nobody);
    assert!(err.is_err(), "six pieces besides the one that stays, three slots");
    let c = cut(&grid(&cells), &nothing, 1, SIZE, DENSITY, 1.0, &policy(Overflow::Dust, 3), &nobody).unwrap();
    // the part that stays is the largest (7 cells), the three largest of the rest separate (6, 5, 4), the rest are dust (3 + 2 + 1)
    assert_eq!(c.stays.len(), 7);
    let mut sizes: Vec<usize> = c.cut.pieces.iter().map(|p| p.cells.len()).collect();
    sizes.sort();
    assert_eq!(sizes, vec![4, 5, 6]);
    assert_eq!(c.dust.len(), 6);
    parts(&c, &cells);
    // the pieces are in the order of their first cell in the scan; the same whatever order the cells were put in
    let again = {
        let mut shuffled = cells.clone();
        shuffled.reverse();
        cut(&grid(&shuffled), &nothing, 1, SIZE, DENSITY, 1.0, &policy(Overflow::Dust, 3), &nobody).unwrap()
    };
    assert_eq!(again, c);
    let firsts: Vec<[i32; 3]> = c.cut.pieces.iter().map(|p| p.cells[0]).collect();
    let mut by_scan = firsts.clone();
    by_scan.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(firsts, by_scan);
    // equal parts: the choice is by position, the same every time
    let tie: Vec<[i32; 3]> = (0..5).flat_map(|k| [[0, 3 * k, 0], [1, 3 * k, 0]]).collect();
    let a = cut(&grid(&tie), &nothing, 1, SIZE, DENSITY, 1.0, &policy(Overflow::Dust, 2), &nobody).unwrap();
    // equal parts put in in any order give the same cut: the choice is by position and not by the order of arrival
    for seed in 1..=6u64 {
        let mut shuffled = tie.clone();
        let mut state = seed;
        for i in (1..shuffled.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            shuffled.swap(i, (state >> 33) as usize % (i + 1));
        }
        let b = cut(&grid(&shuffled), &nothing, 1, SIZE, DENSITY, 1.0, &policy(Overflow::Dust, 2), &nobody).unwrap();
        assert_eq!(a, b, "shuffle {seed}");
    }
    // the kept pieces are the first two of the equal ones in the scan, the rest dust
    assert_eq!(a.cut.pieces.len(), 2);
    assert_eq!(a.cut.pieces[0].cells, vec![[0, 3, 0], [1, 3, 0]]);
    assert_eq!(a.cut.pieces[1].cells, vec![[0, 6, 0], [1, 6, 0]]);
    assert_eq!(a.dust.len(), 4);
    assert_eq!(a.stays, vec![[0, 0, 0], [1, 0, 0]], "the first of equals stays");
}

#[test]
fn a_body_that_is_held_keeps_every_part_that_touches_the_anchor() {
    // a slab of terrain 20 by 6 by 1 whose bottom row (y = 5) is held; a cut across it leaves two parts held and one that is not
    let slab = cells_of(0..20, 0..6, 0..1);
    let destroyed: Vec<[i32; 3]> = [[8, 2, 0], [8, 3, 0], [8, 4, 0], [8, 5, 0], [8, 0, 0], [8, 1, 0]].to_vec();
    let held = |c: &[i32; 3]| c[1] == 5;
    let p = Policy { stay: Stay::Anchored, ..policy() };
    let c = cut(&grid(&slab), &destroyed, 1, SIZE, DENSITY, 1.0, &p, &held).unwrap();
    assert!(c.cut.pieces.is_empty(), "both halves reach the held row, so nothing is loose");
    assert_eq!(c.stays.len(), 20 * 6 - 6);
    // a part of the slab cut free of the row is a piece, and the held parts stay although they are two
    let destroyed: Vec<[i32; 3]> =
        (0..6).flat_map(|y| [[8, y, 0], [12, y, 0]]).chain((8..13).map(|x| [x, 4, 0])).collect();
    let c = cut(&grid(&slab), &destroyed, 1, SIZE, DENSITY, 1.0, &p, &held).unwrap();
    // the cells at x 9..12, y 0..4 touch the destroyed row y = 4 below them: free
    assert_eq!(c.cut.pieces.len(), 1);
    assert_eq!(c.cut.pieces[0].cells.len(), 3 * 4);
    parts(&c, &slab);
}
