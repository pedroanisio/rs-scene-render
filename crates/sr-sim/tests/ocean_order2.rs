use sr_sim::ocean::{Boundary, Cell, Error, Impulse, ImpulseKind, Ocean, Order, Spec};

const ORDERS: [Order; 2] = [Order::First, Order::Second];

fn digest(cells: &[Cell]) -> u64 {
    // FNV-1a over the exact bit patterns.
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for c in cells {
        for v in [c.depth, c.velocity[0], c.velocity[1]] {
            for b in v.to_bits().to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h
}

/// Irregular bathymetry, a dry shore, damping, two impulses and every boundary kind.
fn baseline(boundary: Boundary) -> u64 {
    let spec = Spec {
        cells: [37, 23],
        origin: [-9.25, -5.75],
        cell_size: 0.5,
        dt: 0.05,
        damping: 0.1,
        boundary,
        ..Default::default()
    };
    let bed: Vec<_> = (0..37 * 23).map(|i| 1.5 - ((i % 37) as f64 * 0.07) + ((i / 37) % 5) as f64 * 0.11).collect();
    let cells: Vec<_> = bed
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
    let mut ocean = Ocean::new(spec, bed, cells, impulses).unwrap();
    digest(&ocean.at(1.37).unwrap().cells)
}

/// Recorded from the solver at commit 60a458c, before `Spec::order` existed.
#[test]
fn first_order_is_bit_identical_to_the_pre_order2_solver() {
    assert_eq!(baseline(Boundary::Closed), 0x2f17e2ba55397e9d);
    assert_eq!(baseline(Boundary::Open), 0x8b420d88c529f839);
    assert_eq!(baseline(Boundary::Periodic), 0xbe55e09d501719ee);
}

fn volume(cells: &[Cell], dx: f64) -> f64 {
    cells.iter().map(|c| c.depth * dx * dx).sum()
}

/// Amplitude of the `k` Fourier mode of the surface of a one-row periodic channel.
fn amplitude(cells: &[Cell], k: f64, dx: f64, mean: f64) -> f64 {
    let (mut c, mut s) = (0.0, 0.0);
    for (i, cell) in cells.iter().enumerate() {
        let phase = k * (i as f64 + 0.5) * dx;
        c += (cell.depth - mean) * phase.cos();
        s += (cell.depth - mean) * phase.sin();
    }
    2.0 * c.hypot(s) / cells.len() as f64
}

/// Relative amplitude error after the wave travels `waves` wavelengths.
fn travelling_wave_error(order: Order, cells_per_wavelength: usize, waves: usize) -> f64 {
    let (wavelength, h, g) = (2.0, 1.0, 4.0);
    let dx = wavelength / cells_per_wavelength as f64;
    let n = cells_per_wavelength * 4;
    let spec = Spec {
        cells: [n, 1],
        cell_size: dx,
        dt: 0.05,
        gravity: g,
        boundary: Boundary::Periodic,
        order,
        ..Default::default()
    };
    let k = std::f64::consts::TAU / wavelength;
    let a0 = 1e-3;
    let initial: Vec<_> = (0..n)
        .map(|i| {
            let eta = a0 * (k * (i as f64 + 0.5) * dx).cos();
            Cell { depth: h + eta, velocity: [(g / h).sqrt() * eta, 0.0] }
        })
        .collect();
    let mut ocean = Ocean::new(spec, vec![h; n], initial, vec![]).unwrap();
    let time = waves as f64 * wavelength / (g * h).sqrt();
    (amplitude(&ocean.at(time).unwrap().cells, k, dx, h) / a0 - 1.0).abs()
}

/// Declared improvements in amplitude error after five wavelengths. Minmod
/// clips smooth extrema, so the gain grows with resolution: measured 3.8x at 40
/// cells per wavelength and 13.6x at 80 (first order: 85% and 62% lost).
const FACTOR_40_CELLS: f64 = 3.0;
const FACTOR_80_CELLS: f64 = 10.0;

#[test]
fn second_order_loses_less_wave_amplitude_than_first_order() {
    let (first40, second40) = (travelling_wave_error(Order::First, 40, 5), travelling_wave_error(Order::Second, 40, 5));
    let (first80, second80) = (travelling_wave_error(Order::First, 80, 5), travelling_wave_error(Order::Second, 80, 5));
    println!(
        "AMPLITUDE 40/wavelength order1={first40:e} order2={second40:e} ratio={:.2}; 80/wavelength order1={first80:e} order2={second80:e} ratio={:.2}",
        first40 / second40,
        first80 / second80
    );
    assert!(second40 * FACTOR_40_CELLS < first40, "40 cells: order 1 {first40:e}, order 2 {second40:e}");
    assert!(second80 * FACTOR_80_CELLS < first80, "80 cells: order 1 {first80:e}, order 2 {second80:e}");
    // Refinement keeps paying off: better than first-order convergence (2x).
    assert!(second80 * 3.0 < second40, "order 2 did not converge: {second40:e} -> {second80:e}");
}

fn spec(cells: [usize; 2], order: Order) -> Spec {
    Spec { cells, cell_size: 0.5, dt: 0.05, order, ..Default::default() }
}

#[test]
fn irregular_lakes_stay_exactly_at_rest_in_both_orders() {
    for order in ORDERS {
        // Dry shore.
        let bed: Vec<_> = (0..240).map(|i| 2.0 - (i % 24) as f64 * 0.15).collect();
        let cells: Vec<_> = bed.iter().map(|&y| Cell { depth: y.max(0.0), velocity: [0.0; 2] }).collect();
        let mut ocean = Ocean::new(spec([24, 10], order), bed, cells.clone(), vec![]).unwrap();
        for (a, b) in ocean.at(2.0).unwrap().cells.iter().zip(&cells) {
            assert!((a.depth - b.depth).abs() < 1e-12, "{order:?}: {a:?} != {b:?}");
            assert!(a.velocity.iter().all(|v| v.abs() < 1e-11), "{order:?}: {a:?}");
        }
        // Fully submerged, rough bed with cliffs, periodic.
        let bed: Vec<_> = (0..24 * 10).map(|i| 3.0 + ((i * 7919) % 23) as f64 * 0.13).collect();
        let cells: Vec<_> = bed.iter().map(|&y| Cell { depth: y, velocity: [0.0; 2] }).collect();
        let mut s = spec([24, 10], order);
        s.boundary = Boundary::Periodic;
        let mut ocean = Ocean::new(s, bed, cells.clone(), vec![]).unwrap();
        for (a, b) in ocean.at(2.0).unwrap().cells.iter().zip(&cells) {
            assert!((a.depth - b.depth).abs() < 1e-12, "{order:?}: {a:?} != {b:?}");
            assert!(a.velocity.iter().all(|v| v.abs() < 1e-11), "{order:?}: {a:?}");
        }
    }
}

#[test]
fn closed_and_periodic_boundaries_conserve_water_in_both_orders() {
    for order in ORDERS {
        let initial: Vec<_> =
            (0..256).map(|i| Cell { depth: if i % 64 < 24 { 2.0 } else { 0.0 }, velocity: [0.0; 2] }).collect();
        let before = volume(&initial, 0.5);
        let mut ocean = Ocean::new(spec([64, 4], order), vec![2.0; 256], initial, vec![]).unwrap();
        let frame = ocean.at(1.0).unwrap();
        assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.depth.is_finite()));
        assert!(frame.cells[28].depth > 0.1, "{order:?}: front did not propagate");
        assert!((volume(&frame.cells, 0.5) - before).abs() < 1e-10, "{order:?}");

        let mut s = spec([24, 21], order);
        s.boundary = Boundary::Periodic;
        let initial: Vec<_> = (0..504)
            .map(|i| Cell {
                depth: 1.0 + (i % 13) as f64 * 0.1,
                velocity: [(i % 3) as f64 - 1.0, (i % 5) as f64 - 2.0],
            })
            .collect();
        let before = volume(&initial, 0.5);
        let bed: Vec<_> = (0..504).map(|i| 3.0 + (i % 11) as f64 * 0.15).collect();
        let mut ocean = Ocean::new(s, bed, initial, vec![]).unwrap();
        assert!((volume(&ocean.at(0.6).unwrap().cells, 0.5) - before).abs() < 1e-10, "{order:?}");
    }
}

#[test]
fn wet_dry_fronts_over_irregular_bathymetry_stay_positive_in_both_orders() {
    for order in ORDERS {
        let initial: Vec<_> = (0..504)
            .map(|i| {
                let h = if i % 7 == 0 { 0.0 } else { 0.01 + (i % 13) as f64 * 0.17 };
                let u = if h == 0.0 { [0.0; 2] } else { [(i % 3) as f64 * 8.0 - 8.0, (i % 5) as f64 * 4.0 - 8.0] };
                Cell { depth: h, velocity: u }
            })
            .collect();
        let expected = volume(&initial, 0.5);
        let bed: Vec<_> = (0..504).map(|i| (i % 11) as f64 * 0.15).collect();
        let mut ocean = Ocean::new(spec([24, 21], order), bed, initial, vec![]).unwrap();
        let frame = ocean.at(0.6).unwrap();
        assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.velocity.iter().all(|u| u.is_finite())), "{order:?}");
        assert!((volume(&frame.cells, 0.5) - expected).abs() < 1e-10, "{order:?}");

        // Dam break onto a dry, rising and rough beach.
        let initial: Vec<_> =
            (0..64 * 8).map(|i| Cell { depth: if i % 64 < 20 { 2.0 } else { 0.0 }, velocity: [0.0; 2] }).collect();
        let expected = volume(&initial, 0.5);
        let bed: Vec<_> = (0..64 * 8)
            .map(|i| {
                let x = (i % 64) as f64;
                if x < 20.0 {
                    2.0
                } else {
                    2.0 - (x - 20.0) * 0.03 + ((i * 31) % 7) as f64 * 0.02
                }
            })
            .collect();
        let mut ocean = Ocean::new(spec([64, 8], order), bed, initial, vec![]).unwrap();
        let frame = ocean.at(2.0).unwrap();
        assert!(frame.cells.iter().all(|c| c.depth >= 0.0 && c.depth.is_finite()), "{order:?}");
        assert!((volume(&frame.cells, 0.5) - expected).abs() < 1e-10, "{order:?}");
    }
}

#[test]
fn reverse_replay_is_bit_identical_even_after_checkpoints_are_discarded() {
    for order in ORDERS {
        let mut s = spec([8, 8], order);
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
        assert_eq!(ocean.at(0.83).unwrap(), &expected, "{order:?}");
        // Zero checkpoints: every backwards seek replays from the initial state.
        s.checkpoint_bytes = 0;
        let mut bare = Ocean::new(s.clone(), vec![1.0; 64], initial.clone(), vec![]).unwrap();
        bare.at(1.2).unwrap();
        assert_eq!(bare.at(0.83).unwrap(), &expected, "{order:?}");
    }
}

/// Smallest `max_work` that lets a one-second seek finish.
fn seek_work(order: Order) -> u64 {
    let initial: Vec<_> =
        (0..64).map(|i| Cell { depth: if i % 8 < 4 { 1.0 } else { 0.5 }, velocity: [0.0; 2] }).collect();
    let (mut lo, mut hi) = (1_u64, 1_u64 << 30);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let mut s = spec([8, 8], order);
        s.max_work = mid;
        // The initial advance can already exceed a tiny budget; that is a failure too.
        let ok = Ocean::new(s, vec![1.0; 64], initial.clone(), vec![]).and_then(|mut o| o.at(1.0).map(|_| ())).is_ok();
        if ok {
            hi = mid
        } else {
            lo = mid + 1
        }
    }
    lo
}

#[test]
fn second_order_work_is_charged_for_its_extra_cost() {
    let (first, second) = (seek_work(Order::First), seek_work(Order::Second));
    println!("WORK order1={first} order2={second} ratio={}", second as f64 / first as f64);
    // Half the CFL number doubles the substeps, and each substep has two stages.
    assert!(second >= 4 * first, "order 2 charged {second} against {first}");
}

/// Rms surface disturbance over the interior (outside the sponge) after a
/// positive-mass Gaussian hump, released at the centre of a square domain, has
/// radiated away, relative to the same rms at time zero. A reference run in a
/// large closed domain, where waves never leave, would hold the free-space wake;
/// the point of an absorbing edge is that no lingering level offset or sloshing
/// is left, which the zero-gradient edge cannot guarantee.
fn residual(boundary: Boundary, order: Order, times: &[f64]) -> Vec<f64> {
    let (n, sponge, h, g, a0, sigma) = (96_usize, 16_usize, 2.0, 1.0, 0.05, 4.0);
    let spec = Spec {
        cells: [n, n],
        origin: [-(n as f64) / 2.0, -(n as f64) / 2.0],
        cell_size: 1.0,
        dt: 0.25,
        gravity: g,
        boundary,
        order,
        sponge_cells: sponge,
        max_work: 1 << 40,
        ..Default::default()
    };
    let cells: Vec<_> = (0..n * n)
        .map(|i| {
            let (x, z) = ((i % n) as f64 + 0.5 - 48.0, (i / n) as f64 + 0.5 - 48.0);
            Cell { depth: h + a0 * (-(x * x + z * z) / (2.0 * sigma * sigma)).exp(), velocity: [0.0; 2] }
        })
        .collect();
    let interior_rms = |cells: &[Cell]| {
        let mut sum = 0.0;
        for z in sponge..n - sponge {
            for x in sponge..n - sponge {
                sum += (cells[z * n + x].depth - h).powi(2);
            }
        }
        (sum / ((n - 2 * sponge) as f64).powi(2)).sqrt()
    };
    let initial = interior_rms(&cells);
    let mut ocean = Ocean::new(spec, vec![h; n * n], cells, vec![]).unwrap();
    times.iter().map(|&t| interior_rms(&ocean.at(t).unwrap().cells) / initial).collect()
}

/// The hump's pulse leaves the 32-cell interior half width in about 23 s at
/// sqrt(g*h) = 1.41 cells/s. Measured relative rms at 60 s and 120 s:
/// closed 0.23/0.12, open 0.045/0.016, absorbing 0.014/0.0013 (both orders).
/// Plane pulses are a poorer test: normally incident ones leave the open edge
/// almost untouched (about 0% in one dimension, 3-7% against free space for a
/// finite beam), so the boundaries differ in what they leave behind.
#[test]
fn absorbing_boundary_leaves_less_behind_than_the_open_boundary() {
    let times = [60.0, 120.0];
    let closed = residual(Boundary::Closed, Order::First, &times);
    let open = residual(Boundary::Open, Order::First, &times);
    let first = residual(Boundary::Absorbing, Order::First, &times);
    let second = residual(Boundary::Absorbing, Order::Second, &times);
    println!("RESIDUAL closed={closed:.4?} open={open:.4?} absorbing1={first:.4?} absorbing2={second:.4?}");
    for i in 0..2 {
        assert!(first[i] < 0.05 && second[i] < 0.05, "t={}: absorbing {first:?} {second:?}", times[i]);
        assert!(open[i] > 2.0 * first[i].max(second[i]), "t={}: open {open:?}", times[i]);
        assert!(closed[i] > 0.1, "t={}: closed {closed:?}", times[i]);
    }
}

#[test]
fn absorbing_boundary_leaves_a_uniform_current_untouched() {
    for order in ORDERS {
        let mut s = spec([40, 40], order);
        s.boundary = Boundary::Absorbing;
        s.sponge_cells = 8;
        let initial = vec![Cell { depth: 2.0, velocity: [0.3, -0.1] }; 1600];
        let mut ocean = Ocean::new(s, vec![2.0; 1600], initial.clone(), vec![]).unwrap();
        for (a, b) in ocean.at(3.0).unwrap().cells.iter().zip(&initial) {
            assert!((a.depth - b.depth).abs() < 1e-12 && (a.velocity[0] - 0.3).abs() < 1e-12, "{order:?}: {a:?}");
        }
    }
}

#[test]
fn sponge_state_counts_against_resident_memory_but_not_checkpoints() {
    let n = 40 * 40;
    let cells = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; n];
    let make = |boundary, max_bytes| {
        let mut s = spec([40, 40], Order::First);
        s.boundary = boundary;
        s.sponge_cells = 8;
        s.max_bytes = max_bytes;
        Ocean::new(s, vec![2.0; n], cells.clone(), vec![])
    };
    let plain = n * 256 + 4096;
    assert!(make(Boundary::Open, plain).is_ok());
    assert!(matches!(make(Boundary::Absorbing, plain), Err(Error::Limit(_))));
    assert!(make(Boundary::Absorbing, plain + 24 * n).is_ok());
    let held = |boundary| {
        let mut o = make(boundary, 1 << 28).unwrap();
        o.at(0.3).unwrap();
        o.at(0.1).unwrap();
        o.checkpoint_bytes()
    };
    assert_eq!(held(Boundary::Absorbing), held(Boundary::Open));
}

#[test]
fn sponge_width_is_validated_instead_of_clamped() {
    for width in [0, 3, 20, 100] {
        let mut s = spec([40, 40], Order::First);
        s.boundary = Boundary::Absorbing;
        s.sponge_cells = width;
        let cells = vec![Cell { depth: 2.0, velocity: [0.0; 2] }; 1600];
        assert!(matches!(Ocean::new(s, vec![2.0; 1600], cells, vec![]), Err(Error::Invalid(_))), "{width}");
    }
}
