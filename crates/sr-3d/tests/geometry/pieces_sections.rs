//! The section of a joint: the area, the centre and the exact second moments of the faces that two pieces share, and the box that holds them, which is what
//! the fracture by stress needs of a joint to work out a stress from a load. The oracles are the closed forms of a rectangle, the brute force over the faces
//! in floats for a joint that is not flat, and the invariance of the moments under a move of the whole body (the sums are exact integers, taken from the
//! first face, so a joint far from the origin has the moments of one near it).
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_3d::pieces::{partition, sections, Partition, Plane};

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

/// A bar of 4 x 3 x 2 cells at `at`, cut in two by the plane x = 2 cells from its corner.
fn bar_cut_in_two(at: [i32; 3]) -> (Occupancy, sr_3d::pieces::PieceGraph) {
    let o = occupancy(&block(at, [4, 3, 2]));
    // the plane normal . u >= offset, u the doubled coordinates: x >= at + 2 cells is 2 x + 1 >= 2 (at + 2) + 1, which is the offset 2 at + 5 for the cells
    // from at + 2 on (the centre of that cell is 2 at + 5)
    let offset = 2 * i128::from(at[0]) + 5;
    let g = partition(&o, Partition::Planes(&[Plane { normal: [1, 0, 0], offset }]), 10).unwrap();
    assert_eq!((g.pieces().len(), g.edges().len()), (2, 1));
    (o, g)
}

#[test]
fn a_flat_rectangular_joint_has_the_area_centre_and_second_moments_of_a_rectangle() {
    let size = [0.5, 0.2, 0.4];
    let (o, g) = bar_cut_in_two([0, 0, 0]);
    let s = sections(&g, &o, size).unwrap();
    assert_eq!(s.len(), 1);
    let s = &s[0];
    // the face is 3 cells of 0.2 (0.6) by 2 cells of 0.4 (0.8), at x = 2 cells of 0.5
    let (b, h) = (0.6, 0.8);
    assert!((s.area - b * h).abs() < 1e-15, "{}", s.area);
    assert!(
        (s.centroid[0] - 1.0).abs() < 1e-15
            && (s.centroid[1] - 0.3).abs() < 1e-15
            && (s.centroid[2] - 0.4).abs() < 1e-15,
        "{:?}",
        s.centroid
    );
    // the integral of r r^T dA about the centre: b^3 h / 12 along y, b h^3 / 12 along z, none across, no products
    let want = [[0.0, 0.0, 0.0], [0.0, b * b * b * h / 12.0, 0.0], [0.0, 0.0, b * h * h * h / 12.0]];
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (s.second[i][j] - want[i][j]).abs() < 1e-15,
                "[{i}][{j}] {} against {}",
                s.second[i][j],
                want[i][j]
            );
        }
    }
    // from the piece 0 to the piece 1: up the x axis; and the box is the face's corners
    assert_eq!(s.normal, Some([1.0, 0.0, 0.0]));
    for j in 0..3 {
        assert!(
            (s.lo[j] - [1.0, 0.0, 0.0][j]).abs() < 1e-15 && (s.hi[j] - [1.0, 0.6, 0.8][j]).abs() < 1e-15,
            "{:?} {:?}",
            s.lo,
            s.hi
        );
    }
}

#[test]
fn the_second_moments_do_not_depend_on_where_the_body_is() {
    let size = [0.5, 0.2, 0.4];
    let (o0, g0) = bar_cut_in_two([0, 0, 0]);
    let near = &sections(&g0, &o0, size).unwrap()[0];
    for at in [[1_000_000, -2_000_000, 3], [-1_000_000_000, 7, 1_000_000_000]] {
        let (o, g) = bar_cut_in_two(at);
        let far = &sections(&g, &o, size).unwrap()[0];
        assert_eq!(far.area, near.area);
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (far.second[i][j] - near.second[i][j]).abs() <= 1e-12 * near.second[1][1].max(near.second[2][2]),
                    "{at:?} [{i}][{j}] {} against {}",
                    far.second[i][j],
                    near.second[i][j]
                );
            }
            // the centre is the same, moved by the body's own move
            let moved = f64::from(at[i]) * size[i] + near.centroid[i];
            assert!(
                (far.centroid[i] - moved).abs() <= 1e-9 * moved.abs().max(1.0),
                "{at:?} axis {i}: {} against {moved}",
                far.centroid[i]
            );
        }
    }
}

/// The section of the faces that pieces `a` and `b` share, worked out in floats from the cells, face by face: what the exact sums have to agree with.
fn brute(o: &Occupancy, g: &sr_3d::pieces::PieceGraph, e: usize, size: [f64; 3]) -> (f64, [f64; 3], [[f64; 3]; 3]) {
    let edge = &g.edges()[e];
    let (mut area, mut first, mut second) = (0.0, [0.0; 3], [[0.0; 3]; 3]);
    let mut faces = Vec::new();
    for c in o.cells() {
        let mine = g.piece_of(c).unwrap();
        for k in 0..3 {
            let mut next = c;
            next[k] += 1;
            let Some(theirs) = g.piece_of(next) else { continue };
            if (mine.min(theirs), mine.max(theirs)) != (edge.a(), edge.b()) {
                continue;
            }
            // the face between the two: its centre, and its area
            let centre: [f64; 3] =
                std::array::from_fn(|j| (f64::from(c[j]) + if j == k { 1.0 } else { 0.5 }) * size[j]);
            faces.push((k, centre, size[(k + 1) % 3] * size[(k + 2) % 3]));
        }
    }
    for (_, centre, a) in &faces {
        area += a;
        for j in 0..3 {
            first[j] += a * centre[j];
        }
    }
    let centroid: [f64; 3] = std::array::from_fn(|j| first[j] / area);
    for (k, centre, a) in &faces {
        for i in 0..3 {
            for j in 0..3 {
                let own = if i == j && i != *k { size[i] * size[i] / 12.0 } else { 0.0 };
                second[i][j] += a * ((centre[i] - centroid[i]) * (centre[j] - centroid[j]) + own);
            }
        }
    }
    (area, centroid, second)
}

#[test]
fn a_joint_that_is_not_flat_has_the_moments_of_its_faces_added_up_one_by_one() {
    // a Voronoi cut of a blob of cells of different sizes along the axes: the joints are stair-steps of faces normal to the three axes
    let o = occupancy(&block([0, 0, 0], [7, 6, 5]));
    let size = [0.3, 0.45, 0.7];
    let g = partition(&o, Partition::Voronoi { seeds: 5, seed: 17 }, 100).unwrap();
    let all = sections(&g, &o, size).unwrap();
    assert_eq!(all.len(), g.edges().len());
    assert!(
        g.edges().iter().any(|e| e.faces().iter().filter(|f| **f > 0).count() > 1),
        "a staircase joint is in this body"
    );
    for (e, s) in all.iter().enumerate() {
        let (area, centroid, second) = brute(&o, &g, e, size);
        assert!((s.area - area).abs() <= 1e-12 * area, "joint {e}: {} against {area}", s.area);
        assert!((s.area - g.edges()[e].area(size)).abs() <= 1e-12 * area);
        for j in 0..3 {
            assert!(
                (s.centroid[j] - centroid[j]).abs() <= 1e-12,
                "joint {e} centre {:?} against {centroid:?}",
                s.centroid
            );
        }
        let scale = second[0][0].max(second[1][1]).max(second[2][2]);
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (s.second[i][j] - second[i][j]).abs() <= 1e-12 * scale,
                    "joint {e} [{i}][{j}] {} against {}",
                    s.second[i][j],
                    second[i][j]
                );
            }
        }
        // the box holds the centre and is no larger than the body
        for j in 0..3 {
            assert!(s.lo[j] <= s.centroid[j] + 1e-12 && s.centroid[j] <= s.hi[j] + 1e-12, "joint {e} axis {j}");
        }
    }
}

#[test]
fn a_wrapped_joint_has_the_normal_of_the_faces_that_do_not_cancel_and_one_that_cancels_has_none() {
    // a ring of 3 x 3 cells with the cell (2, 1) missing round the two cells (1, 1) and (2, 1): one face normal to x and four normal to y, two up and two down
    let ring: Vec<[i32; 3]> =
        [[0, 0], [1, 0], [2, 0], [0, 1], [0, 2], [1, 2], [2, 2]].iter().map(|c| [c[0], c[1], 0]).collect();
    let core = [[1, 1, 0], [2, 1, 0]];
    let o = occupancy(&[ring, core.to_vec()].concat());
    let g = partition(&o, Partition::Labels(&|c| u32::from(core.contains(&c))), 10).unwrap();
    let s = &sections(&g, &o, [1.0; 3]).unwrap()[0];
    assert_eq!(s.normal, Some([1.0, 0.0, 0.0]));
    // a core wrapped on every side, all the faces cancelling: a single cell inside a shell has no mean normal
    let shell: Vec<[i32; 3]> = block([0, 0, 0], [3, 3, 3]);
    let o = occupancy(&shell);
    let g = partition(&o, Partition::Labels(&|c| u32::from(c == [1, 1, 1])), 10).unwrap();
    let s = &sections(&g, &o, [1.0; 3]).unwrap()[0];
    assert_eq!(g.edges()[0].faces(), [2, 2, 2]);
    assert_eq!(s.normal, None);
    assert!((s.area - 6.0).abs() < 1e-15);
}

#[test]
fn a_graph_of_another_body_is_an_error_and_a_size_that_is_nothing_is_too() {
    let (o, g) = bar_cut_in_two([0, 0, 0]);
    let other = occupancy(&block([10, 10, 10], [2, 2, 2]));
    assert!(sections(&g, &other, [1.0; 3]).is_err());
    for size in [[0.0, 1.0, 1.0], [1.0, -1.0, 1.0], [f64::NAN, 1.0, 1.0], [1.0, 1.0, f64::INFINITY]] {
        assert!(sections(&g, &o, size).is_err(), "{size:?}");
    }
}
