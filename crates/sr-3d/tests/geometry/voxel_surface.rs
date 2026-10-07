//! The surface of a grid of voxels: which faces of its cells are exposed, and the quads they merge into. The oracles are counts that
//! can be worked out by hand and a naive extraction (the six neighbours of every cell, read one by one) that shares nothing with the
//! module; `tools/voxel_reference_mesher.py` is the same rule in another language.

use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::surface::{exposed_faces, Classes, Face};
use std::collections::BTreeSet;

/// The cells of a box of `n` cells a side with its corner at `at`, all of palette index `index`.
fn block(at: [i32; 3], n: i32, index: u8) -> Vec<([i32; 3], u8)> {
    let mut cells = Vec::new();
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                cells.push(([at[0] + x, at[1] + y, at[2] + z], index));
            }
        }
    }
    cells
}

/// A block of `n` with a square hole of `h` cells a side right through it along z, `a` cells from the faces it does not open.
fn holed(n: i32, h: i32, a: i32) -> Vec<([i32; 3], u8)> {
    block([0, 0, 0], n, 1)
        .into_iter()
        .filter(|(c, _)| !((a..a + h).contains(&c[0]) && (a..a + h).contains(&c[1])))
        .collect()
}

fn grid(cells: Vec<([i32; 3], u8)>) -> Occupancy {
    Occupancy::from_cells(cells).unwrap()
}

/// Every exposed face of `grid` by the rule, read cell by cell and neighbour by neighbour: the face of cell A toward B is exposed when
/// B is empty, or A is opaque and B is see-through, or both are see-through of different classes and A's class is the lesser.
fn naive(grid: &Occupancy, classes: &Classes) -> BTreeSet<Face> {
    let mut out = BTreeSet::new();
    for cell in grid.cells() {
        let a = classes.class(grid.get(cell));
        for axis in 0..3usize {
            for positive in [false, true] {
                let mut other = cell;
                other[axis] += if positive { 1 } else { -1 };
                let b = classes.class(grid.get(other));
                let (sa, sb) = (classes.see_through(a), classes.see_through(b));
                let exposed = b == 0 || (!sa && sb) || (sa && sb && a != b && a < b);
                if exposed {
                    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                    let plane = cell[axis] + i32::from(positive);
                    out.insert(Face { axis: axis as u8, positive, plane, u: cell[u], v: cell[v], class: a });
                }
            }
        }
    }
    out
}

fn faces(grid: &Occupancy, classes: &Classes) -> BTreeSet<Face> {
    exposed_faces(grid, classes).into_iter().collect()
}

#[test]
fn a_single_cell_has_six_faces_one_for_each_direction_in_the_coordinates_of_their_plane() {
    let g = grid(vec![([2, 3, 4], 1)]);
    let f = exposed_faces(&g, &Classes::identity());
    assert_eq!(f.len(), 6);
    // the plane of a face is the integer coordinate it lies on along its axis; u and v are the two other coordinates in cyclic order
    // (axis 0: y, z; axis 1: z, x; axis 2: x, y)
    let want = [
        Face { axis: 0, positive: false, plane: 2, u: 3, v: 4, class: 1 },
        Face { axis: 0, positive: true, plane: 3, u: 3, v: 4, class: 1 },
        Face { axis: 1, positive: false, plane: 3, u: 4, v: 2, class: 1 },
        Face { axis: 1, positive: true, plane: 4, u: 4, v: 2, class: 1 },
        Face { axis: 2, positive: false, plane: 4, u: 2, v: 3, class: 1 },
        Face { axis: 2, positive: true, plane: 5, u: 2, v: 3, class: 1 },
    ];
    assert_eq!(f.into_iter().collect::<BTreeSet<_>>(), want.into_iter().collect());
}

#[test]
fn an_empty_grid_has_no_faces() {
    assert!(exposed_faces(&Occupancy::new(), &Classes::identity()).is_empty());
}

#[test]
fn two_cells_that_touch_hide_the_faces_between_them_unless_one_is_see_through() {
    let domino = |a: u8, b: u8| grid(vec![([0, 0, 0], a), ([1, 0, 0], b)]);
    // opaque against opaque: 5 + 5 faces
    assert_eq!(exposed_faces(&domino(1, 2), &Classes::identity()).len(), 10);
    // an opaque cell against a see-through one (index 2 is glass): the opaque one keeps the face toward the glass and the glass has none
    // toward it, so 6 + 5
    let glass = Classes::identity().with_see_through(&[2]);
    let f = exposed_faces(&domino(1, 2), &glass);
    assert_eq!(f.len(), 11);
    assert!(
        f.iter().any(|f| f.axis == 0 && f.positive && f.plane == 1 && f.class == 1),
        "the opaque cell's face toward the glass"
    );
    assert!(
        !f.iter().any(|f| f.axis == 0 && !f.positive && f.plane == 1),
        "the glass has no face toward the opaque cell"
    );
    // the other way round the same: the opaque cell is the one with the face
    let f = exposed_faces(&domino(2, 1), &glass);
    assert_eq!(f.len(), 11);
    assert!(f.iter().any(|f| f.axis == 0 && !f.positive && f.plane == 1 && f.class == 1));
    // two glasses of one class are one body of glass: no face between them; of two classes the lesser class owns the one face
    let two = Classes::identity().with_see_through(&[1, 2]);
    assert_eq!(exposed_faces(&domino(1, 1), &two).len(), 10);
    let f = exposed_faces(&domino(1, 2), &two);
    assert_eq!(f.len(), 11);
    assert!(f.iter().any(|f| f.axis == 0 && f.positive && f.plane == 1 && f.class == 1), "the lesser class owns it");
}

#[test]
fn a_block_of_n_cubed_has_six_n_squared_faces_wherever_it_is_even_across_the_bricks_of_negative_keys() {
    for n in [1, 2, 3, 7, 8, 9, 16, 17] {
        for at in [[0, 0, 0], [-3, 5, -7], [5, 5, 5], [-8, -8, -8], [7, -9, 15]] {
            let g = grid(block(at, n, 1));
            assert_eq!(exposed_faces(&g, &Classes::identity()).len() as i32, 6 * n * n, "n = {n} at {at:?}");
        }
    }
}

#[test]
fn a_block_with_a_square_hole_through_it_has_the_faces_of_the_outside_and_of_the_hole() {
    // 6 n^2 outside less the two openings of h^2 plus the four walls of the hole of h by n
    for (n, h, a) in [(3, 1, 1), (4, 2, 1), (8, 2, 3), (8, 4, 2), (9, 3, 1), (16, 4, 6)] {
        let g = grid(holed(n, h, a));
        assert_eq!(
            exposed_faces(&g, &Classes::identity()).len() as i32,
            6 * n * n - 2 * h * h + 4 * h * n,
            "n {n} h {h} a {a}"
        );
    }
}

#[test]
fn a_hole_through_a_corner_opens_two_sides_and_has_only_two_walls() {
    // the walls of the hole are two of h by n and the outside loses two faces of h by n: 6 n^2 - 2 h^2
    for (n, h) in [(9, 3), (8, 2), (5, 1)] {
        let g = grid(holed(n, h, 0));
        assert_eq!(exposed_faces(&g, &Classes::identity()).len() as i32, 6 * n * n - 2 * h * h, "n {n} h {h}");
    }
}

#[test]
fn cells_that_touch_only_by_edges_or_corners_hide_nothing_of_each_other() {
    // a checkerboard: every cell has all six faces
    let cells: Vec<_> =
        block([-2, 0, 1], 6, 1).into_iter().filter(|(c, _)| (c[0] + c[1] + c[2]).rem_euclid(2) == 0).collect();
    let n = cells.len();
    assert_eq!(exposed_faces(&grid(cells), &Classes::identity()).len(), 6 * n);
}

#[test]
fn the_faces_are_those_of_the_naive_extraction_on_clouds_with_glass_of_several_classes() {
    let mut seed = 0x2545f4914f6cdd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for density in [20u64, 50, 80] {
        let mut cells = Vec::new();
        for z in -9..11 {
            for y in -5..13 {
                for x in -11..9 {
                    if next() % 100 < density {
                        cells.push(([x, y, z], 1 + (next() % 5) as u8));
                    }
                }
            }
        }
        let g = grid(cells);
        // 2 and 3 are one glass and 4 another: see-through, of the classes 2 and 4 (3 is class 2 too)
        let classes = Classes::new(
            |i| match i {
                3 => 2,
                i => i,
            },
            &[2, 4],
        );
        assert_eq!(faces(&g, &classes), naive(&g, &classes), "density {density}");
        let opaque = Classes::identity();
        assert_eq!(faces(&g, &opaque), naive(&g, &opaque), "density {density}, all opaque");
    }
}

#[test]
fn the_faces_do_not_depend_on_the_order_the_cells_were_given_in() {
    let cells = block([-4, -4, -4], 9, 1);
    let mut shuffled = cells.clone();
    let mut seed = 7u64;
    for i in (1..shuffled.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        shuffled.swap(i, (seed >> 33) as usize % (i + 1));
    }
    assert_eq!(
        exposed_faces(&grid(cells), &Classes::identity()),
        exposed_faces(&grid(shuffled), &Classes::identity()),
        "the same list, in the same order"
    );
}
