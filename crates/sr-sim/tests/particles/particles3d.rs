use sr_sim::particles3d::{Burst, Driver, Emission, Emitter, Error, Hit, Shape, Spec};

#[derive(Default)]
struct Scene {
    rate: f64,
    wall: bool,
    bad: bool,
}
impl Driver for Scene {
    fn emission(&mut self, t: f64) -> Result<Emission, Error> {
        let mut m = sr_volume::Transform::identity().columns();
        m[12] = t;
        Ok(Emission {
            transform: sr_volume::Transform::new(m).unwrap(),
            rate: if self.bad { f64::NAN } else { self.rate },
            ..Default::default()
        })
    }
    fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.; 3])
    }
    fn sweep(&mut self, _t: f64, _dt: f64, from: [f64; 3], to: [f64; 3], radius: f64) -> Result<Option<Hit>, Error> {
        if !self.wall || to[1] <= 2. - radius {
            return Ok(None);
        }
        let f = if from[1] >= 2. - radius { 0. } else { (2. - radius - from[1]) / (to[1] - from[1]) };
        Ok(Some(Hit {
            fraction: f,
            position: [from[0] + (to[0] - from[0]) * f, 2. - radius, from[2] + (to[2] - from[2]) * f],
            normal: [0., -1., 0.],
            velocity: [0.; 3],
        }))
    }
}
fn burst(time: f64, count: u64) -> Burst {
    Burst { time, count, repeat: 0, interval: 1. }
}
fn spec() -> Spec {
    Spec { step: 0.1, lifetime: 10., max_particles: 100, gravity: [0.; 3], ..Default::default() }
}

#[test]
fn outgoing_curved_flight_does_not_repeat_spurious_chord_contacts() {
    struct NearFloor(Scene);
    impl Driver for NearFloor {
        fn emission(&mut self, _: f64) -> Result<Emission, Error> {
            let mut columns = sr_volume::Transform::identity().columns();
            columns[13] = 1.9 - 1e-8;
            Ok(Emission { transform: sr_volume::Transform::new(columns).unwrap(), ..Default::default() })
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, t: f64, dt: f64, a: [f64; 3], b: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            self.0.sweep(t, dt, a, b, r)
        }
    }
    let mut s = spec();
    s.step = 0.01;
    s.radius = 0.1;
    s.restitution = 0.;
    s.velocity = [0., -0.02, 0.];
    s.gravity = [0., 10., 0.];
    s.bursts = vec![burst(0., 1)];
    let mut e = Emitter::new(s).unwrap();
    let mut driver = NearFloor(Scene { wall: true, ..Default::default() });
    let frame = e.at(0.01, &mut driver).unwrap();
    assert!((frame.particles[0].position[1] - 1.9).abs() < 1e-6);
    assert!(frame.particles[0].velocity[1].abs() < 1e-8);
}

#[test]
fn births_use_their_exact_time_and_ballistic_motion_in_all_three_axes() {
    let mut s = spec();
    s.bursts = vec![burst(0., 1), burst(0.15, 1)];
    s.velocity = [1., 2., 3.];
    s.gravity = [0., 10., -2.];
    let mut e = Emitter::new(s).unwrap();
    let mut d = Scene::default();
    let first = e.at(0., &mut d).unwrap();
    assert_eq!(first.particles.len(), 1);
    assert_eq!(first.particles[0].position, [0.; 3]);
    let f = e.at(0.2, &mut d).unwrap();
    assert_eq!(f.particles.len(), 2);
    for (p, birth) in f.particles.iter().zip([0., 0.15]) {
        let t = 0.2 - birth;
        for (a, b) in p.position.iter().zip([birth + t, 2. * t + 5. * t * t, 3. * t - t * t]) {
            assert!((a - b).abs() < 1e-12, "{p:?}");
        }
        assert!((p.age(f.time) - t).abs() < 1e-12);
    }
    let sub = e.at(0.175, &mut d).unwrap();
    assert_eq!(sub.particles.len(), 2);
    assert!((sub.particles[1].age(sub.time) - 0.025).abs() < 1e-12);
}

#[test]
fn rate_carry_caps_and_expiry_are_deterministic_across_seeks() {
    let mut s = spec();
    s.max_particles = 2;
    s.lifetime = 0.125;
    s.bursts = vec![burst(0.02, 1000)];
    s.seed = u64::MAX;
    s.speed = 3.;
    s.spread = 180.;
    let mut e = Emitter::new(s).unwrap();
    let mut d = Scene { rate: 40., ..Default::default() };
    let a = e.at(0.3, &mut d).unwrap().clone();
    assert!(a.dropped >= 998);
    assert!(a.particles.len() <= 2);
    assert!(a.particles.iter().all(|p| p.age(a.time) < p.lifetime));
    let b = e.at(0.12, &mut d).unwrap().clone();
    assert!(b.particles.iter().all(|p| p.birth <= 0.12));
    assert_eq!(e.at(0.3, &mut d).unwrap(), &a);
    assert_eq!(e.at(0.12, &mut d).unwrap(), &b);
    assert!(e.checkpoint_bytes() <= e.spec().checkpoint_bytes);
}

#[test]
fn fast_particles_sweep_against_3d_geometry_and_rest_under_gravity() {
    let mut s = spec();
    s.bursts = vec![burst(0., 1)];
    s.velocity = [0., 100., 0.];
    s.radius = 0.1;
    s.restitution = 1.;
    let mut e = Emitter::new(s.clone()).unwrap();
    let mut d = Scene { wall: true, ..Default::default() };
    let f = e.at(0.1, &mut d).unwrap();
    assert!(f.particles[0].position[1] < 0.);
    assert!(f.particles[0].velocity[1] < -99.);
    s.velocity = [0.; 3];
    s.gravity = [0., 10., 0.];
    s.restitution = 0.;
    let mut e = Emitter::new(s).unwrap();
    let f = e.at(2., &mut d).unwrap();
    assert!((f.particles[0].position[1] - 1.9).abs() < 1e-5, "{f:?}");
    assert!(f.particles[0].velocity[1].abs() < 1e-6);
}

#[test]
fn curved_flight_cannot_skip_a_wall_when_both_step_endpoints_are_on_the_same_side() {
    let mut s = spec();
    s.step = 1.;
    s.bursts = vec![burst(0., 1)];
    s.velocity = [0., 100., 0.];
    s.gravity = [0., -200., 0.];
    s.radius = 0.1;
    s.restitution = 1.;
    let mut e = Emitter::new(s).unwrap();
    let f = e.at(1., &mut Scene { wall: true, ..Default::default() }).unwrap();
    // Without contact, y(0) = y(1) = 0, but y(0.5) = 25. The ceiling
    // at y=1.9 must reverse the velocity on the way up.
    assert!(f.particles[0].position[1] < -90., "{f:?}");
    assert!(f.particles[0].velocity[1] < -200., "{f:?}");
}

#[test]
fn particle_instances_keep_the_birth_affine_and_spin_in_world_space() {
    struct AffineSource;
    impl Driver for AffineSource {
        fn emission(&mut self, t: f64) -> Result<Emission, Error> {
            Ok(Emission {
                transform: sr_volume::Transform::new([
                    0.,
                    2.,
                    0.,
                    0.,
                    -3.,
                    0.,
                    0.,
                    0.,
                    0.,
                    0.,
                    4.,
                    0.,
                    t * 100.,
                    0.,
                    0.,
                    1.,
                ])
                .unwrap(),
                ..Default::default()
            })
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, _: f64, _: f64, _: [f64; 3], _: [f64; 3], _: f64) -> Result<Option<Hit>, Error> {
            Ok(None)
        }
    }
    let mut s = spec();
    s.rotation = [90., 0., 0.];
    s.angular_velocity = [0., 0., 90.];
    s.bursts = vec![burst(0., 1), burst(0.5, 1)];
    let mut emitter = Emitter::new(s).unwrap();
    let f = emitter.at(1., &mut AffineSource).unwrap().clone();
    assert_eq!(f.particles[0].position, [0.; 3]);
    assert_eq!(f.particles[1].position, [50., 0., 0.]);
    let m = f.particles[0].transform(f.time).unwrap();
    for (p, expected) in [([1., 0., 0.], [-2., 0., 0.]), ([0., 1., 0.], [0., 0., 4.])] {
        for (a, b) in m.index_to_world(p).into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-12, "{m:?}");
        }
    }
    assert!(f.particles[0].transform(f64::NAN).is_err());
    emitter.at(0.23, &mut AffineSource).unwrap();
    assert_eq!(emitter.at(1., &mut AffineSource).unwrap(), &f);
}

#[test]
fn size_and_spin_variance_are_seeded_and_collision_radius_tracks_birth_size() {
    struct Radii(Vec<f64>);
    impl Driver for Radii {
        fn emission(&mut self, _: f64) -> Result<Emission, Error> {
            Ok(Emission::default())
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, _: f64, _: f64, _: [f64; 3], _: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            self.0.push(r);
            Ok(None)
        }
    }
    let mut s = spec();
    s.bursts = vec![burst(0., 100)];
    s.scale_variance = 0.25;
    s.angular_velocity = [1., 2., 3.];
    s.angular_velocity_variance = [4., 5., 6.];
    s.rotation_variance = [180.; 3];
    s.radius = 2.;
    let mut e = Emitter::new(s.clone()).unwrap();
    let mut d = Radii(Vec::new());
    let f = e.at(0.1, &mut d).unwrap().clone();
    assert_eq!(d.0.len(), 100);
    for (p, r) in f.particles.iter().zip(d.0) {
        assert!((0.75..1.25).contains(&p.scale));
        assert_eq!(r, 2. * p.scale);
        for i in 0..3 {
            assert!((p.angular_velocity[i] - s.angular_velocity[i]).abs() <= s.angular_velocity_variance[i]);
        }
        let m = p.transform(f.time).unwrap().columns();
        for i in 0..3 {
            assert!((m[4 * i].hypot(m[4 * i + 1]).hypot(m[4 * i + 2]) - p.scale).abs() < 1e-12);
        }
    }
    assert_eq!(Emitter::new(s.clone()).unwrap().at(0.1, &mut Radii(Vec::new())).unwrap(), &f);
    s.scale_variance = 1.;
    assert!(Emitter::new(s).is_err());
}

#[test]
fn unused_mesh_vertices_do_not_change_area_sampling_or_reject_valid_faces() {
    use sr_sim::particles3d::mesh::Mesh;
    let points = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
    let mut extra = points.to_vec();
    extra.push([1e300; 3]);
    let original = Mesh::new(&points, &[[0, 1, 2]], 4096).unwrap();
    let padded = Mesh::new(&extra, &[[0, 1, 2]], 4096).unwrap();
    for id in 0..100 {
        assert_eq!(original.sample(42, id), padded.sample(42, id));
    }
}

#[test]
fn work_and_event_limits_are_atomic_and_do_not_prevent_shorter_recovery_seeks() {
    let mut s = spec();
    s.max_work = 4;
    s.bursts = vec![burst(0., 1)];
    let mut e = Emitter::new(s).unwrap();
    let mut d = Scene::default();
    let first = e.at(0., &mut d).unwrap().clone();
    assert!(matches!(e.at(1., &mut d), Err(Error::Limit(_))));
    assert_eq!(e.frame(), &first);
    assert_eq!(e.at(0.1, &mut d).unwrap().particles.len(), 1);

    let mut s = spec();
    s.max_events = 2;
    s.bursts = vec![Burst { time: 0., count: 1, repeat: 100, interval: 0.01 }];
    let mut e = Emitter::new(s).unwrap();
    let first = e.at(0., &mut d).unwrap().clone();
    assert!(matches!(e.at(0.1, &mut d), Err(Error::Limit(_))));
    assert_eq!(e.frame(), &first);
    assert_eq!(e.at(0.01, &mut d).unwrap().emitted, 2);
}

#[test]
fn friction_uses_contact_surface_velocity_and_caps_the_tangent_impulse() {
    struct SlidingPlane(Scene);
    impl Driver for SlidingPlane {
        fn emission(&mut self, t: f64) -> Result<Emission, Error> {
            self.0.emission(t)
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, t: f64, dt: f64, from: [f64; 3], to: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            Ok(self.0.sweep(t, dt, from, to, r)?.map(|hit| Hit { velocity: [4., 0., 0.], ..hit }))
        }
    }
    let mut s = spec();
    s.step = 0.5;
    s.bursts = vec![burst(0., 1)];
    s.radius = 0.1;
    s.velocity = [2., 10., 0.];
    s.restitution = 0.5;
    s.friction = 0.2;
    let mut e = Emitter::new(s).unwrap();
    let f = e.at(0.5, &mut SlidingPlane(Scene { wall: true, ..Default::default() })).unwrap();
    let p = &f.particles[0];
    // Contact occurs at .19s. The normal impulse is 15; friction may remove
    // up to 3 of tangential relative speed, but never overshoot the surface's 4.
    assert_eq!(p.velocity, [4., -5., 0.]);
    assert!((p.position[0] - 1.62).abs() < 1e-12);
    assert!((p.position[1] - 0.35).abs() < 1e-7);
}

#[test]
fn huge_burst_counts_are_bounded_and_particle_ids_never_wrap() {
    let mut s = spec();
    s.max_particles = 4;
    s.bursts = vec![burst(0., u64::MAX), burst(0.1, 2)];
    let mut e = Emitter::new(s).unwrap();
    let first = e.at(0., &mut Scene::default()).unwrap().clone();
    assert_eq!(first.emitted, u64::MAX);
    assert_eq!(first.dropped, u64::MAX - 4);
    assert_eq!(first.particles.iter().map(|p| p.id).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert!(matches!(e.at(0.1, &mut Scene::default()), Err(Error::Limit("particle ID overflow"))));
    assert_eq!(e.frame(), &first);
}

#[test]
fn shared_triangle_collider_geometry_is_validated_and_budgeted() {
    use sr_sim::particles3d::collider::Geometry;
    let p = [[-2., 0., -2.], [2., 0., -2.], [0., 0., 2.]];
    assert!(Geometry::new(&p, &[[0, 1, 9]], 1 << 20).is_err());
    assert!(Geometry::new(&p, &[[0, 1, 2]], 1).is_err());
    let geometry = Geometry::new(&p, &[[0, 1, 2]], 1 << 20).unwrap();
    let collider = geometry.moving(Default::default(), [0.; 3], [0.; 3], 0.).unwrap();
    let hit = collider.sweep(0., 1., [0., -2., 0.], [0., 2., 0.], 0.1).unwrap().unwrap();
    assert!((hit.fraction - 0.475).abs() < 1e-8);
}

#[test]
fn dense_ejecta_replays_identically_after_scrubbing_with_evicted_checkpoints() {
    let mut s = spec();
    s.step = 1. / 60.;
    s.max_particles = 2048;
    s.lifetime = 1.3;
    s.lifetime_variance = 0.2;
    s.speed = 10.;
    s.speed_variance = 3.;
    s.spread = 150.;
    s.gravity = [0., 1., 0.];
    s.scale_variance = 0.1;
    s.angular_velocity_variance = [120.; 3];
    s.checkpoint_bytes = 400_000;
    s.bursts = vec![Burst { time: 0., count: 700, repeat: 8, interval: 0.4 }];
    s.seed = u64::MAX - 41;
    let mut sequential = Emitter::new(s.clone()).unwrap();
    let mut d = Scene { rate: 27., ..Default::default() };
    let times = [0., 0.237, 1.25, 2.337, 3.21];
    let references: Vec<_> = times.iter().map(|&t| sequential.at(t, &mut d).unwrap().clone()).collect();
    let mut scrubbed = Emitter::new(s).unwrap();
    for i in [4, 2, 0, 3, 1, 4] {
        assert_eq!(scrubbed.at(times[i], &mut d).unwrap(), &references[i]);
        assert!(scrubbed.checkpoint_bytes() <= 400_000);
    }
}

#[test]
fn invalid_inputs_preserve_the_last_frame_and_budgets_fail_before_allocation() {
    let mut s = spec();
    s.bursts = vec![burst(0., 1)];
    s.max_bytes = 1;
    assert!(Emitter::new(s).is_err());
    let mut e = Emitter::new(spec()).unwrap();
    let mut d = Scene { rate: 10., ..Default::default() };
    let good = e.at(0.2, &mut d).unwrap().clone();
    d.bad = true;
    assert!(e.at(0.3, &mut d).is_err());
    assert_eq!(e.frame(), &good);
    d.bad = false;
    assert!(e.at(0.3, &mut d).is_ok());
    assert!(e.at(f64::INFINITY, &mut d).is_err());
}

#[test]
fn spherical_emission_and_cone_spread_are_three_dimensional_and_seeded() {
    let mut s = spec();
    s.bursts = vec![burst(0., 100)];
    s.shape = Shape::Sphere { radius: 2. };
    s.speed = 3.;
    s.direction = [0., 0., 1.];
    s.spread = 60.;
    s.seed = 9007199254740993;
    let mut a = Emitter::new(s.clone()).unwrap();
    let first = a.at(0., &mut Scene::default()).unwrap().clone();
    for p in &first.particles {
        assert!(p.position.iter().map(|v| v * v).sum::<f64>() <= 4.00000001);
        assert!((p.velocity.iter().map(|v| v * v).sum::<f64>().sqrt() - 3.).abs() < 1e-12);
        assert!(p.velocity[2] >= 3. * 30f64.to_radians().cos() - 1e-12);
    }
    assert!(first.particles.iter().any(|p| p.position[2].abs() > 0.5));
    s.seed -= 1;
    let mut b = Emitter::new(s).unwrap();
    assert_ne!(first, b.at(0., &mut Scene::default()).unwrap().clone());
}

#[test]
fn mesh_emission_samples_triangle_area_and_rejects_bad_or_oversized_geometry() {
    use sr_sim::particles3d::mesh::Mesh;
    let points = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 2.], [2., 0., 2.], [0., 2., 2.]];
    let faces = [[0, 1, 2], [3, 4, 5]];
    assert!(Mesh::new(&points, &faces, 1).is_err());
    assert!(Mesh::new(&points, &[[0, 1, 99]], 1 << 20).is_err());
    assert!(Mesh::new(&points, &[[0, 0, 1]], 1 << 20).is_err());
    let mesh = std::sync::Arc::new(Mesh::new(&points, &faces, 1 << 20).unwrap());
    let mut s = spec();
    s.max_particles = 10_000;
    s.bursts = vec![burst(0., 10_000)];
    s.shape = Shape::Mesh(mesh);
    let mut e = Emitter::new(s).unwrap();
    let frame = e.at(0., &mut Scene::default()).unwrap();
    let mut large = 0;
    for p in &frame.particles {
        let [x, y, z] = p.position;
        assert!(x >= 0. && y >= 0.);
        if z == 0. {
            assert!(x + y <= 1. + 1e-12);
        } else {
            assert!((z - 2.).abs() < 1e-12 && x + y <= 2. + 1e-12);
            large += 1;
        }
    }
    assert!((7600..8400).contains(&large), "area-weighted large triangle count: {large}");
}

#[test]
fn drag_matches_the_analytic_constant_force_solution() {
    let mut s = spec();
    s.bursts = vec![burst(0., 1)];
    s.velocity = [5., 0., 0.];
    s.gravity = [2., 0., 0.];
    s.drag = 3.;
    let mut e = Emitter::new(s).unwrap();
    let f = e.at(0.37, &mut Scene::default()).unwrap();
    let p = &f.particles[0];
    let exponential = (-3f64 * 0.37).exp();
    let velocity = 2. / 3. + (5. - 2. / 3.) * exponential;
    let position = 2. / 3. * 0.37 + (5. - 2. / 3.) * (1. - exponential) / 3.;
    assert!((p.velocity[0] - velocity).abs() < 1e-12);
    assert!((p.position[0] - position).abs() < 1e-12);
}

#[test]
fn collision_overflow_is_an_error_even_at_the_end_of_a_step() {
    struct Overflow(Scene);
    impl Driver for Overflow {
        fn emission(&mut self, t: f64) -> Result<Emission, Error> {
            self.0.emission(t)
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, _: f64, _: f64, _: [f64; 3], _: [f64; 3], _: f64) -> Result<Option<Hit>, Error> {
            Ok(Some(Hit { fraction: 1., position: [f64::MAX, 0., 0.], normal: [1., 0., 0.], velocity: [0.; 3] }))
        }
    }
    let mut s = spec();
    s.bursts = vec![burst(0., 1)];
    s.radius = f64::MAX;
    let mut e = Emitter::new(s).unwrap();
    let mut d = Overflow(Scene::default());
    let first = e.at(0., &mut d).unwrap().clone();
    assert!(e.at(0.1, &mut d).is_err());
    assert_eq!(e.frame(), &first);
}

#[test]
fn activation_bursts_caps_and_small_checkpoint_budget_survive_long_reverse_seeks() {
    let mut s = spec();
    s.start = 1.;
    s.end = Some(1.25);
    s.bursts = vec![Burst { time: 1., count: 2, repeat: 10, interval: 0.1 }];
    s.checkpoint_bytes = 1024;
    s.lifetime = 10.;
    let mut e = Emitter::new(s).unwrap();
    let mut d = Scene::default();
    assert!(e.at(0.9, &mut d).unwrap().particles.is_empty());
    let end = e.at(4., &mut d).unwrap().clone();
    assert_eq!(end.emitted, 6);
    assert_eq!(end.particles.len(), 6);
    let start = e.at(1., &mut d).unwrap().clone();
    assert_eq!(start.particles.len(), 2);
    assert_eq!(e.at(4., &mut d).unwrap(), &end);
    assert_eq!(e.at(1., &mut d).unwrap(), &start);
    assert!(e.checkpoint_bytes() <= 1024);
}

#[test]
fn rigid_colliders_sweep_thin_geometry_with_translation_and_rotation() {
    use rapier3d_f64::parry::shape::SharedShape;
    use sr_sim::{particles3d::collider::Collider, physics3d::Pose3};
    let plane = SharedShape::cuboid(1., 1., 0.025);
    for speed in [0., -2.] {
        let c = Collider::new(
            plane.clone(),
            Pose3 { pos: [0., 0., 5.], ..Default::default() },
            [0., 0., speed],
            [0.; 3],
            0.,
        )
        .unwrap();
        let hit = c.sweep(0., 1., [0.; 3], [0., 0., 100.], 0.1).unwrap().unwrap();
        assert!((hit.fraction - 4.875 / (100. - speed)).abs() < 1e-6, "{hit:?}");
        assert!(hit.normal[2] < -0.99);
        assert_eq!(hit.velocity, [0., 0., speed]);
    }
    let rod = Collider::new(SharedShape::cuboid(2., 0.1, 0.1), Pose3::default(), [0.; 3], [0., 0., 90.], 0.).unwrap();
    let hit = rod.sweep(0., 1., [0., 1., 0.], [0., 1., 0.], 0.1).unwrap().unwrap();
    assert!((0.8..1.).contains(&hit.fraction), "{hit:?}");
    assert!(hit.velocity[0] < -1.);
    let inside = Collider::new(SharedShape::ball(1.), Pose3::default(), [0.; 3], [0.; 3], 0.).unwrap();
    let hit = inside.sweep(0., 0.1, [0.5, 0., 0.], [0.6, 0., 0.], 0.1).unwrap().unwrap();
    assert_eq!(hit.fraction, 0.);
    assert!(hit.position[0] >= 1.1 - 1e-8);
}

#[test]
fn constant_emission_counts_do_not_lose_particles_at_fractional_steps() {
    for step in [1. / 60., 1. / 24., 0.03, 0.07] {
        for rate in [1., 3., 7., 10., 17.] {
            let mut s = spec();
            s.step = step;
            s.lifetime = 100.;
            let mut e = Emitter::new(s).unwrap();
            let mut d = Scene { rate, ..Default::default() };
            for t in [1., 2., 5.] {
                assert_eq!(e.at(t, &mut d).unwrap().emitted, (rate * t) as u64, "step={step},rate={rate},time={t}");
            }
        }
    }
}

#[test]
fn particles_land_on_real_3d_colliders_and_follow_a_moving_surface() {
    use rapier3d_f64::parry::{math::Vector, shape::SharedShape};
    use sr_sim::{particles3d::collider::Collider, physics3d::Pose3};
    struct World {
        collider: Collider,
    }
    impl Driver for World {
        fn emission(&mut self, _: f64) -> Result<Emission, Error> {
            Ok(Emission::default())
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, t: f64, dt: f64, a: [f64; 3], b: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            self.collider.sweep(t, dt, a, b, r)
        }
    }
    let mesh = SharedShape::trimesh(
        vec![
            Vector::new(-10., 2., -10.),
            Vector::new(10., 2., -10.),
            Vector::new(10., 2., 10.),
            Vector::new(-10., 2., 10.),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    )
    .unwrap();
    let mut w = World { collider: Collider::new(mesh, Pose3::default(), [0., 0.1, 0.], [0.; 3], 0.).unwrap() };
    let mut s = spec();
    s.bursts = vec![burst(0., 1)];
    s.gravity = [0., 10., 0.];
    s.restitution = 0.;
    s.radius = 0.1;
    let mut e = Emitter::new(s).unwrap();
    let f = e.at(2., &mut w).unwrap();
    assert!((f.particles[0].position[1] - 2.1).abs() < 1e-5, "{f:?}");
    assert!((f.particles[0].velocity[1] - 0.1).abs() < 1e-8, "{f:?}");
}

#[test]
fn dissipative_micro_bounces_settle_within_collision_tolerance() {
    struct Floor(Scene);
    impl Driver for Floor {
        fn emission(&mut self, _: f64) -> Result<Emission, Error> {
            let mut m = sr_volume::Transform::identity().columns();
            m[13] = 1.9 - 1e-8;
            Ok(Emission { transform: sr_volume::Transform::new(m).unwrap(), ..Default::default() })
        }
        fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.; 3])
        }
        fn sweep(&mut self, t: f64, dt: f64, a: [f64; 3], b: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            self.0.sweep(t, dt, a, b, r)
        }
    }
    let mut spec = spec();
    spec.step = 1. / 24.;
    spec.radius = 0.1;
    spec.restitution = 0.15;
    spec.gravity = [0., 20., 0.];
    spec.bursts = vec![burst(0., 1)];
    let mut emitter = Emitter::new(spec).unwrap();
    let mut driver = Floor(Scene { wall: true, ..Default::default() });
    for time in [0.5, 1., 0.25, 1.] {
        let frame = emitter.at(time, &mut driver).unwrap();
        let p = &frame.particles[0];
        assert!((p.position[1] - 1.9).abs() <= 0.0011, "{:?}", p.position);
        assert!(p.velocity[1].abs() < 1e-8, "{:?}", p.velocity);
    }
}

#[test]
fn resolvable_rebounds_keep_their_authored_restitution() {
    for restitution in [0.15, 1.] {
        let mut spec = spec();
        spec.radius = 0.1;
        spec.collision_tolerance = 1e-8;
        spec.velocity = [0., 20., 0.];
        spec.gravity = [0., 20., 0.];
        spec.restitution = restitution;
        spec.bursts = vec![burst(0., 1)];
        let mut emitter = Emitter::new(spec).unwrap();
        let mut driver = Scene { wall: true, ..Default::default() };
        let frame = emitter.at(0.1, &mut driver).unwrap();
        // Solve y=20t+10t²=1.9 independently, then integrate the rebound.
        let impact_time = ((476f64).sqrt() - 20.) / 20.;
        let incoming = 20. + 20. * impact_time;
        let remaining = 0.1 - impact_time;
        let expected_velocity = -restitution * incoming + 20. * remaining;
        let expected_y = 1.9 - restitution * incoming * remaining + 10. * remaining * remaining;
        let p = &frame.particles[0];
        assert!((p.velocity[1] - expected_velocity).abs() < 1e-6, "{restitution}: {:?}", p.velocity);
        assert!((p.position[1] - expected_y).abs() < 1e-6, "{restitution}: {:?}", p.position);
    }
}
