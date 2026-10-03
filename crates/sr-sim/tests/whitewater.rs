use sr_sim::ocean::{
    whitewater::{Kind, Settings, Whitewater},
    Boundary, Cell, Error, Frame, Spec,
};

fn grid() -> Spec {
    Spec { cells: [8, 4], dt: 0.1, boundary: Boundary::Periodic, ..Default::default() }
}
fn water(time: f64, velocity: [f64; 2]) -> Result<Frame, Error> {
    Ok(Frame { time, cells: vec![Cell { depth: 2., velocity }; 32] })
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
    assert!(sim.at(0.2, |_| Ok(Frame { time: 0., cells: vec![] })).is_err());
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
    assert!(dry.at(1., |t| Ok(Frame { time: t, cells: vec![Cell::default(); 32] })).unwrap().particles.is_empty());
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
    let bad = |t| Ok(Frame { time: t, cells: vec![Cell { depth: f64::MAX, velocity: [f64::MAX; 2] }; 32] });
    assert!(sim.at(0.1, bad).is_err(), "source velocity/depth must be numerically usable, even with emission disabled");
    let mut bed = Vec::with_capacity(100_000);
    bed.extend([2.; 32]);
    assert!(Whitewater::new(grid(), bed, Settings { max_particles: 1, max_bytes: 10_000, ..settings() }).is_err());
}
