//! Timed replacement of one body by an already partitioned set of rigid pieces.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Fragment3 {
    /// Index into the world's bodies. Must be dynamic and owned by one event.
    pub body: usize,
    /// Body-origin offset in the source's local scene axes/units.
    pub offset: [f64; 3],
    /// World-scene impulse at the fragment centre of mass, in kg·scene-unit/s.
    pub impulse: [f64; 3],
}

/// A fracture that fires on an impact instead of at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FractureContact {
    /// The impact watch (an index into [`World3::with_impact_watches`]) whose owner is the source of the fracture:
    /// the first impact of its projectile on the source, above that watch's threshold, fires it.
    pub watch: usize,
    /// The part, 0 to 1, of the kinetic energy of the impact relative to the pair, 1/2 mu v_n^2 with `mu` the reduced
    /// mass of the projectile and the source and `v_n` the closing speed, that becomes the pieces' push on each other.
    pub energy_fraction: f64,
}

/// Cells of a fracture's source that are not bodies when it breaks (too small to be one, or more than there are slots for): the source weighs them
/// until it breaks and they leave with it, so that the fragments and the dust sum to the source. Their momentum at the instant is not kept by
/// anything: the world records it as lost ([`World3::fracture_lost`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Dust3 {
    /// Kilograms.
    pub mass: f64,
    /// Its centre of mass in the source's own axes and units (as the offsets of the fragments are).
    pub centre: [f64; 3],
    /// The tensor about its centre of mass, in the source's own axes, in kilograms metres squared.
    pub inertia: [[f64; 3]; 3],
}

/// The linear and angular momentum (about the source's centre of mass) that the dust took away when a fracture fired, in the scene's axes and units:
/// kilograms scene units a second, and kilograms scene units squared a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FractureLost3 {
    pub momentum: [f64; 3],
    pub angular_momentum: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct Fracture3 {
    pub source: usize,
    /// Composition time. Fires on the first fixed-step boundary at or after
    /// this time for which the source participates. Fragments inherit its pose,
    /// angular velocity and velocity at their respective centres of mass. Not used with a `contact`.
    pub at: f64,
    /// Total outward impulse in kg·scene-unit/s, distributed by piece mass.
    /// Directions use actual centres of mass at release, including source rotation. Not used with a `contact`,
    /// whose push comes from the energy of the impact.
    pub radial_impulse: f64,
    pub fragments: Vec<Fragment3>,
    /// With a contact the fracture fires on the step after the impact of the watch is noticed, and not at `at`.
    pub contact: Option<FractureContact>,
    /// What of the source does not become a fragment. The fragments' masses and the dust's sum to the source's.
    pub dust: Option<Dust3>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid rigid fracture: {0}")]
pub struct FractureError(&'static str);

impl World3 {
    /// Register partitions before sampling the world. Fragment masses must sum
    /// to source mass. Their combined mass properties define the source inertia;
    /// this preserves linear/angular momentum when the partition separates.
    /// Decomposed mesh inertia uses the supplied closed triangle surface, not
    /// the approximate convex collision hulls. Callers must supply a geometric
    /// partition (this API validates ownership/mass, not surface intersections).
    /// Source joints detach at fracture. Chained/repeated ownership is rejected.
    pub fn with_fractures(mut self, events: Vec<Fracture3>) -> Result<Self, FractureError> {
        if self.state.step != 0 || !self.fractures.is_empty() {
            return Err(FractureError("register before simulation"));
        }
        let ppm = self.spec.pixels_per_meter;
        if !ppm.is_finite()
            || ppm <= 0.
            || !self.spec.start.is_finite()
            || !self.spec.step.is_finite()
            || self.spec.step <= 0.
            || events.len() > 4096
        {
            return Err(FractureError("invalid world scale, clock or event count"));
        }
        let mut owned = BTreeSet::new();
        let mut properties = Vec::new();
        for e in &events {
            let timed = e.contact.is_none();
            if !e.at.is_finite()
                || !e.radial_impulse.is_finite()
                || e.radial_impulse < 0.
                || (timed && e.at < self.spec.start)
                || !((e.at - self.spec.start) / self.spec.step).is_finite()
                || e.contact.is_some_and(|c| !c.energy_fraction.is_finite() || !(0. ..=1.).contains(&c.energy_fraction))
                || e.source >= self.spec.bodies.len()
                || !owned.insert(e.source)
                || self.owned_by_a_split(e.source)
                || e.fragments.is_empty()
                || e.fragments.len() > 4096
            {
                return Err(FractureError("invalid event time, source or piece count"));
            }
            let source_mass = self.spec.bodies[e.source].mass;
            let mut combined = Vec::with_capacity(e.fragments.len() + 1);
            for p in &e.fragments {
                if p.body >= self.spec.bodies.len()
                    || !owned.insert(p.body)
                    || self.owned_by_a_split(p.body)
                    || self.spec.bodies[p.body].kind != BodyKind::Dynamic
                    || p.offset.iter().chain(&p.impulse).any(|v| !v.is_finite() || !(v / ppm).is_finite())
                {
                    return Err(FractureError("invalid fragment ownership, type, offset or impulse"));
                }
                let props = self.fragment_properties(p.body)?;
                let pose = Pose::from_parts(vec3(flip(p.offset).map(|c| c / ppm)), Rotation::IDENTITY);
                combined.push(props.transform_by(&pose));
                properties.push((p.body, props));
            }
            // the dust is part of the source until it breaks: its mass and its tensor, about its centre of mass in the source's axes, are in the sum
            if let Some(d) = &e.dust {
                let centre = vec3(flip(d.centre).map(|c| c / ppm));
                // the tensor in the physics axes: the half turn about x changes the sign of the products with x and leaves the others
                let t = |i: usize, j: usize| {
                    let sign = |k: usize| if k == 0 { 1.0 } else { -1.0 };
                    d.inertia[i][j] * sign(i) * sign(j)
                };
                let inertia: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| t(i, j)));
                let props = tensor_mass_properties(centre, d.mass, inertia)
                    .ok_or(FractureError("the dust has no mass, centre or tensor that is a number"))?;
                valid_properties(&props)?;
                combined.push(props);
            }
            // the sum of the fragments' tensors, worked out exactly (Parry's Sum diagonalises with the solver whose mistake voxel_mass works around: for
            // a sum that is diagonal with a repeated smallest moment it would give the source another body's tensor)
            let total: MassProperties = sum_mass_properties(&combined);
            valid_properties(&total)?;
            if !source_mass.is_finite() || source_mass <= 0. || (total.mass() / source_mass - 1.).abs() > 1e-9 {
                return Err(FractureError("fragment and dust masses must conserve source mass"));
            }
            properties.push((e.source, total));
        }
        for (k, props) in properties {
            let h = self.state.handles[k];
            let collider = self.state.bodies[h].colliders()[0];
            self.state.colliders[collider].set_mass_properties(props);
            self.state.bodies[h].recompute_mass_properties_from_colliders(&self.state.colliders);
        }
        for (i, e) in events.iter().enumerate() {
            self.fracture_sources[e.source] = Some(i);
            for p in &e.fragments {
                self.fragment_owners[p.body] = Some(i);
                self.state.bodies[self.state.handles[p.body]].set_enabled(false);
            }
        }
        self.state.fractured = vec![false; events.len()];
        self.state.fracture_lost = vec![None; events.len()];
        self.fractures = events;
        self.checkpoints.clear();
        self.checkpoints.insert(0, Checkpoint { state: self.state.clone(), charge: 0 });
        Ok(self)
    }

    fn fragment_properties(&self, k: usize) -> Result<MassProperties, FractureError> {
        let spec = &self.spec.bodies[k];
        if !spec.mass.is_finite() || spec.mass <= 0. {
            return Err(FractureError("fragment mass must be positive finite"));
        }
        let body = &self.state.bodies[self.state.handles[k]];
        let mut props = match &spec.shape {
            Shape3::Decomposition(vertices, indices) => {
                if vertices.len() > 1_000_000
                    || indices.len() > 1_000_000
                    || vertices.iter().flatten().any(|v| !v.is_finite())
                    || indices.iter().flatten().any(|&i| i as usize >= vertices.len())
                {
                    return Err(FractureError("invalid fragment mesh"));
                }
                let points: Vec<_> =
                    vertices.iter().map(|v| vec3(flip(*v).map(|x| x / self.spec.pixels_per_meter))).collect();
                // the closed mesh's own properties, worked out exactly (Parry's from_trimesh diagonalises with the solver that mistakes the axes of a plate)
                mesh_mass_properties(&points, indices, spec.mass).ok_or(FractureError("fragment mesh has no volume"))?
            }
            _ => {
                let collider = &self.state.colliders[body.colliders()[0]];
                // a convex hull has its properties worked out exactly too, as a mesh's are
                match collider.shape().as_convex_polyhedron() {
                    Some(hull) => {
                        hull_mass_properties(hull, spec.mass).ok_or(FractureError("fragment hull has no volume"))?
                    }
                    None => collider.mass_properties(),
                }
            }
        };
        valid_properties(&props)?;
        props.set_mass(spec.mass, true);
        valid_properties(&props)?;
        Ok(props)
    }

    /// Whether body `k` is the parent or a slot of a split of bodies of cells: it cannot be part of a fracture too.
    fn owned_by_a_split(&self, k: usize) -> bool {
        self.slot_owners[k].is_some() || self.voxel_splits.iter().any(|s| s.parent == k)
    }

    pub(super) fn fracture_enabled(&self, k: usize) -> bool {
        self.fracture_sources[k].is_none_or(|i| !self.state.fractured[i])
            && self.fragment_owners[k].is_none_or(|i| self.state.fractured[i])
            && self.slot_owners[k].is_none_or(|_| self.state.slot_active[k])
            && !self.state.voxel_spent[k]
    }

    pub(super) fn apply_fractures(&mut self, t: f64, driver: &mut dyn Driver3) -> Result<(), String> {
        let ppm = self.spec.pixels_per_meter;
        let mut pending = Vec::new();
        // Prepare every due event before mutating anything: numerical failure
        // cannot leave a partially activated partition or an applied impulse.
        for (i, e) in self.fractures.iter().enumerate() {
            let source = &self.state.bodies[self.state.handles[e.source]];
            if self.state.fractured[i] || !source.is_enabled() {
                continue;
            }
            // a fracture by contact fires from the step after its projectile's impact is noticed
            let due = match e.contact {
                None => t >= e.at,
                Some(c) => match (self.state.impacts.get(c.watch), self.watches.get(c.watch)) {
                    (Some(found), Some(watch)) if watch.owner == e.source => {
                        found.is_some_and(|impact| t >= impact.time)
                    }
                    _ => return Err("a fracture contact names no impact watch against its source".into()),
                },
            };
            if !due {
                continue;
            }
            let (m_source, mu_energy) = match e.contact {
                Some(c) => {
                    let impact = self.state.impacts[c.watch].expect("a due contact has its impact");
                    let projectile = self.spec.bodies[self.watches[c.watch].source].mass;
                    let reduced =
                        projectile * self.spec.bodies[e.source].mass / (projectile + self.spec.bodies[e.source].mass);
                    let speed = impact.closing_speed / ppm;
                    (self.spec.bodies[e.source].mass, Some(c.energy_fraction * 0.5 * reduced * speed * speed))
                }
                None => (self.spec.bodies[e.source].mass, None),
            };
            let mut parts = Vec::with_capacity(e.fragments.len());
            // The radial push is the pieces pushing each other: it adds no momentum. Each piece gets the same speed
            // along the line from the source's centre of mass to its own, and the mass-weighted mean of those
            // velocities, which is not zero unless the partition is symmetric, is taken off every piece.
            let mut pushed = Vec::with_capacity(e.fragments.len());
            let mut mean = Vec3::ZERO;
            let source_mass = m_source;
            // the mean that is taken off is over the fragments (the dust is not pushed): without it the push would add the momentum of the dust's share
            let fragment_mass: f64 = e.fragments.iter().map(|p| self.spec.bodies[p.body].mass).sum();
            // where each fragment is: its pose and its centre of mass
            let placed: Vec<(Pose, Vec3)> = e
                .fragments
                .iter()
                .map(|p| {
                    let rb = &self.state.bodies[self.state.handles[p.body]];
                    let position = *source.position()
                        * Pose::from_parts(vec3(flip(p.offset).map(|x| x / ppm)), Rotation::IDENTITY);
                    (position, rb.mass_properties().local_mprops.world_com(&position))
                })
                .collect();
            // the push comes out of the centre of mass of what it pushes: the source's when every cell is a fragment (they are the same point, and the
            // bits are what they were), the fragments' own when there is dust, because the mean that is taken off is over the fragments and the push has to
            // leave no angular momentum about the centre of mass it is measured from
            let centre = if e.dust.is_some() {
                let mut sum = Vec3::ZERO;
                for (p, (_, com)) in e.fragments.iter().zip(&placed) {
                    sum += *com * (self.spec.bodies[p.body].mass / fragment_mass);
                }
                sum
            } else {
                source.center_of_mass()
            };
            for (p, (position, com)) in e.fragments.iter().zip(placed) {
                let direction = (com - centre).try_normalize().unwrap_or_default();
                // by impulse: the same speed for every piece; by contact the speed is found below
                let radial = direction * (e.radial_impulse / ppm / source_mass);
                mean += radial * (self.spec.bodies[p.body].mass / fragment_mass);
                pushed.push((position, com, direction, radial));
            }
            if let Some(energy) = mu_energy {
                // the push of the contact: one speed `s` along every line out of the centre, with the mean taken off,
                // chosen so that the kinetic energy of the pieces' motion relative to their mean is `energy` exactly
                mean = Vec3::ZERO;
                for (p, (_, _, direction, _)) in e.fragments.iter().zip(&pushed) {
                    mean += *direction * (self.spec.bodies[p.body].mass / fragment_mass);
                }
                let spread: f64 = e
                    .fragments
                    .iter()
                    .zip(&pushed)
                    .map(|(p, (_, _, direction, _))| {
                        self.spec.bodies[p.body].mass * (*direction - mean).length_squared()
                    })
                    .sum();
                let speed = if spread > 0. && energy > 0. { (2. * energy / spread).sqrt() } else { 0. };
                for (_, (_, _, direction, radial)) in e.fragments.iter().zip(pushed.iter_mut()) {
                    *radial = *direction * speed;
                }
                mean *= speed;
            }
            for (p, (position, com, _, radial)) in e.fragments.iter().zip(pushed) {
                let velocity = source.velocity_at_point(com)
                    + vec3(flip(p.impulse).map(|x| x / ppm)) / self.spec.bodies[p.body].mass
                    + (radial - mean);
                let angular = source.angvel();
                if !position.translation.is_finite()
                    || !position.rotation.is_finite()
                    || !com.is_finite()
                    || !velocity.is_finite()
                    || !angular.is_finite()
                {
                    return Err("rigid fracture pose or velocity exceeds numerical range".into());
                }
                parts.push((p.body, position, velocity, angular));
            }
            // what the dust takes with it: its momentum at the point of the source where its centre of mass is, and its angular momentum about the source's
            // centre of mass (the orbit of the dust and its own spin), which nothing in the world keeps
            let lost = e.dust.as_ref().map(|d| {
                let rotation = source.position().rotation;
                let at = (*source.position()
                    * Pose::from_parts(vec3(flip(d.centre).map(|c| c / ppm)), Rotation::IDENTITY))
                .translation;
                let momentum = source.velocity_at_point(at) * d.mass;
                let local = rotation.inverse() * source.angvel();
                let spin = |i: usize| {
                    let sign = |k: usize| if k == 0 { 1.0 } else { -1.0 };
                    let l = [local.x, local.y, local.z];
                    (0..3).map(|j| d.inertia[i][j] * sign(i) * sign(j) * l[j]).sum::<f64>()
                };
                let own = rotation * vec3([spin(0), spin(1), spin(2)]);
                let angular = (at - source.center_of_mass()).cross(momentum) + own;
                FractureLost3 {
                    momentum: flip([momentum.x, momentum.y, momentum.z]).map(|c| c * ppm),
                    angular_momentum: flip([angular.x, angular.y, angular.z]).map(|c| c * ppm * ppm),
                }
            });
            if lost.is_some_and(|l| !l.momentum.iter().chain(&l.angular_momentum).all(|c| c.is_finite())) {
                return Err("rigid fracture dust momentum exceeds numerical range".into());
            }
            pending.push((i, parts, lost));
        }
        for (i, parts, lost) in pending {
            self.state.fracture_lost[i] = lost;
            let source = self.fractures[i].source;
            self.state.bodies[self.state.handles[source]].set_enabled(false);
            for (j, spec) in self.spec.joints.iter().enumerate() {
                if spec.a == source || spec.b == Some(source) {
                    if let Some(h) = self.state.joint_handles[j].take() {
                        self.state.joints.remove(h, true);
                    }
                }
            }
            for (k, position, velocity, angular) in parts {
                let body = &mut self.state.bodies[self.state.handles[k]];
                body.set_body_type(RigidBodyType::Dynamic, true);
                body.set_position(position, true);
                body.set_linvel(velocity, true);
                body.set_angvel(angular, true);
                body.set_enabled(driver.enabled(t, k));
                self.state.active[k] = true;
            }
            self.state.fractured[i] = true;
        }
        Ok(())
    }
}

fn valid_properties(p: &MassProperties) -> Result<(), FractureError> {
    let inertia = p.principal_inertia();
    if !p.mass().is_finite()
        || p.mass() <= 0.
        || !p.local_com.is_finite()
        || !inertia.is_finite()
        || inertia.min_element() <= 0.
        || !p.principal_inertia_local_frame.is_finite()
    {
        return Err(FractureError("unrepresentable solid mass properties"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_mass_properties_match_analytic_tetrahedron_in_scene_units() {
        // A right tetrahedron has centroid at one quarter of each edge;
        // covariance diag=3*l²/80, offdiag=-li*lj/80. Verify the complete
        // inertia tensor independently of V-HACD's approximate collision hull.
        let b = Body3Spec {
            kind: BodyKind::Dynamic,
            shape: Shape3::Decomposition(
                vec![[0., 0., 0.], [2., 0., 0.], [0., 4., 0.], [0., 0., 6.]],
                vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
            ),
            mass: 5.,
            friction: 0.,
            restitution: 0.,
            linear_damping: 0.,
            angular_damping: 0.,
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            group: 0,
            collides_with: None,
            sensor: false,
            fixed_rotation: false,
            bullet: false,
            activate_at: 0.,
            start: Pose3::default(),
        };
        let world = World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.,
            step: 0.01,
            gravity: [0.; 3],
            pixels_per_meter: 2.,
            iterations: 8,
            bounds: Bounds3::None,
            bodies: vec![b],
            joints: vec![],
        });
        let props = world.fragment_properties(0).unwrap();
        assert!((props.mass() - 5.).abs() < 1e-12);
        for (a, b) in props.local_com.to_array().into_iter().zip([0.25, -0.5, -0.75]) {
            assert!((a - b).abs() < 1e-12);
        }
        let expected = [2.4375, -0.125, -0.1875, -0.125, 1.875, 0.375, -0.1875, 0.375, 0.9375];
        for (a, b) in props.reconstruct_inertia_matrix().to_cols_array().into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-12, "{a} != {b}");
        }
    }
}
