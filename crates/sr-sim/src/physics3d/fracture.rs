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
                || e.fragments.is_empty()
                || e.fragments.len() > 4096
            {
                return Err(FractureError("invalid event time, source or piece count"));
            }
            let source_mass = self.spec.bodies[e.source].mass;
            let mut combined = Vec::with_capacity(e.fragments.len());
            for p in &e.fragments {
                if p.body >= self.spec.bodies.len()
                    || !owned.insert(p.body)
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
            let total: MassProperties = combined.into_iter().sum();
            valid_properties(&total)?;
            if !source_mass.is_finite() || source_mass <= 0. || (total.mass() / source_mass - 1.).abs() > 1e-9 {
                return Err(FractureError("fragment masses must conserve source mass"));
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
                MassProperties::from_trimesh(1., &points, indices)
            }
            _ => self.state.colliders[body.colliders()[0]].mass_properties(),
        };
        valid_properties(&props)?;
        props.set_mass(spec.mass, true);
        valid_properties(&props)?;
        Ok(props)
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
            for p in &e.fragments {
                let rb = &self.state.bodies[self.state.handles[p.body]];
                let position =
                    *source.position() * Pose::from_parts(vec3(flip(p.offset).map(|x| x / ppm)), Rotation::IDENTITY);
                let com = rb.mass_properties().local_mprops.world_com(&position);
                let direction = (com - source.center_of_mass()).try_normalize().unwrap_or_default();
                // by impulse: the same speed for every piece; by contact the speed is found below
                let radial = direction * (e.radial_impulse / ppm / source_mass);
                mean += radial * (self.spec.bodies[p.body].mass / source_mass);
                pushed.push((position, com, direction, radial));
            }
            if let Some(energy) = mu_energy {
                // the push of the contact: one speed `s` along every line out of the centre, with the mean taken off,
                // chosen so that the kinetic energy of the pieces' motion relative to their mean is `energy` exactly
                mean = Vec3::ZERO;
                for (p, (_, _, direction, _)) in e.fragments.iter().zip(&pushed) {
                    mean += *direction * (self.spec.bodies[p.body].mass / source_mass);
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
            pending.push((i, parts));
        }
        for (i, parts) in pending {
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
