//! Mass properties of a body of cells, from exact integer moments.
//!
//! Parry gives a voxel shape the mass properties of its cells, but it diagonalises the tensor (`MassProperties::with_inertia_matrix`, which calls
//! `symmetric_eigen` of the `glamx` crate) and for a tensor that is already diagonal with two equal moments that are smaller than the third, which is
//! on x (a plate that is thin along x, a block that is shorter along x than it is wide) the principal values and the frame it returns do not
//! belong together: the tensor that the world simulates is then another body's (a plate with a hole, thin along x, had half its moment about x).
//!
//! The smallest case that shows it is a block of 3 by 4 by 4 cubic cells of 0.25 m at 2400 kg/m3, whose tensor about its centre is
//! diag(300, 234.375, 234.375): `MassProperties::with_inertia_matrix` of it, read back with `reconstruct_inertia_matrix`, gives
//! diag(234.375, 300, 234.375), and diag(234.375, 300, 234.375) gives the very same principal moments and frame, so one of the two is wrong
//! (it is the first: the second is read back right; `cargo run -p sr-sim --example parry_inertia_defect` prints both). The cause, read in
//! `glamx` 0.3.1 `eigen3.rs` (the review of this branch by Urano found it): `eigenvector1` returns the x axis whenever every column cross product of
//! `M - l1 I` is zero, which is the case for any repeated smallest eigenvalue, not only for three equal ones; the x axis is then right only if the
//! distinct, larger moment is not on x. The same solver is behind `MassProperties`' `Sum` (and `Add`), which `with_fractures` used to give a
//! source the tensor of its fragments: for fragments that are bodies of cells it now uses [`sum_mass_properties`]. If a later Parry returns
//! the right thing for all of them, `voxel_mass_properties` can go back to `with_inertia_matrix`; the tests in sr-eval (`mass_properties`,
//! `world_inertia`) compare with exact moments and would show it.
//!
use super::*;

/// Mass, centre of mass and inertia tensor about it of the union of cubes `keys` of side `size` (metres, along each axis) and total `mass`, in the
/// axes of the lattice.
pub(super) struct Tensor {
    pub(super) mass: f64,
    pub(super) centre: [f64; 3],
    pub(super) inertia: [[f64; 3]; 3],
}

/// The tensor of the cells `keys` (each cell once), or none for no cells or a mass or size that is not positive.
pub(super) fn voxel_tensor(keys: &[IVector], size: [f64; 3], mass: f64) -> Option<Tensor> {
    if keys.is_empty() || !(mass.is_finite() && mass > 0.0) || !size.iter().all(|s| s.is_finite() && *s > 0.0) {
        return None;
    }
    // sums over the doubled coordinates u = 2 key + 1: n, sum u, and sum u u (xx, yy, zz, xy, xz, yz)
    let (mut sum, mut cross) = ([0i128; 3], [0i128; 6]);
    for k in keys {
        let u = [2 * i128::from(k.x) + 1, 2 * i128::from(k.y) + 1, 2 * i128::from(k.z) + 1];
        for a in 0..3 {
            sum[a] += u[a];
        }
        for (slot, (a, b)) in [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)].into_iter().enumerate() {
            cross[slot] += u[a] * u[b];
        }
    }
    let n = keys.len() as f64;
    let ni = keys.len() as i128;
    let cell_mass = mass / n;
    let centre: [f64; 3] = std::array::from_fn(|a| size[a] * (sum[a] as f64) / (2.0 * n));
    // the covariance of the doubled coordinates, exact in the numerator
    let pairs = [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)];
    let covariance = |slot: usize| {
        let (a, b) = pairs[slot];
        (ni * cross[slot] - sum[a] * sum[b]) as f64 / (n * n)
    };
    let var = |a: usize| size[a] * size[a] / 4.0 * covariance(a);
    let mut inertia = [[0.0; 3]; 3];
    for (a, row) in inertia.iter_mut().enumerate() {
        let (b, c) = ((a + 1) % 3, (a + 2) % 3);
        let own = cell_mass / 12.0 * (size[b] * size[b] + size[c] * size[c]);
        row[a] = n * own + mass * (var(b) + var(c));
    }
    for (slot, (a, b)) in pairs.into_iter().enumerate().skip(3) {
        let product = -mass * size[a] * size[b] / 4.0 * covariance(slot);
        inertia[a][b] = product;
        inertia[b][a] = product;
    }
    Some(Tensor { mass, centre, inertia })
}

/// The principal moments and the right-handed frame (the columns are the principal axes) of a symmetric tensor, by cyclic Jacobi rotations in a
/// fixed order. A tensor that is already diagonal is not rotated: the frame is the identity and the moments are in the order of the axes.
fn principal(inertia: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = inertia;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        let scale = a[0][0].abs() + a[1][1].abs() + a[2][2].abs();
        if off <= 1e-300 || off <= 1e-17 * scale {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = if theta >= 0.0 { 1.0 } else { -1.0 } / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in a.iter_mut() {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (rp, rq) = (a[p], a[q]);
            a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
            a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            for row in v.iter_mut() {
                let (vkp, vkq) = (row[p], row[q]);
                row[p] = c * vkp - s * vkq;
                row[q] = s * vkp + c * vkq;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// The mass properties of the cells `keys`, as Rapier takes them: centre of mass, mass, principal moments and their frame.
pub(super) fn voxel_mass_properties(keys: &[IVector], size: [f64; 3], mass: f64) -> Option<MassProperties> {
    let t = voxel_tensor(keys, size, mass)?;
    let (moments, axes) = principal(t.inertia);
    // a right-handed frame: the third axis is the cross product of the first two
    let cross =
        |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let (x, y) = ([axes[0][0], axes[1][0], axes[2][0]], [axes[0][1], axes[1][1], axes[2][1]]);
    let z = cross(x, y);
    let frame = Rotation::from_mat3(&Matrix::from_cols(vec3(x), vec3(y), vec3(z)));
    Some(MassProperties::with_principal_inertia_frame(vec3(t.centre), t.mass, vec3(moments), frame))
}

/// The mass properties of a body that is the union of `parts` (each in the frame of the body): the masses added, the centre of mass as the mean
/// weighted by mass, and the tensor about it as the sum of the tensors of the parts (each read back from its principal moments and frame) and their
/// parallel-axis terms, diagonalised as [`voxel_mass_properties`] does. Arithmetic in `f64`, in the order of the parts.
pub(super) fn sum_mass_properties(parts: &[MassProperties]) -> MassProperties {
    let mass: f64 = parts.iter().map(|p| p.mass()).sum();
    let centre = parts.iter().fold(Vector::ZERO, |sum, p| sum + p.local_com * p.mass()) / mass;
    let mut inertia = [[0.0; 3]; 3];
    for p in parts {
        let own = p.reconstruct_inertia_matrix();
        let d = p.local_com - centre;
        let d2 = d.length_squared();
        let d = [d.x, d.y, d.z];
        for (i, row) in inertia.iter_mut().enumerate() {
            for (j, entry) in row.iter_mut().enumerate() {
                *entry += own.col(j)[i] + p.mass() * (if i == j { d2 } else { 0.0 } - d[i] * d[j]);
            }
        }
    }
    let (moments, axes) = principal(inertia);
    let (x, y) = ([axes[0][0], axes[1][0], axes[2][0]], [axes[0][1], axes[1][1], axes[2][1]]);
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    let frame = Rotation::from_mat3(&Matrix::from_cols(vec3(x), vec3(y), vec3(z)));
    MassProperties::with_principal_inertia_frame(centre, mass, vec3(moments), frame)
}

/// The mass properties of a body whose surface is the closed triangle mesh `triangles` over `points` (physics axes, metres, outward windings
/// or all inward) and total `mass`, uniformly dense: the volume integrals of the signed tetrahedra that each triangle makes with the origin, which
/// are exact for a polyhedron, and the tensor diagonalised as [`voxel_mass_properties`] does. A mesh that is not closed and consistently wound has no
/// volume to speak of (the integrals depend on where the origin is) and is refused: every edge must be met once in each direction, the corners
/// that are at one place being one (a mesh cut apart has corners of its own for every triangle).
pub(super) fn mesh_mass_properties(points: &[Vector], triangles: &[[u32; 3]], mass: f64) -> Option<MassProperties> {
    if triangles.is_empty() || !(mass.is_finite() && mass > 0.0) || !closed_and_consistent(points, triangles)? {
        return None;
    }
    let (mut volume, mut moment) = (0.0f64, Vector::ZERO);
    let mut second = [[0.0f64; 3]; 3];
    for t in triangles {
        let (a, b, c) = (*points.get(t[0] as usize)?, *points.get(t[1] as usize)?, *points.get(t[2] as usize)?);
        let v = a.dot(b.cross(c)) / 6.0;
        let sum = a + b + c;
        volume += v;
        moment += sum * (v / 4.0);
        let (a, b, c, s) = ([a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z], [sum.x, sum.y, sum.z]);
        for i in 0..3 {
            for j in 0..3 {
                second[i][j] += v / 20.0 * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j] + s[i] * s[j]);
            }
        }
    }
    // windings that are all inward: the same body, with its signs reversed
    if volume < 0.0 {
        volume = -volume;
        moment = -moment;
        for row in second.iter_mut() {
            for e in row.iter_mut() {
                *e = -*e;
            }
        }
    }
    if !(volume.is_finite() && volume > 0.0) {
        return None;
    }
    let centre = moment / volume;
    let c = [centre.x, centre.y, centre.z];
    let density = mass / volume;
    let covariance = |i: usize, j: usize| second[i][j] - volume * c[i] * c[j];
    let trace = covariance(0, 0) + covariance(1, 1) + covariance(2, 2);
    let inertia: [[f64; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| density * (if i == j { trace } else { 0.0 } - covariance(i, j)))
    });
    tensor_mass_properties(centre, mass, inertia)
}

/// The mass properties of a body of `mass` with its centre of mass at `centre` and the tensor `inertia` about it (in the same axes), the tensor
/// diagonalised as [`voxel_mass_properties`] does. `None` for one that is not symmetric or not a number.
pub(super) fn tensor_mass_properties(centre: Vector, mass: f64, inertia: [[f64; 3]; 3]) -> Option<MassProperties> {
    let scale = inertia.iter().flatten().fold(0.0f64, |m, e| m.max(e.abs()));
    let symmetric = (0..3).all(|i| (0..3).all(|j| (inertia[i][j] - inertia[j][i]).abs() <= 1e-9 * scale));
    if !(mass.is_finite() && mass > 0.0 && centre.is_finite() && scale.is_finite() && symmetric) {
        return None;
    }
    let (moments, axes) = principal(inertia);
    let (x, y) = ([axes[0][0], axes[1][0], axes[2][0]], [axes[0][1], axes[1][1], axes[2][1]]);
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    let frame = Rotation::from_mat3(&Matrix::from_cols(vec3(x), vec3(y), vec3(z)));
    Some(MassProperties::with_principal_inertia_frame(centre, mass, vec3(moments), frame))
}

/// Whether the surface is closed and its triangles are wound alike: every directed edge is met exactly once and so is its reverse (corners at the same
/// place are the same corner). `None` if a triangle names a corner that is not there.
fn closed_and_consistent(points: &[Vector], triangles: &[[u32; 3]]) -> Option<bool> {
    let mut ids: std::collections::HashMap<[u64; 3], u32> = std::collections::HashMap::new();
    let mut corner = Vec::with_capacity(points.len());
    for p in points {
        // + 0.0 makes -0.0 into 0.0: the same place has one key
        let key = [(p.x + 0.0).to_bits(), (p.y + 0.0).to_bits(), (p.z + 0.0).to_bits()];
        let next = ids.len() as u32;
        corner.push(*ids.entry(key).or_insert(next));
    }
    let mut seen: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in triangles {
        let c = [*corner.get(t[0] as usize)?, *corner.get(t[1] as usize)?, *corner.get(t[2] as usize)?];
        for (a, b) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[0])] {
            if a != b {
                *seen.entry((a, b)).or_insert(0) += 1;
            }
        }
    }
    Some(seen.iter().all(|(&(a, b), &n)| n == 1 && seen.get(&(b, a)) == Some(&1)))
}

/// The mass properties of a convex hull of total `mass`, from the exact integrals over its faces (fanned into triangles, wound as their face's
/// normal says), and not from Parry's hull routine, which diagonalises with the solver whose mistake the rest of this module works around.
pub(super) fn hull_mass_properties(
    hull: &rapier3d_f64::parry::shape::ConvexPolyhedron,
    mass: f64,
) -> Option<MassProperties> {
    let points = hull.points();
    let adjacent = hull.vertices_adj_to_face();
    let mut triangles = Vec::new();
    for face in hull.faces() {
        let first = face.first_vertex_or_edge as usize;
        let ring = adjacent.get(first..first + face.num_vertices_or_edges as usize)?;
        for w in 1..ring.len().saturating_sub(1) {
            let mut t = [ring[0], ring[w], ring[w + 1]];
            let (a, b, c) = (*points.get(t[0] as usize)?, *points.get(t[1] as usize)?, *points.get(t[2] as usize)?);
            if (b - a).cross(c - a).dot(face.normal) < 0.0 {
                t.swap(1, 2);
            }
            triangles.push(t);
        }
    }
    mesh_mass_properties(points, &triangles, mass)
}
