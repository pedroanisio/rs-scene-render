//! The ocean against an exact answer: a Gaussian hump on still shallow water, whose linear solution is known in closed
//! form as an integral, so that the error and the order of the scheme are measured against something outside the code.
//!
//! Over water of depth `h` at rest the surface starts as `A exp(-r^2 / 2 sigma^2)` and, in the linear shallow-water
//! equations with speed `c = sqrt(g h)`, becomes (Hankel transform of the initial surface, each wavenumber `k = s / sigma`
//! oscillating as `cos(c k t)`)
//!
//! ```text
//! eta(r, t) = A integral_0^inf s exp(-s^2 / 2) J0(s r / sigma) cos(c s t / sigma) ds.
//! ```
//!
//! The integral is evaluated here with Simpson's rule on `s` in `[0, 14]` (the Gaussian is 1e-43 there) with `J0` by its
//! integral representation `(1/pi) integral_0^pi cos(x sin(theta)) dtheta`, which the trapezoid rule on a periodic integrand
//! converges to rounding with more nodes than `x`; no dependency. The solver is nonlinear, so what is compared is the
//! linear solution of a hump of `A / h = 1e-3`, whose nonlinear correction is of that order.
use sr_sim::ocean::{Boundary, Cell, Ocean, Order, Spec};

const G: f64 = 10.0;
const DEPTH: f64 = 10.0;
const SIGMA: f64 = 8.0;
const AMPLITUDE: f64 = 0.01;
const TIME: f64 = 4.0;
/// Half the side of the basin: the first reflection reaches the hump's centre at `2 * 80 / c = 16 s`, and the front of the
/// wave the walls at `80 / c = 8 s`, after the time asked for.
const HALF: f64 = 80.0;

fn j0(x: f64) -> f64 {
    let nodes = 320;
    (0..nodes).map(|i| (x * (std::f64::consts::PI * (i as f64 + 0.5) / nodes as f64).sin()).cos()).sum::<f64>()
        / nodes as f64
}

/// `eta(r, t) / A` by Simpson's rule on 1200 intervals of `s`.
fn exact(r: f64, t: f64) -> f64 {
    let c = (G * DEPTH).sqrt();
    let (intervals, top) = (1200, 14.0);
    let h = top / intervals as f64;
    let f = |s: f64| s * (-0.5 * s * s).exp() * j0(s * r / SIGMA) * (c * s * t / SIGMA).cos();
    let mut sum = f(0.0) + f(top);
    for i in 1..intervals {
        sum += f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    sum * h / 3.0
}

/// The exact profile on a table of radii every 0.25, read back by cubic interpolation (error under 1e-5 of the amplitude).
struct Profile(Vec<f64>);

const STEP: f64 = 0.25;

impl Profile {
    fn new(t: f64) -> Profile {
        Profile((0..=480).map(|i| exact(i as f64 * STEP, t)).collect())
    }
    fn at(&self, r: f64) -> f64 {
        let x = r / STEP;
        let i = (x.floor() as usize).clamp(1, 478);
        let f = x - i as f64;
        let (a, b, c, d) = (self.0[i - 1], self.0[i], self.0[i + 1], self.0[i + 2]);
        b + 0.5 * f * (c - a + f * (2.0 * a - 5.0 * b + 4.0 * c - d + f * (3.0 * (b - c) + d - a)))
    }
}

/// The largest error in the surface, over the cells, as a fraction of the amplitude, and the relative change of the
/// water's volume, for `sigma / dx` cells to a hump width.
fn error(order: Order, dx: f64, exact: &Profile) -> (f64, f64) {
    let n = (2.0 * HALF / dx) as usize;
    let spec = Spec {
        cells: [n, n],
        cell_size: dx,
        origin: [-HALF, -HALF],
        dt: 0.05,
        order,
        gravity: G,
        boundary: Boundary::Closed,
        max_work: 1 << 44,
        ..Default::default()
    };
    let centre = |i: usize| (-HALF + (i % n) as f64 * dx + 0.5 * dx, -HALF + (i / n) as f64 * dx + 0.5 * dx);
    let hump = |i: usize| {
        let (x, z) = centre(i);
        AMPLITUDE * (-(x * x + z * z) / (2.0 * SIGMA * SIGMA)).exp()
    };
    let cells: Vec<Cell> = (0..n * n).map(|i| Cell { depth: DEPTH + hump(i), velocity: [0.0; 2] }).collect();
    let volume = |cells: &[Cell]| cells.iter().map(|c| c.depth).sum::<f64>() * dx * dx;
    let before = volume(&cells);
    let mut ocean = Ocean::new(spec, vec![DEPTH; n * n], cells, vec![]).unwrap();
    let frame = ocean.at(TIME).unwrap();
    let worst = frame
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let (x, z) = centre(i);
            (cell.depth - DEPTH - AMPLITUDE * exact.at(x.hypot(z))).abs()
        })
        .fold(0.0, f64::max);
    (worst / AMPLITUDE, ((volume(&frame.cells) - before) / before).abs())
}

#[test]
fn the_exact_profile_is_the_hump_at_zero_and_a_wave_that_has_moved_out_later() {
    let at_zero = Profile::new(0.0);
    for r in [0.0, 5.0, 12.0, 30.0] {
        let hump = (-(r * r) / (2.0 * SIGMA * SIGMA)).exp();
        assert!((at_zero.at(r) - hump).abs() < 1e-5, "r = {r}: {} against {hump}", at_zero.at(r));
    }
    // later its crest is near c t from the centre, and lower than the hump by the spreading in two dimensions
    let later = Profile::new(TIME);
    let (crest, at) =
        (0..=480).map(|i| (later.0[i], i as f64 * STEP)).fold((0.0, 0.0), |m, v| if v.0 > m.0 { v } else { m });
    println!("REFERENCE crest {crest:.4} of the amplitude at r = {at:.2}, c t = {}", (G * DEPTH).sqrt() * TIME);
    assert!((at - (G * DEPTH).sqrt() * TIME).abs() < 6.0 && crest > 0.1 && crest < 0.2);
}

#[test]
fn the_ocean_reproduces_the_exact_linear_wave_and_converges_at_the_scheme_s_order() {
    let started = std::time::Instant::now();
    let exact = Profile::new(TIME);
    let (mut first, mut second) = (Vec::new(), Vec::new());
    // sigma / dx = 4, 8 and 16
    for dx in [2.0, 1.0, 0.5] {
        let (a, volume_a) = error(Order::First, dx, &exact);
        let (b, volume_b) = error(Order::Second, dx, &exact);
        println!("REFERENCE sigma/dx {}: first order {a:.3e} of the amplitude, second {b:.3e}; volume changes {volume_a:.1e}, {volume_b:.1e}", SIGMA / dx);
        assert!(volume_a < 1e-12 && volume_b < 1e-12, "the closed basin keeps its water: {volume_a:e} {volume_b:e}");
        first.push(a);
        second.push(b);
    }
    println!("REFERENCE took {:.1} s", started.elapsed().as_secs_f64());
    // by the largest error at sigma / dx = 16 and the order over the two halvings from 4
    assert!(second[2] < 2e-3, "second order: {second:?}");
    assert!(first[2] < 4e-2, "first order: {first:?}");
    let order = |e: &[f64]| (e[0] / e[2]).log2() / 2.0;
    assert!(order(&first) > 0.6, "first order: {first:?}");
    assert!(order(&second) > 1.5, "second order: {second:?}");
}
