//! The pieces that a body of cells is cut into, and which of them touch: every cell in exactly one piece, face-connected pieces numbered
//! by their first cell, the joints between them in exact integers. The oracles are worked out by hand on small bodies and by a brute force
//! over all the pairs of cells of a larger one, with none of the code of the module.
// the tests index three axes at a time
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::{components, Moments, Occupancy, KEY_LIMIT};
use sr_3d::pieces::{partition, seeds_in, Edge, Partition, PieceGraph, Plane, MAX_PLANES, MAX_SEEDS, MAX_SEED_COORD};
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

fn edge(a: u32, b: u32, faces: [u32; 3], face_sum: [[i64; 3]; 3], normal_sum: [i32; 3]) -> Edge {
    Edge::new(a, b, faces, face_sum, normal_sum)
}

/// What every graph must be, whatever the rule: every cell in one piece, pieces in scan order and face-connected, the moments of each
/// piece those of its cells, edges sorted and a < b, and the owner of every cell the piece that lists it.
fn check(o: &Occupancy, g: &PieceGraph) {
    let mut seen = BTreeSet::new();
    for (i, p) in g.pieces().iter().enumerate() {
        let cells = p.cells();
        assert!(!cells.is_empty(), "piece {i} is empty");
        assert!(cells.windows(2).all(|w| scan(&w[0]) < scan(&w[1])), "piece {i} is not in the order of the scan");
        assert_eq!(components(cells).len(), 1, "piece {i} is not connected by faces");
        assert_eq!(p.moments(), Moments::of(cells.iter().copied()), "the moments of piece {i}");
        for c in cells {
            assert!(seen.insert(*c), "{c:?} is in two pieces");
            assert!(o.get(*c) != 0, "{c:?} is not a cell of the body");
            assert_eq!(g.piece_of(*c), Some(i as u32), "the owner of {c:?}");
        }
    }
    assert_eq!(seen.len() as u64, o.count(), "a cell of the body is in no piece");
    assert_eq!(g.piece_of([i32::MIN, 0, 0]), None);
    let firsts: Vec<_> = g.pieces().iter().map(|p| scan(&p.cells()[0])).collect();
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "pieces are not numbered by their first cell");
    let order: Vec<_> = g.edges().iter().map(|e| (e.a(), e.b())).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "edges are not sorted by (a, b)");
    assert!(g.edges().iter().all(|e| e.a() < e.b() && (e.b() as usize) < g.pieces().len() && e.total_faces() > 0));
}

/// The edges by brute force: every pair of cells that share a face and are in two pieces.
fn brute_edges(g: &PieceGraph) -> Vec<Edge> {
    let mut piece = BTreeMap::new();
    for (i, p) in g.pieces().iter().enumerate() {
        for c in p.cells() {
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
            let e = edges.entry((a, b)).or_insert_with(|| edge(a, b, [0; 3], [[0; 3]; 3], [0; 3]));
            let mut faces = e.faces();
            let mut face_sum = e.face_sum();
            let mut normal_sum = e.normal_sum();
            faces[k] += 1;
            for j in 0..3 {
                face_sum[k][j] += 2 * i64::from(c[j]) + 1 + i64::from(j == k);
            }
            // the unit normal from a to b: +axis if the lower cell is in a
            normal_sum[k] += if pc == a { 1 } else { -1 };
            *e = edge(a, b, faces, face_sum, normal_sum);
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
    assert_eq!(g.pieces().len(), 2);
    assert_eq!(g.pieces()[0].cells(), block([0, 0, 0], [2, 2, 2]));
    assert_eq!(g.pieces()[1].cells(), block([2, 0, 0], [2, 2, 2]));
    // the joint: 2 x 2 = 4 faces, all normal to x, between x = 1 and x = 2, their centres at u_x = 4, and u_y, u_z in {1, 3} twice each: the
    // sums are 4 x 4 = 16, and (1 + 3) x 2 = 8 on y and on z; the normal from piece 0 to piece 1 is +x, four times
    assert_eq!(g.edges(), [edge(0, 1, [4, 0, 0], [[16, 8, 8], [0; 3], [0; 3]], [4, 0, 0])]);
    // the moments of a piece are the exact moments of its cells: 8 cells
    assert_eq!(g.pieces()[0].moments().count(), 8);
}

#[test]
fn two_seeds_cut_a_bar_at_the_middle_and_the_cell_on_it_goes_to_the_seed_of_the_lower_index() {
    // ten cells, u = 1, 3, ..., 19; the seeds at u = 3 and u = 15 are 12 apart, so the middle is u = 9, the cell x = 4, equally far
    let o = occupancy(&block([0, 0, 0], [10, 1, 1]));
    let g = partition(&o, Partition::VoronoiAt(&[[3, 1, 1], [15, 1, 1]]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces()[0].cells(), block([0, 0, 0], [5, 1, 1]));
    assert_eq!(g.pieces()[1].cells(), block([5, 0, 0], [5, 1, 1]));
    // the seeds in the other order: the tie goes to the lower index, which is now the seed at 15, so the cell x = 4 is in the right piece
    let g = partition(&o, Partition::VoronoiAt(&[[15, 1, 1], [3, 1, 1]]), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces()[0].cells(), block([0, 0, 0], [4, 1, 1]));
    assert_eq!(g.pieces()[1].cells(), block([4, 0, 0], [6, 1, 1]));
    assert_eq!(g.edges(), [edge(0, 1, [1, 0, 0], [[8, 1, 1], [0; 3], [0; 3]], [1, 0, 0])]);
}

#[test]
fn a_part_that_is_not_connected_is_split_into_its_components_and_pieces_are_numbered_by_their_first_cell() {
    // labels that alternate along a bar of six cells: every cell is its own piece, and the five joints are one face each
    let o = occupancy(&block([0, 0, 0], [6, 1, 1]));
    let g = partition(&o, Partition::Labels(&|c| (c[0] % 2) as u32), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces().len(), 6);
    assert_eq!(g.edges().len(), 5);
    for (i, e) in g.edges().iter().enumerate() {
        assert_eq!((e.a(), e.b(), e.faces()), (i as u32, i as u32 + 1, [1, 0, 0]));
        assert_eq!(e.normal_sum(), [1, 0, 0]);
    }
    // one label for two blocks that do not touch: two pieces, the one with the first cell in the scan first, and no joint
    let apart = occupancy(&[block([5, 5, 5], [2, 2, 2]), block([0, 0, 0], [2, 2, 2])].concat());
    let g = partition(&apart, Partition::Labels(&|_| 7), 100).unwrap();
    check(&apart, &g);
    assert_eq!((g.pieces().len(), g.edges().len()), (2, 0));
    assert_eq!(g.pieces()[0].cells()[0], [0, 0, 0]);
    // a body that is one piece is one piece with no joint
    let one = partition(&o, Partition::Labels(&|_| 0), 100).unwrap();
    check(&o, &one);
    assert_eq!((one.pieces().len(), one.edges().len()), (1, 0));
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
    assert_eq!(g.pieces().len(), 3);
    // numbered by the first cell in the scan: the part of the seed 1 first (its first cell is (0, 0, 0)), then the arm tops
    assert_eq!(g.pieces()[0].cells().len(), 5 + 2);
    assert_eq!(g.pieces()[1].cells(), [[0, 2, 0], [0, 3, 0]]);
    assert_eq!(g.pieces()[2].cells(), [[4, 2, 0], [4, 3, 0]]);
    // each arm top touches the rest by one face, and the two arm tops do not touch
    let faces: Vec<(u32, u32, [u32; 3])> = g.edges().iter().map(|e| (e.a(), e.b(), e.faces())).collect();
    assert_eq!(faces, [(0, 1, [0, 1, 0]), (0, 2, [0, 1, 0])]);
    // the joint of the first arm: the face between the cells (0, 1) and (0, 2), at u = (1, 2 * 1 + 2, 1), normal +y from the piece 0
    assert_eq!(g.edges()[0].face_sum(), [[0; 3], [1, 4, 1], [0; 3]]);
    assert_eq!(g.edges()[0].normal_sum(), [0, 1, 0]);
}

#[test]
fn the_cells_of_a_blob_go_to_the_nearest_seed_and_the_joints_are_the_brute_force_count_of_the_faces_that_two_pieces_share(
) {
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
    for (count, seed) in [(3u32, 1u64), (7, 2), (20, 3), (1, 4)] {
        let g = partition(&o, Partition::Voronoi { seeds: count, seed }, 10_000).unwrap();
        check(&o, &g);
        assert_eq!(g.edges(), brute_edges(&g), "seeds {count} seed {seed}");
        // every cell is in a piece of the part of its nearest seed (the lowest index of the nearest), by a brute force argmin over the seeds
        let nearest = |c: [i32; 3]| {
            let seeds = seeds_in(count, seed, doubled_lo(&o), doubled_hi(&o)).unwrap();
            let mut best = (i128::MAX, 0usize);
            for (i, s) in seeds.iter().enumerate() {
                let u = [2 * i64::from(c[0]) + 1, 2 * i64::from(c[1]) + 1, 2 * i64::from(c[2]) + 1];
                let d: i128 = (0..3).map(|a| i128::from(u[a] - s[a]) * i128::from(u[a] - s[a])).sum();
                if d < best.0 {
                    best = (d, i);
                }
            }
            best.1
        };
        for p in g.pieces() {
            let part = nearest(p.cells()[0]);
            assert!(p.cells().iter().all(|c| nearest(*c) == part), "a piece has cells of two parts");
        }
        // and no joint is between two pieces of the same part (a part that was cut in two for nothing)
        for e in g.edges() {
            let (a, b) = (&g.pieces()[e.a() as usize], &g.pieces()[e.b() as usize]);
            assert_ne!(
                nearest(a.cells()[0]),
                nearest(b.cells()[0]),
                "two pieces of one part touch: seeds {count} seed {seed}"
            );
        }
    }
}

/// The doubled coordinates of the corners of the box of the cells of `o`, as `Partition::Voronoi` draws its seeds in.
fn doubled_lo(o: &Occupancy) -> [i64; 3] {
    o.bounds().unwrap().0.map(|k| 2 * i64::from(k) + 1)
}

fn doubled_hi(o: &Occupancy) -> [i64; 3] {
    o.bounds().unwrap().1.map(|k| 2 * i64::from(k) + 1)
}

#[test]
fn the_graph_is_the_same_for_the_same_body_however_it_was_built_and_every_time() {
    let cells = block([-3, -2, -4], [9, 6, 8]);
    let reference = partition(&occupancy(&cells), Partition::Voronoi { seeds: 6, seed: 99 }, 100).unwrap();
    for shuffle in 1..5 {
        // the occupancy keeps its cells in the order of the scan whatever order they were put in, so this is the whole path from the cells
        let o = occupancy(&shuffled(&cells, shuffle));
        let g = partition(&o, Partition::Voronoi { seeds: 6, seed: 99 }, 100).unwrap();
        assert_eq!(g, reference);
    }
    // another seed is another cut (the seeds come from the seed and the index and nothing else)
    let other = partition(&occupancy(&cells), Partition::Voronoi { seeds: 6, seed: 100 }, 100).unwrap();
    assert_ne!(other.pieces(), reference.pieces());
}

#[test]
fn planes_cut_by_the_sides_they_put_a_cell_on_and_the_limits_say_what_was_over() {
    let o = occupancy(&block([0, 0, 0], [4, 4, 4]));
    // two planes, x >= 2 and y >= 2 (over u): four pieces of 2 x 2 x 4 cells
    let planes = [Plane { normal: [1, 0, 0], offset: 4 }, Plane { normal: [0, 1, 0], offset: 4 }];
    let g = partition(&o, Partition::Planes(&planes), 100).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces().len(), 4);
    assert!(g.pieces().iter().all(|p| p.cells().len() == 16));
    // 4 joints of 8 faces: every piece touches two others (not the diagonal one)
    assert_eq!(g.edges().len(), 4);
    assert!(g.edges().iter().all(|e| e.total_faces() == 8));
    assert_eq!(g.edges(), brute_edges(&g));
    // a plane that no cell is on the far side of leaves the body whole
    let none = partition(&o, Partition::Planes(&[Plane { normal: [1, 0, 0], offset: 1000 }]), 100).unwrap();
    assert_eq!((none.pieces().len(), none.edges().len()), (1, 0));
    // the limits, in the words that name the numbers
    assert_eq!(
        partition(&o, Partition::Planes(&planes), 3).unwrap_err(),
        "the partition makes 4 pieces and the limit is 3 pieces"
    );
    let many: Vec<Plane> = (0..64).map(|i| Plane { normal: [1, 0, 0], offset: i }).collect();
    assert_eq!(
        partition(&o, Partition::Planes(&many), 100).unwrap_err(),
        format!("64 planes, and a partition takes at most {MAX_PLANES}")
    );
    assert_eq!(MAX_PLANES, 63);
    let none = partition(&o, Partition::Voronoi { seeds: 0, seed: 1 }, 100).unwrap_err();
    assert_eq!(none, "a Voronoi partition needs at least one seed");
    assert_eq!(partition(&o, Partition::VoronoiAt(&[]), 100).unwrap_err(), none);
    assert_eq!(
        partition(&Occupancy::new(), Partition::Labels(&|_| 0), 100).unwrap_err(),
        "a body with no cell has no pieces"
    );
}

#[test]
fn the_seeds_are_the_splitmix64_of_the_seed_and_the_index_and_lie_in_the_box() {
    // splitmix64 seeded with 0 gives 0xE220A8397B1DCDAF, 0x6E789E6AA1B965F4, 0x06C45D188009454F (the reference sequence); the first seed's
    // three axes are the first three draws, and in a box 0..2^40 the coordinate is the draw's low 40 bits
    let top = (1i64 << 40) - 1;
    let first = seeds_in(1, 0, [0; 3], [top; 3]).unwrap();
    let mask = (1u64 << 40) - 1;
    assert_eq!(
        first[0].map(|v| v as u64),
        [0xE220_A839_7B1D_CDAF & mask, 0x6E78_9E6A_A1B9_65F4 & mask, 0x06C4_5D18_8009_454F & mask]
    );
    // the draws of the next seed are the next ones of the sequence, and a seed never depends on the count asked for
    let more = seeds_in(5, 0, [0; 3], [top; 3]).unwrap();
    assert_eq!(more[0], first[0]);
    assert_eq!(seeds_in(3, 0, [0; 3], [top; 3]).unwrap()[..], more[..3]);
    // in a small box every seed is inside, on a box of one cell all of them are its centre
    let (lo, hi) = ([-9, 4, -3], [14, 4, 20]);
    for seed in 0..50u64 {
        for s in seeds_in(10, seed, lo, hi).unwrap() {
            assert!((0..3).all(|a| lo[a] <= s[a] && s[a] <= hi[a]), "{s:?}");
        }
    }
    assert!(seeds_in(4, 7, [3; 3], [3; 3]).unwrap().iter().all(|s| *s == [3; 3]));
    // a box that is upside down, a box too far out, and too many seeds are errors and not a division by zero or an overflow
    assert!(seeds_in(1, 0, [0; 3], [-1, 0, 0]).unwrap_err().contains("box"));
    assert!(seeds_in(1, 0, [0; 3], [MAX_SEED_COORD + 1, 0, 0]).unwrap_err().contains("box"));
    assert!(seeds_in(1, 0, [i64::MIN, 0, 0], [i64::MAX, 0, 0]).unwrap_err().contains("box"));
    assert_eq!(
        seeds_in(MAX_SEEDS + 1, 0, [0; 3], [9; 3]).unwrap_err(),
        format!("{} seeds, and a partition takes at most {MAX_SEEDS}", MAX_SEEDS + 1)
    );
}

#[test]
fn a_single_cell_is_one_piece_with_its_own_moments_and_no_joint() {
    let o = occupancy(&[[-5, 7, 0]]);
    for rule in [Partition::Voronoi { seeds: 4, seed: 3 }, Partition::Labels(&|_| 9), Partition::Planes(&[])] {
        let g = partition(&o, rule, 1).unwrap();
        check(&o, &g);
        assert_eq!((g.pieces().len(), g.edges().len()), (1, 0));
        assert_eq!(g.pieces()[0].cells(), [[-5, 7, 0]]);
        assert_eq!(g.piece_of([-5, 7, 0]), Some(0));
        assert_eq!(g.piece_of([-5, 7, 1]), None);
    }
}

#[test]
fn cells_at_the_last_keys_of_an_occupancy_are_cut_and_joined_without_overflow() {
    // two cells at the end of the keys on every axis: the neighbours, the doubled coordinates and the sums are all near the end of an i32
    let last = KEY_LIMIT - 1;
    let o = occupancy(&[[last - 1, last, -KEY_LIMIT], [last, last, -KEY_LIMIT], [-KEY_LIMIT, -KEY_LIMIT, last]]);
    let g = partition(&o, Partition::Labels(&|c| u32::from(c[0] == last)), 10).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces().len(), 3);
    // the joint of the first two: the face between x = last - 1 and x = last, at u_x = 2 (last - 1) + 2, u_y = 2 last + 1, u_z = -2 KEY_LIMIT + 1
    let joint = g.edges().iter().find(|e| (e.a(), e.b()) == (0, 1)).expect("the two cells touch");
    assert_eq!(joint.faces(), [1, 0, 0]);
    assert_eq!(
        joint.face_sum()[0],
        [2 * i64::from(last - 1) + 2, 2 * i64::from(last) + 1, -2 * i64::from(KEY_LIMIT) + 1]
    );
    // extreme seeds, offsets and normals are decided and not overflowed: the nearest seed at the far end of the allowed range
    let far = MAX_SEED_COORD;
    let g = partition(&o, Partition::VoronoiAt(&[[far, far, far], [-far, -far, -far]]), 10).unwrap();
    check(&o, &g);
    for offset in [i128::MIN, i128::MAX, 0] {
        let planes = [Plane { normal: [i64::MAX, i64::MIN, i64::MAX], offset }];
        let g = partition(&o, Partition::Planes(&planes), 10).unwrap();
        check(&o, &g);
    }
    // a seed beyond the range is refused by name
    let error = partition(&o, Partition::VoronoiAt(&[[i64::MAX, 0, 0]]), 10).unwrap_err();
    assert!(error.contains("seed") && error.contains(&MAX_SEED_COORD.to_string()), "{error}");
    let error = partition(&o, Partition::Voronoi { seeds: u32::MAX, seed: 0 }, 10).unwrap_err();
    assert_eq!(error, format!("{} seeds, and a partition takes at most {MAX_SEEDS}", u32::MAX));
}

#[test]
fn an_offset_that_is_the_smallest_or_the_largest_number_puts_every_cell_on_one_side() {
    let o = occupancy(&block([0, 0, 0], [3, 3, 3]));
    // normal . u - offset >= 0: with the offset i128::MIN every cell is on the positive side (a subtraction would overflow), with i128::MAX on the
    // negative side; either way the body is whole
    for offset in [i128::MIN, i128::MAX] {
        let g = partition(&o, Partition::Planes(&[Plane { normal: [1, -1, 1], offset }]), 10).unwrap();
        assert_eq!((g.pieces().len(), g.edges().len()), (1, 0));
    }
    // a plane through a cell centre is on the positive side of that cell: u = 3 in x is the cell 1, normal . u - offset = 0
    let g = partition(&o, Partition::Planes(&[Plane { normal: [1, 0, 0], offset: 3 }]), 10).unwrap();
    assert_eq!(g.pieces()[0].cells(), block([0, 0, 0], [1, 3, 3]));
    assert_eq!(g.pieces()[1].cells(), block([1, 0, 0], [2, 3, 3]));
}

#[test]
fn two_cubes_that_touch_only_by_an_edge_or_a_corner_are_not_joined() {
    // two cells that share an edge, and two that share a corner: no face, so no joint (and two pieces of one label, as they are not connected)
    let edge_only = occupancy(&[[0, 0, 0], [1, 1, 0]]);
    let corner_only = occupancy(&[[0, 0, 0], [1, 1, 1]]);
    for o in [&edge_only, &corner_only] {
        let split = partition(o, Partition::Labels(&|c| c[0] as u32), 10).unwrap();
        check(o, &split);
        assert_eq!((split.pieces().len(), split.edges().len()), (2, 0));
        let one = partition(o, Partition::Labels(&|_| 0), 10).unwrap();
        check(o, &one);
        assert_eq!((one.pieces().len(), one.edges().len()), (2, 0), "one label, but not connected by faces");
    }
}

#[test]
fn the_faces_of_a_joint_are_counted_by_axis_and_a_wrapped_joint_keeps_its_area_where_its_normals_cancel() {
    // a ring of 3 x 3 cells with the cell (2, 1) missing, wrapped round the two cells (1, 1) and (2, 1): the piece 0 is the ring, the piece
    // 1 the two cells, and the joint has one face normal to x and four normal to y, two up and two down
    let ring: Vec<[i32; 3]> =
        [[0, 0], [1, 0], [2, 0], [0, 1], [0, 2], [1, 2], [2, 2]].iter().map(|c| [c[0], c[1], 0]).collect();
    let core = [[1, 1, 0], [2, 1, 0]];
    let o = occupancy(&[ring.clone(), core.to_vec()].concat());
    let g = partition(&o, Partition::Labels(&|c| u32::from(core.contains(&c))), 10).unwrap();
    check(&o, &g);
    assert_eq!(g.pieces()[0].cells().len(), 7);
    let joint = &g.edges()[0];
    assert_eq!(joint.faces(), [1, 4, 0]);
    // the normals up and down cancel along y, so they cannot give the area: the faces by axis do
    assert_eq!(joint.normal_sum(), [1, 0, 0]);
    assert_eq!(g.edges(), brute_edges(&g));
    // cells of 2 by 3 by 5 units: the area is 1 face of 3 x 5 and 4 faces of 2 x 5
    assert_eq!(joint.area([2.0, 3.0, 5.0]), 15.0 + 40.0);
    // and the centroid is the mean of the centres of the faces, weighted by their areas, in units
    let centroid = joint.centroid([2.0, 3.0, 5.0]);
    let (x, y) = (centroid[0], centroid[1]);
    // the x face is between (0, 1) and (1, 1): centre (1, 1.5, .5) cells; the y faces: (1, 1, .), (1, 2, .), (2, 1, .), (2, 2, .) at x 1.5 and 2.5
    // and y 1 or 2 (cells 1 up, the one above) -> centres (1.5, 1), (1.5, 2), (2.5, 1), (2.5, 2) cells: the weights are 15 and 4 x 10
    let wx = (15.0 * 1.0 * 2.0 + 10.0 * (1.5 + 1.5 + 2.5 + 2.5) * 2.0) / 55.0;
    let wy = (15.0 * 1.5 * 3.0 + 10.0 * (1.0 + 2.0 + 1.0 + 2.0) * 3.0) / 55.0;
    assert!((x - wx).abs() < 1e-12 && (y - wy).abs() < 1e-12, "{centroid:?} against {wx} {wy}");
}
