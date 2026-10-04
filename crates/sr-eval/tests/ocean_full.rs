//! A rigid body that moves through an ocean that couples to it with `full` gives the water
//! momentum and gets the same back: the body and the water together keep their momentum until the
//! waves reach the walls, and the body slows down as it pushes the water.

use sr_eval::Evaluator;

const RHO: f64 = 1000.0;
/// A ball of radius 2 and half the density of water floats half under.
const MASS: f64 = 16755.16;

struct Basin {
    coupling: &'static str,
    /// The ocean's canonical step.
    dt: f64,
    /// Metres a second along x.
    speed: f64,
}

impl Default for Basin {
    fn default() -> Self {
        Basin { coupling: "full", dt: 0.05, speed: 3.0 }
    }
}

impl Basin {
    fn xml(&self) -> String {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="20" duration="4"/><composition>
              <object3D id="ball" primitive="sphere" radius="2" segments="24" x="-10" y="0" z="1">
                <rigidBody shape="sphere" mass="{MASS}" velocityX="{speed}" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" bedResponse="hydrostatic" width="160" depth="160" cellSize="2" bottomDepth="10" dt="{dt}" boundary="closed" colliders="ball" bodyCoupling="{coupling}"/>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##,
            speed = self.speed,
            dt = self.dt,
            coupling = self.coupling,
        )
    }

    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}

/// The ball's x at `t` and the momentum of the water along x and z, kilograms metres a second.
fn at(ev: &Evaluator, t: f64) -> (f64, [f64; 2]) {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    let x = frame.nodes.iter().find(|n| &*n.id == "ball").unwrap().pose3.expect("simulated")[12];
    let sea = frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    let mut water = [0.0; 2];
    for cell in &sea.frame.cells {
        for (sum, velocity) in water.iter_mut().zip(cell.velocity) {
            *sum += RHO * cell.depth * velocity * 4.0;
        }
    }
    (x, water)
}

/// The speed of the ball at `t` from its positions a tenth of a second either side.
fn speed(ev: &Evaluator, t: f64) -> f64 {
    (at(ev, t + 0.05).0 - at(ev, t - 0.05).0) / 0.1
}

#[test]
fn the_ball_and_the_water_keep_their_momentum_to_a_step_that_falls_with_the_step() {
    let mut errors = Vec::new();
    for dt in [0.1, 0.05, 0.025] {
        let basin = Basin { dt, ..Basin::default() };
        let ev = basin.evaluator();
        // before the waves reach the walls: 80 m away at the wave speed of 10 m/s. The water's momentum
        // reaches the ball one step late, so the total is off by what is on its way: the most over
        // three instants is the measure.
        let mut worst = 0.0f64;
        for t in [1.0, 2.0, 3.0] {
            let (_, water) = at(&ev, t);
            let ball = MASS * speed(&ev, t);
            let total = [ball + water[0], water[1]];
            let error = (total[0] - MASS * basin.speed).hypot(total[1]) / (MASS * basin.speed);
            println!("FULL dt {dt} t {t}: ball {ball:.0}, water {water:.0?}, error {error:.4} of the ball's momentum");
            worst = worst.max(error);
        }
        errors.push(worst);
    }
    println!("FULL worst error by step: {errors:?}");
    assert!(errors.windows(2).all(|w| w[1] < w[0]), "the error falls with the step: {errors:?}");
    assert!(errors[2] < 0.06, "{errors:?}");
}

#[test]
fn only_a_full_coupling_slows_the_body_and_the_default_does_not() {
    let distance = |coupling: &'static str| {
        let ev = Basin { coupling, ..Basin::default() }.evaluator();
        let x = at(&ev, 2.0).0;
        x - at(&ev, 0.0).0
    };
    let (none, buoyancy, full) = (distance("none"), distance("buoyancy"), distance("full"));
    println!("FULL distance in 2 s: none {none:.3}, buoyancy {buoyancy:.3}, full {full:.3}");
    // without horizontal drag the body slides at its speed, whatever the water does vertically
    assert!((buoyancy - 6.0).abs() < 1e-3 && (none - 6.0).abs() < 1e-3, "{none} {buoyancy}");
    assert!(full < 5.5, "the water slows it: {full}");
}

#[test]
fn any_order_of_instants_and_a_fresh_evaluator_give_the_same_motion_bit_for_bit() {
    let times = [2.0, 0.5, 1.5, 0.0, 2.5, 1.0, 2.0];
    let ev = Basin::default().evaluator();
    let first: Vec<(u64, [u64; 2])> = times
        .iter()
        .map(|&t| {
            let (x, w) = at(&ev, t);
            (x.to_bits(), w.map(f64::to_bits))
        })
        .collect();
    let fresh = Basin::default().evaluator();
    for (k, &t) in times.iter().enumerate() {
        let (x, w) = at(&fresh, t);
        assert_eq!(first[k], (x.to_bits(), w.map(f64::to_bits)), "t = {t}");
    }
}
