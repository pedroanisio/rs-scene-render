//! Multigrid preconditioner and deterministic block reductions for the pressure
//! conjugate-gradient solve (`PressureSolver::Multigrid`).
//!
//! Numerical contract (every item below defines the result bits):
//!
//! * Coarsening is algebraic aggregation of 2x2x2 cell blocks (`ceil(n/2)` per
//!   axis; the last block of an odd axis has one child) with the Galerkin
//!   operator `Pᵀ A P` for piecewise-constant `P`. This is the graph Laplacian of
//!   the aggregated cells: the weight between two aggregates is the number of
//!   open fluid faces between their children, and the diagonal is the number of
//!   open-boundary (zero-pressure) faces plus the incident weights. Solid cells
//!   have no unknown, so solids and open or closed edges follow the same rule at
//!   every level.
//! * Coarsening stops at `DIRECT_CELLS` cells or fewer; that level is solved
//!   directly by dense Cholesky. Every connected component without a
//!   zero-pressure face (a closed domain, or a fluid pocket sealed by solids) is
//!   singular, so one unknown per such component is pinned to zero.
//! * One V(1,1) cycle per application: a red-then-black Gauss-Seidel sweep from
//!   a zero guess, restriction of the residual by summing children, the coarse
//!   correction scaled by `OVERCORRECTION`, then black-then-red post-smoothing.
//!   The pre and post sweeps are adjoint, so the cycle is a symmetric positive
//!   definite operator, as conjugate gradients requires.
//! * Reductions sum fixed blocks of `BLOCK` consecutive cells serially in index
//!   order, then combine the partial sums pairwise (0+1, 2+3, ...) in a fixed
//!   binary tree. No value depends on the thread count.

//!
//! Parameter experiment behind `OVERCORRECTION = 1.5` and V(1,1). Impact scene
//! (hero domain, open edges, impulse at step 3), 8 threads, load average ~7 so
//! times are +-30%. Cells are `iterations of CG / ms of the pressure stage`:
//!
//! ```text
//!                       128^3 impulse  128^3 regime  192^3 impulse  192^3 regime
//! V(1,1) alpha 1.0        22 / 287       10 / 197      28 / 1187      14 / 699
//! V(1,1) alpha 1.25       16 / 216        7 / 131      19 / 1014      10 / 629
//! V(1,1) alpha 1.5        14 / 233        7 / 157      16 /  784       9 / 595  <- chosen
//! V(1,1) alpha 1.75       13 / 242        7 / 158      15 /  825       9 / 618
//! V(1,1) alpha 1.9        14 / 261        8 / 151      15 / 1238       9 / 734
//! V(2,2) alpha 1.5         9 / 287        5 / 192      10 / 1139       5 / 649
//! W(1,1) alpha 1.5         7 / 187        4 / 143       7 /  802       4 / 568
//! W(1,1) alpha 1.25        8 / 178        5 / 125       8 /  799       5 / 565
//! ```
//!
//! V(1,1) at 1.5 has the lowest or tied pressure time with margin below 20
//! iterations (1.25 reaches 19); V(2,2) and W only trade iterations for cost per
//! iteration. A trilinear-interpolation Galerkin hierarchy was not built: the
//! criterion is already met, and it needs 27 coefficients per coarse cell and a
//! triple product every step, because the operator changes with the solids.

use super::{coords, LIGHT};
use rayon::prelude::*;

/// Cells per reduction block.
pub(super) const BLOCK: usize = 2048;
/// Scale of the coarse-grid correction; strictly below 2 for convergence.
const OVERCORRECTION: f64 = 1.5;
/// Coarsening stops at this many cells; that level is solved exactly.
const DIRECT_CELLS: usize = 128;

/// Sums of `N` terms per cell over `0..len`, deterministic for any thread count.
pub(super) fn blocked_sums<const N: usize>(len: usize, term: impl Fn(usize) -> [f64; N] + Sync) -> [f64; N] {
    let blocks = len.div_ceil(BLOCK);
    let mut partial: Vec<[f64; N]> = (0..blocks)
        .into_par_iter()
        .with_min_len(4)
        .map(|b| {
            let mut sum = [0.0; N];
            for k in b * BLOCK..((b + 1) * BLOCK).min(len) {
                let t = term(k);
                for n in 0..N {
                    sum[n] += t[n];
                }
            }
            sum
        })
        .collect();
    if partial.is_empty() {
        return [0.0; N];
    }
    while partial.len() > 1 {
        partial = partial
            .chunks(2)
            .map(|pair| if pair.len() == 2 { std::array::from_fn(|n| pair[0][n] + pair[1][n]) } else { pair[0] })
            .collect();
    }
    partial[0]
}

/// Makes the right-hand side consistent: in every connected fluid component
/// without a zero-pressure face (a closed domain, or a pocket sealed by solids)
/// the operator is singular and solvable only when the right-hand side sums to
/// zero, so the component mean is removed. Whatever mean was removed stays in the
/// divergence the caller measures afterwards, so a component that is genuinely
/// inconsistent still fails the tolerance check; it is never silently absorbed.
pub(super) fn remove_floating_means(dims: [usize; 3], solid: &[bool], open: &[u8], diagonal: &[f64], rhs: &mut [f64]) {
    fn find(parent: &mut [u32], mut v: u32) -> u32 {
        while parent[v as usize] != v {
            let grand = parent[parent[v as usize] as usize];
            parent[v as usize] = grand;
            v = grand;
        }
        v
    }
    let n = rhs.len();
    let [nx, ny, _] = dims;
    let plane = nx * ny;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    for k in 0..n {
        if solid[k] {
            continue;
        }
        for (bit, stride) in [(0b10, 1), (0b1000, nx), (0b100000, plane)] {
            if open[k] & bit != 0 {
                let (a, b) = (find(&mut parent, k as u32), find(&mut parent, (k + stride) as u32));
                // The lowest index is the root, so labels do not depend on visiting order.
                parent[a.max(b) as usize] = a.min(b);
            }
        }
    }
    // A zero-pressure face adds to the diagonal without adding an open neighbour.
    let mut grounded = vec![false; n];
    for k in 0..n {
        if !solid[k] && diagonal[k] > f64::from(open[k].count_ones()) {
            let root = find(&mut parent, k as u32);
            grounded[root as usize] = true;
        }
    }
    if !(0..n).any(|k| !solid[k] && parent[k] as usize == k && !grounded[k]) {
        return;
    }
    let (mut sum, mut count) = (vec![0.0; n], vec![0u32; n]);
    for k in 0..n {
        if !solid[k] {
            let root = find(&mut parent, k as u32) as usize;
            if !grounded[root] {
                sum[root] += rhs[k];
                count[root] += 1;
            }
        }
    }
    for k in 0..n {
        if !solid[k] {
            let root = find(&mut parent, k as u32) as usize;
            if !grounded[root] {
                rhs[k] -= sum[root] / f64::from(count[root]);
            }
        }
    }
}

/// A symmetric positive semidefinite pressure operator on a regular grid with
/// 6-neighbour edges: `(A x)_k = d_k x_k - sum_s w_ks x_n(k,s)`. Sides are
/// -x, +x, -y, +y, -z, +z; a zero weight means no edge. A cell with `d == 0`
/// has no unknown.
pub(super) trait Level: Sync {
    fn dims(&self) -> [usize; 3];
    fn diag(&self, k: usize) -> f64;
    fn weight(&self, k: usize, side: usize) -> f64;
}

/// The cell-level operator, from the per-cell open-face mask and diagonal that
/// the Jacobi-preconditioned solver also uses.
pub(super) struct Fine<'a> {
    pub dims: [usize; 3],
    pub open: &'a [u8],
    pub diagonal: &'a [f64],
}

impl Level for Fine<'_> {
    fn dims(&self) -> [usize; 3] {
        self.dims
    }
    fn diag(&self, k: usize) -> f64 {
        self.diagonal[k]
    }
    fn weight(&self, k: usize, side: usize) -> f64 {
        f64::from((self.open[k] >> side) & 1)
    }
}

/// Aggregated operator. Weights are small integers (face counts), exact in f32.
struct Coarse {
    dims: [usize; 3],
    diag: Vec<f64>,
    weights: Vec<[f32; 6]>,
}

impl Level for Coarse {
    fn dims(&self) -> [usize; 3] {
        self.dims
    }
    fn diag(&self, k: usize) -> f64 {
        self.diag[k]
    }
    fn weight(&self, k: usize, side: usize) -> f64 {
        f64::from(self.weights[k][side])
    }
}

fn cells(dims: [usize; 3]) -> usize {
    dims.iter().product()
}

fn strides(dims: [usize; 3]) -> [usize; 3] {
    [1, dims[0], dims[0] * dims[1]]
}

fn neighbour(k: usize, side: usize, strides: [usize; 3]) -> usize {
    if side % 2 == 0 {
        k - strides[side / 2]
    } else {
        k + strides[side / 2]
    }
}

/// `sum_s w_ks x_n`, in side order.
fn gather<L: Level>(level: &L, k: usize, x: &[f64], strides: [usize; 3]) -> f64 {
    let mut sum = 0.0;
    for side in 0..6 {
        let w = level.weight(k, side);
        if w != 0.0 {
            sum += w * x[neighbour(k, side, strides)];
        }
    }
    sum
}

/// One half-sweep of Gauss-Seidel over the cells of `color` (parity of i+j+k):
/// they read only the other colour, which this sweep does not write, so cells
/// update independently. `dst` receives the updated cells and a copy of the
/// others from `src` (zeros when `src` is `None`, a zero initial guess).
fn sweep<L: Level>(level: &L, color: usize, b: &[f64], src: Option<&[f64]>, dst: &mut [f64]) {
    let [nx, ny, _] = level.dims();
    let st = strides(level.dims());
    dst.par_chunks_mut(nx).enumerate().with_min_len((LIGHT / nx).max(1)).for_each(|(row, out)| {
        let (j, kz) = (row % ny, row / ny);
        for (i, o) in out.iter_mut().enumerate() {
            let k = row * nx + i;
            if (i + j + kz) & 1 == color {
                let d = level.diag(k);
                *o = if d > 0.0 {
                    let g = src.map_or(0.0, |s| gather(level, k, s, st));
                    (b[k] + g) / d
                } else {
                    0.0
                };
            } else {
                *o = src.map_or(0.0, |s| s[k]);
            }
        }
    });
}

/// `out = b - A x`, zero for cells without an unknown.
fn residual<L: Level>(level: &L, b: &[f64], x: &[f64], out: &mut [f64]) {
    let st = strides(level.dims());
    out.par_iter_mut().enumerate().with_min_len(LIGHT).for_each(|(k, o)| {
        let d = level.diag(k);
        *o = if d > 0.0 {
            let mut ax = d * x[k];
            for side in 0..6 {
                let w = level.weight(k, side);
                if w != 0.0 {
                    ax -= w * x[neighbour(k, side, st)];
                }
            }
            b[k] - ax
        } else {
            0.0
        };
    });
}

/// Sum the fine values of each aggregate, children in x-fastest order.
fn restrict(fine: [usize; 3], coarse: [usize; 3], r: &[f64], out: &mut [f64]) {
    let [nx, ny, nz] = fine;
    let [cx, cy, _] = coarse;
    out.par_chunks_mut(cx).enumerate().with_min_len((LIGHT / cx).max(1)).for_each(|(row, o)| {
        let (cj, ck) = (row % cy, row / cy);
        for (ci, v) in o.iter_mut().enumerate() {
            let mut sum = 0.0;
            for kz in 2 * ck..(2 * ck + 2).min(nz) {
                for j in 2 * cj..(2 * cj + 2).min(ny) {
                    for i in 2 * ci..(2 * ci + 2).min(nx) {
                        sum += r[(kz * ny + j) * nx + i];
                    }
                }
            }
            *v = sum;
        }
    });
}

/// `x += alpha * e` with `e` the aggregate value, on cells that have an unknown.
fn prolong_add<L: Level>(level: &L, coarse: [usize; 3], e: &[f64], x: &mut [f64]) {
    let [nx, ny, _] = level.dims();
    let [cx, cy, _] = coarse;
    x.par_chunks_mut(nx).enumerate().with_min_len((LIGHT / nx).max(1)).for_each(|(row, xr)| {
        let (j, kz) = (row % ny, row / ny);
        let base = ((kz / 2) * cy + j / 2) * cx;
        for (i, v) in xr.iter_mut().enumerate() {
            if level.diag(row * nx + i) > 0.0 {
                *v += OVERCORRECTION * e[base + i / 2];
            }
        }
    });
}

/// Galerkin aggregation of a level: `diag_c = sum d_i - 2 * internal edges`,
/// `w_c(side)` = total weight of edges leaving the block through `side`.
fn coarsen<L: Level>(fine: &L) -> Coarse {
    let [nx, ny, nz] = fine.dims();
    let dims = [nx.div_ceil(2), ny.div_ceil(2), nz.div_ceil(2)];
    let mut diag = vec![0.0; cells(dims)];
    let mut weights = vec![[0.0f32; 6]; cells(dims)];
    diag.par_iter_mut().zip(weights.par_iter_mut()).enumerate().with_min_len(64).for_each(|(c, (d, w))| {
        let block = coords(c, dims);
        let lo = block.map(|v| 2 * v);
        let hi = [(lo[0] + 2).min(nx), (lo[1] + 2).min(ny), (lo[2] + 2).min(nz)];
        let (mut sum, mut internal, mut leaving) = (0.0, 0.0, [0.0f64; 6]);
        for kz in lo[2]..hi[2] {
            for j in lo[1]..hi[1] {
                for i in lo[0]..hi[0] {
                    let k = (kz * ny + j) * nx + i;
                    sum += fine.diag(k);
                    let at = [i, j, kz];
                    for (side, leaves) in leaving.iter_mut().enumerate() {
                        let weight = fine.weight(k, side);
                        if weight == 0.0 {
                            continue;
                        }
                        let axis = side / 2;
                        let outside = if side % 2 == 1 { at[axis] + 1 >= hi[axis] } else { at[axis] <= lo[axis] };
                        if outside {
                            *leaves += weight;
                        } else {
                            internal += weight;
                        }
                    }
                }
            }
        }
        *d = sum - internal;
        *w = leaving.map(|v| v as f32);
    });
    Coarse { dims, diag, weights }
}

/// Exact solve of the coarsest level by dense Cholesky, with one unknown pinned
/// to zero in every component that has no zero-pressure face.
struct Direct {
    n: usize,
    /// Unknown index of each cell, or `usize::MAX` for cells with none (no
    /// unknown, pinned or dropped).
    unknown: Vec<usize>,
    cells: Vec<usize>,
    /// Row-major lower factor.
    factor: Vec<f64>,
}

impl Direct {
    fn new<L: Level>(level: &L) -> Self {
        let m = cells(level.dims());
        let st = strides(level.dims());
        let mut parent: Vec<usize> = (0..m).collect();
        fn find(parent: &mut [usize], mut v: usize) -> usize {
            while parent[v] != v {
                parent[v] = parent[parent[v]];
                v = parent[v];
            }
            v
        }
        let mut grounded = vec![false; m];
        for (k, ground) in grounded.iter_mut().enumerate() {
            if level.diag(k) <= 0.0 {
                continue;
            }
            let mut incident = 0.0;
            for side in 0..6 {
                let w = level.weight(k, side);
                if w != 0.0 {
                    incident += w;
                    let (a, b) = (find(&mut parent, k), find(&mut parent, neighbour(k, side, st)));
                    parent[a.max(b)] = a.min(b);
                }
            }
            // Integer-valued: exact. A positive remainder is a zero-pressure face.
            *ground = level.diag(k) - incident > 0.0;
        }
        let mut component_grounded = vec![false; m];
        for (k, &ground) in grounded.iter().enumerate() {
            if level.diag(k) > 0.0 && ground {
                let root = find(&mut parent, k);
                component_grounded[root] = true;
            }
        }
        let mut unknown = vec![usize::MAX; m];
        let mut cell_of = Vec::new();
        for (k, slot) in unknown.iter_mut().enumerate() {
            if level.diag(k) <= 0.0 {
                continue;
            }
            let root = find(&mut parent, k);
            if !component_grounded[root] {
                // The lowest-index cell of a floating component is its pin.
                component_grounded[root] = true;
                continue;
            }
            *slot = cell_of.len();
            cell_of.push(k);
        }
        let n = cell_of.len();
        let mut a = vec![0.0; n * n];
        for (i, &k) in cell_of.iter().enumerate() {
            a[i * n + i] = level.diag(k);
            for side in 0..6 {
                let w = level.weight(k, side);
                if w != 0.0 {
                    let j = unknown[neighbour(k, side, st)];
                    if j != usize::MAX {
                        a[i * n + j] = -w;
                    }
                }
            }
        }
        // Cholesky; an unknown whose pivot is not safely positive is dropped
        // (treated as pinned) rather than producing a NaN.
        let mut dead = vec![false; n];
        for j in 0..n {
            let original = a[j * n + j];
            let mut pivot = original;
            for k in 0..j {
                if !dead[k] {
                    pivot -= a[j * n + k] * a[j * n + k];
                }
            }
            let healthy = pivot.is_finite() && pivot > 1e-10 * original.abs();
            if !healthy {
                dead[j] = true;
                for i in j..n {
                    a[i * n + j] = 0.0;
                }
                continue;
            }
            let root = pivot.sqrt();
            a[j * n + j] = root;
            for i in j + 1..n {
                let mut t = a[i * n + j];
                for k in 0..j {
                    if !dead[k] {
                        t -= a[i * n + k] * a[j * n + k];
                    }
                }
                a[i * n + j] = t / root;
            }
        }
        for (j, &is_dead) in dead.iter().enumerate() {
            if is_dead {
                unknown[cell_of[j]] = usize::MAX;
            }
        }
        // Dead unknowns keep their row, but solve() skips them via `unknown`.
        Self { n, unknown, cells: cell_of, factor: a }
    }

    fn solve(&self, b: &[f64], x: &mut [f64]) {
        x.fill(0.0);
        let n = self.n;
        let mut y = vec![0.0; n];
        for i in 0..n {
            if self.unknown[self.cells[i]] == usize::MAX {
                continue;
            }
            let mut t = b[self.cells[i]];
            for (k, yk) in y.iter().enumerate().take(i) {
                t -= self.factor[i * n + k] * yk;
            }
            y[i] = t / self.factor[i * n + i];
        }
        for i in (0..n).rev() {
            if self.unknown[self.cells[i]] == usize::MAX {
                continue;
            }
            let mut t = y[i];
            for k in i + 1..n {
                t -= self.factor[k * n + i] * x[self.cells[k]];
            }
            x[self.cells[i]] = t / self.factor[i * n + i];
        }
    }
}

/// Per-level work vectors of one V-cycle.
pub(super) struct Scratch {
    b: Vec<f64>,
    x: Vec<f64>,
    tmp: Vec<f64>,
}

pub(super) struct Hierarchy<'a> {
    fine: Fine<'a>,
    coarse: Vec<Coarse>,
    direct: Direct,
}

impl<'a> Hierarchy<'a> {
    pub(super) fn new(fine: Fine<'a>) -> Self {
        let mut coarse: Vec<Coarse> = Vec::new();
        loop {
            let size = coarse.last().map_or(cells(fine.dims), |c| cells(c.dims));
            if size <= DIRECT_CELLS {
                break;
            }
            let next = match coarse.last() {
                None => coarsen(&fine),
                Some(c) => coarsen(c),
            };
            coarse.push(next);
        }
        let direct = match coarse.last() {
            None => Direct::new(&fine),
            Some(c) => Direct::new(c),
        };
        Self { fine, coarse, direct }
    }

    #[cfg(test)]
    pub(super) fn levels(&self) -> usize {
        self.coarse.len() + 1
    }

    pub(super) fn scratch(&self) -> Vec<Scratch> {
        self.coarse
            .iter()
            .map(|c| {
                let n = cells(c.dims);
                Scratch { b: vec![0.0; n], x: vec![0.0; n], tmp: vec![0.0; n] }
            })
            .collect()
    }

    /// `z = M r` for the symmetric positive definite V(1,1) cycle `M`; `tmp` is
    /// fine-sized scratch. Cells without an unknown map `r` to itself, like the
    /// Jacobi preconditioner.
    pub(super) fn precondition(&self, r: &[f64], z: &mut [f64], tmp: &mut [f64], scratch: &mut [Scratch]) {
        self.cycle(&self.fine, 0, r, z, tmp, scratch);
        z.par_iter_mut().zip(r.par_iter()).enumerate().with_min_len(LIGHT).for_each(|(k, (z, r))| {
            if self.fine.diagonal[k] == 0.0 {
                *z = *r;
            }
        });
    }

    fn cycle<L: Level>(
        &self,
        level: &L,
        depth: usize,
        b: &[f64],
        x: &mut [f64],
        tmp: &mut [f64],
        scratch: &mut [Scratch],
    ) {
        if depth == self.coarse.len() {
            self.direct.solve(b, x);
            return;
        }
        sweep(level, 0, b, None, tmp);
        sweep(level, 1, b, Some(tmp), x);
        residual(level, b, x, tmp);
        let (this, rest) = scratch.split_first_mut().expect("scratch has one entry per coarse level");
        let coarse = &self.coarse[depth];
        restrict(level.dims(), coarse.dims, tmp, &mut this.b);
        self.cycle(coarse, depth + 1, &this.b, &mut this.x, &mut this.tmp, rest);
        prolong_add(level, coarse.dims, &this.x, x);
        sweep(level, 1, b, Some(x), tmp);
        sweep(level, 0, b, Some(tmp), x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pyro::{neighbours, Boundary};

    /// Diagonal and open-face mask exactly as `project` builds them.
    fn system(dims: [usize; 3], solid: &[bool], boundary: Boundary) -> (Vec<f64>, Vec<u8>) {
        let n = cells(dims);
        let (mut diagonal, mut open) = (vec![0.0; n], vec![0u8; n]);
        for k in 0..n {
            if solid[k] {
                continue;
            }
            for (side, nb) in neighbours(coords(k, dims), dims).into_iter().enumerate() {
                if nb.map_or(boundary == Boundary::Open, |nb| !solid[nb]) {
                    diagonal[k] += 1.0;
                }
                if nb.is_some_and(|nb| !solid[nb]) {
                    open[k] |= 1 << side;
                }
            }
        }
        (diagonal, open)
    }

    fn random_solid(dims: [usize; 3], fraction: f64, seed: u64) -> Vec<bool> {
        (0..cells(dims)).map(|k| crate::rng::unit(seed, k as u64, 99) < fraction).collect()
    }

    fn random_vector(n: usize, solid: &[bool], seed: u64) -> Vec<f64> {
        (0..n).map(|k| if solid[k] { 0.0 } else { crate::rng::signed(seed, k as u64, 7) }).collect()
    }

    fn apply_m(h: &Hierarchy, r: &[f64]) -> Vec<f64> {
        let (mut z, mut tmp, mut scratch) = (vec![0.0; r.len()], vec![0.0; r.len()], h.scratch());
        h.precondition(r, &mut z, &mut tmp, &mut scratch);
        z
    }

    fn dot(a: &[f64], b: &[f64]) -> f64 {
        a.iter().zip(b).map(|(a, b)| a * b).sum()
    }

    #[test]
    fn the_v_cycle_is_symmetric_and_positive_definite() {
        let shapes = [[9, 7, 6], [16, 12, 14], [23, 19, 17], [40, 3, 5], [2, 2, 2]];
        for dims in shapes {
            for boundary in [Boundary::Open, Boundary::Closed] {
                for fraction in [0.0, 0.1, 0.35] {
                    let solid = random_solid(dims, fraction, 3);
                    let (diagonal, open) = system(dims, &solid, boundary);
                    let h = Hierarchy::new(Fine { dims, open: &open, diagonal: &diagonal });
                    for seed in 0..4 {
                        let (x, y) =
                            (random_vector(cells(dims), &solid, seed), random_vector(cells(dims), &solid, seed + 50));
                        let (mx, my) = (apply_m(&h, &x), apply_m(&h, &y));
                        let (a, b) = (dot(&mx, &y), dot(&x, &my));
                        let scale = dot(&mx, &mx).sqrt() * dot(&y, &y).sqrt() + 1e-300;
                        assert!(
                            (a - b).abs() <= 1e-10 * scale,
                            "{dims:?} {boundary:?} solid {fraction}: <Mx,y> {a} vs <x,My> {b}"
                        );
                        let xmx = dot(&x, &mx);
                        assert!(
                            xmx > 0.0 || dot(&x, &x) == 0.0,
                            "{dims:?} {boundary:?} solid {fraction}: x^T M x = {xmx} ({} levels)",
                            h.levels()
                        );
                    }
                }
            }
        }
    }

    /// Dense `A` of a level.
    fn dense<L: Level>(level: &L) -> Vec<Vec<f64>> {
        let n = cells(level.dims());
        let st = strides(level.dims());
        let mut a = vec![vec![0.0; n]; n];
        for k in 0..n {
            a[k][k] = level.diag(k);
            for side in 0..6 {
                let w = level.weight(k, side);
                if w != 0.0 {
                    a[k][neighbour(k, side, st)] -= w;
                }
            }
        }
        a
    }

    /// `Pᵀ A P` for piecewise-constant 2x2x2 aggregation.
    fn galerkin(a: &[Vec<f64>], fine: [usize; 3], coarse: [usize; 3]) -> Vec<Vec<f64>> {
        let aggregate = |k: usize| {
            let [i, j, kz] = coords(k, fine);
            (kz / 2 * coarse[1] + j / 2) * coarse[0] + i / 2
        };
        let m = cells(coarse);
        let mut c = vec![vec![0.0; m]; m];
        for (i, row) in a.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                c[aggregate(i)][aggregate(j)] += v;
            }
        }
        c
    }

    #[test]
    fn coarse_operators_are_exact_galerkin_products() {
        for dims in [[5, 4, 3], [7, 1, 6], [4, 4, 4], [3, 5, 2]] {
            for boundary in [Boundary::Open, Boundary::Closed] {
                for fraction in [0.0, 0.25] {
                    let solid = random_solid(dims, fraction, 11);
                    let (diagonal, open) = system(dims, &solid, boundary);
                    let fine = Fine { dims, open: &open, diagonal: &diagonal };
                    let first = coarsen(&fine);
                    assert_eq!(
                        dense(&first),
                        galerkin(&dense(&fine), dims, first.dims),
                        "{dims:?} {boundary:?} {fraction}"
                    );
                    let second = coarsen(&first);
                    assert_eq!(
                        dense(&second),
                        galerkin(&dense(&first), first.dims, second.dims),
                        "second level {dims:?} {boundary:?} {fraction}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_direct_solve_pins_each_floating_component_and_never_returns_nan() {
        // Closed 8x1x1 strip split by a solid cell: two floating components.
        let dims = [8, 1, 1];
        let mut solid = vec![false; 8];
        solid[4] = true;
        let (diagonal, open) = system(dims, &solid, Boundary::Closed);
        let level = Fine { dims, open: &open, diagonal: &diagonal };
        let direct = Direct::new(&level);
        // Consistent: each component sums to zero, so Ax = b must hold exactly.
        let b = [1.0, -2.0, 0.5, 0.5, 0.0, 3.0, -1.0, -2.0];
        let mut x = vec![0.0; 8];
        direct.solve(&b, &mut x);
        let a = dense(&level);
        for k in 0..8 {
            let ax: f64 = (0..8).map(|j| a[k][j] * x[j]).sum();
            assert!((ax - b[k]).abs() < 1e-12, "row {k}: {ax} vs {}", b[k]);
        }
        // An inconsistent right-hand side stays finite, and the part that cannot
        // be solved (the component sum) lands on the pinned cell, nowhere else.
        let b = [1.0; 8];
        direct.solve(&b, &mut x);
        assert!(x.iter().all(|v| v.is_finite()));
        let residual: Vec<f64> = (0..8).map(|k| b[k] - (0..8).map(|j| a[k][j] * x[j]).sum::<f64>()).collect();
        for (k, expected) in [(0, 4.0), (1, 0.0), (2, 0.0), (3, 0.0), (5, 3.0), (6, 0.0), (7, 0.0)] {
            assert!((residual[k] - expected).abs() < 1e-12, "{residual:?}");
        }
        // A fully enclosed single cell has no unknown at all.
        let solid = [true, false, true, true, true, true, true, true];
        let (diagonal, open) = system([8, 1, 1], &solid, Boundary::Closed);
        let level = Fine { dims: [8, 1, 1], open: &open, diagonal: &diagonal };
        let mut x = vec![9.0; 8];
        Direct::new(&level).solve(&[1.0; 8], &mut x);
        assert!(x.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn blocked_sums_do_not_depend_on_the_thread_count() {
        let values: Vec<f64> = (0..100_003).map(|k| crate::rng::signed(5, k, 1) * 1e6 + 1e-3).collect();
        let sum = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| blocked_sums(values.len(), |k| [values[k], values[k] * values[k]]))
        };
        let one = sum(1);
        assert_eq!(sum(2).map(f64::to_bits), one.map(f64::to_bits));
        assert_eq!(sum(7).map(f64::to_bits), one.map(f64::to_bits));
        assert_eq!(blocked_sums::<1>(0, |_| [1.0]), [0.0]);
    }

    #[test]
    fn floating_means_are_removed_only_from_ungrounded_components() {
        // Closed 8x1x1 strip cut by a solid cell: both halves float.
        let dims = [8, 1, 1];
        let mut solid = vec![false; 8];
        solid[4] = true;
        let (diagonal, open) = system(dims, &solid, Boundary::Closed);
        let mut rhs = [1.0, -2.0, 0.5, 4.0, 0.0, 3.0, 5.0, 7.0];
        remove_floating_means(dims, &solid, &open, &diagonal, &mut rhs);
        assert!(rhs[..4].iter().sum::<f64>().abs() < 1e-12 && rhs[5..].iter().sum::<f64>().abs() < 1e-12);
        assert_eq!(rhs[4], 0.0);
        assert!((rhs[0] - (1.0 - 0.875)).abs() < 1e-12 && (rhs[5] - (3.0 - 5.0)).abs() < 1e-12);

        // Open 3x3x3 with the centre cell sealed by six solid neighbours: every
        // other fluid cell touches the open edge and keeps its value; the isolated
        // centre (no unknown) is zeroed.
        let dims = [3, 3, 3];
        let mut solid = vec![false; 27];
        for k in [4, 10, 12, 14, 16, 22] {
            solid[k] = true;
        }
        let (diagonal, open) = system(dims, &solid, Boundary::Open);
        let original: Vec<f64> = (0..27).map(|k| k as f64 + 1.0).collect();
        let mut rhs = original.clone();
        remove_floating_means(dims, &solid, &open, &diagonal, &mut rhs);
        for k in (0..27).filter(|&k| k != 13) {
            assert_eq!(rhs[k], original[k], "cell {k}");
        }
        assert_eq!(rhs[13], 0.0);
    }
}
