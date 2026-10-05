use sr_sim::{fields::Field, physics3d::*};

struct Surface {
    invalid: bool,
    budget: usize,
    calls: Vec<Option<u64>>,
}
impl Driver3 for Surface {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn collider(&mut self, t: f64, _: usize, revision: Option<u64>) -> Result<Option<ColliderUpdate3>, String> {
        self.calls.push(revision);
        let next = t.to_bits();
        if revision == Some(next) {
            return Ok(None);
        }
        Ok(Some(ColliderUpdate3 {
            revision: next,
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            triangles: vec![[0, 1, if self.invalid { 9 } else { 2 }]],
            max_bytes: self.budget,
        }))
    }
}
fn world(kind: BodyKind) -> World3 {
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![Body3Spec {
            kind,
            shape: Shape3::Box([1.; 3]),
            mass: 1.,
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
        }],
    })
}

#[test]
fn collider_failure_has_no_poses_and_can_retry_without_advancing() {
    let mut world = world(BodyKind::Static);
    let mut driver = Surface { invalid: true, budget: 1 << 20, calls: vec![] };
    let failed = world.frame_at(0.5, &mut driver);
    assert!(failed.bodies.is_empty());
    assert!(failed.errors[0].contains("invalid"));
    assert_eq!(world.progress().0, 0);
    driver.invalid = false;
    let repaired = world.frame_at(0.5, &mut driver);
    assert!(repaired.errors.is_empty());
    assert_eq!(repaired.bodies.len(), 1);
    assert_eq!(world.progress().0, 50);
}

#[test]
fn collider_budget_and_dynamic_body_rejections_are_explicit() {
    for (kind, budget, diagnostic) in
        [(BodyKind::Static, 1, "budget"), (BodyKind::Dynamic, 1 << 20, "static or kinematic")]
    {
        let mut world = world(kind);
        let mut driver = Surface { invalid: false, budget, calls: vec![] };
        let failed = world.frame_at(1., &mut driver);
        assert!(failed.bodies.is_empty());
        assert!(failed.errors[0].contains(diagnostic), "{:?}", failed.errors);
        assert_eq!(world.progress().0, 0);
    }
}

#[test]
fn collider_revision_restores_with_checkpoint() {
    // the replay path, which the frame memory would otherwise bypass
    let mut world = world(BodyKind::Static).with_frame_log_budget(0);
    let mut driver = Surface { invalid: false, budget: 1 << 20, calls: vec![] };
    assert!(world.frame_at(2., &mut driver).errors.is_empty());
    driver.calls.clear();
    assert!(world.frame_at(1., &mut driver).errors.is_empty());
    assert_eq!(driver.calls, vec![Some(1_f64.to_bits())]);
    driver.calls.clear();
    assert!(world.frame_at(0., &mut driver).errors.is_empty());
    assert_eq!(driver.calls, vec![None]);
}

#[test]
fn an_earlier_time_already_simulated_asks_the_surface_for_nothing() {
    let mut world = world(BodyKind::Static);
    let mut driver = Surface { invalid: false, budget: 1 << 20, calls: vec![] };
    let late = world.frame_at(2., &mut driver);
    driver.calls.clear();
    assert!(world.frame_at(1., &mut driver).errors.is_empty());
    assert!(world.frame_at(0., &mut driver).errors.is_empty());
    assert!(driver.calls.is_empty(), "{:?}", driver.calls);
    assert_eq!(world.frame_at(2., &mut driver), late);
}

#[test]
fn changing_surfaces_do_not_retain_an_unbounded_checkpoint_per_second() {
    let mut world = world(BodyKind::Static);
    let mut driver = Surface { invalid: false, budget: 1 << 20, calls: vec![] };
    assert!(world.frame_at(70., &mut driver).errors.is_empty());
    assert!(world.progress().1 <= 65, "retained {} checkpoints", world.progress().1);
    driver.calls.clear();
    assert!(world.frame_at(1., &mut driver).errors.is_empty());
    assert!(world.frame_at(70., &mut driver).errors.is_empty());
    assert!(world.progress().1 <= 65);
    let mut world = world.with_checkpoint_budget(128 << 10);
    assert!(world.frame_at(71., &mut driver).errors.is_empty());
    assert_eq!(world.progress().1, 2, "resetting the cache restores its initial sampling cadence");
}

#[test]
fn checkpoint_admission_evicts_before_cloning_and_zero_budget_replays() {
    for budget in [0, 48 << 10, 128 << 10] {
        let mut world = world(BodyKind::Static).with_checkpoint_budget(budget);
        let mut driver = Surface { invalid: false, budget: 1 << 20, calls: vec![] };
        let expected = world.frame_at(12., &mut driver);
        assert!(expected.errors.is_empty());
        assert!(world.checkpoint_bytes() <= budget);
        if budget == 0 {
            assert_eq!(world.progress().1, 1);
        }
        for t in [1., 9., 0., 6.] {
            assert!(world.frame_at(t, &mut driver).errors.is_empty());
            assert!(world.checkpoint_bytes() <= budget);
        }
        assert_eq!(world.frame_at(12., &mut driver), expected);
        let world = world.with_checkpoint_budget(0);
        assert_eq!(world.checkpoint_bytes(), 0);
        assert_eq!(world.progress().1, 1);
    }
}

#[test]
fn checkpoint_admission_counts_triangle_geometry_not_just_body_count() {
    struct Dense(Surface);
    impl Driver3 for Dense {
        fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
            self.0.kinematic(t, which)
        }
        fn fields(&mut self, t: f64) -> Vec<Field> {
            self.0.fields(t)
        }
        fn collider(&mut self, t: f64, which: usize, revision: Option<u64>) -> Result<Option<ColliderUpdate3>, String> {
            let mut update = self.0.collider(t, which, revision)?;
            if let Some(update) = &mut update {
                update.triangles = vec![[0, 1, 2]; 512];
            }
            Ok(update)
        }
    }
    let budget = 64 << 10;
    let mut small = world(BodyKind::Static).with_checkpoint_budget(budget);
    let mut large = world(BodyKind::Static).with_checkpoint_budget(budget);
    let mut small_driver = Surface { invalid: false, budget: 1 << 20, calls: vec![] };
    let mut large_driver = Dense(Surface { invalid: false, budget: 1 << 20, calls: vec![] });
    assert!(small.frame_at(1., &mut small_driver).errors.is_empty());
    assert!(large.frame_at(1., &mut large_driver).errors.is_empty());
    assert_eq!(small.progress().1, 2, "small geometry fits the optional budget");
    assert_eq!(large.progress().1, 1, "dense surface exceeds it and must not be cloned");
    assert_eq!(large.checkpoint_bytes(), 0);
}
