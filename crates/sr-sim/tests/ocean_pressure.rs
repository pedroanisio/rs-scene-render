//! The horizontal momentum that the slope of the bed a body raises gives the water, credited to the body.
use sr_sim::ocean::{BodySample, Boundary, Cell, Error, Forcing, Lift, Ocean, Order, Spec};

const N: usize = 64;
const DX: f64 = 1.0;
const DT: f64 = 0.05;
const DEPTH: f64 = 6.0;

fn spec(order: Order) -> Spec {
    Spec {
        cells: [N, N],
        cell_size: DX,
        origin: [0.0, 0.0],
        dt: DT,
        order,
        moving_bed: true,
        bodies: true,
        body_owners: 2,
        body_push: true,
        boundary: Boundary::Periodic,
        max_work: 1 << 40,
        ..Default::default()
    }
}
fn basin(order: Order) -> Ocean {
    Ocean::new(spec(order), vec![DEPTH; N * N], vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N], vec![]).unwrap()
}
/// A Gaussian mound of height `height` and width `sigma` that sits at x = `x0 + speed t`, z = 32.
fn mound(owner: u32, x0: f64, speed: f64, height: f64, sigma: f64, t: f64) -> (Lift, Vec<f64>) {
    let mut columns = Vec::new();
    let mut field = vec![0.0; N * N];
    for (c, cell) in field.iter_mut().enumerate() {
        let (x, z) = ((c % N) as f64 + 0.5, (c / N) as f64 + 0.5);
        let r2 = (x - x0 - speed * t).powi(2) + (z - 32.0).powi(2);
        let h = height * (-r2 / (2.0 * sigma * sigma)).exp();
        if h > 1e-9 {
            columns.push((c as u32, h));
            *cell = h;
        }
    }
    (Lift { owner, columns }, field)
}
type Offers = Vec<(u64, Vec<BodySample>)>;
fn run(order: Order, speed: f64, steps: u64, offers: &mut Offers) -> Ocean {
    let mut ocean = basin(order);
    let mut driver = |time: f64, f: &mut Forcing| -> Result<(), Error> {
        let (lift, field) = mound(0, 16.0, speed, 1.0, 3.0, time);
        for (bed, raised) in f.bed.iter_mut().zip(&field) {
            *bed = DEPTH - raised;
        }
        f.occupancy.fill(0.0);
        f.owner.fill(0);
        f.lifts = vec![lift];
        if let Some((k, _)) = f.exchange {
            offers.push((k, f.bodies.clone()));
        }
        Ok(())
    };
    ocean.at_driven(steps as f64 * DT, &mut driver).unwrap();
    ocean
}
fn momentum(o: &Ocean) -> [f64; 2] {
    o.frame()
        .cells
        .iter()
        .fold([0.0; 2], |m, c| [m[0] + c.depth * c.velocity[0] * DX * DX, m[1] + c.depth * c.velocity[1] * DX * DX])
}

#[test]
fn what_the_water_gains_from_the_slope_of_the_bed_is_credited_to_the_body_that_raised_it() {
    for order in [Order::First, Order::Second] {
        for speed in [0.0, 4.0, 10.0] {
            let mut offers = Offers::new();
            let ocean = run(order, speed, 10, &mut offers);
            let gained = momentum(&ocean);
            let mut credited = [0.0; 2];
            for (k, bodies) in &offers {
                if (1..=10).contains(k) {
                    for b in bodies.iter().filter(|b| b.owner == 0) {
                        credited[0] += b.pressure[0];
                        credited[1] += b.pressure[1];
                    }
                }
            }
            println!("PRESSURE {order:?} speed {speed}: water gained {gained:?}, credited {credited:?}");
            // the mound is still: it only stirs the water about; moving it, the water takes momentum along x
            if speed > 0.0 {
                assert!(credited[0].abs() > 0.0 && gained[0].abs() > 0.0);
                let residual = (gained[0] - credited[0]).abs() / gained[0].abs();
                let allowed = if order == Order::First { 0.08 } else { 0.03 };
                assert!(residual < allowed, "{order:?} speed {speed}: residual {residual}");
            }
        }
    }
}

#[test]
fn without_lifts_nothing_is_credited_and_bad_lifts_are_errors() {
    let mut ocean = basin(Order::First);
    let mut quiet = |_: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(DEPTH);
        f.occupancy.fill(0.0);
        f.owner.fill(0);
        Ok(())
    };
    ocean.at_driven(0.2, &mut quiet).unwrap();
    let bad = |lift: Lift| {
        let mut ocean = basin(Order::First);
        ocean
            .at_driven(0.2, &mut |_: f64, f: &mut Forcing| -> Result<(), Error> {
                f.bed.fill(DEPTH);
                f.occupancy.fill(0.0);
                f.owner.fill(0);
                f.lifts = vec![lift.clone()];
                Ok(())
            })
            .is_err()
    };
    assert!(bad(Lift { owner: 7, columns: vec![(1, 0.5)] }));
    assert!(bad(Lift { owner: 0, columns: vec![(N as u32 * N as u32, 0.5)] }));
    assert!(bad(Lift { owner: 0, columns: vec![(1, -0.5)] }));
    assert!(bad(Lift { owner: 0, columns: vec![(1, f64::NAN)] }));
}

/// A driver of the moving mound that lists what it was offered with every completed step.
fn mound_driver(speed: f64, offers: &mut Offers) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> + '_ {
    move |time, f| {
        let (lift, field) = mound(0, 16.0, speed, 1.0, 3.0, time);
        for (bed, raised) in f.bed.iter_mut().zip(&field) {
            *bed = DEPTH - raised;
        }
        f.occupancy.fill(0.0);
        f.owner.fill(0);
        f.lifts = vec![lift];
        if let Some((k, _)) = f.exchange {
            offers.push((k, f.bodies.clone()));
        }
        Ok(())
    }
}

#[test]
fn the_pressure_of_a_step_is_the_same_whenever_and_from_wherever_the_step_is_computed() {
    // checkpoints one canonical step apart or none: a seek back restarts from a checkpoint or from zero, and the
    // record of every step it offers again must equal the first one to the bit
    for (order, checkpoint_bytes) in
        [(Order::First, 0), (Order::Second, 0), (Order::Second, 1 << 20), (Order::First, 1 << 20)]
    {
        let mut ocean = Ocean::new(
            Spec { checkpoint_bytes, ..spec(order) },
            vec![DEPTH; N * N],
            vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N],
            vec![],
        )
        .unwrap();
        let mut first = Offers::new();
        ocean.at_driven(1.0, &mut mound_driver(8.0, &mut first)).unwrap();
        assert!(first.iter().any(|(_, bodies)| bodies.iter().any(|b| b.pressure[0] != 0.0)));
        for back in [0.0, 0.3, 0.55, 0.95] {
            let mut replay = Offers::new();
            ocean.at_driven(back, &mut mound_driver(8.0, &mut replay)).unwrap();
            ocean.at_driven(1.0, &mut mound_driver(8.0, &mut replay)).unwrap();
            assert!(!replay.is_empty());
            for (k, bodies) in &replay {
                let kept = &first.iter().find(|(j, _)| j == k).expect("same step").1;
                for (a, b) in bodies.iter().zip(kept) {
                    assert_eq!(
                        a.pressure.map(f64::to_bits),
                        b.pressure.map(f64::to_bits),
                        "{order:?} checkpoints {checkpoint_bytes}: step {k} after going back to {back}"
                    );
                }
                assert_eq!(bodies, kept, "{order:?} checkpoints {checkpoint_bytes}: step {k} after {back}");
            }
        }
    }
}

#[test]
fn the_pressure_does_not_depend_on_the_number_of_threads() {
    let record = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| {
            let mut ocean = basin(Order::Second);
            let mut offers = Offers::new();
            ocean.at_driven(1.0, &mut mound_driver(8.0, &mut offers)).unwrap();
            offers
                .iter()
                .flat_map(|(_, bodies)| bodies.iter().flat_map(|b| b.pressure.map(f64::to_bits)))
                .collect::<Vec<u64>>()
        })
    };
    let reference = record(1);
    assert!(reference.iter().any(|b| *b != 0));
    assert_eq!(record(2), reference);
    assert_eq!(record(8), reference);
}
