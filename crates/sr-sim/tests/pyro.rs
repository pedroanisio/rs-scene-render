use sr_sim::pyro::{Boundary, Inputs, Obstacle, Shape, Simulation, Source, Spec};

#[test]
fn checkpointed_seeking_matches_fresh_replay_with_a_tight_memory_budget() {
    use sr_sim::pyro::Timeline;
    let mut s = spec();
    s.turbulence = 0.2;
    s.seed = 17;
    let input = |_: u64, _: f64| {
        Ok(Inputs {
            sources: vec![Source {
                start: 0.15,
                end: Some(0.35),
                density_rate: 4.0,
                ..source(Shape::Sphere { center: [4.0; 3], radius: 2.0 })
            }],
            ..Inputs::default()
        })
    };
    let one = Simulation::new(s.clone()).unwrap().state().bytes();
    assert!(Timeline::new(s.clone(), one - 1).is_err());
    let mut timeline = Timeline::new(s.clone(), one * 2).unwrap();
    for time in [3.3, 0.3, 2.1, 0.0, 3.3, 0.3] {
        let got = timeline.at(time, &mut input.clone()).unwrap().clone();
        let mut fresh = Simulation::new(s.clone()).unwrap();
        for step in 0..(time * 10.0_f64).round() as u64 {
            fresh.step(&input(step, step as f64 * 0.1).unwrap()).unwrap();
        }
        assert_eq!(&got, fresh.state(), "seek at {time}");
        assert!(timeline.checkpoint_bytes() <= one * 2);
    }
    assert!(timeline.at(f64::NAN, &mut input.clone()).is_err());
    assert!(timeline.at(1e100, &mut input.clone()).is_err());
}

#[test]
fn confinement_changes_swirl_without_violating_the_pressure_constraint() {
    let mut s = spec();
    s.turbulence = 0.7;
    let mut unconstrained = Simulation::new(s.clone()).unwrap();
    s.vorticity = 3.0;
    let mut confined = Simulation::new(s).unwrap();
    for _ in 0..4 {
        unconstrained.step(&Inputs::default()).unwrap();
        let report = confined.step(&Inputs::default()).unwrap();
        assert!(report.divergence_after < 1e-7);
    }
    assert_ne!(confined.state(), unconstrained.state());
}

#[test]
fn moving_solid_faces_carry_the_prescribed_velocity() {
    let mut s = spec();
    s.boundary = Boundary::Open;
    let mut sim = Simulation::new(s).unwrap();
    let report = sim
        .step(&Inputs {
            obstacles: vec![Obstacle {
                velocity: [0.2, -0.3, 0.4],
                ..Obstacle::stationary(Shape::Box { min: [3.0; 3], max: [5.0; 3] })
            }],
            ..Inputs::default()
        })
        .unwrap();
    assert!(report.divergence_after < 1e-7);
    assert_eq!(sim.state().velocity_at([3.0, 3.5, 3.5])[0], 0.2);
    assert_eq!(sim.state().velocity_at([3.5, 3.0, 3.5])[1], -0.3);
    assert_eq!(sim.state().velocity_at([3.5, 3.5, 3.0])[2], 0.4);
}

#[test]
fn affine_solid_motion_prescribes_rotation_and_scaling_at_faces() {
    let mut s = spec();
    s.boundary = Boundary::Open;
    let mut sim = Simulation::new(s).unwrap();
    let report = sim
        .step(&Inputs {
            obstacles: vec![Obstacle {
                velocity: [0.2, -0.3, 0.4],
                velocity_origin: [4.0; 3],
                velocity_gradient: [[0.1, -0.6, 0.0], [0.6, 0.2, 0.0], [0.0, 0.0, -0.15]],
                ..Obstacle::stationary(Shape::Box { min: [3.0; 3], max: [5.0; 3] })
            }],
            ..Inputs::default()
        })
        .unwrap();
    assert!(report.divergence_after < 1e-7);
    for (p, axis, expected) in [
        ([3.0, 3.5, 3.5], 0, 0.4),
        ([3.5, 3.0, 3.5], 1, -0.8),
        ([3.5, 3.5, 3.0], 2, 0.55),
        ([4.0, 3.5, 3.5], 0, 0.5),
        ([5.0, 4.5, 4.5], 0, 0.0),
    ] {
        let got = sim.state().velocity_at(p)[axis];
        assert!((got - expected).abs() < 1e-12, "{p:?} axis {axis}: {got} != {expected}");
    }
}

#[test]
fn invalid_affine_collider_motion_is_atomic() {
    let mut sim = Simulation::new(spec()).unwrap();
    let before = sim.state().clone();
    for invalid in [f64::INFINITY, f64::NAN, f64::MAX] {
        let mut obstacle = Obstacle::stationary(whole());
        obstacle.velocity_origin = [4.0; 3];
        obstacle.velocity_gradient[0][1] = invalid;
        assert!(sim.step(&Inputs { obstacles: vec![obstacle], ..Inputs::default() }).is_err());
        assert_eq!(sim.state(), &before);
        assert_eq!(sim.time(), 0.0);
    }
}

#[test]
fn state_dependent_inputs_replay_from_checkpoint_velocity() {
    use sr_sim::pyro::Timeline;
    let mut s = spec();
    s.boundary = Boundary::Open;
    let mut timeline = Timeline::new(s, 1 << 20).unwrap();
    let mut input = |step: u64, _: f64, state: &sr_sim::pyro::State| {
        let vx = state.velocity_at([4.0; 3])[0];
        if step < 2 {
            assert!((vx - step as f64 * 0.1).abs() < 1e-12);
        }
        Ok(Inputs { acceleration: [1.0 - vx, 0.0, 0.0], ..Inputs::default() })
    };
    let first = timeline.at_with_state(0.2, &mut input).unwrap().clone();
    // Open-boundary advection slightly reduces the otherwise uniform 0.19.
    assert!((first.velocity_at([4.0; 3])[0] - 0.19).abs() < 1e-4);
    timeline.at_with_state(1.0, &mut input).unwrap();
    assert_eq!(timeline.at_with_state(0.2, &mut input).unwrap(), &first);
}

#[test]
fn curved_collider_primitives_preserve_holes_caps_and_taper() {
    for (shape, inside, outside) in [
        (Shape::Cylinder { radius: 1.0, half_height: 3.0 }, [0.5, 2.5, 0.5], [0.5, 3.5, 0.5]),
        (Shape::Cone { radius: 3.0, half_height: 3.0 }, [2.5, 2.5, 0.5], [0.5, -2.5, 0.5]),
        (Shape::Capsule { radius: 1.0, half_segment: 2.0 }, [0.5, 2.5, 0.5], [0.5, 3.5, 0.5]),
        (Shape::Torus { major_radius: 2.0, minor_radius: 1.0 }, [2.5, 0.5, 0.5], [0.5, 0.5, 0.5]),
    ] {
        let mut s = spec();
        s.origin = [-4.0; 3];
        let mut sim = Simulation::new(s).unwrap();
        sim.step(&Inputs {
            sources: vec![Source { density_rate: 10.0, ..source(Shape::Box { min: [-4.0; 3], max: [4.0; 3] }) }],
            obstacles: vec![Obstacle::stationary(shape)],
            ..Inputs::default()
        })
        .unwrap();
        let grid = sim.state().volume(1 << 20).unwrap();
        assert_eq!(grid.grid("density").unwrap().sample_world(inside), 0.0);
        assert_eq!(grid.grid("density").unwrap().sample_world(outside), 1.0);
    }
}

fn spec() -> Spec {
    Spec { cells: [8, 8, 8], dt: 0.1, pressure_iterations: 500, pressure_tolerance: 1e-8, ..Spec::default() }
}

fn whole() -> Shape {
    Shape::Box { min: [0.0; 3], max: [8.0; 3] }
}

#[test]
fn impulses_fire_once_at_the_half_open_step_boundary_and_replay_identically() {
    use sr_sim::pyro::{Impulse, Timeline};
    let mut timeline = Timeline::new(spec(), 1 << 20).unwrap();
    let mut input = |_: u64, _: f64| {
        Ok(Inputs {
            impulses: vec![Impulse {
                time: 0.3,
                shape: whole(),
                density: 2.0,
                temperature: 500.0,
                velocity: [0.0; 3],
                expansion: 0.0,
            }],
            ..Inputs::default()
        })
    };
    assert!(timeline.at(0.3, &mut input).unwrap().density().iter().all(|&v| v == 0.0));
    let later = timeline.at(0.8, &mut input).unwrap().clone();
    assert!(later.density().iter().all(|&v| v == 2.0));
    assert!(later.temperature().iter().all(|&v| v == 800.0));
    assert_eq!(&later, timeline.at(0.4, &mut input).unwrap());
    timeline.at(0.0, &mut input).unwrap();
    assert_eq!(&later, timeline.at(0.8, &mut input).unwrap());
}

#[test]
fn transformed_source_uses_local_shape_and_spatial_forces_act_in_three_dimensions() {
    use sr_volume::Transform;
    let shape = Shape::Transformed {
        shape: Box::new(Shape::Sphere { center: [0.0; 3], radius: 1.0 }),
        transform: Box::new(
            Transform::new([3.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 4.0, 4.0, 1.0]).unwrap(),
        ),
    };
    let mut sim = Simulation::new(spec()).unwrap();
    sim.step(&Inputs {
        sources: vec![Source { density_rate: 1.0, ..source(shape) }],
        spatial_acceleration: (0..512)
            .map(|k| {
                let x = (k % 8) as f64 - 3.5;
                let z = (k / 64) as f64 - 3.5;
                [z * 0.1, 0.0, -x * 0.1]
            })
            .collect(),
        ..Inputs::default()
    })
    .unwrap();
    let at = |x: usize, y: usize, z: usize| sim.state().density()[(z * 8 + y) * 8 + x];
    assert!(at(2, 3, 3) > 0.0);
    assert_eq!(at(3, 2, 3), 0.0);
    assert!(sim.state().velocity_at([4.0, 4.0, 6.0])[0] > 0.0);
    assert!(sim.state().velocity_at([6.0, 4.0, 4.0])[2] < 0.0);
    let old = sim.state().clone();
    assert!(sim.step(&Inputs { spatial_acceleration: vec![[0.0; 3]; 1], ..Inputs::default() }).is_err());
    assert_eq!(sim.state(), &old);
}

fn source(shape: Shape) -> Source {
    Source { shape, ..Source::default() }
}

#[test]
fn source_windows_integrate_partial_steps_and_cooling_is_exponential() {
    let mut sim = Simulation::new(spec()).unwrap();
    let input = Inputs {
        sources: vec![Source {
            start: 0.15,
            end: Some(0.25),
            density_rate: 10.0,
            temperature_rate: 1000.0,
            ..source(whole())
        }],
        ..Inputs::default()
    };
    sim.step(&input).unwrap();
    assert!(sim.state().density().iter().all(|&v| v == 0.0));
    sim.step(&input).unwrap();
    assert!(sim.state().density().iter().all(|&v| (v - 0.5).abs() < 1e-12));
    sim.step(&input).unwrap();
    assert!(sim.state().density().iter().all(|&v| (v - 1.0).abs() < 1e-12));
    assert!(sim.state().temperature().iter().all(|&v| (v - 400.0).abs() < 1e-10));
    sim.step(&input).unwrap();
    assert!(sim.state().density().iter().all(|&v| (v - 1.0).abs() < 1e-12));

    let mut s = spec();
    s.dissipation = 2.0;
    s.cooling = 3.0;
    let mut sim = Simulation::new(s).unwrap();
    sim.step(&Inputs {
        sources: vec![Source { density_rate: 10.0, temperature_rate: 1000.0, ..source(whole()) }],
        ..Inputs::default()
    })
    .unwrap();
    sim.step(&Inputs::default()).unwrap();
    assert!((sim.state().density()[0] - (-0.2_f64).exp()).abs() < 1e-10);
    assert!((sim.state().temperature()[0] - 300.0 - 100.0 * (-0.3_f64).exp()).abs() < 1e-10);
}

#[test]
fn projection_reduces_three_dimensional_divergence_and_preserves_walls() {
    let mut sim = Simulation::new(spec()).unwrap();
    let report = sim
        .step(&Inputs {
            sources: vec![Source {
                velocity_rate: [7.0, -11.0, 19.0],
                ..source(Shape::Sphere { center: [4.0; 3], radius: 2.1 })
            }],
            ..Inputs::default()
        })
        .unwrap();
    assert!(report.divergence_before > 0.01, "{report:?}");
    assert!(report.divergence_after < 1e-7, "{report:?}");
    for axis in 0..3 {
        for edge in [0.0, 8.0] {
            let mut p = [4.0; 3];
            p[axis] = edge;
            assert_eq!(sim.state().velocity_at(p)[axis], 0.0);
        }
    }
    assert!(sim.state().velocity_at([4.0; 3])[2].abs() > 0.01);
}

#[test]
fn open_domain_expansion_is_projected_to_the_authored_divergence() {
    let mut s = spec();
    s.boundary = Boundary::Open;
    let mut sim = Simulation::new(s).unwrap();
    let report =
        sim.step(&Inputs { sources: vec![Source { expansion: 0.5, ..source(whole()) }], ..Inputs::default() }).unwrap();
    assert!(report.divergence_after < 1e-7, "{report:?}");
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                assert!((sim.state().divergence_at([x, y, z]) - 0.5).abs() < 1e-7);
            }
        }
    }
    let mut closed = Simulation::new(spec()).unwrap();
    assert!(
        closed
            .step(&Inputs { sources: vec![Source { expansion: 0.5, ..source(whole()) }], ..Inputs::default() })
            .is_err(),
        "a sealed domain cannot sustain positive net expansion"
    );
    assert_eq!(closed.time(), 0.0, "failed steps are atomic");
}

#[test]
fn obstacle_wall_prevents_smoke_crossing_and_resets_covered_cells() {
    let mut s = spec();
    s.boundary = Boundary::Open;
    let mut sim = Simulation::new(s).unwrap();
    let input = Inputs {
        sources: vec![Source {
            density_rate: 5.0,
            velocity_rate: [40.0, 0.0, 0.0],
            ..source(Shape::Box { min: [0.0; 3], max: [3.0, 8.0, 8.0] })
        }],
        obstacles: vec![Obstacle::stationary(Shape::Box { min: [3.0, 0.0, 0.0], max: [5.0, 8.0, 8.0] })],
        ..Inputs::default()
    };
    for _ in 0..10 {
        sim.step(&input).unwrap();
    }
    assert!(sim.state().density().iter().any(|&v| v > 0.1));
    for z in 0..8 {
        for y in 0..8 {
            for x in 3..8 {
                assert_eq!(sim.state().density()[(z * 8 + y) * 8 + x], 0.0);
            }
            assert_eq!(sim.state().velocity_at([3.0, y as f64 + 0.5, z as f64 + 0.5])[0], 0.0);
        }
    }
    sim.step(&Inputs { obstacles: vec![Obstacle::stationary(whole())], ..Inputs::default() }).unwrap();
    assert!(sim.state().density().iter().all(|&v| v == 0.0));
}

#[test]
fn buoyancy_moves_hot_smoke_up_and_seeded_turbulence_is_repeatable() {
    let mut s = spec();
    s.boundary = Boundary::Open;
    s.buoyancy = 0.05;
    s.turbulence = 0.1;
    s.seed = 42;
    let input = Inputs {
        sources: vec![Source {
            density_rate: 1.0,
            temperature_rate: 1000.0,
            ..source(Shape::Sphere { center: [4.0; 3], radius: 1.5 })
        }],
        ..Inputs::default()
    };
    let mut a = Simulation::new(s.clone()).unwrap();
    let mut b = Simulation::new(s.clone()).unwrap();
    s.seed = 43;
    let mut c = Simulation::new(s).unwrap();
    for _ in 0..5 {
        a.step(&input).unwrap();
        b.step(&input).unwrap();
        c.step(&input).unwrap();
    }
    assert_eq!(a.state(), b.state());
    assert_ne!(a.state(), c.state());
    assert!(a.state().velocity_at([4.0; 3])[1] < -0.1);
}

#[test]
fn volume_export_preserves_cell_centres_temperature_and_all_velocity_components() {
    let mut s = spec();
    s.origin = [-20.0, 10.0, 30.0];
    s.voxel_size = 2.0;
    s.boundary = Boundary::Open;
    let mut sim = Simulation::new(s).unwrap();
    sim.step(&Inputs {
        sources: vec![Source {
            density_rate: 3.0,
            temperature_rate: 1000.0,
            velocity_rate: [2.0, 3.0, 4.0],
            ..source(Shape::Box { min: [-20.0, 10.0, 30.0], max: [-4.0, 26.0, 46.0] })
        }],
        ..Inputs::default()
    })
    .unwrap();
    let volume = sim.state().volume(1 << 20).unwrap();
    let p = [-19.0, 11.0, 31.0];
    assert!((volume.grid("density").unwrap().sample_world(p) - 0.3).abs() < 1e-6);
    assert_eq!(volume.grid("temperature").unwrap().sample_world(p), 400.0);
    let velocity = sim.state().velocity_at(p);
    for (axis, name) in ["velocity.x", "velocity.y", "velocity.z"].iter().enumerate() {
        assert!((f64::from(volume.grid(name).unwrap().sample_world(p)) - velocity[axis]).abs() < 1e-6);
    }
    assert!(sim.state().volume(100).is_err());
}

#[test]
fn invalid_inputs_and_exhausted_budgets_fail_before_changing_state() {
    for s in [
        Spec { cells: [usize::MAX; 3], ..spec() },
        Spec { max_bytes: 100, ..spec() },
        Spec { dt: f64::NAN, ..spec() },
        Spec { voxel_size: 0.0, ..spec() },
        Spec { ambient_temperature: -1.0, ..spec() },
    ] {
        assert!(Simulation::new(s).is_err());
    }
    let mut sim = Simulation::new(spec()).unwrap();
    let old = sim.state().clone();
    for input in [
        Inputs { acceleration: [f64::INFINITY, 0.0, 0.0], ..Inputs::default() },
        Inputs { sources: vec![Source { density_rate: -1.0, ..source(whole()) }], ..Inputs::default() },
        Inputs { sources: vec![source(Shape::Sphere { center: [0.0; 3], radius: -1.0 })], ..Inputs::default() },
        Inputs { sources: vec![Source { temperature_rate: f64::MAX, ..source(whole()) }], ..Inputs::default() },
    ] {
        assert!(sim.step(&input).is_err());
        assert_eq!(sim.time(), 0.0);
        assert_eq!(sim.state(), &old);
    }
}

#[test]
fn the_step_workspace_budget_is_320_bytes_per_cell_plus_a_fixed_overhead() {
    let exact = 16usize.pow(3) * 320 + 8192;
    let with = |max_bytes| Simulation::new(Spec { cells: [16; 3], max_bytes, ..spec() });
    assert!(with(exact).is_ok());
    assert!(with(exact - 1).is_err());
}
