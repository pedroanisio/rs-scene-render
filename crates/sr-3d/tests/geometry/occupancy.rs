//! The occupancy of a voxel object: the contract of the grid (cells, palette, revisions, bricks), the exact mass properties of
//! a union of cubes, and the connected components of the cells that are left.
// the tensors are compared entry by entry, which reads best by index
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::{components, Moments, Occupancy};

/// Cells of a box of `a` by `b` by `c` cells with its corner at `at`.
fn block(at: [i32; 3], [a, b, c]: [i32; 3]) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for z in 0..c {
        for y in 0..b {
            for x in 0..a {
                cells.push([at[0] + x, at[1] + y, at[2] + z]);
            }
        }
    }
    cells
}

/// A deterministic shuffle (a linear congruential generator over a Fisher-Yates), so that the tests do not depend on a library.
fn shuffled<T: Clone>(items: &[T], mut seed: u64) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        out.swap(i, (seed >> 33) as usize % (i + 1));
    }
    out
}

fn filled(cells: &[[i32; 3]]) -> Occupancy {
    let mut o = Occupancy::new();
    for c in cells {
        o.set(*c, 1).unwrap();
    }
    o
}

#[test]
fn cells_have_a_palette_index_and_zero_is_empty_across_the_bricks_and_negative_keys() {
    let mut o = Occupancy::new();
    assert_eq!(o.count(), 0);
    assert_eq!(o.get([3, -5, 70]), 0);
    assert!(o.set([3, -5, 70], 7).unwrap());
    assert!(o.set([-1, 0, 0], 2).unwrap());
    assert!(o.set([0, 0, 0], 255).unwrap());
    assert_eq!((o.get([3, -5, 70]), o.get([-1, 0, 0]), o.get([0, 0, 0])), (7, 2, 255));
    assert_eq!(o.count(), 3);
    // a cell that is set to what it holds changes nothing; a new palette index is a change; zero empties it
    assert!(!o.set([0, 0, 0], 255).unwrap());
    assert!(o.set([0, 0, 0], 4).unwrap());
    assert_eq!(o.count(), 3);
    assert!(o.set([0, 0, 0], 0).unwrap());
    assert_eq!((o.get([0, 0, 0]), o.count()), (0, 2));
    assert!(!o.set([0, 0, 0], 0).unwrap());
    // cells in the order of the scan: z, then y, then x
    let cells: Vec<[i32; 3]> = o.cells().collect();
    assert_eq!(cells, vec![[-1, 0, 0], [3, -5, 70]]);
}

#[test]
fn the_revision_changes_with_the_content_and_the_fingerprint_is_the_content() {
    let cells = block([-3, -3, -3], [7, 5, 6]);
    let mut a = Occupancy::new();
    let r0 = a.revision();
    for c in &cells {
        a.set(*c, 1).unwrap();
    }
    assert!(a.revision() > r0);
    let r = a.revision();
    assert!(!a.set(cells[10], 1).unwrap());
    assert_eq!(a.revision(), r, "setting a cell to what it holds is not an edit");
    // the same content built in another order, with a different history, has the same fingerprint
    let mut b = Occupancy::new();
    for c in shuffled(&cells, 5) {
        b.set(c, 3).unwrap();
        b.set(c, 1).unwrap();
    }
    assert_ne!(a.revision(), b.revision());
    assert_eq!(a.fingerprint(), b.fingerprint());
    // different content or a different palette index is another fingerprint
    let before = a.fingerprint();
    a.set(cells[0], 2).unwrap();
    assert_ne!(a.fingerprint(), before);
    a.set(cells[0], 1).unwrap();
    assert_eq!(a.fingerprint(), before);
    a.set([100, 100, 100], 1).unwrap();
    assert_ne!(a.fingerprint(), before);
}

#[test]
fn the_bricks_that_changed_since_a_revision_are_told_in_order_emptied_ones_included() {
    let mut o = filled(&block([0, 0, 0], [16, 8, 8]));
    let before = o.revision();
    assert!(o.changed_bricks_since(o.revision()).is_empty());
    o.set([9, 1, 1], 0).unwrap();
    o.set([-9, 0, 0], 1).unwrap();
    assert_eq!(o.changed_bricks_since(before), vec![[-2, 0, 0], [1, 0, 0]]);
    // a brick that was emptied is told too, until it is compacted away
    let mid = o.revision();
    for c in block([0, 0, 0], [8, 8, 8]) {
        o.set(c, 0).unwrap();
    }
    assert!(o.changed_bricks_since(mid).contains(&[0, 0, 0]));
    assert_eq!(o.bricks().filter(|(k, _)| *k == [0, 0, 0]).count(), 0, "an empty brick is not a brick of the content");
    o.compact(o.revision());
    assert!(o.changed_bricks_since(mid).is_empty() || !o.changed_bricks_since(mid).contains(&[0, 0, 0]));
}

#[test]
fn a_grid_with_a_limit_refuses_the_brick_that_goes_over_it() {
    let mut o = Occupancy::with_limit(2);
    assert!(o.set([0, 0, 0], 1).is_ok());
    assert!(o.set([8, 0, 0], 1).is_ok());
    assert!(o.set([9, 0, 0], 1).is_ok(), "the same brick");
    assert!(o.set([16, 0, 0], 1).is_err());
    assert_eq!(o.count(), 3);
    assert!(o.set([0, 0, 0], 0).is_ok());
}

#[test]
fn the_mass_of_a_box_of_cells_is_exact_and_so_are_its_centre_and_its_inertia() {
    // 4 by 3 by 5 cells of 0.5 by 0.25 by 0.2 m, 2400 kg/m3, the corner at (-2, 7, 1) cells
    let (a, b, c) = (4, 3, 5);
    let size = [0.5, 0.25, 0.2];
    let cells = block([-2, 7, 1], [a, b, c]);
    let p = Moments::of(cells.iter().copied()).properties(size, 2400.0).unwrap();
    let (dx, dy, dz) = (a as f64 * size[0], b as f64 * size[1], c as f64 * size[2]);
    let mass = 2400.0 * dx * dy * dz;
    assert!((p.mass - mass).abs() < 1e-12 * mass, "{} against {mass}", p.mass);
    let wanted = [(-2.0 * size[0]) + 0.5 * dx, 7.0 * size[1] + 0.5 * dy, 1.0 * size[2] + 0.5 * dz];
    for k in 0..3 {
        assert!((p.centre[k] - wanted[k]).abs() < 1e-12, "{k}: {} against {}", p.centre[k], wanted[k]);
    }
    let diagonal =
        [mass * (dy * dy + dz * dz) / 12.0, mass * (dx * dx + dz * dz) / 12.0, mass * (dx * dx + dy * dy) / 12.0];
    for k in 0..3 {
        assert!(
            (p.inertia[k][k] - diagonal[k]).abs() < 1e-12 * diagonal[k],
            "{k}: {} against {}",
            p.inertia[k][k],
            diagonal[k]
        );
        for l in 0..3 {
            if k != l {
                assert_eq!(p.inertia[k][l], 0.0, "a box has no products of inertia");
            }
        }
    }
}

#[test]
fn an_l_of_three_cells_has_the_mass_properties_worked_out_by_hand() {
    // cells (0,0,0), (1,0,0), (0,1,0) of unit size and density: m = 3, centre (5/6, 5/6, 1/2); about it
    // Ixx = Iyy = 7/6, Izz = 11/6, Ixy = +1/3 (minus the sum of dx dy, which is -1/3), and none with z
    let p = Moments::of([[0, 0, 0], [1, 0, 0], [0, 1, 0]]).properties([1.0; 3], 1.0).unwrap();
    assert_eq!(p.mass, 3.0);
    for (got, want) in p.centre.iter().zip([5.0 / 6.0, 5.0 / 6.0, 0.5]) {
        assert!((got - want).abs() < 1e-15, "{got} against {want}");
    }
    let wanted = [[7.0 / 6.0, 1.0 / 3.0, 0.0], [1.0 / 3.0, 7.0 / 6.0, 0.0], [0.0, 0.0, 11.0 / 6.0]];
    for k in 0..3 {
        for l in 0..3 {
            assert!(
                (p.inertia[k][l] - wanted[k][l]).abs() < 1e-15,
                "[{k}][{l}]: {} against {}",
                p.inertia[k][l],
                wanted[k][l]
            );
        }
    }
    // the principal axes put the products to zero: the sum of the principal moments is the trace
    let principal = p.principal();
    let trace = principal.moments.iter().sum::<f64>();
    assert!((trace - (7.0 / 6.0 + 7.0 / 6.0 + 11.0 / 6.0)).abs() < 1e-14);
    // R diag R^T is the tensor
    for k in 0..3 {
        for l in 0..3 {
            let rebuilt: f64 = (0..3).map(|j| principal.axes[j][k] * principal.moments[j] * principal.axes[j][l]).sum();
            assert!((rebuilt - p.inertia[k][l]).abs() < 1e-13, "[{k}][{l}]: {rebuilt} against {}", p.inertia[k][l]);
        }
    }
    // a right-handed frame of unit axes, the moments in ascending order
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    assert!((det(&principal.axes) - 1.0).abs() < 1e-13);
    assert!(principal.moments[0] <= principal.moments[1] && principal.moments[1] <= principal.moments[2]);
}

#[test]
fn the_properties_are_the_same_bits_whatever_the_order_of_the_cells() {
    let cells: Vec<[i32; 3]> = block([-4, 2, 0], [5, 4, 3]).into_iter().chain(block([1, 2, 3], [2, 6, 2])).collect();
    let p = Moments::of(cells.iter().copied()).properties([0.3, 0.3, 0.3], 1800.0).unwrap();
    for seed in [1, 2, 3, 99] {
        let q = Moments::of(shuffled(&cells, seed)).properties([0.3, 0.3, 0.3], 1800.0).unwrap();
        assert_eq!(p.mass.to_bits(), q.mass.to_bits());
        for k in 0..3 {
            assert_eq!(p.centre[k].to_bits(), q.centre[k].to_bits());
            for l in 0..3 {
                assert_eq!(p.inertia[k][l].to_bits(), q.inertia[k][l].to_bits());
            }
        }
    }
    // the moments of two parts add to the moments of the whole, exactly
    let (left, right) = cells.split_at(40);
    let mut sum = Moments::of(left.iter().copied());
    sum.add(&Moments::of(right.iter().copied()));
    let whole = Moments::of(cells.iter().copied());
    assert_eq!(sum, whole);
    assert!(Moments::of(std::iter::empty()).properties([1.0; 3], 1.0).is_err());
    assert!(Moments::of([[0, 0, 0]]).properties([0.0, 1.0, 1.0], 1.0).is_err());
    assert!(Moments::of([[0, 0, 0]]).properties([1.0; 3], -1.0).is_err());
}

fn sizes(parts: &[Vec<[i32; 3]>]) -> Vec<usize> {
    parts.iter().map(Vec::len).collect()
}

#[test]
fn two_cubes_that_touch_by_a_face_are_one_and_by_an_edge_or_a_corner_are_two() {
    assert_eq!(sizes(&components(&[[0, 0, 0], [1, 0, 0]])), vec![2]);
    assert_eq!(sizes(&components(&[[0, 0, 0], [1, 1, 0]])), vec![1, 1], "an edge is not a face");
    assert_eq!(sizes(&components(&[[0, 0, 0], [1, 1, 1]])), vec![1, 1], "a corner is not a face");
    assert!(components(&[]).is_empty());
    // across the border of a brick, and across the origin
    assert_eq!(sizes(&components(&[[7, 7, 7], [8, 7, 7], [8, 8, 7], [8, 8, 8]])), vec![4]);
    assert_eq!(sizes(&components(&[[-1, -1, -1], [0, -1, -1], [0, 0, -1], [0, 0, 0]])), vec![4]);
}

#[test]
fn a_ring_is_one_cut_once_it_is_one_cut_twice_it_is_two_and_a_sealed_shell_is_one() {
    // the ring of eight cells around the middle of a square of 3 by 3, in the plane z = 0
    let ring: Vec<[i32; 3]> = block([0, 0, 0], [3, 3, 1]).into_iter().filter(|c| *c != [1, 1, 0]).collect();
    assert_eq!(sizes(&components(&ring)), vec![8]);
    let once: Vec<[i32; 3]> = ring.iter().copied().filter(|c| *c != [1, 0, 0]).collect();
    assert_eq!(sizes(&components(&once)), vec![7], "a ring cut once is a path");
    let twice: Vec<[i32; 3]> = once.iter().copied().filter(|c| *c != [1, 2, 0]).collect();
    let parts = components(&twice);
    assert_eq!(sizes(&parts), vec![3, 3], "two columns of three");
    assert!(
        parts[0].contains(&[0, 0, 0]) && parts[1].contains(&[2, 0, 0]),
        "in the order of their first cell in the scan"
    );
    // a shell of 3 by 3 by 3 with the middle cell out is one component
    let shell: Vec<[i32; 3]> = block([0, 0, 0], [3, 3, 3]).into_iter().filter(|c| *c != [1, 1, 1]).collect();
    assert_eq!(sizes(&components(&shell)), vec![26]);
    // a staircase of n cells that touch by edges only is n components
    let stairs: Vec<[i32; 3]> = (0..9).map(|i| [i, i, 0]).collect();
    assert_eq!(sizes(&components(&stairs)), vec![1; 9]);
}

#[test]
fn components_are_labelled_by_their_first_cell_in_the_scan_and_do_not_depend_on_the_input_order() {
    let cells: Vec<[i32; 3]> = block([0, 0, 0], [2, 2, 2])
        .into_iter()
        .chain(block([5, 0, 0], [3, 1, 1]))
        .chain(block([-6, 3, -4], [2, 2, 1]))
        .chain([[20, 20, 20]])
        .collect();
    let parts = components(&cells);
    assert_eq!(sizes(&parts), vec![4, 8, 3, 1], "ordered by the smallest cell of each in the scan (z, then y, then x)");
    for part in &parts {
        let mut scan = part.clone();
        scan.sort_by_key(|c| (c[2], c[1], c[0]));
        assert_eq!(&scan, part, "each component lists its cells in the scan order");
    }
    for seed in [3, 4, 5, 6] {
        assert_eq!(components(&shuffled(&cells, seed)), parts);
    }
    // duplicated cells count once
    let doubled: Vec<[i32; 3]> = cells.iter().chain(cells.iter()).copied().collect();
    assert_eq!(components(&doubled), parts);
    // and from the grid itself
    assert_eq!(filled(&cells).components(), parts);
}

/// A dense brick with the cells of `cells` that fall in it, palette `value`.
fn dense(key: [i32; 3], cells: &[[i32; 3]], value: u8) -> [u8; 512] {
    let mut brick = [0u8; 512];
    for c in cells {
        if c.map(|k| k.div_euclid(8)) == key {
            let l = c.map(|k| k.rem_euclid(8) as usize);
            brick[l[0] + 8 * (l[1] + 8 * l[2])] = value;
        }
    }
    brick
}

#[test]
fn an_occupancy_built_in_bulk_from_bricks_is_the_one_built_cell_by_cell() {
    let cells: Vec<[i32; 3]> =
        block([-5, -3, 2], [17, 6, 4]).into_iter().filter(|c| (c[0] + c[1] + c[2]) % 3 != 0).collect();
    let by_cell = filled(&cells);
    let mut keys: Vec<[i32; 3]> = cells.iter().map(|c| c.map(|k| k.div_euclid(8))).collect();
    keys.sort();
    keys.dedup();
    let bulk = Occupancy::from_bricks(shuffled(&keys, 9).into_iter().map(|k| (k, dense(k, &cells, 1)))).unwrap();
    assert_eq!(bulk.count(), by_cell.count());
    assert_eq!(bulk.fingerprint(), by_cell.fingerprint());
    assert_eq!(bulk.cells().collect::<Vec<_>>(), by_cell.cells().collect::<Vec<_>>());
    assert_eq!(bulk.components(), by_cell.components());
    // the whole of it is new to a reader that has seen nothing: every brick has changed since revision 0
    assert_eq!(bulk.changed_bricks_since(0), keys);
    assert!(bulk.revision() > 0);
    // the palette index is kept, and an empty brick is not a brick
    let mixed =
        Occupancy::from_bricks([([0, 0, 0], dense([0, 0, 0], &[[1, 2, 3]], 9)), ([5, 5, 5], [0u8; 512])]).unwrap();
    assert_eq!((mixed.get([1, 2, 3]), mixed.count(), mixed.bricks().count()), (9, 1, 1));
    assert_eq!(Occupancy::from_bricks(std::iter::empty()).unwrap().revision(), 0);
    // a brick twice is an error, and so is one over the limit
    assert!(Occupancy::from_bricks([
        ([0, 0, 0], dense([0, 0, 0], &[[1, 1, 1]], 1)),
        ([0, 0, 0], dense([0, 0, 0], &[[2, 2, 2]], 1))
    ])
    .is_err());
    assert!(Occupancy::from_bricks_with_limit(1, keys.iter().map(|k| (*k, dense(*k, &cells, 1)))).is_err());
}

#[test]
fn an_occupancy_built_from_a_list_of_cells_and_palette_indices_is_the_one_built_cell_by_cell() {
    let cells = block([-9, 4, -2], [11, 5, 3]);
    let listed: Vec<([i32; 3], u8)> =
        cells.iter().map(|c| (*c, 1 + ((c[0] * 7 + c[1] * 3 + c[2]).rem_euclid(5)) as u8)).collect();
    let mut by_cell = Occupancy::new();
    for (c, v) in &listed {
        by_cell.set(*c, *v).unwrap();
    }
    for seed in [1, 2, 3] {
        let bulk = Occupancy::from_cells(shuffled(&listed, seed)).unwrap();
        assert_eq!(bulk.fingerprint(), by_cell.fingerprint());
        assert_eq!(bulk.count(), by_cell.count());
        assert!(listed.iter().all(|(c, v)| bulk.get(*c) == *v));
    }
    // a cell listed twice, or with the empty index, is an error
    assert!(Occupancy::from_cells([([0, 0, 0], 1), ([0, 0, 0], 2)]).is_err());
    assert!(Occupancy::from_cells([([0, 0, 0], 0)]).is_err());
    assert!(Occupancy::from_cells_with_limit(1, [([0, 0, 0], 1), ([8, 0, 0], 1)]).is_err());
}

#[test]
fn the_palette_has_256_colours_beside_the_cells_and_recolouring_is_not_a_change_of_the_cells() {
    let mut o = filled(&block([0, 0, 0], [3, 3, 3]));
    // index 0 is empty and has no colour that matters; the others start opaque white
    assert_eq!(o.palette().color(1), [255, 255, 255, 255]);
    assert_eq!(o.palette().colors().len(), 256);
    let (cells, revision, palette_revision) = (o.fingerprint(), o.revision(), o.palette_revision());
    let look = o.appearance_fingerprint();
    assert!(o.set_color(7, [10, 20, 30, 255]));
    assert!(!o.set_color(7, [10, 20, 30, 255]), "the same colour is no change");
    assert_eq!(o.palette().color(7), [10, 20, 30, 255]);
    assert_eq!(o.fingerprint(), cells, "the geometry is the same, so a collider or a mass computed from it is too");
    assert_eq!(o.revision(), revision);
    assert!(o.palette_revision() > palette_revision);
    assert_ne!(o.appearance_fingerprint(), look, "but a mesh with colours is not");
    // a whole palette, as an importer reads it, in one go
    let mut colours = [[0u8; 4]; 256];
    for (i, c) in colours.iter_mut().enumerate() {
        *c = [i as u8, 255 - i as u8, (i * 3) as u8, 255];
    }
    let mut other = filled(&block([0, 0, 0], [3, 3, 3]));
    other.set_palette(colours);
    assert_eq!(other.palette().color(200), colours[200]);
    assert_eq!(other.fingerprint(), cells);
    // equal cells and equal colours are equal in appearance whatever the history
    let mut again = filled(&block([0, 0, 0], [3, 3, 3]));
    again.set_palette(colours);
    assert_eq!(other.appearance_fingerprint(), again.appearance_fingerprint());
}
