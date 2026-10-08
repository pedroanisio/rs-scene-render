use sr_sim::ocean::{Boundary, Cell, Impulse, ImpulseKind, Ocean, Spec};

fn spec(cells: [usize; 2]) -> Spec {
    Spec { cells, cell_size: 0.5, dt: 0.05, ..Default::default() }
}
fn volume(cells: &[Cell], dx: f64) -> f64 {
    cells.iter().map(|c| c.depth * dx * dx).sum()
}

#[test]
fn uneven_lake_and_dry_shore_stay_at_rest() {
    let s = spec([24, 10]);
    let bed: Vec<_> = (0..240).map(|i| 2.0 - (i % 24) as f64 * 0.15).collect();
    let cells: Vec<_> = bed.iter().map(|&y| Cell { depth: y.max(0.0), velocity: [0.0; 2] }).collect();
    let mut ocean = Ocean::new(s, bed, cells.clone(), vec![]).unwrap();
    let frame = ocean.at(2.0).unwrap();
    for (a, b) in frame.cells.iter().zip(cells) {
        assert!((a.depth - b.depth).abs() < 1e-12, "{a:?} != {b:?}");
        assert!(a.velocity.iter().all(|v| v.abs() < 1e-11), "{a:?}");
    }
}

#[test]
fn dam_break_wets_dry_cells_without_losing_water() {
    let s = spec([64, 4]);
    let initial: Vec<_> =
        (0..256).map(|i| Cell { depth: if i % 64 < 24 { 2.0 } else { 0.0 }, velocity: [0.0; 2] }).collect();
    let before = volume(&initial, s.cell_size);
    let mut ocean = Ocean::new(s, vec![2.0; 256], initial, vec![]).unwrap();
    let frame = ocean.at(1.0).unwrap();
    assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.depth.is_finite()));
    assert!(frame.cells[28].depth > 0.1, "front did not propagate");
    assert!((volume(&frame.cells, 0.5) - before).abs() < 1e-10);
}

#[test]
fn timed_displacement_is_conservative_and_seek_order_independent() {
    let s = spec([20, 20]);
    let initial = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 400];
    let impulse = Impulse {
        time: 0.23,
        center: [5.0, 5.0],
        radius: 2.0,
        amplitude: 0.5,
        velocity: [1.0, -0.25],
        kind: ImpulseKind::Displace,
    };
    let make = || Ocean::new(s.clone(), vec![2.0; 400], initial.clone(), vec![impulse.clone()]).unwrap();
    let mut ocean = make();
    assert_eq!(ocean.at(0.22).unwrap().cells, initial);
    assert_ne!(ocean.at(0.23).unwrap().cells, initial);
    let at = ocean.at(1.037).unwrap().clone();
    assert!((volume(&at.cells, 0.5) - 200.0).abs() < 1e-10);
    ocean.at(0.17).unwrap();
    assert_eq!(ocean.at(1.037).unwrap(), &at);
    let mut fresh = make();
    fresh.at(0.4).unwrap();
    assert_eq!(fresh.at(1.037).unwrap(), &at);
}

#[test]
fn periodic_uniform_flow_has_analytic_damping() {
    let mut s = spec([8, 7]);
    s.boundary = Boundary::Periodic;
    s.damping = 0.4;
    let initial = vec![Cell { depth: 3.0, velocity: [2.0, -1.0] }; 56];
    let mut ocean = Ocean::new(s, vec![4.0; 56], initial, vec![]).unwrap();
    for cell in &ocean.at(0.73).unwrap().cells {
        assert!((cell.depth - 3.0).abs() < 1e-12);
        assert!((cell.velocity[0] - 2.0 * (-0.4_f64 * 0.73).exp()).abs() < 1e-12);
        assert!((cell.velocity[1] + (-0.4_f64 * 0.73).exp()).abs() < 1e-12);
    }
}

#[test]
fn small_right_travelling_wave_matches_the_shallow_water_speed() {
    let mut s = spec([400, 1]);
    s.cell_size = 0.1;
    s.boundary = Boundary::Periodic;
    s.gravity = 4.0;
    let k = std::f64::consts::TAU / 40.0;
    let initial: Vec<_> = (0..400)
        .map(|i| {
            let eta = 0.001 * (k * (i as f64 + 0.5) * 0.1).cos();
            Cell { depth: 1.0 + eta, velocity: [2.0 * eta, 0.0] }
        })
        .collect();
    let mut ocean = Ocean::new(s, vec![1.0; 400], initial, vec![]).unwrap();
    let frame = ocean.at(2.0).unwrap();
    // Linearized characteristic speed sqrt(g*h)=2; expected phase advance 4*k.
    let (mut cosine, mut sine) = (0.0, 0.0);
    for (i, c) in frame.cells.iter().enumerate() {
        let phase = k * (i as f64 + 0.5) * 0.1;
        cosine += (c.depth - 1.0) * phase.cos();
        sine += (c.depth - 1.0) * phase.sin();
    }
    assert!((sine.atan2(cosine) - 4.0 * k).abs() < 0.005);
    assert!(2.0 * cosine.hypot(sine) / 400.0 > 0.00095, "excessive numerical diffusion");
}

#[test]
fn injection_adds_water_and_displacement_never_borrows_from_outside_its_radius() {
    let s = spec([24, 24]);
    let impulse = Impulse {
        time: 0.0,
        center: [6.0, 6.0],
        radius: 2.0,
        amplitude: -0.4,
        velocity: [0.0; 2],
        kind: ImpulseKind::Displace,
    };
    let cells = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 576];
    let ocean = Ocean::new(s.clone(), vec![1.0; 576], cells.clone(), vec![impulse.clone()]).unwrap();
    assert!((volume(&ocean.frame().cells, 0.5) - 144.0).abs() < 1e-11);
    for (i, c) in ocean.frame().cells.iter().enumerate() {
        let x = (i % 24) as f64 * 0.5 + 0.25;
        let z = (i / 24) as f64 * 0.5 + 0.25;
        if (x - 6.0).hypot(z - 6.0) >= 2.0 {
            assert_eq!(*c, cells[i]);
        }
    }
    let add = Impulse { kind: ImpulseKind::AddWater, amplitude: 0.4, ..impulse };
    let ocean = Ocean::new(s, vec![1.0; 576], cells, vec![add]).unwrap();
    assert!(volume(&ocean.frame().cells, 0.5) > 145.0);
}

#[test]
fn insufficient_displacement_water_and_work_failure_are_atomic() {
    let s = spec([12, 12]);
    let cells = vec![Cell { depth: 0.01, velocity: [0.0; 2] }; 144];
    let impulse = Impulse {
        time: 0.3,
        center: [3.0, 3.0],
        radius: 2.0,
        amplitude: 100.0,
        velocity: [0.0; 2],
        kind: ImpulseKind::Displace,
    };
    let mut ocean = Ocean::new(s, vec![1.0; 144], cells, vec![impulse]).unwrap();
    let before = ocean.at(0.2).unwrap().clone();
    assert!(ocean.at(0.4).is_err());
    assert_eq!(ocean.frame(), &before);
    assert_eq!(ocean.at(0.2).unwrap(), &before);
    let mut s = spec([2, 2]);
    s.max_work = 100;
    let mut ocean = Ocean::new(s, vec![1.0; 4], vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 4], vec![]).unwrap();
    let before = ocean.frame().clone();
    assert!(ocean.at(1000.0).is_err());
    assert_eq!(ocean.frame(), &before);
    assert!(ocean.at(0.01).is_ok());
}

#[test]
fn budgets_and_invalid_data_fail_before_solver_allocation() {
    let run = |s: Spec, bed: Vec<f64>, cells: Vec<Cell>| Ocean::new(s, bed, cells, vec![]);
    let cells = vec![Cell { depth: 1.0, velocity: [0.0; 2] }; 4];
    let mut s = spec([2, 2]);
    s.max_bytes = 100;
    assert!(run(s, vec![1.0; 4], cells.clone()).is_err());
    let mut s = spec([usize::MAX, 2]);
    s.max_bytes = usize::MAX;
    assert!(run(s, vec![], vec![]).is_err());
    for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
        let mut s = spec([2, 2]);
        s.gravity = bad;
        assert!(run(s, vec![1.0; 4], cells.clone()).is_err());
    }
    assert!(run(spec([2, 2]), vec![f64::NAN; 4], cells.clone()).is_err());
    assert!(run(spec([2, 2]), vec![1.0; 3], cells).is_err());
    let mut ocean = run(spec([1, 1]), vec![1.0], vec![Cell::default()]).unwrap();
    for bad in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(ocean.at(bad).is_err());
    }
}

#[test]
fn checkpoint_eviction_preserves_exact_replay() {
    let mut s = spec([8, 8]);
    s.checkpoint_bytes = 4000;
    let initial: Vec<_> =
        (0..64).map(|i| Cell { depth: if i % 8 < 4 { 1.0 } else { 0.2 }, velocity: [0.0; 2] }).collect();
    let make = || Ocean::new(s.clone(), vec![1.0; 64], initial.clone(), vec![]).unwrap();
    let mut reference = make();
    let expected = reference.at(0.83).unwrap().clone();
    let mut ocean = make();
    for t in [0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 0.1, 0.3, 0.71] {
        ocean.at(t).unwrap();
        assert!(ocean.checkpoint_bytes() <= 4000);
    }
    assert_eq!(ocean.at(0.83).unwrap(), &expected);
}

#[test]
fn two_dimensional_high_energy_flow_stays_positive_and_conservative() {
    let s = spec([24, 21]);
    let initial: Vec<_> = (0..504)
        .map(|i| {
            let h = if i % 7 == 0 { 0.0 } else { 0.01 + (i % 13) as f64 * 0.17 };
            Cell {
                depth: h,
                velocity: if h == 0.0 { [0.0; 2] } else { [(i % 3) as f64 * 8.0 - 8.0, (i % 5) as f64 * 4.0 - 8.0] },
            }
        })
        .collect();
    let expected = volume(&initial, 0.5);
    let bed: Vec<_> = (0..504).map(|i| (i % 11) as f64 * 0.15).collect();
    let mut ocean = Ocean::new(s, bed, initial, vec![]).unwrap();
    let frame = ocean.at(0.6).unwrap();
    assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.velocity.iter().all(|u| u.is_finite())));
    assert!((volume(&frame.cells, 0.5) - expected).abs() < 1e-10);
}

#[test]
fn retained_input_capacity_is_part_of_the_memory_budget() {
    let mut bed = Vec::with_capacity(100_000);
    bed.push(1.0);
    let mut s = spec([1, 1]);
    s.max_bytes = 16_384;
    assert!(Ocean::new(s, bed, vec![Cell::default()], vec![]).is_err());
}

#[test]
fn an_impulse_does_not_modify_flow_outside_its_support() {
    let mut s = spec([12, 12]);
    s.dry_tolerance = 0.1;
    let initial = vec![Cell { depth: 0.05, velocity: [0.0; 2] }; 144];
    let impulse = Impulse {
        time: 0.0,
        center: [3.0, 3.0],
        radius: 1.0,
        amplitude: 0.2,
        velocity: [2.0, 1.0],
        kind: ImpulseKind::AddWater,
    };
    let ocean = Ocean::new(s, vec![1.0; 144], initial.clone(), vec![impulse]).unwrap();
    assert_eq!(ocean.frame().cells[0], initial[0]);
    assert!(ocean.frame().cells.iter().any(|c| c.depth > 0.1 && c.velocity[0] > 0.0));
}

#[test]
fn floating_point_tick_rounding_cannot_fire_a_future_impulse() {
    let s = spec([1, 1]);
    let event_time = 17.0 * s.dt;
    assert!(event_time > 0.85);
    let event = Impulse {
        time: event_time,
        center: [0.25; 2],
        radius: 1.0,
        amplitude: 1.0,
        velocity: [0.0; 2],
        kind: ImpulseKind::AddWater,
    };
    let mut ocean = Ocean::new(s, vec![1.0], vec![Cell::default()], vec![event]).unwrap();
    let before = ocean.at(0.85).unwrap();
    assert_eq!(before.time, 0.85);
    assert_eq!(before.cells[0].depth, 0.0);
    assert_eq!(ocean.at(event_time).unwrap().cells[0].depth, 1.0);
    assert_eq!(ocean.at(0.85).unwrap().cells[0].depth, 0.0);
}

#[test]
fn overflowing_displacement_capacity_is_an_error_not_created_water() {
    let mut s = spec([8, 8]);
    s.gravity = 1e-308;
    let event = Impulse {
        time: 0.0,
        center: [2.0; 2],
        radius: 1.5,
        amplitude: 1.0,
        velocity: [0.0; 2],
        kind: ImpulseKind::Displace,
    };
    let cells = vec![Cell { depth: 8e307, velocity: [0.0; 2] }; 64];
    assert!(Ocean::new(s, vec![8e307; 64], cells, vec![event]).is_err());
}

#[test]
fn event_sort_workspace_is_included_in_the_resident_budget() {
    let mut s = spec([1, 1]);
    let events: Vec<_> = (0..2_000)
        .map(|i| Impulse {
            time: ((i * 997) % 2_000 + 1) as f64,
            center: [0.25; 2],
            radius: 1.0,
            amplitude: 0.0,
            velocity: [0.0; 2],
            kind: ImpulseKind::AddWater,
        })
        .collect();
    // The budget is derived from the size of an event, whatever that is: one cell (256 bytes) and the fixed
    // 4096 are always charged, the event vector fits in what is left with half as much again, and the
    // vector together with the temporary allocation of a stable sort (twice the vector) does not.
    let vector = events.capacity() * std::mem::size_of::<Impulse>();
    s.max_bytes = 256 + 4096 + vector + vector / 2;
    assert!(vector < s.max_bytes);
    assert!(Ocean::new(s.clone(), vec![1.0], vec![Cell::default()], events.clone()).is_err());
    // with room for the sort as well it is accepted
    s.max_bytes = 256 + 4096 + 2 * vector + 1024;
    assert!(Ocean::new(s, vec![1.0], vec![Cell::default()], events).is_ok());
}

#[test]
fn dry_dam_break_matches_the_analytic_rarefaction() {
    // Ritter solution: h=(2*sqrt(g*h0)-x/t)^2/(9*g),
    // u=2/3*(sqrt(g*h0)+x/t) between the rarefaction's edges.
    // Reference: Clawpack Riemann book, Shallow_water.html, dry initial states.
    let error = |n: usize| {
        let mut s = spec([n, 1]);
        let dx = 60.0 / n as f64;
        s.cell_size = dx;
        s.gravity = 1.0;
        let initial: Vec<_> =
            (0..n).map(|i| Cell { depth: if i < n / 2 { 1.0 } else { 0.0 }, velocity: [0.0; 2] }).collect();
        let mut ocean = Ocean::new(s, vec![1.0; n], initial, vec![]).unwrap();
        let frame = ocean.at(2.0).unwrap();
        let mut error = [0.0_f64; 2];
        for x in [30.0, 31.0, 32.0] {
            let i = (x / dx) as usize;
            let xi = ((i as f64 + 0.5) * dx - 30.0) / 2.0;
            let h = (2.0 - xi).powi(2) / 9.0;
            let u = 2.0 / 3.0 * (1.0 + xi);
            let c = frame.cells[i];
            error[0] = error[0].max((c.depth - h).abs());
            error[1] = error[1].max((c.velocity[0] - u).abs());
        }
        assert!(frame.cells[(35.0 / dx) as usize].depth < 0.01);
        error
    };
    // A first-order shock solver has resolution-dependent diffusion. Require
    // refinement to approach the independent solution, as well as accuracy.
    let coarse = error(600);
    let fine = error(2400);
    assert!(fine[0] < 0.035 && fine[1] < 0.10, "coarse={coarse:?}, fine={fine:?}");
    for i in 0..2 {
        assert!(fine[i] < 0.65 * coarse[i], "coarse={coarse:?}, fine={fine:?}");
    }
}

#[test]
fn procedural_swell_has_authored_speed_and_preserves_water_and_the_solver_state() {
    use sr_sim::ocean::{waves, Frame};
    let mut s = spec([8, 1]);
    s.cell_size = 1.0;
    let base = Frame { time: 0.0, cells: vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 8], bed: vec![] };
    let w = waves::Wave { wavelength: 8.0, amplitude: 0.1, direction: 0.0, phase: 0.0, speed: 2.0 };
    let a = waves::apply(&s, &base, &[w]).unwrap();
    let b = waves::apply(&s, &Frame { time: 1.0, ..base.clone() }, &[w]).unwrap();
    for i in 0..8 {
        assert!((a.cells[(i + 6) % 8].depth - b.cells[i].depth).abs() < 1e-12);
        assert!((a.cells[i].velocity[0] - (a.cells[i].depth - 2.0)).abs() < 1e-12);
    }
    assert_eq!(base.cells, vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 8]);
    assert!((a.cells.iter().map(|c| c.depth).sum::<f64>() - 16.0).abs() < 1e-12);
    let coast = Frame {
        time: 2.0,
        cells: (0..8).map(|i| Cell { depth: i as f64 * 0.01, velocity: [0.0; 2] }).collect(),
        bed: vec![],
    };
    let water = waves::apply(&s, &coast, &[waves::Wave { amplitude: 2.0, ..w }; 4]).unwrap();
    assert!(water.cells.iter().all(|c| c.depth >= 0.0));
    assert_eq!(water.cells[0].depth, 0.0);
    assert!((water.cells.iter().map(|c| c.depth).sum::<f64>() - 0.28).abs() < 1e-12);
    assert!(waves::apply(&s, &base, &[waves::Wave { wavelength: 1.0, ..w }]).is_err());
}

#[test]
fn the_canonical_step_is_the_last_whole_step_that_has_ended_by_the_time() {
    use sr_sim::ocean::canonical_step;
    assert_eq!(canonical_step(0.0, 0.05), 0);
    assert_eq!(canonical_step(0.049, 0.05), 0);
    assert_eq!(canonical_step(0.05, 0.05), 1);
    // 0.85 / 0.05 rounds up to 17, but 17 steps of 0.05 are more than 0.85: the step is 16
    assert_eq!(0.85_f64 / 0.05, 17.0);
    assert_eq!(canonical_step(0.85, 0.05), 16);
    // and a time that is a tick by the product but not by the quotient is the tick
    assert_eq!(canonical_step(17.0 * 0.05, 0.05), 17);
    // every tick is the step it is named by, and a time before the next is that step
    for k in 0..2000u64 {
        let tick = k as f64 * 0.05;
        assert_eq!(canonical_step(tick, 0.05), k, "tick {k}");
        assert_eq!(canonical_step(tick + 0.0249, 0.05), k, "after tick {k}");
    }
}
