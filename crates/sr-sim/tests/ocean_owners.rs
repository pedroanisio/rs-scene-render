//! Bodies are tagged per cell by the driver, and at the end of every canonical step the driver is
//! offered, per body, the momentum it gave the water and the water it stands in.
use sr_sim::ocean::{BodySample, Boundary, Cell, Error, Forcing, Ocean, Order, Spec};

const N: usize = 8;

fn spec(order: Order, owners: usize, checkpoint_bytes: usize) -> Spec {
    Spec {
        cells: [N, N],
        cell_size: 1.0,
        origin: [10.0, -20.0],
        dt: 0.1,
        order,
        moving_bed: true,
        bodies: true,
        body_owners: owners,
        boundary: Boundary::Periodic,
        max_work: 1 << 40,
        checkpoint_bytes,
        ..Default::default()
    }
}

fn flat(spec: Spec) -> Ocean {
    Ocean::new(spec, vec![4.0; N * N], vec![Cell { depth: 4.0, velocity: [0.0; 2] }; N * N], vec![]).unwrap()
}

/// What a driver was offered with each completed step.
type Offers = Vec<(u64, Vec<BodySample>)>;

/// Body 0 over the left half (thickness 2, velocity (1, 0)) and body 1 over the right (thickness 1,
/// velocity (-2, 0.5)), over a flat bed of 4.
fn two_bodies(offers: &mut Offers) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> + '_ {
    move |_, f| {
        for c in 0..N * N {
            let left = c % N < N / 2;
            let (thickness, velocity, owner) = if left { (2.0, [1.0, 0.0], 0) } else { (1.0, [-2.0, 0.5], 1) };
            f.bed[c] = 4.0 - thickness;
            f.occupancy[c] = thickness;
            f.velocity[c] = velocity;
            f.owner[c] = owner;
        }
        if let Some((k, _)) = f.exchange {
            offers.push((k, f.bodies.clone()));
        }
        Ok(())
    }
}

#[test]
fn what_each_body_gave_adds_up_to_what_all_of_them_gave() {
    for order in [Order::First, Order::Second] {
        let mut ocean = flat(spec(order, 2, 0));
        let mut offers = Vec::new();
        for k in 1..=4 {
            ocean.at_driven(k as f64 * 0.1, &mut two_bodies(&mut offers)).unwrap();
            let total = ocean.exchanged_impulse();
            let (_, bodies) = offers.iter().find(|(j, _)| *j == k).expect("offered");
            assert_eq!(bodies.iter().map(|b| b.owner).collect::<Vec<_>>(), [0, 1], "{order:?} step {k}");
            for a in 0..2 {
                let sum: f64 = bodies.iter().map(|b| b.impulse[a]).sum();
                assert!(
                    (sum - total[a]).abs() <= 1e-12 * total[a].abs().max(1.0),
                    "{order:?} step {k}: {sum} vs {total:?}"
                );
            }
            // body 0 pushes the water toward +x, body 1 toward -x
            assert!(bodies[0].impulse[0] > 0.0 && bodies[1].impulse[0] < 0.0, "{order:?} step {k}: {bodies:?}");
            assert!(bodies[1].impulse[1] > 0.0, "body 1 also moves in z");
        }
        let (_, first) = &offers[0];
        assert!(first.iter().all(|b| b.impulse == [0.0; 2]), "nothing has been given before the first step");
    }
}

#[test]
fn a_single_body_gave_what_all_the_bodies_gave() {
    let mut ocean = flat(spec(Order::Second, 1, 0));
    let mut offers = Vec::new();
    let mut alone = |_: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(2.0);
        f.occupancy.fill(2.0);
        f.velocity.fill([1.5, -0.5]);
        f.owner.fill(0);
        if let Some((k, _)) = f.exchange {
            offers.push((k, f.bodies.clone()));
        }
        Ok(())
    };
    ocean.at_driven(0.35, &mut alone).unwrap();
    let total = ocean.exchanged_impulse();
    let (_, bodies) = offers.iter().find(|(k, _)| *k == 3).unwrap();
    assert_eq!(bodies.len(), 1);
    // summed in another order (per cell and substep against per substep), so equal to rounding
    for a in 0..2 {
        assert!(
            (bodies[0].impulse[a] - total[a]).abs() <= 1e-12 * total[a].abs(),
            "{:?} vs {total:?}",
            bodies[0].impulse
        );
    }
    assert_eq!(bodies[0].columns, N * N);
    assert_eq!(bodies[0].wet, N * N);
}

#[test]
fn a_replay_offers_the_same_samples_bit_for_bit_with_and_without_checkpoints() {
    for (order, checkpoints) in [(Order::First, 0), (Order::Second, 0), (Order::Second, 1 << 20)] {
        let mut ocean = flat(spec(order, 2, checkpoints));
        let mut first = Offers::new();
        ocean.at_driven(0.5, &mut two_bodies(&mut first)).unwrap();
        let mut replay = Offers::new();
        ocean.at_driven(0.2, &mut two_bodies(&mut replay)).unwrap();
        ocean.at_driven(0.5, &mut two_bodies(&mut replay)).unwrap();
        assert!(!replay.is_empty());
        for (k, bodies) in &replay {
            let kept = &first.iter().find(|(j, _)| j == k).expect("same step").1;
            assert_eq!(bodies, kept, "{order:?} checkpoints {checkpoints}: replay of step {k}");
        }
    }
}

#[test]
fn tagging_does_not_change_the_water() {
    for order in [Order::First, Order::Second] {
        let mut plain = flat(spec(order, 0, 0));
        let mut tagged = flat(spec(order, 2, 0));
        let mut ignored = Offers::new();
        let mut untagged = |_: f64, f: &mut Forcing| -> Result<(), Error> {
            for c in 0..N * N {
                let left = c % N < N / 2;
                let (thickness, velocity) = if left { (2.0, [1.0, 0.0]) } else { (1.0, [-2.0, 0.5]) };
                f.bed[c] = 4.0 - thickness;
                f.occupancy[c] = thickness;
                f.velocity[c] = velocity;
            }
            assert!(f.owner.is_empty() && f.bodies.is_empty());
            Ok(())
        };
        let a = plain.at_driven(0.7, &mut untagged).unwrap().clone();
        let b = tagged.at_driven(0.7, &mut two_bodies(&mut ignored)).unwrap().clone();
        assert_eq!(a.cells, b.cells, "{order:?}");
        assert_eq!(plain.exchanged_impulse(), tagged.exchanged_impulse());
    }
}

/// A patch of cells `x in [x0, x1) , z in [z0, z1)` holding one body, over a sloped free surface.
fn sloped(patch: [usize; 4]) -> (Ocean, impl FnMut(f64, &mut Forcing) -> Result<(), Error>) {
    let s = spec(Order::First, 1, 0);
    // The depth is 3 - 0.1 x - 0.2 z at the cell centres (x, z in the domain's coordinates), and the
    // water moves at (0.3, -0.2).
    let centre = |c: usize| (10.0 + (c % N) as f64 + 0.5, -20.0 + (c / N) as f64 + 0.5);
    let cells: Vec<Cell> = (0..N * N)
        .map(|c| {
            let (x, z) = centre(c);
            Cell { depth: 3.0 - 0.1 * (x - 10.0) - 0.2 * (z + 20.0), velocity: [0.3, -0.2] }
        })
        .collect();
    let ocean = Ocean::new(s, vec![4.0; N * N], cells, vec![]).unwrap();
    let driver = move |_: f64, f: &mut Forcing| -> Result<(), Error> {
        for c in 0..N * N {
            let (ix, iz) = (c % N, c / N);
            let inside = (patch[0]..patch[1]).contains(&ix) && (patch[2]..patch[3]).contains(&iz);
            f.occupancy[c] = if inside { 0.5 } else { 0.0 };
            f.bed[c] = 4.0 - f.occupancy[c];
            f.velocity[c] = [0.0; 2];
            f.owner[c] = 0;
        }
        Ok(())
    };
    (ocean, driver)
}

#[test]
fn the_surface_under_a_body_is_the_plane_through_the_water_of_its_footprint() {
    let (mut ocean, driver) = sloped([2, 6, 1, 5]);
    let mut offers = Offers::new();
    let mut driver = driver;
    ocean
        .at_driven(0.0, &mut |t: f64, f: &mut Forcing| {
            driver(t, f)?;
            if let Some((k, _)) = f.exchange {
                offers.push((k, f.bodies.clone()));
            }
            Ok(())
        })
        .unwrap();
    let (_, bodies) = &offers[0];
    assert_eq!(bodies.len(), 1);
    let b = &bodies[0];
    assert_eq!((b.columns, b.wet), (16, 16));
    // footprint centre: x = 10 + 4, z = -20 + 3
    assert!((b.centroid[0] - 14.0).abs() < 1e-12 && (b.centroid[1] + 17.0).abs() < 1e-12, "{:?}", b.centroid);
    // surface y = (bed - thickness) - depth = 3.5 - (3 - 0.1 x' - 0.2 z'), x', z' from the corner: rises with 0.1 and 0.2
    let at_centroid = 3.5 - (3.0 - 0.1 * 4.0 - 0.2 * 3.0);
    assert!((b.surface[0] - at_centroid).abs() < 1e-12, "{:?} against {at_centroid}", b.surface);
    assert!((b.surface[1] - 0.1).abs() < 1e-12 && (b.surface[2] - 0.2).abs() < 1e-12, "{:?}", b.surface);
    assert!((b.velocity[0] - 0.3).abs() < 1e-12 && (b.velocity[1] + 0.2).abs() < 1e-12, "{:?}", b.velocity);
    // the bed counts the body's thickness as bed: 3.5 + 0.5
    assert!((b.bed - 4.0).abs() < 1e-12, "{}", b.bed);
}

#[test]
fn a_footprint_without_extent_in_one_direction_has_no_slope_there() {
    // one row of cells along x: the slope across it cannot be known
    let (mut ocean, mut driver) = sloped([1, 7, 3, 4]);
    let mut seen = Vec::new();
    ocean
        .at_driven(0.0, &mut |t: f64, f: &mut Forcing| {
            driver(t, f)?;
            if f.exchange.is_some() {
                seen = f.bodies.clone();
            }
            Ok(())
        })
        .unwrap();
    assert!((seen[0].surface[1] - 0.1).abs() < 1e-12 && seen[0].surface[2] == 0.0, "{:?}", seen[0].surface);
    // a single cell: no slope at all, and its own level
    let (mut ocean, mut driver) = sloped([2, 3, 2, 3]);
    ocean
        .at_driven(0.0, &mut |t: f64, f: &mut Forcing| {
            driver(t, f)?;
            if f.exchange.is_some() {
                seen = f.bodies.clone();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!((seen[0].columns, seen[0].surface[1], seen[0].surface[2]), (1, 0.0, 0.0));
    let (x, z) = (10.0 + 2.5, -20.0 + 2.5);
    assert!((seen[0].centroid[0] - x).abs() < 1e-12 && (seen[0].centroid[1] - z).abs() < 1e-12);
}

#[test]
fn a_tag_the_solver_does_not_know_is_an_error() {
    let mut ocean = flat(spec(Order::First, 2, 0));
    let result = ocean.at_driven(0.1, &mut |_: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(2.0);
        f.occupancy.fill(2.0);
        f.owner.fill(2);
        Ok(())
    });
    assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
    // and so is asking for tags without bodies
    let bad = Spec { bodies: false, body_owners: 1, ..spec(Order::First, 1, 0) };
    assert!(Ocean::new(bad, vec![4.0; N * N], vec![Cell { depth: 4.0, velocity: [0.0; 2] }; N * N], vec![]).is_err());
}
