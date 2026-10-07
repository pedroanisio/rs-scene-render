//! The pieces that a body of cells is cut into, and which of them touch: every cell in exactly one piece, face-connected pieces numbered
//! by their first cell, the joints between them in exact integers. The oracles are worked out by hand on small bodies and by a brute force
//! over all the pairs of cells of a larger one, with none of the code of the module.
// the tests index three axes at a time
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::{components, Occupancy};
use sr_3d::pieces::{partition, seeds_in, Edge, Partition, Piece, PieceGraph, Plane};
use std::collections::{BTreeMap, BTreeSet};

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

fn occupancy(cells: &[[i32; 3]]) -> Occupancy {
    Occupancy::from_cells(cells.iter().map(|c| (*c, 1u8))).unwrap()
}

fn shuffled<T: Clone>(items: &[T], mut seed: u64) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        out.swap(i, (seed >> 33) as usize % (i + 1));
    }
    out
}

fn scan(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

/// What every graph must be, whatever the rule: every cell in one piece, pieces in scan order and face-connected, edges sorted and a < b.
fn check(o: &Occupancy, g: &PieceGraph) {
    let mut seen = BTreeSet::new();
    for (i, p) in g.pieces.iter().enumerate() {
        assert!(!p.cells.is_empty(), "piece {i} is empty");
        assert!(p.cells.windows(2).all(|w| scan(&w[0]) < scan(&w[1])), "piece {i} is not in the order of the scan");
        assert_eq!(components(&p.cells).len(), 1, "piece {i} is not connected by faces");
        for c in &p.cells {
            assert!(seen.insert(*c), "{c:?} is in two pieces");
            assert!(o.get(*c) != 0, "{c:?} is not a cell of the body");
        }
    }
    assert_eq!(seen.len() as u64, o.count(), "a cell of the body is in no piece");
    assert!(
        g.pieces.windows(2).all(|w| scan(&w[0].cells[0]) < scan(&w[1].cells[0])),
        "pieces are not numbered by their first cell"
    );
    assert!(g.edges.windows(2).all(|w| (w[0].a, w[0].b) < (w[1].a, w[1].b)), "edges are not sorted by (a, b)");
    assert!(g.edges.iter().all(|e| e.a < e.b && (e.b as usize) < g.pieces.len() && e.faces > 0));
}

/// The edges by brute force: every pair of cells that share a face and are in two pieces.
fn brute_edges(g: &PieceGraph) -> Vec<Edge> {
    let mut piece = BTreeMap::new();
    for (i, p) in g.pieces.iter().enumerate() {
        for c in &p.cells {
            piece.insert(*c, i as u32);
        }
    }
    let mut edges: BTreeMap<(u32, u32), Edge> = BTreeMap::new();
    for (c, &pc) in &piece {
        for (&d, &pd) in &piece {
            let diff = [d[0] - c[0], d[1] - c[1], d[2] - c[2]];
            let axis = (0..3).find(|&k| diff[k] == 1 && (0..3).all(|j| j == k || diff[j] == 0));
            let (Some(k), true) = (axis, pc != pd) else { continue };
            let (a, b) = (pc.min(pd), pc.max(pd));
            let e = edges.entry((a, b)).or_insert(Edge { a, b, faces: 0, face_sum: [0; 3], normal_sum: [0; 3] });
            e.faces += 1;
            for j in 0..3 {
                e.face_sum[j] += 2 * i64::from(c[j]) + 1 + i64::from(j == k);
            }
            // the unit normal from a to b: +axis if the lower cell is in a
            e.normal_sum[k] += if pc == a { 1 } else { -1 };
        }
    }
    edges.into_values().collect()
}

#[test]
fn a_bar_cut_by_a_plane_is_two_pieces_with_one_joint_whose_numbers_are_worked_out_by_hand() {
    // 4 x 2 x 2 cells, x 0 to 3; u = 2x + 1 is 1, 3, 5, 7; the plane x >= 2 is the normal (1, 0, 0) and the offset 4 over u
    let o = occupancy(&block([0, 0, 0], [4, 2, 2]));
    let g = partition(&o, Partition::Planes(&[Plane { normal: [1, 0, 0], offset: 4 }]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces.len(), 2);
    assert_eq!(g.pieces[0].cells, block([0, 0, 0], [2, 2, 2]));
    assert_eq!(g.pieces[1].cells, block([2, 0, 0], [2, 2, 2]));
    // the joint: 2 x 2 = 4 faces between x = 1 and x = 2, their centres at u_x = 4, and u_y, u_z in {1, 3} twice each: the sums are
    // 4 x 4 = 16, and (1 + 3) x 2 = 8 on y and on z; the normal from piece 0 to piece 1 is +x, four times
    assert_eq!(g.edges, vec![Edge { a: 0, b: 1, faces: 4, face_sum: [16, 8, 8], normal_sum: [4, 0, 0] }]);
}

#[test]
fn two_seeds_cut_a_bar_at_the_middle_and_the_cell_on_it_goes_to_the_seed_of_the_lower_index() {
    // ten cells, u = 1, 3, ..., 19; the seeds at u = 3 and u = 15 are 12 apart, so the middle is u = 9, the cell x = 4, equally far
    let o = occupancy(&block([0, 0, 0], [10, 1, 1]));
    let g = partition(&o, Partition::VoronoiAt(&[[3, 1, 1], [15, 1, 1]]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces[0].cells, block([0, 0, 0], [5, 1, 1]));
    assert_eq!(g.pieces[1].cells, block([5, 0, 0], [5, 1, 1]));
    // the seeds in the other order: the tie goes to the lower index, which is now the seed at 15, so the cell x = 4 is in the right piece
    let g = partition(&o, Partition::VoronoiAt(&[[15, 1, 1], [3, 1, 1]]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces[0].cells, block([0, 0, 0], [4, 1, 1]));
    assert_eq!(g.pieces[1].cells, block([4, 0, 0], [6, 1, 1]));
    assert_eq!(g.edges, vec![Edge { a: 0, b: 1, faces: 1, face_sum: [8, 1, 1], normal_sum: [1, 0, 0] }]);
}

#[test]
fn a_part_that_is_not_connected_is_split_into_its_components_and_pieces_are_numbered_by_their_first_cell() {
    // labels that alternate along a bar of six cells: every cell is its own piece, and the five joints are one face each
    let o = occupancy(&block([0, 0, 0], [6, 1, 1]));
    let g = partition(&o, Partition::Labels(&|c| (c[0] % 2) as u32), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces.len(), 6);
    assert_eq!(g.edges.len(), 5);
    for (i, e) in g.edges.iter().enumerate() {
        assert_eq!((e.a, e.b, e.faces), (i as u32, i as u32 + 1, 1));
        assert_eq!(e.normal_sum, [1, 0, 0]);
    }
    // one label for two blocks that do not touch: two pieces, the one with the first cell in the scan first, and no joint
    let apart = occupancy(&[block([5, 5, 5], [2, 2, 2]), block([0, 0, 0], [2, 2, 2])].concat());
    let g = partition(&apart, Partition::Labels(&|_| 7), 100).unwrap();
    check(&apart, &g);
    assert_eq!((g.pieces.len(), g.edges.len()), (2, 0));
    assert_eq!(g.pieces[0].cells[0], [0, 0, 0]);
    // a body that is one piece is one piece with no joint
    let one = partition(&o, Partition::Labels(&|_| 0), 100).unwrap();
    assert_eq!((one.pieces.len(), one.edges.len()), (1, 0));
}

#[test]
fn the_joints_of_a_blob_cut_by_seeds_are_the_brute_force_count_of_the_faces_that_two_pieces_share() {
    // an irregular body on both sides of the origin: a ball with a bite out of it and an arm
    let mut cells: Vec<[i32; 3]> = block([-6, -6, -6], [12, 12, 12])
        .into_iter()
        .filter(|c| {
            let d = [2 * c[0] + 1, 2 * c[1] + 1, 2 * c[2] + 1];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= 144 && !(c[0] > 1 && c[1] > 1 && c[2] > 0)
        })
        .collect();
    cells.extend(block([6, -1, -1], [5, 2, 2]));
    let o = occupancy(&cells);
    for (seeds, seed) in [(3, 1), (7, 2), (20, 3), (1, 4)] {
        let g = partition(&o, Partition::Voronoi { seeds, seed }, 10_000).unwrap();
        check(&o, &g);
        assert_eq!(g.edges, brute_edges(&g), "seeds {seeds} seed {seed}");
        assert!(!g.edges.is_empty() || g.pieces.len() == 1);
    }
}

#[test]
fn the_graph_is_the_same_whatever_the_order_the_cells_came_in_and_every_time() {
    let cells = block([-3, -2, -4], [9, 6, 8]);
    let reference = partition(&occupancy(&cells), Partition::Voronoi { seeds: 6, seed: 99 }, 100).unwrap();
    for shuffle in 1..5 {
        let o = occupancy(&shuffled(&cells, shuffle));
        let g = partition(&o, Partition::Voronoi { seeds: 6, seed: 99 }, 100).unwrap();
        assert_eq!(g.pieces, reference.pieces);
        assert_eq!(g.edges, reference.edges);
    }
    // another seed is another cut (the seeds come from the seed and the index and nothing else)
    let other = partition(&occupancy(&cells), Partition::Voronoi { seeds: 6, seed: 100 }, 100).unwrap();
    assert_ne!(other.pieces, reference.pieces);
    // a cut by more seeds is a graph that is still whole
    let more = partition(&occupancy(&cells), Partition::Voronoi { seeds: 7, seed: 99 }, 100).unwrap();
    check(&occupancy(&cells), &more);
}

#[test]
fn planes_cut_by_the_sides_they_put_a_cell_on_and_the_limits_say_what_was_over() {
    let o = occupancy(&block([0, 0, 0], [4, 4, 4]));
    // two planes, x >= 2 and y >= 2 (over u): four pieces of 2 x 2 x 4 cells
    let planes = [Plane { normal: [1, 0, 0], offset: 4 }, Plane { normal: [0, 1, 0], offset: 4 }];
    let g = partition(&o, Partition::Planes(&planes), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces.len(), 4);
    assert!(g.pieces.iter().all(|p| p.cells.len() == 16));
    // 4 joints of 8 faces: every piece touches two others (not the diagonal one)
    assert_eq!(g.edges.len(), 4);
    assert!(g.edges.iter().all(|e| e.faces == 8));
    assert_eq!(g.edges, brute_edges(&g));
    // a plane that no cell is on the far side of leaves the body whole
    let none = partition(&o, Partition::Planes(&[Plane { normal: [1, 0, 0], offset: 1000 }]), 100).unwrap();
    assert_eq!((none.pieces.len(), none.edges.len()), (1, 0));
    // the limits
    let error = partition(&o, Partition::Planes(&planes), 3).unwrap_err();
    assert!(error.contains('4') && error.contains('3') && error.contains("pieces"), "{error}");
    let many: Vec<Plane> = (0..64).map(|i| Plane { normal: [1, 0, 0], offset: i }).collect();
    assert!(partition(&o, Partition::Planes(&many), 100).unwrap_err().contains("63"));
    assert!(partition(&o, Partition::Voronoi { seeds: 0, seed: 1 }, 100).unwrap_err().contains("seed"));
    assert!(partition(&o, Partition::VoronoiAt(&[]), 100).unwrap_err().contains("seed"));
    assert!(partition(&Occupancy::new(), Partition::Labels(&|_| 0), 100).unwrap_err().contains("no cell"));
}

#[test]
fn a_piece_is_what_the_world_needs_of_it_cells_in_scan_order() {
    // the type is what Netuno's fragments are made from: Piece { cells }
    let p = Piece { cells: vec![[0, 0, 0]] };
    assert_eq!(p.cells.len(), 1);
}

#[test]
fn the_seeds_are_the_splitmix64_of_the_seed_and_the_index_and_lie_in_the_box() {
    // splitmix64 seeded with 0 gives 0xE220A8397B1DCDAF, 0x6E789E6AA1B965F4, 0x06C45D188009454F (the reference sequence); the first seed's
    // three axes are the first three draws, and in a box 0..2^62 the coordinate is the draw's low 62 bits
    let first = seeds_in(1, 0, [0; 3], [(1 << 62) - 1; 3]);
    let mask = (1u64 << 62) - 1;
    assert_eq!(
        first[0].map(|v| v as u64),
        [0xE220_A839_7B1D_CDAF & mask, 0x6E78_9E6A_A1B9_65F4 & mask, 0x06C4_5D18_8009_454F & mask]
    );
    // the draws of the next seed are the next ones of the sequence, and a seed never depends on the count asked for
    let more = seeds_in(5, 0, [0; 3], [(1 << 62) - 1; 3]);
    assert_eq!(more[0], first[0]);
    assert_eq!(seeds_in(3, 0, [0; 3], [(1 << 62) - 1; 3])[..], more[..3]);
    // in a small box every seed is inside, on a box of one cell all of them are its centre
    let (lo, hi) = ([-9, 4, -3], [14, 4, 20]);
    for seed in 0..50u64 {
        for s in seeds_in(10, seed, lo, hi) {
            assert!((0..3).all(|a| lo[a] <= s[a] && s[a] <= hi[a]), "{s:?}");
        }
    }
    assert!(seeds_in(4, 7, [3; 3], [3; 3]).iter().all(|s| *s == [3; 3]));
}

#[test]
fn a_voronoi_part_that_is_not_connected_is_split_into_its_components() {
    // a U of cells: a base x 0 to 4 at y 0, and arms up x = 0 and x = 4 to y 3. The seed 0 is above the middle, at the cell (2, 3, 0), and the
    // seed 1 is at the middle of the base, at the cell (2, 0, 0): the tops of the two arms (y = 2 and 3) are nearer the seed 0, every other
    // cell nearer the seed 1 (the cell (0, 2): 20 against 32 over u; the cell (0, 1): 32 against 20). So the part of the seed 0 is two
    // pieces that do not touch, and the part of the seed 1 is one.
    let mut u = block([0, 0, 0], [5, 1, 1]);
    u.extend(block([0, 1, 0], [1, 3, 1]));
    u.extend(block([4, 1, 0], [1, 3, 1]));
    let o = occupancy(&u);
    let g = partition(&o, Partition::VoronoiAt(&[[5, 7, 1], [5, 1, 1]]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces.len(), 3);
    // numbered by the first cell in the scan: the part of the seed 1 first (its first cell is (0, 0, 0)), then the arm tops
    assert_eq!(g.pieces[0].cells.len(), 5 + 2);
    assert_eq!(g.pieces[1].cells, vec![[0, 2, 0], [0, 3, 0]]);
    assert_eq!(g.pieces[2].cells, vec![[4, 2, 0], [4, 3, 0]]);
    // each arm top touches the rest by one face, and the two arm tops do not touch
    let faces: Vec<(u32, u32, u32)> = g.edges.iter().map(|e| (e.a, e.b, e.faces)).collect();
    assert_eq!(faces, vec![(0, 1, 1), (0, 2, 1)]);
    // the joint of the first arm: the face between the cells (0, 1) and (0, 2), at u = (1, 2 * 1 + 2, 1), normal +y from the piece 0
    assert_eq!(g.edges[0].face_sum, [1, 4, 1]);
    assert_eq!(g.edges[0].normal_sum, [0, 1, 0]);
}
