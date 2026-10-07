//! A body of cells that breaks at run time into pieces that take slots made for them.
//!
//! The world is built once, from a fixed list of bodies, and reached at any time by replay from a checkpoint, so a piece cannot be a body
//! that is made when the cut happens. It is a *slot*: a dynamic body the scene reserves for the body of cells (`maxFragments` of them),
//! disabled and with a placeholder shape until a cut gives it the cells of a piece. A cut is what the driver says the body of cells has
//! lost at a time (see [`Driver3::voxel_cut`]), a pure function of that time and of the impact the world has noticed, so that a replay
//! from a checkpoint made before the cut finds it again at the same step and installs the same bits.
//!
//! A piece starts in the frame of the body it came out of, so its cells keep their keys, and moves as its cells moved: its centre of mass has
//! the velocity `v + w x (c - c0)` of that point of the body, `c0` the centre of mass of the body before the cut, and it has the same spin.
//! What stays of the body gets the same rule for its new centre of mass (the centre of mass of a body is where its velocity is kept).
//! Mass, centre of mass and inertia of every part are those of its cells (Rapier's, which are the exact ones: tested to 1e-12).
use super::*;
use std::collections::BTreeSet;

/// A body of cells and the slots that its pieces take, in order.
#[derive(Clone, Debug)]
pub struct VoxelSplit3 {
    /// Index of the body of cells; its shape is [`Shape3::Voxels`].
    pub parent: usize,
    /// Indices of dynamic bodies, each with a [`Shape3::Voxels`] placeholder of the same cell size, which the pieces take in this order.
    /// A body is the slot of one split at most.
    pub slots: Vec<usize>,
}

/// One part that separates: its cells (their keys in the frame of the body they came out of) and its mass, in kilograms.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelPiece3 {
    pub cells: Vec<[i32; 3]>,
    pub mass: f64,
}

/// What a body of cells has lost, as of the revision `revision`.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelCut3 {
    /// Identifies this state of the body: the world asks again with the one it has installed, and installs a cut only for another.
    pub revision: u64,
    /// Cells that are gone (they leave the body and are no part of any piece).
    pub destroyed: Vec<[i32; 3]>,
    /// The mass of what stays of the body, in kilograms; zero if nothing stays (the body is then disabled).
    pub parent_mass: f64,
    /// The parts that separate, each into the next free slot of the split, in this order.
    pub pieces: Vec<VoxelPiece3>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid voxel split: {0}")]
pub struct VoxelSplitError(&'static str);

/// The key of a cell in the physics lattice: the scene's half turn about x maps the cell `[i, j, k]` to `[i, -j - 1, -k - 1]`.
pub(super) fn voxel_key(c: &[i32; 3]) -> IVector {
    IVector::new(i64::from(c[0]), -i64::from(c[1]) - 1, -i64::from(c[2]) - 1)
}

impl World3 {
    /// Register the bodies of cells that can break, before simulating. Each parent is a body of cells, each slot a dynamic body with a
    /// placeholder of cells of the same size; they are all disabled until a cut gives them a piece.
    pub fn with_voxel_splits(mut self, splits: Vec<VoxelSplit3>) -> Result<Self, VoxelSplitError> {
        if self.state.step != 0 || !self.voxel_splits.is_empty() {
            return Err(VoxelSplitError("register before simulation, once"));
        }
        if splits.len() > 4096 {
            return Err(VoxelSplitError("too many splits"));
        }
        let n = self.spec.bodies.len();
        let mut taken = BTreeSet::new();
        for split in &splits {
            let Some(Shape3::Voxels { size, .. }) = self.spec.bodies.get(split.parent).map(|b| &b.shape) else {
                return Err(VoxelSplitError("a parent must be a body of cells"));
            };
            if split.slots.len() > 4096 {
                return Err(VoxelSplitError("too many slots"));
            }
            for &slot in &split.slots {
                let spec =
                    self.spec.bodies.get(slot).ok_or(VoxelSplitError("a slot names a body that is not there"))?;
                let same_cells = matches!(&spec.shape, Shape3::Voxels { size: s, .. } if s == size);
                if slot == split.parent
                    || slot >= n
                    || !same_cells
                    || spec.kind != BodyKind::Dynamic
                    || !taken.insert(slot)
                    || self.fracture_sources[slot].is_some()
                    || self.fragment_owners[slot].is_some()
                {
                    return Err(VoxelSplitError(
                        "a slot is a dynamic body of cells of the parent's size that no other split or fracture owns",
                    ));
                }
            }
        }
        // a body is the parent of one split, and no body that a fracture replaces is a parent
        let mut parents = BTreeSet::new();
        for split in &splits {
            if !parents.insert(split.parent)
                || self.fracture_sources[split.parent].is_some()
                || self.fragment_owners[split.parent].is_some()
            {
                return Err(VoxelSplitError(
                    "a body is the parent of one split at most, and a fracture's source or piece is not one",
                ));
            }
        }
        // no ring: a body that makes a slot of its own maker could never be the first to be in use
        for start in 0..splits.len() {
            let mut seen = BTreeSet::new();
            let mut frontier: Vec<usize> = splits[start].slots.clone();
            while let Some(body) = frontier.pop() {
                if body == splits[start].parent {
                    return Err(VoxelSplitError("the slots of a split lead back to its parent"));
                }
                if !seen.insert(body) {
                    continue;
                }
                for other in splits.iter().filter(|o| o.parent == body) {
                    frontier.extend(other.slots.iter().copied());
                }
            }
        }
        for (s, split) in splits.iter().enumerate() {
            for &slot in &split.slots {
                self.slot_owners[slot] = Some(s);
                self.state.bodies[self.state.handles[slot]].set_enabled(false);
            }
        }
        for split in &splits {
            let Shape3::Voxels { cells, .. } = &self.spec.bodies[split.parent].shape else {
                unreachable!("checked above")
            };
            self.state.voxel_cells[split.parent] = Some(std::sync::Arc::new(sorted_unique(cells)));
        }
        self.state.slots_used = vec![0; splits.len()];
        self.voxel_splits = splits;
        self.checkpoints.clear();
        self.checkpoints.insert(0, Checkpoint { state: self.state.clone(), charge: 0 });
        Ok(self)
    }

    /// Installs the cuts that the driver says the bodies of cells have suffered, before the step that starts at `t`. Every cut of the call is
    /// asked for and checked before any of them is applied: if one cannot be installed none is, and the world is as it was. (A body that
    /// becomes a slot in use in this call is cut from the next step: the splits are independent within a call.)
    pub(super) fn apply_voxel_cuts(&mut self, step: u64, t: f64, driver: &mut dyn Driver3) -> Result<(), String> {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        struct Prepared {
            split: usize,
            parent: usize,
            cut: VoxelCut3,
            shape: SharedShape,
            parts: Vec<(SharedShape, Vec3, MassProperties)>,
            parent_props: Option<MassProperties>,
            remaining: std::sync::Arc<Vec<[i32; 3]>>,
            pose: Pose,
            v: Vec3,
            w: Vec3,
            c_old: Vec3,
        }
        let mut prepared: Vec<Prepared> = Vec::new();
        for s in 0..self.voxel_splits.len() {
            let (parent, slots) = (self.voxel_splits[s].parent, self.voxel_splits[s].slots.clone());
            if !self.fracture_enabled(parent) || !driver.enabled(t, parent) {
                continue;
            }
            // a body that is a slot is cut from the step after the one it was taken into use in, however often that step is asked for: what a
            // second request of the step would find is not what the first did
            if self.state.slot_since[parent].is_some_and(|since| since >= step) {
                continue;
            }
            let impact = self.impact_of(parent);
            let Some(cut) = driver.voxel_cut(t, parent, self.state.voxel_revisions[parent], impact.as_ref())? else {
                continue;
            };
            if Some(cut.revision) == self.state.voxel_revisions[parent] {
                continue;
            }
            let free = slots.len() - self.state.slots_used[s];
            if cut.pieces.len() > free {
                return Err(format!(
                    "a cut of body {parent} separates {} pieces and only {free} of its {} slots are free (maxFragments)",
                    cut.pieces.len(),
                    slots.len()
                ));
            }
            if !cut.parent_mass.is_finite()
                || cut.parent_mass < 0.0
                || cut.pieces.iter().any(|p| !(p.mass.is_finite() && p.mass > 0.0) || p.cells.is_empty())
            {
                return Err(format!(
                    "a cut of body {parent} has a mass that is not positive, or a piece with no cells"
                ));
            }
            let Shape3::Voxels { size, .. } = &self.spec.bodies[parent].shape else {
                unreachable!("a parent is a body of cells")
            };
            let vsize = vec3(size.map(|c| c / ppm));
            // the body before the cut
            let h = self.state.handles[parent];
            let (pose, v, w, c_old) = {
                let rb = &self.state.bodies[h];
                (*rb.position(), rb.linvel(), rb.angvel(), rb.center_of_mass())
            };
            // the cells that stay: the cells the body has, less those that are destroyed and those that go to the pieces (the body's own record of
            // its cells, not Parry's iteration over its shape, which is a half-open range of floor(p / size) that loses a cell at the edge of a chunk)
            let Some(have) = self.state.voxel_cells[parent].clone() else {
                return Err(format!("body {parent} has no record of its cells"));
            };
            let mut leaving: Vec<[i32; 3]> =
                cut.destroyed.iter().chain(cut.pieces.iter().flat_map(|p| &p.cells)).copied().collect();
            leaving.sort_unstable();
            if let Some(twice) = leaving.windows(2).find(|w| w[0] == w[1]) {
                return Err(format!("a cut of body {parent} takes the cell {:?} twice", twice[0]));
            }
            if let Some(missing) = leaving.iter().find(|c| have.binary_search(c).is_err()) {
                return Err(format!("a cut of body {parent} takes the cell {missing:?}, which the body has not"));
            }
            let remaining: Vec<[i32; 3]> = have.iter().filter(|c| leaving.binary_search(c).is_err()).copied().collect();
            // the new shape of the body, edited on a private copy so that nothing changed if anything after this fails
            let collider = self.state.bodies[h].colliders()[0];
            let mut shape = self.state.colliders[collider].shared_shape().clone();
            {
                let voxels = shape.make_mut().as_voxels_mut().ok_or("a body of cells has lost its cells")?;
                for cell in &leaving {
                    voxels.set_voxel(voxel_key(cell), false);
                }
            }
            // the pieces: their shapes, mass properties (of their cells, exactly) and the velocity of their centres of mass
            let mut parts = Vec::with_capacity(cut.pieces.len());
            for p in &cut.pieces {
                let mut keys: Vec<IVector> = p.cells.iter().map(voxel_key).collect();
                keys.sort_by_key(|k| (k.z, k.y, k.x));
                keys.dedup();
                let piece = SharedShape::voxels(vsize, &keys);
                let props = voxel_mass_properties(&keys, size.map(|c| c / ppm), p.mass)
                    .ok_or("a piece of a cut has no cells or no mass")?;
                let centre = pose * props.local_com;
                parts.push((piece, v + w.cross(centre - c_old), props));
            }
            // what stays: the mass properties of the cells that are left, from the shape that has been edited
            let parent_props = if cut.parent_mass > 0.0 {
                let keys: Vec<IVector> = remaining.iter().map(voxel_key).collect();
                Some(
                    voxel_mass_properties(&keys, size.map(|c| c / ppm), cut.parent_mass)
                        .ok_or("what stays of a body of cells has no cells")?,
                )
            } else {
                None
            };
            if parts.iter().any(|(_, velocity, _)| !velocity.is_finite()) || !w.is_finite() {
                return Err("a cut of a body of cells exceeds numerical range".into());
            }
            prepared.push(Prepared {
                split: s,
                parent,
                cut,
                shape,
                parts,
                parent_props,
                remaining: std::sync::Arc::new(remaining),
                pose,
                v,
                w,
                c_old,
            });
        }
        // install: nothing below can fail
        for Prepared { split, parent, cut, shape, parts, parent_props, remaining, pose, v, w, c_old } in prepared {
            let slots = self.voxel_splits[split].slots.clone();
            let h = self.state.handles[parent];
            let collider = self.state.bodies[h].colliders()[0];
            self.state.colliders[collider].set_shape(shape);
            self.state.voxel_cells[parent] = Some(remaining);
            let body = &mut self.state.bodies[h];
            if let Some(props) = parent_props {
                self.state.colliders[collider].set_mass_properties(props);
                body.recompute_mass_properties_from_colliders(&self.state.colliders);
                if body.is_dynamic() {
                    let c_new = body.center_of_mass();
                    body.set_linvel(v + w.cross(c_new - c_old), true);
                }
            } else {
                // nothing stays: the body is gone for good, which the state has to say or the next step shows it again
                body.set_enabled(false);
                self.state.voxel_spent[parent] = true;
            }
            let used = self.state.slots_used[split];
            for (i, (piece, velocity, props)) in parts.into_iter().enumerate() {
                let slot = slots[used + i];
                let hs = self.state.handles[slot];
                let cs = self.state.bodies[hs].colliders()[0];
                self.state.colliders[cs].set_shape(piece);
                self.state.colliders[cs].set_mass_properties(props);
                let body = &mut self.state.bodies[hs];
                body.set_body_type(RigidBodyType::Dynamic, true);
                body.set_position(pose, true);
                body.recompute_mass_properties_from_colliders(&self.state.colliders);
                body.set_linvel(velocity, true);
                body.set_angvel(w, true);
                self.state.slot_active[slot] = true;
                self.state.slot_since[slot] = Some(step);
                self.state.voxel_cells[slot] = Some(std::sync::Arc::new(sorted_unique(&cut.pieces[i].cells)));
                body.set_enabled(driver.enabled(t, slot));
                self.state.active[slot] = true;
            }
            self.state.slots_used[split] = used + cut.pieces.len();
            self.state.voxel_revisions[parent] = Some(cut.revision);
            // what lost its support or gained a neighbour has to be looked at again
            for &handle in &self.state.handles {
                let body = &mut self.state.bodies[handle];
                if body.is_enabled() && body.is_dynamic() {
                    body.wake_up(true);
                }
            }
        }
        Ok(())
    }
}

/// The cells sorted by key, each once.
fn sorted_unique(cells: &[[i32; 3]]) -> Vec<[i32; 3]> {
    let mut v = cells.to_vec();
    v.sort_unstable();
    v.dedup();
    v
}
