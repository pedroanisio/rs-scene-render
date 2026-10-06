//! A bed that moves with time: the water column follows it, gravity radiates the
//! change, and water volume is conserved.
use sr_sim::ocean::{Boundary, Cell, Error, Forcing, Impulse, ImpulseKind, Ocean, Order, Spec};

const ORDERS: [Order; 2] = [Order::First, Order::Second];

fn spec(cells: [usize; 2], order: Order) -> Spec {
    Spec { cells, cell_size: 0.5, dt: 0.05, order, moving_bed: true, max_work: 1 << 40, ..Default::default() }
}
fn volume(cells: &[Cell], dx: f64) -> f64 {
    cells.iter().map(|c| c.depth * dx * dx).sum()
}
/// Smooth 0 -> 1 ramp over `[0, span]`.
fn ramp(t: f64, span: f64) -> f64 {
    let s = (t / span).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}
fn constant(bed: Vec<f64>) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |_, f| {
        f.bed.copy_from_slice(&bed);
        Ok(())
    }
}

/// Irregular bathymetry with a dry shore, damping and two impulses; the same
/// scenario drives the static solver and the driven one.
fn scenario(boundary: Boundary, order: Order) -> (Spec, Vec<f64>, Vec<Cell>, Vec<Impulse>) {
    let (nx, nz) = (37, 23);
    let s = Spec { origin: [-9.25, -5.75], damping: 0.1, boundary, ..spec([nx, nz], order) };
    let bed: Vec<_> = (0..nx * nz).map(|i| 1.5 - (i % nx) as f64 * 0.07 + ((i / nx) % 5) as f64 * 0.11).collect();
    let cells = bed
        .iter()
        .map(|&y| Cell { depth: y.max(0.0), velocity: if y > 0.0 { [0.3, -0.2] } else { [0.0; 2] } })
        .collect();
    let impulses = vec![
        Impulse {
            time: 0.0,
            center: [-3.0, 1.0],
            radius: 2.0,
            amplitude: 0.4,
            velocity: [0.5, 0.0],
            kind: ImpulseKind::AddWater,
        },
        Impulse {
            time: 0.37,
            center: [2.0, -2.0],
            radius: 1.5,
            amplitude: -0.2,
            velocity: [0.0; 2],
            kind: ImpulseKind::Displace,
        },
    ];
    (s, bed, cells, impulses)
}

#[test]
fn a_driver_that_never_moves_the_bed_matches_the_static_solver_bit_for_bit() {
    for order in ORDERS {
        for boundary in [Boundary::Closed, Boundary::Open, Boundary::Periodic] {
            let (s, bed, cells, impulses) = scenario(boundary, order);
            let mut fixed = s.clone();
            fixed.moving_bed = false;
            let mut reference = Ocean::new(fixed, bed.clone(), cells.clone(), impulses.clone()).unwrap();
            let expected = reference.at(1.37).unwrap().clone();
            let mut driven = Ocean::new(s, bed.clone(), cells, impulses).unwrap();
            let got = driven.at_driven(1.37, &mut constant(bed.clone())).unwrap();
            assert_eq!(got.cells, expected.cells, "{order:?} {boundary:?}");
            assert_eq!(got.bed, bed, "the frame carries the bed it was computed over");
            assert!(expected.bed.is_empty(), "a static frame has no bed of its own");
        }
    }
}

#[test]
fn an_uneven_lake_stays_at_rest_under_a_driver_that_returns_its_bed() {
    for order in ORDERS {
        let bed: Vec<_> = (0..240).map(|i| 2.0 - (i % 24) as f64 * 0.15).collect();
        let cells: Vec<_> = bed.iter().map(|&y| Cell { depth: y.max(0.0), velocity: [0.0; 2] }).collect();
        let mut ocean = Ocean::new(spec([24, 10], order), bed.clone(), cells.clone(), vec![]).unwrap();
        let frame = ocean.at_driven(2.0, &mut constant(bed)).unwrap();
        for (a, b) in frame.cells.iter().zip(&cells) {
            assert!((a.depth - b.depth).abs() < 1e-12, "{order:?}: {a:?} != {b:?}");
            assert!(a.velocity.iter().all(|v| v.abs() < 1e-11), "{order:?}: {a:?}");
        }
    }
}

/// Surface elevation above the rest level 0 (scene y is downward).
fn elevation(depth: f64, bed: f64) -> f64 {
    depth - bed
}

/// A Gaussian uplift of 1% of the depth in a channel splits into two pulses of
/// half the height that travel at sqrt(g*h) = 2.
#[test]
fn a_rising_bed_launches_waves_at_the_shallow_water_speed() {
    let (n, dx, h, g, uplift, sigma) = (400, 0.1, 1.0, 4.0, 0.01, 1.0);
    for (order, position_tolerance, least_fraction) in [(Order::First, 0.5, 0.55), (Order::Second, 0.3, 0.85)] {
        let s =
            Spec { cells: [n, 1], cell_size: dx, dt: 0.05, gravity: g, order, moving_bed: true, ..Default::default() };
        let x = |i: usize| (i as f64 + 0.5) * dx - 20.0;
        let origin = [-20.0, 0.0];
        let s = Spec { origin, ..s };
        let base = vec![h; n];
        let cells = vec![Cell { depth: h, velocity: [0.0; 2] }; n];
        let mut ocean = Ocean::new(s, base.clone(), cells, vec![]).unwrap();
        let mut driver = |t: f64, f: &mut Forcing| {
            for i in 0..n {
                f.bed[i] = h - uplift * (-(x(i).powi(2)) / (2.0 * sigma * sigma)).exp() * ramp(t, 0.1);
            }
            Ok(())
        };
        let time = 4.0;
        let frame = ocean.at_driven(time, &mut driver).unwrap();
        let e: Vec<f64> = (0..n).map(|i| elevation(frame.cells[i].depth, frame.bed[i])).collect();
        let right = (n / 2..n).max_by(|&a, &b| e[a].total_cmp(&e[b])).unwrap();
        let left = (0..n / 2).max_by(|&a, &b| e[a].total_cmp(&e[b])).unwrap();
        let expected = (g * h).sqrt() * time;
        println!(
            "WAVE {order:?}: right peak at {:.2} (expected {expected:.2}), left at {:.2}; heights {:.5} and {:.5} of {:.5}",
            x(right),
            x(left),
            e[right],
            e[left],
            uplift / 2.0
        );
        assert!((x(right) - expected).abs() < position_tolerance, "{order:?}: right pulse at {}", x(right));
        assert!((x(left) + expected).abs() < position_tolerance, "{order:?}: left pulse at {}", x(left));
        for peak in [right, left] {
            assert!(e[peak] > least_fraction * uplift / 2.0 && e[peak] < 1.05 * uplift / 2.0, "{order:?}: {}", e[peak]);
        }
        // Nothing has reached the far field, and the bed ends where the driver put it.
        assert!(e[0].abs() < 1e-9 && e[n - 1].abs() < 1e-9);
    }
}

/// A smooth crater that opens and refills; used by the conservation and replay tests.
fn crater(
    cells: [usize; 2],
    dx: f64,
    depth: f64,
    sigma: f64,
    cycle: f64,
    base: f64,
) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |t, f| {
        let phase = (std::f64::consts::PI * (t / cycle).clamp(0.0, 1.0)).sin().powi(2);
        for iz in 0..cells[1] {
            for ix in 0..cells[0] {
                let (x, z) = (
                    (ix as f64 + 0.5) * dx - 0.5 * cells[0] as f64 * dx,
                    (iz as f64 + 0.5) * dx - 0.5 * cells[1] as f64 * dx,
                );
                f.bed[iz * cells[0] + ix] = base + depth * (-(x * x + z * z) / (2.0 * sigma * sigma)).exp() * phase;
            }
        }
        Ok(())
    }
}

#[test]
fn a_moving_bed_conserves_water_and_keeps_depth_non_negative_in_both_orders() {
    for order in ORDERS {
        for boundary in [Boundary::Closed, Boundary::Periodic] {
            let cells = [48, 48];
            let s = Spec { origin: [-12.0, -12.0], boundary, ..spec(cells, order) };
            // The shore on the left is dry at rest (bed ordinate below the water level).
            let base: Vec<f64> = (0..48 * 48).map(|i| 1.0 - f64::from(u8::from((i % 48) < 6)) * 1.2).collect();
            let initial: Vec<_> = base.iter().map(|&y| Cell { depth: y.max(0.0), velocity: [0.0; 2] }).collect();
            let before = volume(&initial, 0.5);
            let mut sink = crater(cells, 0.5, 0.7, 2.0, 1.5, 0.0);
            let mut driver = |t: f64, f: &mut Forcing| {
                sink(t, f)?;
                for (b, base) in f.bed.iter_mut().zip(&base) {
                    *b += *base;
                }
                Ok(())
            };
            let mut ocean = Ocean::new(s, base.clone(), initial, vec![]).unwrap();
            for t in [0.4, 0.9, 1.5, 2.5] {
                let frame = ocean.at_driven(t, &mut driver).unwrap();
                assert!(
                    frame.cells.iter().all(|c| c.depth >= 0.0 && c.depth.is_finite()),
                    "{order:?} {boundary:?} t={t}"
                );
                let error = (volume(&frame.cells, 0.5) - before).abs() / before;
                assert!(error < 1e-12, "{order:?} {boundary:?} t={t}: relative volume error {error:e}");
            }
        }
    }
}

#[test]
fn a_sinking_bed_pulls_water_in_and_then_radiates_outward_at_the_wave_speed() {
    for order in ORDERS {
        let cells = [64, 64];
        let (dx, depth, sigma) = (0.5, 0.8, 2.0);
        let s = Spec { origin: [-16.0, -16.0], ..spec(cells, order) };
        let initial = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 64 * 64];
        let mut ocean = Ocean::new(s, vec![1.0; 64 * 64], initial, vec![]).unwrap();
        // The bed sinks over 0.3 s and stays down.
        let mut driver = |t: f64, f: &mut Forcing| {
            for iz in 0..64 {
                for ix in 0..64 {
                    let (x, z) = ((ix as f64 + 0.5) * dx - 16.0, (iz as f64 + 0.5) * dx - 16.0);
                    f.bed[iz * 64 + ix] = 1.0 + depth * (-(x * x + z * z) / (2.0 * sigma * sigma)).exp() * ramp(t, 0.3);
                }
            }
            Ok(())
        };
        let centre = 32 * 64 + 32;
        let frame = ocean.at_driven(0.3, &mut driver).unwrap().clone();
        let e = elevation(frame.cells[centre].depth, frame.bed[centre]);
        // The column went down with the bed (and began to fill).
        assert!(e < -0.4 * depth, "{order:?}: centre elevation {e}");
        // Inflow: the radial velocity on a ring just outside the sink points inward.
        let mut inward = 0.0;
        for iz in 0..64 {
            for ix in 0..64 {
                let (x, z) = ((ix as f64 + 0.5) * dx - 16.0, (iz as f64 + 0.5) * dx - 16.0);
                let r = x.hypot(z);
                if (2.0..4.0).contains(&r) {
                    inward +=
                        (frame.cells[iz * 64 + ix].velocity[0] * x + frame.cells[iz * 64 + ix].velocity[1] * z) / r;
                }
            }
        }
        assert!(inward < 0.0, "{order:?}: net radial velocity {inward}");
        // Causality and radiation. The elevation of a carried column includes the bed's
        // own displacement, so disturbance is measured as a change of water depth. A ring
        // at r = 10 is untouched at 0.3 s (the Gaussian tail there is under 1e-4 of the
        // sink and the wave covers one unit in that time) and is disturbed later.
        let ring = |frame: &sr_sim::ocean::Frame| {
            let mut worst = 0.0_f64;
            for iz in 0..64 {
                for ix in 0..64 {
                    let (x, z) = ((ix as f64 + 0.5) * dx - 16.0, (iz as f64 + 0.5) * dx - 16.0);
                    if (10.0..11.0).contains(&x.hypot(z)) {
                        worst = worst.max((frame.cells[iz * 64 + ix].depth - 1.0).abs());
                    }
                }
            }
            worst
        };
        assert!(ring(&frame) < 1e-4, "{order:?}: disturbed before the wave could arrive: {}", ring(&frame));
        let mut peak = 0.0_f64;
        for k in 1..=30 {
            peak = peak.max(ring(ocean.at_driven(0.3 + 0.1 * k as f64, &mut driver).unwrap()));
        }
        println!("SINK {order:?}: ring depth change peaks at {peak:.4}");
        assert!(peak > 0.005, "{order:?}: the wave never reached r = 10 ({peak})");
    }
}

#[test]
fn reverse_replay_is_bit_identical_even_after_checkpoints_are_discarded() {
    for order in ORDERS {
        let cells = [24, 24];
        let mut s = Spec { origin: [-6.0, -6.0], ..spec(cells, order) };
        s.checkpoint_bytes = 4000;
        let initial = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 24 * 24];
        let make = || Ocean::new(s.clone(), vec![1.0; 24 * 24], initial.clone(), vec![]).unwrap();
        let mut driver = crater(cells, 0.5, 0.6, 1.5, 1.2, 1.0);
        let mut reference = make();
        let expected = reference.at_driven(0.83, &mut driver).unwrap().clone();
        let mut ocean = make();
        for t in [0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 0.1, 0.3, 0.71] {
            ocean.at_driven(t, &mut driver).unwrap();
            assert!(ocean.checkpoint_bytes() <= 4000);
        }
        assert_eq!(ocean.at_driven(0.83, &mut driver).unwrap(), &expected, "{order:?}");
        // With no checkpoints at all every backward seek replays from the start.
        s.checkpoint_bytes = 0;
        let mut bare = Ocean::new(s, vec![1.0; 24 * 24], initial.clone(), vec![]).unwrap();
        bare.at_driven(1.2, &mut driver).unwrap();
        assert_eq!(bare.at_driven(0.83, &mut driver).unwrap(), &expected, "{order:?}");
    }
}

#[test]
fn sequential_frames_sample_the_driver_once_per_canonical_step() {
    let cells = [16, 16];
    let s = Spec { origin: [-4.0, -4.0], ..spec(cells, Order::First) };
    let initial = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 16 * 16];
    let mut ocean = Ocean::new(s, vec![1.0; 256], initial, vec![]).unwrap();
    let mut calls = Vec::new();
    let mut inner = crater(cells, 0.5, 0.2, 1.0, 1.0, 1.0);
    let mut driver = |t: f64, f: &mut Forcing| {
        calls.push(t);
        inner(t, f)
    };
    // Twenty frames at one canonical step each, then a fractional frame.
    for k in 1..=20 {
        ocean.at_driven(k as f64 * 0.05, &mut driver).unwrap();
    }
    ocean.at_driven(1.02, &mut driver).unwrap();
    let steps = calls.len();
    assert!(steps <= 20 + 3, "{steps} driver calls for 21 frames: {calls:?}");
    assert!(calls.windows(2).all(|w| w[0] <= w[1] + 1e-12), "sampled out of order: {calls:?}");
}

#[test]
fn driver_failures_and_misuse_are_errors_that_leave_the_state_intact() {
    let cells = [16, 16];
    let s = Spec { origin: [-4.0, -4.0], ..spec(cells, Order::First) };
    let initial = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 256];
    let mut ocean = Ocean::new(s.clone(), vec![1.0; 256], initial.clone(), vec![]).unwrap();
    let mut good = crater(cells, 0.5, 0.2, 1.0, 1.0, 1.0);
    let before = ocean.at_driven(0.2, &mut good).unwrap().clone();
    // A driver that fails part-way through a seek.
    let mut failing = |t: f64, f: &mut Forcing| {
        if t > 0.5 {
            return Err(Error::Invalid("driver refused"));
        }
        good(t, f)
    };
    assert!(ocean.at_driven(1.0, &mut failing).is_err());
    assert_eq!(ocean.frame(), &before);
    // A driver that writes a bed of the wrong length.
    let mut short = |_: f64, f: &mut Forcing| {
        f.bed.truncate(3);
        Ok(())
    };
    assert!(ocean.at_driven(0.6, &mut short).is_err());
    assert_eq!(ocean.frame(), &before);
    // A nonfinite bed.
    let mut nan = |_: f64, f: &mut Forcing| {
        f.bed[5] = f64::NAN;
        Ok(())
    };
    assert!(ocean.at_driven(0.6, &mut nan).is_err());
    assert_eq!(ocean.frame(), &before);
    // The two entry points do not mix: a driven solver has no static answer and the reverse.
    assert!(matches!(ocean.at(0.3), Err(Error::Invalid(_))));
    let mut fixed = Spec { moving_bed: false, ..s };
    fixed.moving_bed = false;
    let mut plain = Ocean::new(fixed, vec![1.0; 256], initial, vec![]).unwrap();
    assert!(matches!(plain.at_driven(0.3, &mut crater(cells, 0.5, 0.2, 1.0, 1.0, 1.0)), Err(Error::Invalid(_))));
}

#[test]
fn the_driver_vectors_count_against_resident_memory() {
    let n = 40 * 40;
    let cells = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; n];
    let make = |moving, max_bytes| {
        let s = Spec { cells: [40, 40], moving_bed: moving, max_bytes, ..Default::default() };
        Ocean::new(s, vec![1.0; n], cells.clone(), vec![])
    };
    let plain = n * 256 + 4096;
    assert!(make(false, plain).is_ok());
    assert!(matches!(make(true, plain), Err(Error::Limit(_))));
    // Two bed vectors (the step's start and end) and the interpolated bed: 24 bytes per cell; and the state and the
    // bed vectors of the two ends that are kept of the step passed on the way ahead: 40 more.
    assert!(make(true, plain + 64 * n).is_ok());
    assert!(matches!(make(true, plain + 64 * n - 1), Err(Error::Limit(_))));
}

/// The authored impact crater, seen as a bed: radius 52 and depth 25 under a
/// 12-unit layer of water, with a rim 7 high and 10 wide, growing over 1.5 s.
fn deep_crater(cells: [usize; 2], dx: f64) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |t, f| {
        let growth = ramp(t - 1.0, 1.5);
        for iz in 0..cells[1] {
            for ix in 0..cells[0] {
                let x = (ix as f64 + 0.5) * dx - 0.5 * cells[0] as f64 * dx;
                let z = (iz as f64 + 0.5) * dx - 0.5 * cells[1] as f64 * dx;
                let r = x.hypot(z);
                let bowl = (1.0 - (r / 52.0).powi(2)).max(0.0).powi(2);
                let rim = (-((r - 52.0) / 10.0).powi(2)).exp();
                f.bed[iz * cells[0] + ix] = 12.0 + growth * (25.0 * bowl - 7.0 * rim);
            }
        }
        Ok(())
    }
}

/// A crater twice as deep as the water around it drains the ring that feeds it.
/// The solver must neither fail nor lose water or create it, in both orders.
#[test]
fn a_crater_deeper_than_the_water_conserves_volume_and_never_goes_negative() {
    for order in ORDERS {
        let cells = [100, 100];
        let dx = 2.0;
        let s = Spec {
            cells,
            origin: [-100.0, -100.0],
            cell_size: dx,
            dt: 1.0 / 24.0,
            gravity: 9.81,
            damping: 0.02,
            order,
            moving_bed: true,
            checkpoint_bytes: 0,
            max_work: 1 << 40,
            ..Default::default()
        };
        let initial = vec![Cell { depth: 12.0, velocity: [0.0; 2] }; 100 * 100];
        let before = volume(&initial, dx);
        let mut ocean = Ocean::new(s, vec![12.0; 100 * 100], initial, vec![]).unwrap();
        let mut driver = deep_crater(cells, dx);
        let (mut worst_volume, mut least_depth, mut substeps) = (0.0_f64, f64::MAX, vec![]);
        for k in 1..=144 {
            let frame = ocean.at_driven(k as f64 / 24.0, &mut driver).unwrap();
            worst_volume = worst_volume.max((volume(&frame.cells, dx) - before).abs() / before);
            least_depth = least_depth.min(frame.cells.iter().map(|c| c.depth).fold(f64::MAX, f64::min));
            substeps.push(ocean.last_seek_substeps());
        }
        let quiet: u64 = substeps[..20].iter().sum();
        let growing: u64 = substeps[24..60].iter().sum();
        println!(
            "CRATER {order:?}: worst relative volume error {worst_volume:e}, least depth {least_depth:.4}, substeps per canonical step {:.2} before the crater and {:.2} while it grows",
            quiet as f64 / 20.0,
            growing as f64 / 36.0
        );
        assert!(worst_volume < 1e-12, "{order:?}: volume error {worst_volume:e}");
        assert!(least_depth >= 0.0);
    }
}

fn body_spec(cells: [usize; 2], order: Order) -> Spec {
    Spec { bodies: true, ..spec(cells, order) }
}

/// A uniform patch of body: thickness `t` moving at `u`, over a flat bed.
fn uniform_body(thickness: f64, u: [f64; 2], bed: f64) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |_, f| {
        f.bed.fill(bed - thickness);
        f.occupancy.fill(thickness);
        f.velocity.fill(u);
        Ok(())
    }
}

/// The momentum a body gives over one canonical step is the same whether the CFL
/// bound splits the step in two or in many.
#[test]
fn momentum_transfer_does_not_depend_on_how_many_substeps_the_cfl_chose() {
    let (h, t, u) = (4.0, 2.0, [1.5, -0.5]);
    let mut seen = Vec::new();
    for order in ORDERS {
        for gravity in [2.0, 10.0, 40.0, 160.0, 640.0] {
            let cells = [8, 8];
            // Periodic edges keep the uniform flow uniform; a wall would brake it.
            let s = Spec {
                gravity,
                dt: 0.1,
                cell_size: 1.0,
                origin: [0.0, 0.0],
                boundary: Boundary::Periodic,
                ..body_spec(cells, order)
            };
            let initial = vec![Cell { depth: h, velocity: [0.0; 2] }; 64];
            let mut ocean = Ocean::new(s, vec![h; 64], initial, vec![]).unwrap();
            let mut driver = uniform_body(t, u, h);
            ocean.at_driven(0.1, &mut driver).unwrap();
            let substeps = ocean.last_seek_substeps();
            let given = ocean.exchanged_impulse();
            // After one step the water has closed the fraction f = t / (h + t) of the gap.
            let f = t / (h + t);
            let expected = [64.0 * h * f * u[0], 64.0 * h * f * u[1]];
            println!("TRANSFER {order:?} g={gravity}: {substeps} substeps, impulse {given:?}");
            for a in 0..2 {
                assert!(
                    (given[a] - expected[a]).abs() < 1e-12 * expected[a].abs(),
                    "{order:?} g={gravity}: {given:?} vs {expected:?}"
                );
            }
            seen.push(substeps);
        }
    }
    assert!(
        seen.iter().any(|&n| n <= 3) && seen.iter().any(|&n| n >= 6),
        "the cases should span few and many substeps: {seen:?}"
    );
}

#[test]
fn a_body_moving_through_still_water_drags_it_along_and_water_is_conserved() {
    for order in ORDERS {
        let cells = [48, 32];
        let s = Spec { origin: [-12.0, -8.0], ..body_spec(cells, order) };
        let initial = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 48 * 32];
        let before = volume(&initial, 0.5);
        let mut ocean = Ocean::new(s, vec![2.0; 48 * 32], initial, vec![]).unwrap();
        // A 3-unit square body, 1 unit thick, moves in +x at 4 units/s from x = -6.
        let mut driver = |t: f64, f: &mut Forcing| {
            let centre = -6.0 + 4.0 * t;
            for iz in 0..32 {
                for ix in 0..48 {
                    let (x, z) = ((ix as f64 + 0.5) * 0.5 - 12.0, (iz as f64 + 0.5) * 0.5 - 8.0);
                    let inside = (x - centre).abs() < 1.5 && z.abs() < 1.5;
                    let i = iz * 48 + ix;
                    f.occupancy[i] = if inside { 1.0 } else { 0.0 };
                    f.velocity[i] = [4.0, 0.0];
                    f.bed[i] = 2.0 - f.occupancy[i];
                }
            }
            Ok(())
        };
        let mut impulse = [0.0; 2];
        for k in 1..=20 {
            let frame = ocean.at_driven(k as f64 * 0.05, &mut driver).unwrap();
            assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.depth.is_finite()), "{order:?}");
            let given = ocean.exchanged_impulse();
            impulse[0] += given[0];
            impulse[1] += given[1];
        }
        let frame = ocean.frame();
        assert!((volume(&frame.cells, 0.5) - before).abs() / before < 1e-12, "{order:?}");
        // Water where the body went moves with it, and the sideways impulse cancels by symmetry.
        let momentum_x: f64 = frame.cells.iter().map(|c| c.depth * c.velocity[0]).sum();
        let momentum_z: f64 = frame.cells.iter().map(|c| c.depth * c.velocity[1]).sum();
        println!("DRAG {order:?}: impulse given {impulse:?}, water momentum x {momentum_x:.3}, z {momentum_z:.3e}");
        assert!(impulse[0] > 0.0 && momentum_x > 0.0, "{order:?}");
        assert!(impulse[1].abs() < 1e-9 * impulse[0] && momentum_z.abs() < 1e-9 * momentum_x.abs(), "{order:?}");
    }
}

#[test]
fn bodies_without_occupancy_give_no_momentum() {
    let cells = [8, 8];
    let s = Spec { origin: [0.0, 0.0], ..body_spec(cells, Order::First) };
    let initial = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 64];
    let mut ocean = Ocean::new(s, vec![2.0; 64], initial, vec![]).unwrap();
    let mut driver = uniform_body(0.0, [5.0, 5.0], 2.0);
    let frame = ocean.at_driven(0.3, &mut driver).unwrap();
    assert!(frame.cells.iter().all(|c| c.velocity == [0.0; 2]));
    assert_eq!(ocean.exchanged_impulse(), [0.0; 2]);
}

#[test]
fn replay_with_bodies_is_bit_identical_and_misuse_is_an_error() {
    for order in ORDERS {
        let cells = [16, 16];
        let mut s = Spec { origin: [-4.0, -4.0], ..body_spec(cells, order) };
        s.checkpoint_bytes = 6000;
        let initial = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 256];
        let make = || Ocean::new(s.clone(), vec![2.0; 256], initial.clone(), vec![]).unwrap();
        let mut driver = |t: f64, f: &mut Forcing| {
            for i in 0..256 {
                let (x, z) = ((i % 16) as f64 * 0.5 - 4.0, (i / 16) as f64 * 0.5 - 4.0);
                let inside = (x - 3.0 * t + 2.0).abs() < 1.0 && z.abs() < 1.0;
                f.occupancy[i] = if inside { 0.8 } else { 0.0 };
                f.velocity[i] = [3.0, 0.0];
                f.bed[i] = 2.0 - f.occupancy[i];
            }
            Ok(())
        };
        let mut reference = make();
        let expected = reference.at_driven(0.63, &mut driver).unwrap().clone();
        let impulse = reference.exchanged_impulse();
        let mut ocean = make();
        for t in [0.2, 0.4, 0.8, 1.0, 0.1, 0.3, 0.5] {
            ocean.at_driven(t, &mut driver).unwrap();
        }
        assert_eq!(ocean.at_driven(0.63, &mut driver).unwrap(), &expected, "{order:?}");
        assert_eq!(ocean.exchanged_impulse(), impulse, "{order:?}");
    }
    // Bodies need a moving bed; a driver that forgets the body vectors is an error.
    let no_bed = Spec { moving_bed: false, ..body_spec([8, 8], Order::First) };
    assert!(matches!(
        Ocean::new(no_bed, vec![2.0; 64], vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 64], vec![]),
        Err(Error::Invalid(_))
    ));
    let mut ocean = Ocean::new(
        body_spec([8, 8], Order::First),
        vec![2.0; 64],
        vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 64],
        vec![],
    )
    .unwrap();
    let mut forgetful = |_: f64, f: &mut Forcing| {
        f.occupancy.clear();
        Ok(())
    };
    assert!(ocean.at_driven(0.2, &mut forgetful).is_err());
    let mut negative = |_: f64, f: &mut Forcing| {
        f.occupancy[3] = -1.0;
        Ok(())
    };
    assert!(ocean.at_driven(0.2, &mut negative).is_err());
}

#[test]
fn body_vectors_are_charged_to_resident_memory() {
    let n = 40 * 40;
    let cells = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; n];
    let make = |bodies, max_bytes| {
        let s = Spec { cells: [40, 40], moving_bed: true, bodies, max_bytes, ..Default::default() };
        Ocean::new(s, vec![1.0; n], cells.clone(), vec![])
    };
    let moving = n * (256 + 64) + 4096;
    assert!(make(false, moving).is_ok());
    assert!(matches!(make(true, moving), Err(Error::Limit(_))));
    // thickness and velocity of the two ends and of the interpolated sample (72), and of the two kept ends (48)
    assert!(make(true, moving + 120 * n).is_ok());
    assert!(matches!(make(true, moving + 120 * n - 1), Err(Error::Limit(_))));
}

/// Substeps a seek to `t` integrates after the solver has run on to 6 s, with room
/// for `states` checkpoints.
fn backward_substeps(states: usize, order: Order) -> (u64, sr_sim::ocean::Frame) {
    let cells = [24, 24];
    let state_bytes = 24 * 24 * 24 + 128;
    let mut s =
        Spec { origin: [-6.0, -6.0], dt: 1.0 / 20.0, checkpoint_bytes: states * state_bytes, ..spec(cells, order) };
    s.max_work = 1 << 40;
    let initial = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 24 * 24];
    let mut ocean = Ocean::new(s, vec![1.0; 24 * 24], initial, vec![]).unwrap();
    let mut driver = crater(cells, 0.5, 0.4, 1.5, 4.0, 1.0);
    for k in 1..=120 {
        ocean.at_driven(k as f64 / 20.0, &mut driver).unwrap();
    }
    let frame = ocean.at_driven(3.35, &mut driver).unwrap().clone();
    assert!(ocean.checkpoint_bytes() <= states * state_bytes);
    (ocean.last_seek_substeps(), frame)
}

/// A backward seek restarts from a checkpoint about a second apart, not from the
/// most recent targets: the first second of simulated time is not replayed again.
#[test]
fn a_backward_seek_restarts_from_the_nearest_second_checkpoint() {
    for order in ORDERS {
        let (cold, expected) = backward_substeps(0, order);
        for states in [3, 7, 40] {
            let (warm, frame) = backward_substeps(states, order);
            assert_eq!(frame, expected, "{order:?}, {states} states: replay must be bit-identical");
            println!("CHECKPOINT {order:?} {states} states: {warm} substeps against {cold} cold");
            // Room for six states keeps every second; three thin them to every other
            // second, so the nearest one to 3.35 s is at 2 s instead of 3 s.
            let factor = if states >= 7 { 4 } else { 2 };
            assert!(warm * factor < cold, "{order:?}, {states} states: {warm} substeps against {cold}");
        }
    }
}
