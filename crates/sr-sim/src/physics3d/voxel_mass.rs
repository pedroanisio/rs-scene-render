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
//! (it is the first: the second is read back right; `cargo run -p sr-sim --example parry_inertia_defect` prints both). What I believe, and have not traced in the solver: it sorts the moments and
//! chooses the eigenvector of the distinct one by a rule that is right when that one is on y or on z and when the three are alike, and not when it is on x. If a
//! later Parry returns the right thing for all of them, `voxel_mass_properties` can go back to `with_inertia_matrix`; the tests in sr-eval
//! (`mass_properties`, `world_inertia`) compare with exact moments and would show it.
//! So the tensor is worked out here from the cells: sums of integers, in `i128`, one
//! division each, the same bits in any order of the cells, and diagonalised by Jacobi rotations that leave a diagonal tensor as it is.
use super::*;

/// Mass, centre of mass and inertia tensor about it of the union of cubes `keys` of side `size` (metres, along each axis) and total `mass`, in the
/// axes of the lattice.
pub(super) struct Tensor {
    pub(super) mass: f64,
    pub(super) centre: [f64; 3],
    pub(super) inertia: [[f64; 3]; 3],
}

/// The keys of the filled cells of a voxel shape.
pub(super) fn keys_of(voxels: &Voxels) -> Vec<IVector> {
    voxels.voxels().map(|v| v.grid_coords).collect()
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
