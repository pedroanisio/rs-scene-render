use sr_sim::ocean::{
    whitewater::{Kind, Settings, Whitewater},
    Boundary, Cell, Error, Frame, Spec,
};

fn grid() -> Spec {
    Spec { cells: [8, 4], dt: 0.1, boundary: Boundary::Periodic, ..Default::default() }
}
fn water(time: f64, velocity: [f64; 2]) -> Result<Frame, Error> {
    Ok(Frame { time, cells: vec![Cell { depth: 2., velocity }; 32], bed: vec![] })
}
fn settings() -> Settings {
    Settings { rate: 20., threshold: 0.1, lifetime: 2., spray_fraction: 0., ..Default::default() }
}

#[test]
fn calm_water_is_empty_and_moving_water_emits_only_at_canonical_ticks() {
    let mut calm = Whitewater::new(grid(), vec![2.; 32], settings()).unwrap();
    assert!(calm.at(1., |t| water(t, [0.; 2])).unwrap().particles.is_empty());
    let mut active = Whitewater::new(grid(), vec![2.; 32], settings()).unwrap();
    assert!(active.at(0.09, |t| water(t, [2., 0.])).unwrap().particles.is_empty());
    let f = active.at(0.1, |t| water(t, [2., 0.])).unwrap();
    assert!(!f.particles.is_empty());
    assert!(f.particles.iter().all(|p| p.kind == Kind::Foam && p.birth == 0.1 && p.position[1] < 0.));
}

#[test]
fn foam_advects_wraps_and_dies_without_modifying_source_water() {
    let cfg = Settings { end: Some(0.11), lifetime: 0.8, ..settings() };
    let mut sim = Whitewater::new(grid(), vec![2.; 32], cfg).unwrap();
    let born = sim.at(0.1, |t| water(t, [2., 0.])).unwrap().clone();
    let later = sim.at(0.6, |t| water(t, [2., 0.])).unwrap();
    assert_eq!(later.particles.len(), born.particles.len());
    for (a, b) in born.particles.iter().zip(&later.particles) {
        assert_eq!(a.id, b.id);
        assert!((b.position[0] - (a.position[0] + 1.).rem_euclid(8.)).abs() < 1e-12);
        assert_eq!(a.position[1], b.position[1]);
    }
    assert!(sim.at(1., |t| water(t, [2., 0.])).unwrap().particles.is_empty());
}

#[test]
fn spray_flies_ballistically_then_becomes_surface_foam() {
    let cfg = Settings { end: Some(0.11), spray_fraction: 1., launch_speed: 2., drag: 0., lifetime: 3., ..settings() };
    let mut sim = Whitewater::new(grid(), vec![2.; 32], cfg).unwrap();
    let born = sim.at(0.1, |t| water(t, [2., 0.])).unwrap().clone();
    let aloft = sim.at(0.2, |t| water(t, [2., 0.])).unwrap().clone();
    assert!(!aloft.particles.is_empty());
    for (a, b) in born.particles.iter().zip(&aloft.particles) {
        assert_eq!(b.kind, Kind::Spray);
        assert!((b.position[1] - (a.position[1] + a.velocity[1] * 0.1 + 0.5 * 9.81 * 0.01)).abs() < 1e-12);
    }
    assert!(sim.at(1., |t| water(t, [2., 0.])).unwrap().particles.iter().all(|p| p.kind == Kind::Foam));
}

#[test]
fn replay_fractional_queries_and_all_seed_bits_are_deterministic() {
    let make = |seed| Whitewater::new(grid(), vec![2.; 32], Settings { seed, ..settings() }).unwrap();
    let mut a = make(9_007_199_254_740_992);
    let expected = a.at(0.85, |t| water(t, [2., 0.])).unwrap().clone();
    a.at(0.23, |t| water(t, [2., 0.])).unwrap();
    a.at(0.71, |t| water(t, [2., 0.])).unwrap();
    assert_eq!(a.at(0.85, |t| water(t, [2., 0.])).unwrap(), &expected);
    assert_eq!(make(9_007_199_254_740_992).at(0.85, |t| water(t, [2., 0.])).unwrap(), &expected);
    assert_ne!(make(9_007_199_254_740_993).at(0.85, |t| water(t, [2., 0.])).unwrap(), &expected);
}

#[test]
fn limits_and_bad_source_samples_fail_atomically() {
    let mut sim = Whitewater::new(grid(), vec![2.; 32], Settings { max_particles: 60, ..settings() }).unwrap();
    let initial = sim.at(0., |t| water(t, [2., 0.])).unwrap().clone();
    assert!(sim.at(1., |t| water(t, [2., 0.])).unwrap_err().to_string().contains("particle"));
    assert_eq!(sim.frame(), &initial);
    let good = sim.at(0.1, |t| water(t, [2., 0.])).unwrap().clone();
    assert!(sim.at(0.2, |_| Ok(Frame { time: 0., cells: vec![], bed: vec![] })).is_err());
    assert_eq!(sim.frame(), &good);
    let mut limited = Whitewater::new(grid(), vec![2.; 32], Settings { max_work: 1, ..settings() }).unwrap();
    assert!(limited.at(1., |t| water(t, [2., 0.])).unwrap_err().to_string().contains("work"));
    assert!(Whitewater::new(grid(), vec![2.; 32], Settings { max_bytes: 100, ..settings() }).is_err());
    assert!(Whitewater::new(grid(), vec![2.; 32], Settings { rate: f64::INFINITY, ..settings() }).is_err());
}

#[test]
fn slope_emission_dry_land_open_boundaries_and_emission_windows() {
    let source = |t| {
        let mut f = water(t, [0.; 2])?;
        for (i, c) in f.cells.iter_mut().enumerate() {
            c.depth = if i % 8 < 4 { 1. } else { 3. };
        }
        Ok(f)
    };
    let cfg = Settings { start: 0.2, end: Some(0.3), ..settings() };
    let mut slope = Whitewater::new(grid(), vec![3.; 32], cfg).unwrap();
    assert!(slope.at(0.1, source).unwrap().particles.is_empty());
    let n = slope.at(0.2, source).unwrap().particles.len();
    assert!(n > 0);
    assert_eq!(slope.at(0.8, source).unwrap().particles.len(), n);
    let mut dry = Whitewater::new(grid(), vec![0.; 32], settings()).unwrap();
    assert!(dry
        .at(1., |t| Ok(Frame { time: t, cells: vec![Cell::default(); 32], bed: vec![] }))
        .unwrap()
        .particles
        .is_empty());
    let mut open = Whitewater::new(
        Spec { boundary: Boundary::Open, ..grid() },
        vec![2.; 32],
        Settings { end: Some(0.11), lifetime: 10., ..settings() },
    )
    .unwrap();
    assert!(!open.at(0.1, |t| water(t, [2., 0.])).unwrap().particles.is_empty());
    assert!(open.at(5., |t| water(t, [2., 0.])).unwrap().particles.is_empty());
}

#[test]
fn activity_overflow_is_reported_and_retained_bed_capacity_is_budgeted() {
    let cfg = Settings { rate: 0., ..settings() };
    let mut sim = Whitewater::new(grid(), vec![2.; 32], cfg).unwrap();
    let bad =
        |t| Ok(Frame { time: t, cells: vec![Cell { depth: f64::MAX, velocity: [f64::MAX; 2] }; 32], bed: vec![] });
    assert!(sim.at(0.1, bad).is_err(), "source velocity/depth must be numerically usable, even with emission disabled");
    let mut bed = Vec::with_capacity(100_000);
    bed.extend([2.; 32]);
    assert!(Whitewater::new(grid(), bed, Settings { max_particles: 1, max_bytes: 10_000, ..settings() }).is_err());
}

/// Foam is born on the surface, which sits at the bed minus the depth: a water
/// sample that carries its own bed moves the surface with it.
#[test]
fn foam_follows_the_bed_each_water_sample_carries() {
    let born_at = |shift: f64| {
        let mut sim = Whitewater::new(grid(), vec![2.; 32], settings()).unwrap();
        let frame = sim.at(0.1, |t| Ok(Frame { bed: vec![2. + shift; 32], ..water(t, [2., 0.])? })).unwrap();
        frame.particles.iter().map(|p| p.position[1]).collect::<Vec<_>>()
    };
    let (level, lowered) = (born_at(0.), born_at(3.));
    assert!(!level.is_empty() && level.len() == lowered.len());
    for (a, b) in level.iter().zip(&lowered) {
        assert!((b - a - 3.).abs() < 1e-12, "surface at {a} moved to {b}");
    }
    // A sample whose bed has the wrong length is rejected.
    let mut sim = Whitewater::new(grid(), vec![2.; 32], settings()).unwrap();
    assert!(sim.at(0.1, |t| Ok(Frame { bed: vec![2.; 5], ..water(t, [2., 0.])? })).is_err());
}

fn with_checkpoints(bytes: usize) -> Settings {
    Settings {
        checkpoint_bytes: bytes,
        rate: 20.,
        threshold: 0.1,
        lifetime: 1.5,
        spray_fraction: 0.5,
        ..Default::default()
    }
}
/// Moving water whose speed changes with time, so that history matters.
fn varying(t: f64) -> Result<Frame, Error> {
    water(t, [2. + (3. * t).sin(), 0.5 * (2. * t).cos()])
}

#[test]
fn replay_is_bit_identical_with_and_without_checkpoints_in_any_seek_order() {
    let mut reference = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(0)).unwrap();
    let times = [3.37, 0.2, 2.0, 5.51, 1.04, 4.4, 0.0, 2.95, 5.0];
    let expected: Vec<_> = times.iter().map(|&t| reference.at(t, varying).unwrap().clone()).collect();
    for budget in [0, 2_000, 6_000, 64 << 20] {
        let mut sim = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(budget)).unwrap();
        // Forward to lay checkpoints, then seek in the scrambled order twice.
        sim.at(6.0, varying).unwrap();
        for round in 0..2 {
            for (t, want) in times.iter().zip(&expected) {
                assert_eq!(sim.at(*t, varying).unwrap(), want, "budget {budget}, round {round}, t={t}");
                assert!(sim.checkpoint_bytes() <= budget, "budget {budget}: {} bytes held", sim.checkpoint_bytes());
            }
        }
    }
}

#[test]
fn a_backward_seek_restarts_from_the_nearest_checkpoint_and_not_from_zero() {
    let calls = std::cell::Cell::new(0_u32);
    let source = |t: f64| {
        calls.set(calls.get() + 1);
        varying(t)
    };
    let mut sim = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(64 << 20)).unwrap();
    sim.at(5.0, source).unwrap();
    // The step is 0.1 s and a checkpoint is kept every second: ticks 10, 20, ...
    calls.set(0);
    sim.at(3.35, source).unwrap();
    // From tick 30 to tick 33, then the fractional 0.05 s: four samples, not 34.
    assert_eq!(calls.get(), 4, "{} source samples", calls.get());
    calls.set(0);
    let mut cold = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(0)).unwrap();
    cold.at(5.0, source).unwrap();
    calls.set(0);
    cold.at(3.35, source).unwrap();
    assert_eq!(calls.get(), 34, "without checkpoints the replay starts from zero");
}

#[test]
fn a_full_budget_thins_checkpoints_instead_of_failing_and_never_exceeds_it() {
    let one = {
        let mut probe = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(64 << 20)).unwrap();
        probe.at(1.0, varying).unwrap();
        probe.checkpoint_bytes()
    };
    assert!(one > 0);
    // Room for about two states while the run needs sixteen seconds of them.
    let budget = 2 * one + one / 2;
    let mut sim = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(budget)).unwrap();
    sim.at(16.0, varying).unwrap();
    assert!(sim.checkpoint_bytes() <= budget);
    assert!(!sim.checkpoint_ticks().is_empty(), "thinning must keep some checkpoint");
    let ticks = sim.checkpoint_ticks();
    assert!(ticks.windows(2).all(|w| w[1] > w[0]));
    // Kept ticks are evenly spaced after thinning.
    let gap = ticks[1] - ticks[0];
    assert!(ticks.windows(2).all(|w| w[1] - w[0] == gap), "{ticks:?}");
}

#[test]
fn a_failed_request_leaves_the_published_frame_and_the_canonical_state_alone() {
    let mut sim = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(64 << 20)).unwrap();
    let before = sim.at(2.0, varying).unwrap().clone();
    let failing = |t: f64| if t > 3.0 { Err(Error::Invalid("source refused")) } else { varying(t) };
    assert!(sim.at(5.0, failing).is_err());
    assert_eq!(sim.frame(), &before);
    // The retry from the retained state matches a clean run.
    let mut clean = Whitewater::new(grid(), vec![2.; 32], with_checkpoints(0)).unwrap();
    assert_eq!(sim.at(4.0, varying).unwrap(), clean.at(4.0, varying).unwrap());
}
