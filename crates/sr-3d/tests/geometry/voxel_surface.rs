//! The surface of a grid of voxels: which faces of its cells are exposed, and the quads they merge into. The oracles are counts that
//! can be worked out by hand and a naive extraction (the six neighbours of every cell, read one by one) that shares nothing with the
//! module; `tools/voxel_reference_mesher.py` is the same rule in another language.

use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::surface::{exposed_faces, mesh_quads, quads_hash, Classes, Face, Quad};
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
fn a_box_of_a_by_b_by_c_has_two_of_ab_plus_bc_plus_ca_faces_and_six_quads() {
    for (a, b, c) in [(1, 2, 3), (4, 9, 5), (8, 1, 17), (13, 13, 2)] {
        let mut cells = Vec::new();
        for z in 0..c {
            for y in 0..b {
                for x in 0..a {
                    cells.push(([x - 5, y + 2, z - 9], 1));
                }
            }
        }
        let g = grid(cells);
        assert_eq!(
            exposed_faces(&g, &Classes::identity()).len() as i32,
            2 * (a * b + b * c + c * a),
            "{a} x {b} x {c}"
        );
        assert_eq!(mesh_quads(&g, &Classes::identity()).len(), 6);
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

// ---------------------------------------------------------------------------------------------------------------- quads

/// The cells of the wall of the golden fixture: 24 by 3 by 16, the palette index 1 or 2 by the tile of 4 by 2 cells in x and z.
fn wall() -> Occupancy {
    let mut cells = Vec::new();
    for x in 0..24 {
        for y in 0..3 {
            for z in 0..16 {
                cells.push(([x, y, z], 1 + ((x / 4 + z / 2) % 2) as u8));
            }
        }
    }
    grid(cells)
}

/// Every face the quads cover, with how many times.
fn covered(quads: &[Quad]) -> std::collections::BTreeMap<Face, u32> {
    let mut out = std::collections::BTreeMap::new();
    for q in quads {
        for dv in 0..q.h as i32 {
            for du in 0..q.w as i32 {
                let f = Face {
                    axis: q.axis,
                    positive: q.positive,
                    plane: q.plane,
                    u: q.u0 + du,
                    v: q.v0 + dv,
                    class: q.class,
                };
                *out.entry(f).or_insert(0) += 1;
            }
        }
    }
    out
}

#[test]
fn a_block_of_n_cubed_is_exactly_six_quads_whatever_its_size_and_place() {
    // the bricks are 8 cells a side: a quad that stopped at a brick would make 24 of a block of 16
    for n in (1..=12).chain([16, 17, 25, 33]) {
        for at in [[0, 0, 0], [-3, 5, -7], [5, 5, 5]] {
            let q = mesh_quads(&grid(block(at, n, 1)), &Classes::identity());
            assert_eq!(q.len(), 6, "n = {n} at {at:?}");
            assert!(q.iter().all(|q| q.w as i32 == n && q.h as i32 == n), "each is the whole face");
        }
    }
}

#[test]
fn a_block_with_a_square_hole_through_it_is_sixteen_quads_and_the_cap_is_four_bars() {
    // each cap is a frame around the opening: with the widest run first, a bar along the top, one down each side and the piece below
    // the opening; four sides outside and four walls in the hole
    for (n, h, a) in [
        (3, 1, 1),
        (4, 2, 1),
        (5, 1, 2),
        (5, 3, 1),
        (6, 2, 2),
        (6, 4, 1),
        (8, 2, 3),
        (8, 4, 2),
        (8, 6, 1),
        (9, 3, 1),
        (12, 2, 5),
        (16, 4, 6),
    ] {
        assert_eq!(mesh_quads(&grid(holed(n, h, a)), &Classes::identity()).len(), 16, "n {n} h {h} a {a}");
    }
    let q = mesh_quads(&grid(holed(8, 2, 3)), &Classes::identity());
    for plane in [0, 8] {
        let cap: Vec<_> =
            q.iter().filter(|q| q.axis == 2 && q.plane == plane).map(|q| (q.u0, q.v0, q.w, q.h)).collect();
        assert_eq!(cap, [(0, 0, 8, 3), (0, 3, 3, 5), (5, 3, 3, 5), (3, 5, 2, 3)], "the cap on the plane {plane}");
    }
    // a hole that touches the border opens two sides: 10 quads
    assert_eq!(mesh_quads(&grid(holed(9, 3, 0)), &Classes::identity()).len(), 10);
}

#[test]
fn the_quads_are_in_the_canonical_order_and_have_the_bits_the_reference_mesher_gives() {
    let q = mesh_quads(&grid(block([0, 0, 0], 8, 1)), &Classes::identity());
    assert!(q.windows(2).all(|w| (w[0].axis, w[0].positive, w[0].plane, w[0].v0, w[0].u0)
        < (w[1].axis, w[1].positive, w[1].plane, w[1].v0, w[1].u0)));
    // the hashes of tools/voxel_reference_mesher.py, FNV-1a 64 of the quads as axis u8, facing u8, plane i32, u0 i32, v0 i32,
    // w u16, h u16, class u8, little endian, in that order
    assert_eq!(format!("{:016x}", quads_hash(&q)), "4a7c15d5ebc45cc8", "a block of 8");
    let q = mesh_quads(&grid(holed(8, 2, 3)), &Classes::identity());
    assert_eq!(format!("{:016x}", quads_hash(&q)), "b61f3c69c0f98e99", "a block of 8 with a hole of 2");
    let q = mesh_quads(&wall(), &Classes::identity());
    assert_eq!(q.len(), 124);
    assert_eq!(format!("{:016x}", quads_hash(&q)), "5fdf69dec601e091", "a wall of 24 by 3 by 16 of two kinds");
}

#[test]
fn every_exposed_face_is_covered_by_exactly_one_quad_of_its_class() {
    let classes = Classes::identity();
    let w = wall();
    assert_eq!(covered(&mesh_quads(&w, &classes)), exposed_faces(&w, &classes).into_iter().map(|f| (f, 1)).collect());
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut cells = Vec::new();
    for z in -7..9 {
        for y in -4..9 {
            for x in -6..11 {
                if next() % 100 < 60 {
                    cells.push(([x, y, z], 1 + (next() % 3) as u8));
                }
            }
        }
    }
    let cloud = grid(cells);
    let glass = Classes::identity().with_see_through(&[2]);
    for c in [&classes, &glass] {
        let quads = mesh_quads(&cloud, c);
        let covers = covered(&quads);
        assert!(covers.values().all(|n| *n == 1), "no face is covered twice");
        assert_eq!(covers.keys().copied().collect::<BTreeSet<_>>(), faces(&cloud, c));
        assert!(quads.len() < covers.len(), "something merged");
    }
}

#[test]
fn the_quads_are_the_same_whatever_the_order_the_cells_came_in() {
    let cells = block([-4, 2, -4], 9, 1).into_iter().filter(|(c, _)| c[0] != 0 || c[1] != 4).collect::<Vec<_>>();
    let mut shuffled = cells.clone();
    let mut seed = 11u64;
    for i in (1..shuffled.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        shuffled.swap(i, (seed >> 33) as usize % (i + 1));
    }
    assert_eq!(mesh_quads(&grid(cells), &Classes::identity()), mesh_quads(&grid(shuffled), &Classes::identity()));
}

#[test]
fn indices_of_one_class_merge_and_indices_of_two_do_not() {
    // 16 indices that look alike are one class: the block is six quads; as 16 classes it is a patchwork
    let cells: Vec<_> = block([0, 0, 0], 10, 1)
        .into_iter()
        .map(|(c, _)| (c, 1 + ((c[0] * 3 + c[1] * 5 + c[2] * 7).rem_euclid(16)) as u8))
        .collect();
    let g = grid(cells);
    assert_eq!(mesh_quads(&g, &Classes::new(|_| 1, &[])).len(), 6);
    assert!(mesh_quads(&g, &Classes::identity()).len() > 100);
}

#[test]
fn a_quad_never_runs_past_what_a_u16_can_say_and_is_cut_in_two() {
    // a bar of 70000 cells: the two ends are one quad each, each of the four long sides is 65535 and 4465
    let g = grid((0..70000).map(|x| ([x, 0, 0], 1)).collect());
    let q = mesh_quads(&g, &Classes::identity());
    assert_eq!(q.len(), 2 + 4 * 2, "{q:?}");
    assert!(q.iter().all(|q| q.w <= 65535 && q.h <= 65535));
    assert_eq!(covered(&q).len(), 2 + 4 * 70000);
}

// ---------------------------------------------------------------------------------------------------------- the vertices

use sr_3d::voxel::surface::expand;
use sr_3d::Vertex;

/// The colour of a class in the tests: exact in f32, so that the reference script can write the same bits.
fn colour(class: u8) -> [f32; 4] {
    [f32::from(class) / 16.0, 0.5, 1.0 - f32::from(class) / 16.0, 1.0]
}

/// FNV-1a 64 over the bytes of the vertices and then those of the indices (the hash of tools/voxel_reference_mesher.py).
fn expanded_hash(p: &sr_3d::Primitive) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in bytemuck::cast_slice::<Vertex, u8>(&p.vertices).iter().chain(bytemuck::cast_slice::<u32, u8>(&p.indices)) {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn the_vertex_the_gpu_reads_is_ninety_six_bytes() {
    assert_eq!(std::mem::size_of::<Vertex>(), 96);
}

#[test]
fn the_expanded_bytes_are_the_ones_the_reference_script_writes_for_a_block_a_hole_and_a_wall() {
    let classes = Classes::identity();
    let block = expand(&mesh_quads(&grid(block([0, 0, 0], 8, 1)), &classes), colour);
    assert_eq!((block.vertices.len(), block.indices.len()), (24, 36));
    assert_eq!(format!("{:016x}", expanded_hash(&block)), "a3ef39dada818385", "a block of 8");
    let holed = expand(&mesh_quads(&grid(holed(8, 2, 3)), &classes), colour);
    assert_eq!((holed.vertices.len(), holed.indices.len()), (64, 96));
    assert_eq!(format!("{:016x}", expanded_hash(&holed)), "004a4e2ff70cae05", "a block of 8 with a hole of 2");
    let wall = expand(&mesh_quads(&wall(), &classes), colour);
    assert_eq!((wall.vertices.len(), wall.indices.len()), (496, 744));
    assert_eq!(format!("{:016x}", expanded_hash(&wall)), "e757453f72afe9c5", "the wall");
}

#[test]
fn a_quad_is_four_vertices_of_one_colour_with_a_unit_normal_a_tangent_along_u_and_a_front_that_looks_out() {
    let classes = Classes::identity();
    let mut cells = Vec::new();
    for z in -5i32..6 {
        for y in -3i32..5 {
            for x in -4i32..7 {
                if (x * 7 + y * 13 + z * 3).rem_euclid(5) < 3 {
                    cells.push(([x, y, z], 1 + (x + y + z).rem_euclid(3) as u8));
                }
            }
        }
    }
    let g = grid(cells);
    let quads = mesh_quads(&g, &classes);
    let p = expand(&quads, colour);
    assert_eq!((p.vertices.len(), p.indices.len()), (4 * quads.len(), 6 * quads.len()));
    assert!(p.indices.iter().all(|i| (*i as usize) < p.vertices.len()));
    let mut area = 0.0f64;
    for (n, q) in quads.iter().enumerate() {
        let v = &p.vertices[4 * n..4 * n + 4];
        let sign = if q.positive { 1.0 } else { -1.0 };
        let mut normal = [0.0; 3];
        normal[q.axis as usize] = sign;
        let mut along_u = [0.0; 3];
        along_u[(q.axis as usize + 1) % 3] = 1.0;
        let mut along_v = [0.0; 3];
        along_v[(q.axis as usize + 2) % 3] = 1.0;
        for vertex in v {
            assert_eq!(vertex.normal, normal);
            assert_eq!(vertex.color, colour(q.class), "the colour of the class, the same at the four corners");
            assert_eq!(vertex.map_uv, [[0.0; 2]; 4]);
            assert_eq!(
                vertex.tangent,
                [along_u[0], along_u[1], along_u[2], sign],
                "the tangent runs along +u, its sign is the facing's"
            );
            // the bitangent cross(n, t) * w is +v, the convention of compute_tangents
            let b = cross(vertex.normal, [vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]]);
            assert_eq!([b[0] * sign, b[1] * sign, b[2] * sign], along_v);
            assert_eq!(vertex.pos[q.axis as usize], q.plane as f32);
        }
        assert_eq!(v[0].uv, [q.u0 as f32, q.v0 as f32]);
        assert_eq!(v[2].uv, [(q.u0 + q.w as i32) as f32, (q.v0 + q.h as i32) as f32]);
        // both triangles wind counter-clockwise seen from outside: (b - a) x (c - a) points along the normal
        for t in p.indices[6 * n..6 * n + 6].chunks(3) {
            let (a, b, c) =
                (p.vertices[t[0] as usize].pos, p.vertices[t[1] as usize].pos, p.vertices[t[2] as usize].pos);
            let cr = cross(sub(b, a), sub(c, a));
            assert!(dot(cr, normal) > 0.0, "the front of quad {n} looks in");
            area += f64::from(0.5 * dot(cr, normal));
        }
    }
    // every exposed face is one square of the surface: the area of the triangles is their count
    assert_eq!(area, exposed_faces(&g, &classes).len() as f64);
    // the box of the positions is the box of the cells, each cell a unit
    let (lo, hi) = g.bounds().unwrap();
    for a in 0..3 {
        let (min, max) =
            p.vertices.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.pos[a]), hi.max(v.pos[a])));
        assert_eq!((min, max), (lo[a] as f32, (hi[a] + 1) as f32), "axis {a}");
    }
}

#[test]
fn no_quads_make_an_empty_primitive() {
    let p = expand(&[], colour);
    assert!(p.vertices.is_empty() && p.indices.is_empty());
}

// ------------------------------------------------------------------------------------------------------------------ budget

use sr_3d::voxel::surface::{mesh_quads_within, BYTES_PER_QUAD};

/// A checkerboard of `n` cells a side: every cell has its six faces, none of which merge.
fn checkerboard(n: i32) -> Occupancy {
    grid(block([0, 0, 0], n, 1).into_iter().filter(|(c, _)| (c[0] + c[1] + c[2]) % 2 == 0).collect())
}

#[test]
fn a_surface_that_fits_its_budget_is_made_and_one_a_quad_over_is_refused_with_its_cause() {
    assert_eq!(
        BYTES_PER_QUAD, 1240,
        "16 for the quad, 408 for the builder's vertices, 408 for the copy of the upload, 408 on the device"
    );
    // the quads that a budget admits are its bytes over the bytes a quad costs at its peak, rounded down
    assert_eq!((128usize << 20) / BYTES_PER_QUAD, 108_240);
    assert_eq!((256usize << 20) / BYTES_PER_QUAD, 216_480);
    let g = checkerboard(6);
    let quads = mesh_quads(&g, &Classes::identity()).len();
    assert_eq!(quads, 6 * 108, "six faces for each of the 108 cells");
    let fits =
        mesh_quads_within(&g, &Classes::identity(), quads * BYTES_PER_QUAD).expect("a budget of exactly its quads");
    assert_eq!(fits, mesh_quads(&g, &Classes::identity()), "the same quads as with no budget");
    let refused = mesh_quads_within(&g, &Classes::identity(), quads * BYTES_PER_QUAD - 1).unwrap_err();
    assert!(
        refused.contains("voxel surface exceeds memory budget") && refused.contains("surfaceMemoryMiB"),
        "{refused}"
    );
    assert!(refused.contains(&format!("{} quads", quads - 1)), "it says how many the budget admits: {refused}");
}

#[test]
fn a_surface_over_its_budget_is_refused_with_an_error_that_does_not_depend_on_how_far_over_it_is() {
    // checkerboards of 40 and of 20 under a budget of 1,000 quads: both over it, by 191,000 quads and by 5,000, and the same error
    // (the work stops at the first plane that takes the surface over, so it never counted the rest)
    let budget = 1000 * BYTES_PER_QUAD;
    let (big, small) = (checkerboard(40), checkerboard(20));
    assert_eq!(mesh_quads(&big, &Classes::identity()).len(), 6 * 32_000);
    assert_eq!(mesh_quads(&small, &Classes::identity()).len(), 6 * 4_000);
    let refused = mesh_quads_within(&big, &Classes::identity(), budget).unwrap_err();
    assert_eq!(refused, mesh_quads_within(&small, &Classes::identity(), budget).unwrap_err());
    assert!(refused.contains("1000 quads"), "{refused}");
    // nothing to draw is nothing to pay
    assert!(mesh_quads_within(&Occupancy::new(), &Classes::identity(), 0).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------------------------ the cache

use sr_3d::voxel::surface::SurfaceCache;

/// The planes (axis, plane) that an edit of the bricks `keys` can change: the nine planes of the slab of each brick along each axis.
fn dirty_planes(keys: &[[i32; 3]]) -> BTreeSet<(usize, i32)> {
    keys.iter().flat_map(|k| (0..3).flat_map(move |a| (0..=8).map(move |i| (a, 8 * k[a] + i)))).collect()
}

/// A grid built afresh from the cells of `g`, the history of its edits forgotten.
fn fresh(g: &Occupancy) -> Occupancy {
    Occupancy::from_cells(g.cells().map(|c| (c, g.get(c)))).unwrap()
}

#[test]
fn a_cache_that_is_brought_up_to_date_gives_the_quads_of_a_fresh_mesh_and_remeshes_nothing_if_nothing_changed() {
    let classes = Classes::identity();
    let mut g = grid(block([-3, 2, -5], 20, 1));
    let mut cache = SurfaceCache::new();
    let first = cache.update(&g, &classes, usize::MAX).unwrap();
    assert!(first.full, "the first time is the whole of it");
    assert_eq!(cache.quads(), mesh_quads(&g, &classes));
    let again = cache.update(&g, &classes, usize::MAX).unwrap();
    assert_eq!((again.remeshed, again.full), (0, false), "the same revision of the same grid: nothing to do");
    // a recolouring changes no cell, no revision, no quad
    g.set_color(1, [9, 9, 9, 255]);
    assert_eq!(cache.update(&g, &classes, usize::MAX).unwrap().remeshed, 0);
    assert_eq!(cache.quads(), mesh_quads(&g, &classes));
}

#[test]
fn a_cut_of_k_bricks_remeshes_the_planes_of_those_bricks_and_no_others_and_gives_the_fresh_mesh() {
    let classes = Classes::identity();
    // a block of 64 (8 bricks a side), a sphere of cells cut out of one corner: it touches a few bricks
    let mut g = grid(block([0, 0, 0], 64, 1));
    let mut cache = SurfaceCache::new();
    cache.update(&g, &classes, usize::MAX).unwrap();
    let before = g.revision();
    // 65 planes along each axis have a layer of the 8 bricks of a side on one of their sides
    let total_planes = 3 * 65;
    for z in 0..12i32 {
        for y in 0..12i32 {
            for x in 0..12i32 {
                if (x - 5) * (x - 5) + (y - 5) * (y - 5) + (z - 5) * (z - 5) <= 36 {
                    g.set([x, y, z], 0).unwrap();
                }
            }
        }
    }
    let changed = g.changed_bricks_since(before);
    assert!(changed.len() > 1 && changed.len() <= 8, "the cut spans a few bricks: {}", changed.len());
    let update = cache.update(&g, &classes, usize::MAX).unwrap();
    assert!(!update.full);
    assert_eq!(
        update.remeshed,
        dirty_planes(&changed).len(),
        "the nine planes of each changed brick on each axis, shared ones once"
    );
    assert!(update.remeshed * 3 < total_planes, "{} of {total_planes} planes", update.remeshed);
    assert_eq!(cache.quads(), mesh_quads(&fresh(&g), &classes), "the same quads as a mesh of the cut grid made afresh");
}

#[test]
fn after_any_sequence_of_edits_the_cache_is_the_mesh_of_a_fresh_grid_even_across_emptied_bricks_and_compaction() {
    let classes = Classes::identity().with_see_through(&[3]);
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut g = Occupancy::new();
    let mut cache = SurfaceCache::new();
    for step in 0..60 {
        // a few edits near brick borders, clustered so that bricks fill and empty: a box of 3 or a single cell, set or cleared
        for _ in 0..1 + next(4) {
            let at = [next(40) as i32 - 20, next(24) as i32 - 12, next(40) as i32 - 20];
            let value = if next(3) == 0 { 0 } else { 1 + next(3) as u8 };
            let size = if next(4) == 0 { 4 } else { 1 };
            for z in 0..size {
                for y in 0..size {
                    for x in 0..size {
                        g.set([at[0] + x, at[1] + y, at[2] + z], value).unwrap();
                    }
                }
            }
        }
        let update = cache.update(&g, &classes, usize::MAX).unwrap();
        assert_eq!(
            cache.quads(),
            mesh_quads(&fresh(&g), &classes),
            "step {step} ({} planes remeshed, full: {})",
            update.remeshed,
            update.full
        );
        // the cache has read up to this revision: the grid may forget the bricks it emptied
        if step % 7 == 6 {
            g.compact(g.revision());
        }
    }
}

#[test]
fn another_grid_an_older_state_or_other_classes_are_a_full_remesh() {
    let classes = Classes::identity();
    let mut g = grid(block([0, 0, 0], 12, 1));
    let mut cache = SurfaceCache::new();
    cache.update(&g, &classes, usize::MAX).unwrap();
    // a copy edited has a lineage of its own: its revision is not a revision of the grid the cache holds
    let mut copy = g.clone();
    copy.set([3, 3, 3], 0).unwrap();
    let update = cache.update(&copy, &classes, usize::MAX).unwrap();
    assert!(update.full, "another lineage");
    assert_eq!(cache.quads(), mesh_quads(&copy, &classes));
    // the grid the cache held, edited past the copy's revision, is another grid again
    g.set([1, 1, 1], 0).unwrap();
    g.set([2, 2, 2], 0).unwrap();
    assert!(cache.update(&g, &classes, usize::MAX).unwrap().full);
    // and a revision behind the cached one, of the same grid, cannot be told from what is cached
    let mut older = g.clone();
    older.set([5, 5, 5], 0).unwrap();
    cache.update(&older, &classes, usize::MAX).unwrap();
    let behind = {
        let mut b = g.clone();
        b.set([6, 6, 6], 0).unwrap();
        b
    };
    assert!(cache.update(&behind, &classes, usize::MAX).unwrap().full, "another copy");
    // other classes mean other quads, not the cached ones
    let glass = Classes::identity().with_see_through(&[1]);
    let update = cache.update(&behind, &glass, usize::MAX).unwrap();
    assert!(update.full, "the classes changed");
    assert_eq!(cache.quads(), mesh_quads(&behind, &glass));
}

#[test]
fn a_cache_over_its_budget_is_refused_and_forgets_what_it_had() {
    let classes = Classes::identity();
    let mut g = grid(block([0, 0, 0], 16, 1));
    let mut cache = SurfaceCache::new();
    cache.update(&g, &classes, usize::MAX).unwrap();
    // cells that touch only by their edges: 6 faces each, 16 of them, 96 quads over a budget of 12 quads
    for x in 0..8 {
        g.set([2 * x, 0, 20], 1).unwrap();
        g.set([2 * x + 1, 0, 21], 1).unwrap();
    }
    let refused = cache.update(&g, &classes, 12 * BYTES_PER_QUAD).unwrap_err();
    assert!(refused.contains("voxel surface exceeds memory budget"), "{refused}");
    assert!(cache.quads().is_empty(), "a refused surface leaves nothing to draw");
    let update = cache.update(&g, &classes, usize::MAX).unwrap();
    assert!(update.full, "what was forgotten is made again");
    assert_eq!(cache.quads(), mesh_quads(&g, &classes));
}

#[test]
fn a_grid_given_by_a_key_of_its_owner_is_remeshed_from_the_bricks_the_owner_says_changed() {
    let classes = Classes::identity();
    // the world keeps a body's cells by revision and says which bricks changed between two: each frame hands over a snapshot (a copy,
    // so another lineage each time) with the key (body, revision) and the bricks changed since the revision the cache last read
    let mut live = grid(block([-3, 2, -5], 24, 1));
    let mut cache = SurfaceCache::new();
    let first = cache.update_known(&live.clone(), &classes, usize::MAX, (7, live.revision()), None).unwrap();
    assert!(first.full);
    let mut read = live.revision();
    for step in 0..12 {
        let at = [(step * 5) % 20 - 4, (step * 3) % 16 + 2, (step * 7) % 20 - 6];
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    live.set([at[0] + x, at[1] + y, at[2] + z], u8::from(step % 3 != 0)).unwrap();
                }
            }
        }
        let changed = live.changed_bricks_since(read);
        let update =
            cache.update_known(&live.clone(), &classes, usize::MAX, (7, live.revision()), Some(&changed)).unwrap();
        assert!(!update.full, "step {step}");
        assert_eq!(
            update.remeshed,
            dirty_planes(&changed).len(),
            "step {step}: the planes of the bricks the owner named"
        );
        assert_eq!(cache.quads(), mesh_quads(&fresh(&live), &classes), "step {step}");
        read = live.revision();
    }
    // the same key is nothing to do, even for another copy of the cells
    let again = cache.update_known(&live.clone(), &classes, usize::MAX, (7, live.revision()), Some(&[])).unwrap();
    assert_eq!((again.remeshed, again.full), (0, false));
    // a history the owner no longer has (no list), another body, or other classes: the whole of it, from the cells handed over
    live.set([0, 0, 0], 0).unwrap();
    assert!(cache.update_known(&live.clone(), &classes, usize::MAX, (7, live.revision()), None).unwrap().full);
    assert!(cache.update_known(&live.clone(), &classes, usize::MAX, (8, live.revision()), Some(&[])).unwrap().full);
    let glass = Classes::identity().with_see_through(&[1]);
    assert!(cache.update_known(&live.clone(), &glass, usize::MAX, (8, live.revision()), Some(&[])).unwrap().full);
    assert_eq!(cache.quads(), mesh_quads(&live, &glass));
    // a key behind the one the cache holds cannot be told from what is cached
    live.set([1, 1, 1], 0).unwrap();
    cache
        .update_known(
            &live.clone(),
            &glass,
            usize::MAX,
            (8, live.revision()),
            Some(&live.changed_bricks_since(live.revision() - 1)),
        )
        .unwrap();
    assert!(cache.update_known(&live.clone(), &glass, usize::MAX, (8, live.revision() - 1), Some(&[])).unwrap().full);
}

#[test]
fn a_frame_that_reads_the_steps_of_an_owner_remeshes_the_union_of_the_bricks_of_the_cuts_it_missed() {
    let classes = Classes::identity();
    // an owner counts cuts: revision r has the cells after r cuts, and steps lists the bricks each cut changed, as `(r, bricks)`
    let mut live = grid(block([-3, 2, -5], 24, 1));
    let mut by_cut = vec![live.clone()];
    let mut steps: Vec<(u64, Vec<[i32; 3]>)> = Vec::new();
    for cut in 0..6i32 {
        let before = live.revision();
        for z in 0..5 {
            for y in 0..5 {
                for x in 0..5 {
                    live.set([cut * 4 - 3 + x, 3 + y, 2 * cut - 5 + z], 0).unwrap();
                }
            }
        }
        steps.push((cut as u64 + 1, live.changed_bricks_since(before)));
        by_cut.push(live.clone());
    }
    let mut cache = SurfaceCache::new();
    let ask = |cache: &mut SurfaceCache, body: u64, r: usize, steps: &[(u64, Vec<[i32; 3]>)]| {
        cache.update_steps(&by_cut[r], &classes, usize::MAX, body, r as u64, steps)
    };
    assert!(ask(&mut cache, 7, 0, &[]).unwrap().full, "the first read is the whole of it");
    // reads that skip cuts (0 to 2, then 3, then 5): the planes of the union of the bricks of the cuts after the one read
    let mut held = 0;
    for read in [2usize, 3, 5] {
        let union: Vec<[i32; 3]> = steps[held..read].iter().flat_map(|(_, b)| b.iter().copied()).collect();
        let update = ask(&mut cache, 7, read, &steps[..read]).unwrap();
        assert!(!update.full, "revision {read}");
        assert_eq!(update.remeshed, dirty_planes(&union).len(), "revision {read}");
        assert_eq!(cache.quads(), mesh_quads(&fresh(&by_cut[read]), &classes), "revision {read}");
        held = read;
    }
    let again = ask(&mut cache, 7, 5, &steps[..5]).unwrap();
    assert_eq!((again.remeshed, again.full), (0, false), "the same revision is nothing to do");
    // a cut whose history the owner no longer has (the steps stop short of the revision), a revision behind, another body: the whole of it
    assert!(ask(&mut cache, 7, 6, &steps[..4]).unwrap().full, "a step missing");
    assert_eq!(cache.quads(), mesh_quads(&fresh(&by_cut[6]), &classes));
    assert!(ask(&mut cache, 7, 4, &steps[..4]).unwrap().full, "behind");
    assert!(ask(&mut cache, 8, 4, &steps[..4]).unwrap().full, "another body");
    assert_eq!(cache.quads(), mesh_quads(&fresh(&by_cut[4]), &classes));
    ask(&mut cache, 8, 6, &steps).unwrap();
    assert_eq!(ask(&mut cache, 8, 6, &steps).unwrap().remeshed, 0);
}

#[test]
fn a_surface_over_its_budget_is_refused_at_the_plane_that_takes_it_over_and_not_after_the_whole_of_it_is_made() {
    // a checkerboard of 100 cubed: every cell shows its six faces, 3 000 000 quads in all and 10 000 on every plane but the first of each axis
    let cells: Vec<([i32; 3], u8)> = (0..100)
        .flat_map(|z| (0..100).flat_map(move |y| (0..100).map(move |x| ([x, y, z], 1u8))))
        .filter(|(c, _)| (c[0] + c[1] + c[2]) % 2 == 0)
        .collect();
    let board = grid(cells);
    let classes = Classes::identity();
    for production in [false, true] {
        let mut cache = SurfaceCache::new();
        let budget = 15_000 * BYTES_PER_QUAD;
        let refused = if production {
            cache.update_known(&board, &classes, budget, (3, 1), None)
        } else {
            cache.update(&board, &classes, budget)
        }
        .unwrap_err();
        assert!(refused.contains("voxel surface exceeds memory budget"), "{refused}");
        // the budget admits the first plane (5 000 quads at the edge) and the second (10 000): the third takes it over, and no plane is made after
        assert!(cache.planes_meshed() <= 3, "production {production}: {} planes meshed", cache.planes_meshed());
        assert!(cache.quads().is_empty());
    }
}
