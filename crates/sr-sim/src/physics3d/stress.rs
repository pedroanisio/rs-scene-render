//! A body of cells that is made of pieces joined to each other, and breaks where a load makes more stress in a joint than its strength.
//!
//! The body is ONE rigid body of the world until a joint breaks: the pieces are not bodies and the joints are not constraints of the solver (a weld between rigid
//! bodies gives energy back at 100 m/s: the measurement is `tests/rigid/stress_spike.rs`), so there is no elastic energy, no stress wave and no creep. What
//! a step does to the pieces' joints is worked out after it, from the motion of the body in it and the loads on it ([`crate::stress::balance`]): for every cut of
//! the body ([`crate::stress::plan`]) the force and the moment that the rest of the body puts on the side of it, the stress that these make in the
//! joints of the cut ([`crate::stress::cut_stresses`]), and the joints whose principal tension is at the strength or more. They break together, at the start of
//! the next step, whatever order they were looked at in: the joints are all read on the state at the end of the step, and the ones over strength are
//! put aside until the next step begins. The pieces that the broken joints leave apart are cut from the body with the machinery of a body of cells that is cut
//! (`voxel_split`): the largest stays the body, the others take the slots of the split of the parent, with the mass properties of their cells and the
//! velocity of their own centres on the body that they came out of, so that the momentum and the angular momentum are what they were. A piece that is
//! itself made of pieces joined together is a body that can break again; the slots are one pool for the whole family.
//!
//! The loads are the weight and the fields (a uniform acceleration of every point), the contacts (the normal and friction impulses of the step, at the points
//! the solver gave, on the piece they are in), and the one joint of the world that may hold the body (its force and moment are what the balance of the whole body
//! leaves). What is left over with no joint (damping, a velocity that was set, a torque that the driver puts on the body) is a rigid acceleration and makes no stress.
//!
//! Limits, all of them from the body being rigid until it breaks: no stress wave (a load is carried by the whole body at once), no energy kept in the joints (a
//! piece that is bent springs back for nothing), the load of a joint in a cycle is that of its plane's cut shared as over one rigid section, the dust's momentum
//! leaves with it and is not recorded, and a joint of the world that holds the body stays on the parent when the piece it holds is not.
use super::*;
use crate::stress::balance::{Located, MassSum, Rigid, Step};
use crate::stress::plan::{self, CutPlan, JointGeom, PieceGeom};
use crate::stress::{cut_stresses, JointSection};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The most pieces of a body that can break by stress: the cuts of the body are worked out for every joint, and cost the pieces times the joints.
pub const MAX_STRESS_PIECES: usize = 1024;

/// One piece of a body that breaks by stress: its cells (keys of the body's own lattice, sorted, each once), its mass, and its centre and second moment in the
/// frame of the body (physics axes, metres, the origin of the body's frame): `sum m l` over the mass, and `sum m l l^T`.
#[derive(Clone, Debug, PartialEq)]
pub struct StressPiece3 {
    pub cells: Vec<[i32; 3]>,
    pub mass: f64,
    pub centre: [f64; 3],
    pub second: [[f64; 3]; 3],
}

impl StressPiece3 {
    /// The piece made of `cells` (each once) of cubes of `size` metres and the total `mass`, with its centre and second moment worked out from the cells (the body's
    /// frame is the physics frame of the lattice: the cell `[i, j, k]` is at the key `[i, -j - 1, -k - 1]`, and the origin of the body is the origin of the keys).
    pub fn from_cells(cells: &[[i32; 3]], size: [f64; 3], mass: f64) -> Option<StressPiece3> {
        if cells.is_empty() || !(mass.is_finite() && mass > 0.0) || !size.iter().all(|s| s.is_finite() && *s > 0.0) {
            return None;
        }
        let each = mass / cells.len() as f64;
        let mut first = [0.0; 3];
        let mut second = [[0.0; 3]; 3];
        for c in cells {
            let key = voxel_key(c);
            let at = [(key.x as f64 + 0.5) * size[0], (key.y as f64 + 0.5) * size[1], (key.z as f64 + 0.5) * size[2]];
            for a in 0..3 {
                first[a] += each * at[a];
                for b in 0..3 {
                    second[a][b] += each * at[a] * at[b];
                }
            }
        }
        for a in 0..3 {
            second[a][a] += mass * size[a] * size[a] / 12.0;
        }
        Some(StressPiece3 { cells: cells.to_vec(), mass, centre: first.map(|v| v / mass), second })
    }

    fn sum(&self) -> MassSum {
        MassSum { mass: self.mass, first: self.centre.map(|c| c * self.mass), second: self.second }
    }
}

/// A joint between two pieces, `a < b`, with its section in the frame of the body (physics axes, metres).
#[derive(Clone, Debug, PartialEq)]
pub struct StressJoint3 {
    pub a: u32,
    pub b: u32,
    pub section: JointSection,
}

/// A body of cells that breaks by stress: the parent of a voxel split, whose slots take the pieces that it breaks into.
#[derive(Clone, Debug, PartialEq)]
pub struct Stress3 {
    /// The body (a parent of a [`VoxelSplit3`], dynamic).
    pub parent: usize,
    /// Pascals: the principal tension that breaks a joint.
    pub strength: f64,
    pub pieces: Vec<StressPiece3>,
    pub joints: Vec<StressJoint3>,
    /// Loose parts of fewer cells than this are dust.
    pub min_cells: usize,
    /// More loose parts than slots: the smallest are dust (true) or it is an error (false).
    pub overflow_to_dust: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid stress fracture: {0}")]
pub struct StressError(String);

fn fail<T>(why: impl Into<String>) -> Result<T, StressError> {
    Err(StressError(why.into()))
}

/// What a body that breaks by stress is made of, in the world's own form (registered once, never changed).
pub(super) struct StressFamily {
    parent: usize,
    split: usize,
    strength: f64,
    pieces: Vec<StressPiece3>,
    joints: Vec<StressJoint3>,
    geoms: Vec<PieceGeom>,
    joint_geoms: Vec<JointGeom>,
    /// Every cell and the piece it is in, sorted by cell.
    owner: Vec<([i32; 3], u32)>,
    size: [f64; 3],
    min_cells: usize,
    overflow_to_dust: bool,
    /// The piece that the one joint of the world that holds the parent is anchored in, if there is one.
    anchor_piece: Option<u32>,
}

impl StressFamily {
    fn piece_of_cell(&self, cell: [i32; 3]) -> Option<u32> {
        self.owner.binary_search_by_key(&cell, |(c, _)| *c).ok().map(|i| self.owner[i].1)
    }

    /// The piece that holds the point `l` of the body's frame, looked for a little inside the body (the point is on its surface), and among the pieces that
    /// `held` has: none if the point is nowhere near them.
    fn piece_at(&self, l: [f64; 3], inward: [f64; 3], held: &[u32]) -> Option<u32> {
        for push in [1e-6, 1e-4, 1e-2, 0.25] {
            let p: [f64; 3] = std::array::from_fn(|a| l[a] + inward[a] * push * self.size[a]);
            let key: [i64; 3] = std::array::from_fn(|a| (p[a] / self.size[a]).floor() as i64);
            // the physics key [x, y, z] is the cell [x, -y - 1, -z - 1]
            let cell = [key[0] as i32, (-key[1] - 1) as i32, (-key[2] - 1) as i32];
            if let Some(piece) = self.piece_of_cell(cell) {
                if held.binary_search(&piece).is_ok() {
                    return Some(piece);
                }
            }
        }
        // nowhere: the held piece whose centre is nearest
        held.iter().copied().min_by(|a, b| {
            let d = |i: &u32| (0..3).map(|k| (self.geoms[*i as usize].centre[k] - l[k]).powi(2)).sum::<f64>();
            d(a).total_cmp(&d(b)).then(a.cmp(b))
        })
    }

    fn mass_of(&self, held: &[u32]) -> MassSum {
        let mut sum = MassSum::default();
        for &i in held {
            sum.add(&self.geoms[i as usize].mass);
        }
        sum
    }
}

/// What the cuts of a body were worked out for, and the cuts.
pub(super) struct CachedPlan {
    held: Vec<u32>,
    broken: Vec<bool>,
    plans: Arc<Vec<CutPlan>>,
}

fn vector_norm_of(v: Vector) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn fingerprint(held: &[u32], broken: &[bool]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &i in held {
        h = (h ^ u64::from(i)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h = (h ^ 0xffff_ffff).wrapping_mul(0x0000_0100_0000_01b3);
    for (k, b) in broken.iter().enumerate() {
        if *b {
            h = (h ^ k as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// What the last step left unexplained in a body that breaks by stress: the impulse (newton seconds) and the moment about the centre of mass that the balance of the whole body
/// leaves after the weight, the fields and the contacts, and the scale by which the friction of the last sub-step was taken to the step's. For a test or a probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StressBalance {
    pub force: [f64; 3],
    pub moment: [f64; 3],
    pub friction_scale: f64,
    /// The sum of the friction impulses that were put on the body in the step (the vectors of the manifolds taken to the step's totals), in the world.
    pub friction: [f64; 3],
    /// The sum of the normal impulses of the contacts on the body in the step (newton seconds): what the friction is bounded by.
    pub normal: f64,
}

/// What a break of joints does to the body that has them, ready to install.
pub(super) struct StressInstall {
    /// The cut that separates the pieces, none if the joints that broke leave the body in one piece.
    pub(super) cut: Option<VoxelCut3>,
    pub(super) stays: Vec<u32>,
    /// The pieces of each loose part, in the order of the slots they take (the pieces of the dust are held by no body).
    pub(super) loose: Vec<Vec<u32>>,
    pub(super) broken: Vec<u32>,
}

fn rows(q: &Rotation) -> [[f64; 3]; 3] {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

impl World3 {
    /// Registers bodies of cells that break by stress, after [`World3::with_voxel_splits`] and before simulating: each is the parent of a split, whose slots take the
    /// pieces that it breaks into. See the module's description of what a step does with them.
    pub fn with_stress(mut self, events: Vec<Stress3>) -> Result<Self, StressError> {
        if self.state.step != 0 || !self.stresses.is_empty() {
            return fail("register before simulation, once");
        }
        if events.len() > 4096 {
            return fail("too many bodies");
        }
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut families = Vec::new();
        let mut taken = BTreeSet::new();
        for e in events {
            if !taken.insert(e.parent) {
                return fail("a body breaks by stress once");
            }
            let Some(split) = self.voxel_splits.iter().position(|s| s.parent == e.parent) else {
                return fail("a body that breaks by stress is the parent of a voxel split: its slots take the pieces");
            };
            let spec = &self.spec.bodies[e.parent];
            let Shape3::Voxels { size, cells } = &spec.shape else {
                return fail("a body that breaks by stress is a body of cells");
            };
            if spec.kind != BodyKind::Dynamic {
                return fail("a body that breaks by stress is dynamic: a static one has nothing to hold it up and a kinematic one nothing that moves it by the loads");
            }
            if !(e.strength.is_finite() && e.strength > 0.0) {
                return fail("the strength is a positive number of pascals");
            }
            if e.pieces.is_empty() || e.pieces.len() > MAX_STRESS_PIECES {
                return fail(format!(
                    "a body of 1 to {MAX_STRESS_PIECES} pieces breaks by stress, and this has {}",
                    e.pieces.len()
                ));
            }
            // the pieces are the cells of the body, each in one piece, and weigh what it weighs
            let mut all: Vec<[i32; 3]> = e.pieces.iter().flat_map(|p| p.cells.iter().copied()).collect();
            all.sort_unstable();
            let body_cells = self.state.voxel_cells[e.parent].clone().expect("a parent has its cells");
            let _ = cells;
            if all != *body_cells {
                return fail("the cells of the pieces are not the cells of the body, each once");
            }
            let mut total = 0.0;
            for (i, p) in e.pieces.iter().enumerate() {
                let finite = p.centre.iter().chain(p.second.iter().flatten()).all(|v| v.is_finite());
                if p.cells.is_empty()
                    || !(p.mass.is_finite() && p.mass > 0.0)
                    || !finite
                    || p.cells.windows(2).any(|w| w[0] >= w[1])
                {
                    return fail(format!("the piece {i} has no cells, or a mass, centre or second moment that is not a number, or its cells are not sorted"));
                }
                total += p.mass;
            }
            if (total / spec.mass - 1.0).abs() > 1e-9 {
                return fail("the masses of the pieces are not the mass of the body");
            }
            // a body of cells has one density: the mass of a piece is its cells' (the pieces' own centres and moments are those of cells of one mass, and a body cut into pieces of other
            // densities that add up to the same would be another body)
            let per_cell = spec.mass / body_cells.len() as f64;
            if let Some(i) =
                e.pieces.iter().position(|p| (p.mass / (p.cells.len() as f64 * per_cell) - 1.0).abs() > 1e-9)
            {
                return fail(format!(
                    "the piece {i} does not weigh what its cells weigh in the body: a body of cells has one density"
                ));
            }
            let mut seen = BTreeSet::new();
            for (k, j) in e.joints.iter().enumerate() {
                let s = &j.section;
                let ok = j.a < j.b
                    && (j.b as usize) < e.pieces.len()
                    && seen.insert((j.a, j.b))
                    && s.area.is_finite()
                    && s.area > 0.0
                    && s.centroid
                        .iter()
                        .chain(&s.normal)
                        .chain(&s.lo)
                        .chain(&s.hi)
                        .chain(s.second.iter().flatten())
                        .all(|v| v.is_finite());
                if !ok {
                    return fail(format!("the joint {k} joins pieces that are not a < b in the body, or twice, or has a section that is not a number or has no area"));
                }
            }
            let size_m = size.map(|c| c / ppm);
            let mut owner: Vec<([i32; 3], u32)> =
                e.pieces.iter().enumerate().flat_map(|(i, p)| p.cells.iter().map(move |c| (*c, i as u32))).collect();
            owner.sort_unstable_by_key(|(c, _)| *c);
            let geoms: Vec<PieceGeom> =
                e.pieces.iter().map(|p| PieceGeom { centre: p.centre, mass: p.sum() }).collect();
            let joint_geoms: Vec<JointGeom> = e
                .joints
                .iter()
                .map(|j| JointGeom { a: j.a, b: j.b, centroid: j.section.centroid, normal: j.section.normal })
                .collect();
            let mut family = StressFamily {
                parent: e.parent,
                split,
                strength: e.strength,
                pieces: e.pieces,
                joints: e.joints,
                geoms,
                joint_geoms,
                owner,
                size: size_m,
                min_cells: e.min_cells,
                overflow_to_dust: e.overflow_to_dust,
                anchor_piece: None,
            };
            // the one joint of the world that holds the body, and the piece it is anchored in
            let holding: Vec<usize> = self
                .spec
                .joints
                .iter()
                .enumerate()
                .filter(|(_, j)| j.a == e.parent || j.b == Some(e.parent))
                .map(|(i, _)| i)
                .collect();
            if holding.len() > 1 {
                return fail("a body that breaks by stress is held by at most one joint of the world: the load of two is not determined by the balance of the body");
            }
            if let Some(&i) = holding.first() {
                let j = &self.spec.joints[i];
                let at = j.anchor.unwrap_or(self.spec.bodies[j.b.filter(|b| *b != e.parent).unwrap_or(j.a)].start.pos);
                let (start, q) = (&self.spec.bodies[e.parent].start, flip_q(self.spec.bodies[e.parent].start.rot));
                let rotation = rows(&quat(q));
                let world = flip(at).map(|c| c / ppm);
                let origin = flip(start.pos).map(|c| c / ppm);
                let d: [f64; 3] = std::array::from_fn(|a| world[a] - origin[a]);
                // the body's frame is the rotation's inverse of the offset
                let local: [f64; 3] =
                    std::array::from_fn(|c| rotation[0][c] * d[0] + rotation[1][c] * d[1] + rotation[2][c] * d[2]);
                let all_held: Vec<u32> = (0..family.pieces.len() as u32).collect();
                family.anchor_piece = family.piece_at(local, [0.0; 3], &all_held);
            }
            families.push(family);
        }
        let n = self.spec.bodies.len();
        for (i, f) in families.iter().enumerate() {
            self.stress_of[f.parent] = Some(i);
            for &slot in &self.voxel_splits[f.split].slots {
                self.stress_of[slot] = Some(i);
            }
        }
        self.state.stress_held = vec![Vec::new(); n];
        self.state.stress_pending = vec![Vec::new(); n];
        self.state.stress_broken = families.iter().map(|f| vec![false; f.joints.len()]).collect();
        for f in &families {
            self.state.stress_held[f.parent] = (0..f.pieces.len() as u32).collect();
        }
        self.stresses = families;
        self.checkpoints.clear();
        self.checkpoints.insert(0, Checkpoint { state: self.state.clone(), charge: 0 });
        Ok(self)
    }

    /// The principal tension (pascals) in each intact joint of the body `body` as of the last step that read it, by joint: a reading for a test or a probe, not
    /// part of the state.
    pub fn stress_levels(&self, body: usize) -> &[(u32, f64)] {
        self.stress_levels.get(body).map_or(&[], Vec::as_slice)
    }

    /// The stresses (normal, shear, bending, twist and principal) at the worst fibre of each intact joint of the body `body` as of the last step that read it, by joint.
    #[doc(hidden)]
    pub fn stress_readings(&self, body: usize) -> &[(u32, crate::stress::CutStress)] {
        self.stress_readings.get(body).map_or(&[], Vec::as_slice)
    }

    /// What the last step left unexplained in the body `body`, if the stress was read on it ([`StressBalance`]).
    #[doc(hidden)]
    pub fn stress_balance(&self, body: usize) -> Option<StressBalance> {
        self.stress_balance.get(body).copied().flatten()
    }

    /// The split whose slots take the pieces of the family `family`.
    pub(super) fn stress_split(&self, family: usize) -> usize {
        self.stresses[family].split
    }

    /// The pieces that a body of a family holds now, if it is in one.
    pub fn stress_pieces(&self, body: usize) -> Option<&[u32]> {
        self.stress_of.get(body).copied().flatten()?;
        self.state.stress_held.get(body).map(Vec::as_slice).filter(|h| !h.is_empty())
    }

    /// Whether the joint `joint` of the family of the body `body` has broken.
    pub fn stress_joint_broken(&self, body: usize, joint: usize) -> Option<bool> {
        let family = self.stress_of.get(body).copied().flatten()?;
        self.state.stress_broken[family].get(joint).copied()
    }

    /// The bodies that the stress of this step is read on: a body of a family that is in the world, dynamic, awake, and made of more than one piece.
    pub(super) fn stress_bodies(&self) -> Vec<usize> {
        if self.stresses.is_empty() {
            return Vec::new();
        }
        (0..self.spec.bodies.len())
            .filter(|&k| self.stress_of[k].is_some() && self.state.stress_held[k].len() >= 2)
            .filter(|&k| {
                let b = &self.state.bodies[self.state.handles[k]];
                b.is_enabled() && b.is_dynamic() && !b.is_sleeping()
            })
            .collect()
    }

    /// The motion of the body `k` as the balance reads it.
    pub(super) fn stress_rigid(&self, k: usize) -> Rigid {
        let family = &self.stresses[self.stress_of[k].expect("a body of a family")];
        let b = &self.state.bodies[self.state.handles[k]];
        let held = &self.state.stress_held[k];
        let centre = family.mass_of(held).centre();
        Rigid {
            position: b.translation().to_array(),
            rotation: rows(b.rotation()),
            linear: b.linvel().to_array(),
            angular: b.angvel().to_array(),
            centre,
        }
    }

    /// The contacts on the body `k` in the step that was just solved. The normal impulses are the step's totals, point by point, on the piece that each is on. The friction is
    /// one tangent impulse for each manifold (the world's friction model, the solver's simplified one, solves a single friction constraint for the whole manifold, at its middle)
    /// and the solver gives only the vector of its last sub-step, so it is returned as that, at the middle of the manifold's loaded points, for the caller to scale to the step's
    /// total. The twist of the manifold (a torque about its normal) is not given and is not read.
    fn stress_contacts(
        &self,
        k: usize,
        after: &Rigid,
        family: &StressFamily,
        held: &[u32],
    ) -> (Vec<Located>, Vec<Located>, f64) {
        let mut weighted = (0.0, 0.0);
        let substeps = self.params.num_solver_iterations.max(1) as f64;
        let st = &self.state;
        let handle = st.handles[k];
        let (mut normal_loads, mut friction_loads) = (Vec::new(), Vec::new());
        for pair in st.narrow.contact_pairs() {
            let (c1, c2) = (&st.colliders[pair.collider1], &st.colliders[pair.collider2]);
            if c1.is_sensor() || c2.is_sensor() {
                continue;
            }
            let (Some(h1), Some(h2)) = (c1.parent(), c2.parent()) else { continue };
            if h1 != handle && h2 != handle {
                continue;
            }
            let lever = |c: &Collider| c.position_wrt_parent().copied().unwrap_or(Pose::IDENTITY);
            let (Some(b1), Some(b2)) = (st.bodies.get(h1), st.bodies.get(h2)) else { continue };
            let (pose1, pose2) = (*b1.position() * lever(c1), *b2.position() * lever(c2));
            let mine_first = h1 == handle;
            let side = if mine_first { -1.0 } else { 1.0 };
            for manifold in pair.solver_manifolds() {
                let normal = manifold.data.normal;
                // into the body: against the normal for the first body, with it for the second, in the body's frame
                let inward = after.local_vector([side * normal.x, side * normal.y, side * normal.z]);
                // the loaded points of the manifold, each on a material point of the body (not the middle of the two surfaces: they come apart by what the contact slides between
                // the solver's updates of it)
                let mut loaded: Vec<([f64; 3], [f64; 3])> = Vec::new();
                for contact in &manifold.points {
                    // the force that the contact puts on the first body is along -normal (it is pushed away from the second), and on the second along +normal
                    let along = contact.data.impulse * side;
                    if along == 0.0 {
                        continue;
                    }
                    // a point of a body of cells is in the frame of the cell that it is on (the shape is a composite): the pose of that subshape takes it to the collider's
                    let local1 = manifold.subshape_pos1().map_or(contact.local_p1, |p| *p * contact.local_p1);
                    let local2 = manifold.subshape_pos2().map_or(contact.local_p2, |p| *p * contact.local_p2);
                    let at = if mine_first { pose1 * local1 } else { pose2 * local2 }.to_array();
                    loaded.push((at, [normal.x * along, normal.y * along, normal.z * along]));
                }
                if loaded.is_empty() {
                    continue;
                }
                let middle: [f64; 3] =
                    std::array::from_fn(|c| loaded.iter().map(|(p, _)| p[c]).sum::<f64>() / loaded.len() as f64);
                // the piece of a point is looked for a little toward the middle of the patch: the points of a patch that are on the edge of the cells that it lies on, which is where the
                // corners of a box on a body of cells are, belong to the cells under the patch and not to the ones beside it
                let toward = |p: [f64; 3]| -> [f64; 3] { std::array::from_fn(|c| p[c] + 1e-3 * (middle[c] - p[c])) };
                for (at, impulse) in &loaded {
                    if let Some(piece) = family.piece_at(after.local(toward(*at)), inward, held) {
                        normal_loads.push(Located { at: *at, impulse: *impulse, piece: piece as usize });
                    }
                }
                // the friction of the manifold, once, at the middle of its loaded points
                let Some(first) = manifold.points.first() else { continue };
                // taken to the step's total by the ratio of the manifold's normal impulse over the step to its normal impulse in the last sub-step (both are kept): exact when the friction
                // is steady, and right at an impact, where the normal rises or falls and the friction follows it (a friction that is not steady is not known better than that)
                let (total, last): (f64, f64) = manifold
                    .points
                    .iter()
                    .fold((0.0, 0.0), |(t, l), p| (t + p.data.impulse, l + p.data.warmstart_impulse));
                let ratio = if last > 1e-12 * total.abs() && last > 0.0 { total / last } else { substeps };
                // a friction is not more than the coefficient times the normal impulse, whatever the ratio: when the normal of the last sub-step is the small end of a large one the ratio is
                // large and a vector that is nearly zero times it is not the friction of the step
                let bound = c1.friction().max(c2.friction()) * total.abs();
                let raw = (vector_norm_of(first.data.warmstart_tangent_world), ratio);
                weighted = (weighted.0 + raw.0 * raw.1, weighted.1 + raw.0);
                // the friction vector is the impulse on the first body (the normal's sign above is the other way: it is along the normal, and the first body is pushed against it)
                let vector = first.data.warmstart_tangent_world * -side;
                if vector.x == 0.0 && vector.y == 0.0 && vector.z == 0.0 {
                    continue;
                }
                if let Some(piece) = family.piece_at(after.local(middle), inward, held) {
                    friction_loads.push(Located {
                        at: middle,
                        impulse: {
                            let scaled = [vector.x * ratio, vector.y * ratio, vector.z * ratio];
                            let size = scaled.iter().map(|v| v * v).sum::<f64>().sqrt();
                            if size > bound && size > 0.0 {
                                scaled.map(|v| v * bound / size)
                            } else {
                                scaled
                            }
                        },
                        piece: piece as usize,
                    });
                }
            }
        }
        // the order of the pairs is the solver's; the sum is not, so the order is made
        let order = |a: &Located, b: &Located| {
            a.piece.cmp(&b.piece).then_with(|| {
                a.at.iter()
                    .zip(&b.at)
                    .map(|(p, q)| p.total_cmp(q))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        };
        normal_loads.sort_by(order);
        friction_loads.sort_by(order);
        // the ratio of the friction of the step to that of the last sub-step, by the size of each manifold's, or the sub-steps if there is none
        let scale = if weighted.1 > 0.0 { weighted.0 / weighted.1 } else { substeps };
        (normal_loads, friction_loads, scale)
    }

    /// The cuts of the body `k`, worked out once for the pieces it holds and the joints that are gone. The cache is keyed by a hash of both, and an entry is taken only if what it was
    /// worked out for is what the body holds now, compared whole (a hash of 64 bits that met another would give a body the cuts of another).
    fn stress_plan(&mut self, k: usize) -> Arc<Vec<CutPlan>> {
        let fam = self.stress_of[k].expect("a body of a family");
        let held = &self.state.stress_held[k];
        let broken = &self.state.stress_broken[fam];
        let key = (k, fingerprint(held, broken));
        if let Some(cached) = self.stress_plans.get(&key) {
            if cached.held == *held && cached.broken == *broken {
                return cached.plans.clone();
            }
        }
        let family = &self.stresses[fam];
        let plans = Arc::new(plan::plan(held, broken, &family.geoms, &family.joint_geoms));
        let cached = CachedPlan { held: held.clone(), broken: broken.clone(), plans: plans.clone() };
        if self.stress_plans.len() > 256 {
            self.stress_plans.clear();
        }
        self.stress_plans.insert(key, cached);
        plans
    }

    /// Reads the stress in the joints of the bodies that `before` has, after the step that was just solved, and puts the joints that are at their strength aside
    /// to break at the start of the next step. `before` has the motion of each body at the start of the step and the acceleration (fields, loads) that was put on it.
    pub(super) fn stress_evaluate(&mut self, before: Vec<(usize, Rigid, [f64; 3])>) {
        let dt = self.spec.step;
        let g = self.spec.gravity;
        for (k, start, field) in before {
            let b = &self.state.bodies[self.state.handles[k]];
            if !b.is_enabled() {
                continue;
            }
            let fam = self.stress_of[k].expect("a body of a family");
            let after = self.stress_rigid(k);
            let held = self.state.stress_held[k].clone();
            let plans = self.stress_plan(k);
            let family = &self.stresses[fam];
            let (mut contacts, friction, friction_scale) = self.stress_contacts(k, &after, family, &held);
            let anchor = if k == family.parent
                && self.spec.joints.iter().enumerate().any(|(i, j)| {
                    (j.a == family.parent || j.b == Some(family.parent)) && self.state.joint_handles[i].is_some()
                }) {
                // the piece the joint is anchored in, or the held piece nearest to it if the body does not hold that one now
                family.anchor_piece.and_then(|p| {
                    if held.binary_search(&p).is_ok() {
                        Some(p as usize)
                    } else {
                        held.iter()
                            .copied()
                            .min_by(|a, b| {
                                let d = |i: &u32| {
                                    (0..3)
                                        .map(|c| {
                                            (family.geoms[*i as usize].centre[c] - family.geoms[p as usize].centre[c])
                                                .powi(2)
                                        })
                                        .sum::<f64>()
                                };
                                d(a).total_cmp(&d(b)).then(a.cmp(b))
                            })
                            .map(|i| i as usize)
                    }
                })
            } else {
                None
            };
            let whole = family.mass_of(&held);
            let accel = [g[0] + field[0], g[1] + field[1], g[2] + field[2]];
            // the friction, already taken to the step's total by the contact's own ratio (see stress_contacts)
            let friction_total: [f64; 3] = std::array::from_fn(|c| friction.iter().map(|f| f.impulse[c]).sum::<f64>());
            let normal_total: f64 = contacts.iter().map(|c| c.impulse.iter().map(|v| v * v).sum::<f64>().sqrt()).sum();
            contacts.extend(friction.iter().copied());
            let step = Step { whole, before: start, after, dt, accel, contacts: &contacts, anchor };
            let (left, left_moment) = step.unbalanced();
            self.stress_balance[k] = Some(StressBalance {
                force: left,
                moment: left_moment,
                friction_scale,
                friction: friction_total,
                normal: normal_total,
            });
            let mut pending: Vec<u32> = Vec::new();
            let mut levels: Vec<(u32, f64)> = Vec::new();
            let mut readings: Vec<(u32, crate::stress::CutStress)> = Vec::new();
            for cut in plans.iter() {
                let sections: Vec<&JointSection> = cut.members.iter().map(|&j| &family.joints[j].section).collect();
                let q_local = sections[0].centroid;
                let in_part = |i: usize| cut.side.contains(i as u32, family.geoms[i].centre);
                let local = step.on_part_local(&cut.mass, &in_part, q_local);
                let stresses = cut_stresses(&sections, &cut.side_has_a, &local);
                for &target in &cut.targets {
                    let at = cut.members.iter().position(|&m| m == target).expect("a target is a member of its cut");
                    levels.push((target as u32, stresses[at].principal));
                    readings.push((target as u32, stresses[at]));
                    if stresses[at].principal >= family.strength {
                        pending.push(target as u32);
                    }
                }
            }
            pending.sort_unstable();
            pending.dedup();
            levels.sort_unstable_by_key(|(j, _)| *j);
            readings.sort_unstable_by_key(|(j, _)| *j);
            self.stress_levels[k] = levels;
            self.stress_readings[k] = readings;
            self.state.stress_pending[k] = pending;
        }
    }

    /// The cut that the pending joints of the body `k` make, or none if there are none.
    /// `reserved` is how many of the slots of the pool the other bodies of the family that break in this same step have taken already: the pool is one for all of them, and the
    /// slots are not given out until the cuts are installed, so each body is asked in turn with what the ones before it will take.
    pub(super) fn stress_install_for(
        &self,
        k: usize,
        step: u64,
        reserved: usize,
    ) -> Result<Option<StressInstall>, String> {
        let pending = &self.state.stress_pending[k];
        if pending.is_empty() {
            return Ok(None);
        }
        let fam = self.stress_of[k].expect("a body of a family");
        let family = &self.stresses[fam];
        let held = &self.state.stress_held[k];
        let mut broken = self.state.stress_broken[fam].clone();
        for &j in pending {
            broken[j as usize] = true;
        }
        let parts = plan::components(held, &broken, &family.joint_geoms);
        if parts.len() == 1 {
            return Ok(Some(StressInstall {
                cut: None,
                stays: held.clone(),
                loose: Vec::new(),
                broken: pending.clone(),
            }));
        }
        let cells = |part: &[u32]| part.iter().map(|&i| family.pieces[i as usize].cells.len()).sum::<usize>();
        // the part that stays the body is the one that the joint of the world holds, if the body has one (the rest falls away from it), and the largest if not
        let held_by_world = family
            .anchor_piece
            .filter(|_| k == family.parent)
            .and_then(|p| parts.iter().position(|part| part.binary_search(&p).is_ok()));
        let stays_at = held_by_world.unwrap_or_else(|| {
            (0..parts.len()).max_by_key(|&i| (cells(&parts[i]), std::cmp::Reverse(i))).expect("some part")
        });
        let mut loose: Vec<usize> = (0..parts.len()).filter(|&i| i != stays_at).collect();
        let mut dust: Vec<usize> = Vec::new();
        loose.retain(|&i| {
            let keep = cells(&parts[i]) >= family.min_cells;
            if !keep {
                dust.push(i);
            }
            keep
        });
        let free =
            self.voxel_splits[family.split].slots.len().saturating_sub(self.state.slots_used[family.split] + reserved);
        if loose.len() > free {
            if !family.overflow_to_dust {
                return Err(format!(
                    "a break of body {k} makes {} loose parts and there are {free} slots free for them (maxFragments)",
                    loose.len()
                ));
            }
            // the largest keep their slots, of equals the first; the rest are dust
            let mut by_size = loose.clone();
            by_size.sort_by_key(|&i| (std::cmp::Reverse(cells(&parts[i])), i));
            let kept: BTreeSet<usize> = by_size.into_iter().take(free).collect();
            dust.extend(loose.iter().copied().filter(|i| !kept.contains(i)));
            loose.retain(|i| kept.contains(i));
        }
        let merged = |part: &[u32]| -> Vec<[i32; 3]> {
            let mut out: Vec<[i32; 3]> =
                part.iter().flat_map(|&i| family.pieces[i as usize].cells.iter().copied()).collect();
            out.sort_unstable();
            out
        };
        let mass = |part: &[u32]| -> f64 { part.iter().map(|&i| family.pieces[i as usize].mass).sum() };
        let mut destroyed: Vec<[i32; 3]> = dust.iter().flat_map(|&i| merged(&parts[i])).collect();
        destroyed.sort_unstable();
        let cut = VoxelCut3 {
            added: Vec::new(),
            revision: step + 1,
            destroyed,
            parent_mass: mass(&parts[stays_at]),
            pieces: loose.iter().map(|&i| VoxelPiece3 { cells: merged(&parts[i]), mass: mass(&parts[i]) }).collect(),
        };
        Ok(Some(StressInstall {
            cut: Some(cut),
            stays: parts[stays_at].clone(),
            loose: loose.iter().map(|&i| parts[i].clone()).collect(),
            broken: pending.clone(),
        }))
    }

    /// Records what an installed break did: the joints that broke, the pieces each body holds now.
    pub(super) fn stress_installed(&mut self, body: usize, slots_before: usize, install: &StressInstall) {
        let fam = self.stress_of[body].expect("a body of a family");
        for &j in &install.broken {
            self.state.stress_broken[fam][j as usize] = true;
        }
        self.state.stress_pending[body].clear();
        if install.cut.is_some() {
            let slots = self.voxel_splits[self.stresses[fam].split].slots.clone();
            self.state.stress_held[body] = install.stays.clone();
            for (i, part) in install.loose.iter().enumerate() {
                self.state.stress_held[slots[slots_before + i]] = part.clone();
            }
        }
    }
}
