//! A bed of granular material over a ground: a height field of deposit that material is poured onto and that
//! relaxes to the angle of repose, deterministically and without making or losing any.
//!
//! The deposit is a height above the ground in each cell of a regular grid (the surface is ground plus deposit).
//! [`Bed::relax`] moves material downhill wherever the surface is steeper than the angle of repose, in a fixed order
//! (rows, then columns, then a fixed list of sixteen neighbours), so the result depends on the inputs and on nothing
//! else; it only moves material from one cell to another, so the volume is what was poured, to the rounding of the sums.
//! The sixteen neighbours (the eight around a cell and the eight a knight's move away) are what make the pile round:
//! with four the flank of a cone measured 38.9 to 44.5 degrees for a declared 35, with eight 35.1 to 36.9, with sixteen
//! 35.1 to 35.5 (probe on a pile of ten cells' radius, Jacobi relaxation; the test measures this implementation).

const OFFSETS: [(i32, i32); 16] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (1, 2),
    (2, 1),
    (-1, 2),
    (-2, 1),
    (1, -2),
    (2, -1),
    (-1, -2),
    (-2, -1),
];

/// A deposit over a ground on a grid of `cells` square cells of side `cell`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bed {
    cells: [usize; 2],
    cell: f64,
    tan: f64,
    ground: Vec<f64>,
    deposit: Vec<f64>,
}

impl Bed {
    /// A bed over level ground, with the angle of repose in degrees (over 0 and under 90).
    pub fn flat(cells: [usize; 2], cell: f64, repose: f64) -> Result<Self, String> {
        Self::on(vec![0.0; cells[0].saturating_mul(cells[1])], cells, cell, repose)
    }

    /// A bed over `ground`, the height of the ground in each cell, row by row (`cells[0]` to a row).
    pub fn on(ground: Vec<f64>, cells: [usize; 2], cell: f64, repose: f64) -> Result<Self, String> {
        if cells[0] == 0 || cells[1] == 0 || cells[0].checked_mul(cells[1]) != Some(ground.len()) {
            return Err("a bed needs a ground with a height for each of its cells".into());
        }
        if !(cell.is_finite() && cell > 0.0) {
            return Err("a bed's cells must have a positive size".into());
        }
        if !(repose.is_finite() && repose > 0.0 && repose < 90.0) {
            return Err("the angle of repose must be over 0 and under 90 degrees".into());
        }
        if ground.iter().any(|h| !h.is_finite()) {
            return Err("the ground of a bed must be finite".into());
        }
        Ok(Self { cells, cell, tan: repose.to_radians().tan(), deposit: vec![0.0; ground.len()], ground })
    }

    pub fn cells(&self) -> [usize; 2] {
        self.cells
    }

    /// The height of the deposit in a cell.
    pub fn deposit_at(&self, ix: usize, iz: usize) -> f64 {
        self.deposit[iz * self.cells[0] + ix]
    }

    /// Sets the deposit of a cell, which is not made by pouring (a bed that starts with something on it).
    pub fn set_deposit(&mut self, ix: usize, iz: usize, height: f64) {
        self.deposit[iz * self.cells[0] + ix] = height.max(0.0);
    }

    /// Cubic units of deposit.
    pub fn volume(&self) -> f64 {
        self.deposit.iter().sum::<f64>() * self.cell * self.cell
    }

    /// Pours `volume` at `point`, in the grid's units from its corner (the centre of cell (i, j) is at
    /// ((i + 1/2) cell, (j + 1/2) cell)), shared between the four nearest cells by bilinear weights, so that
    /// the volume is exactly what is given.
    pub fn deposit(&mut self, point: [f64; 2], volume: f64) -> Result<(), String> {
        let extent = [self.cells[0] as f64 * self.cell, self.cells[1] as f64 * self.cell];
        if !(point[0].is_finite() && point[1].is_finite() && (0.0..=extent[0]).contains(&point[0]))
            || !(0.0..=extent[1]).contains(&point[1])
        {
            return Err("the point where material is poured is outside the bed".into());
        }
        if !(volume.is_finite() && volume >= 0.0) {
            return Err("the volume poured must be finite and not negative".into());
        }
        let place = |p: f64, n: usize| {
            let f = (p / self.cell - 0.5).clamp(0.0, (n - 1) as f64);
            let i = (f.floor() as usize).min(n.saturating_sub(2));
            let i = if n == 1 { 0 } else { i };
            (i, if n == 1 { 0.0 } else { f - i as f64 })
        };
        let ((ix, fx), (iz, fz)) = (place(point[0], self.cells[0]), place(point[1], self.cells[1]));
        let height = volume / (self.cell * self.cell);
        for (dz, wz) in [(0, 1.0 - fz), (1, fz)] {
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                let (x, z) = ((ix + dx).min(self.cells[0] - 1), (iz + dz).min(self.cells[1] - 1));
                self.deposit[z * self.cells[0] + x] += height * wx * wz;
            }
        }
        Ok(())
    }

    /// How much the largest transfer that the next pass of [`Bed::relax`] would make, in height units: zero when
    /// no deposit stands on a surface steeper than the angle of repose that it could run down.
    pub fn steepest_transferable(&self) -> f64 {
        let mut most = 0.0f64;
        for iz in 0..self.cells[1] {
            for ix in 0..self.cells[0] {
                for (_, moved) in self.moves(ix, iz) {
                    most = most.max(moved);
                }
            }
        }
        most
    }

    /// The cells a cell's deposit would run to, and how much of it, from the state as it is now.
    fn moves(&self, ix: usize, iz: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let i = iz * self.cells[0] + ix;
        let here = self.ground[i] + self.deposit[i];
        let available = self.deposit[i];
        OFFSETS.iter().filter_map(move |&(ox, oz)| {
            let (x, z) = (ix as i64 + ox as i64, iz as i64 + oz as i64);
            if available <= 0.0 || x < 0 || z < 0 || x >= self.cells[0] as i64 || z >= self.cells[1] as i64 {
                return None;
            }
            let j = z as usize * self.cells[0] + x as usize;
            let slope = self.tan * self.cell * f64::from(ox).hypot(f64::from(oz));
            let excess = here - (self.ground[j] + self.deposit[j]) - slope;
            (excess > 0.0).then(|| (j, (0.5 * excess).min(available)))
        })
    }

    /// Passes over the bed, in order, until no pass moves more than `tolerance` (a height) anywhere; returns the
    /// number of passes made. Each cell in turn gives half of the excess of its surface over the angle of repose to
    /// each lower neighbour, as much as it holds (the cells in front are changed at once), which leaves a pair exactly
    /// at the angle. An error if `max_passes` are not enough. The grid's edge holds the material in.
    pub fn relax(&mut self, tolerance: f64, max_passes: u32) -> Result<u32, String> {
        for pass in 0..=max_passes {
            let mut most = 0.0f64;
            for iz in 0..self.cells[1] {
                for ix in 0..self.cells[0] {
                    let i = iz * self.cells[0] + ix;
                    for &(ox, oz) in &OFFSETS {
                        let (x, z) = (ix as i64 + ox as i64, iz as i64 + oz as i64);
                        if x < 0 || z < 0 || x >= self.cells[0] as i64 || z >= self.cells[1] as i64 {
                            continue;
                        }
                        let j = z as usize * self.cells[0] + x as usize;
                        let slope = self.tan * self.cell * f64::from(ox).hypot(f64::from(oz));
                        let excess = self.ground[i] + self.deposit[i] - self.ground[j] - self.deposit[j] - slope;
                        if excess > 0.0 && self.deposit[i] > 0.0 {
                            let moved = (0.5 * excess).min(self.deposit[i]);
                            self.deposit[i] -= moved;
                            self.deposit[j] += moved;
                            most = most.max(moved);
                        }
                    }
                }
            }
            if most <= tolerance {
                return Ok(pass);
            }
        }
        Err(format!("the bed did not come to rest in {max_passes} passes"))
    }
}
