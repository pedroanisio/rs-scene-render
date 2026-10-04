//! A body gives the water momentum by a push the driver states per canonical step; the solver
//! applies it evenly, column by column, never past the body's velocity, and credits what it applied.
use sr_sim::ocean::{BodySample, Boundary, Cell, Error, Forcing, Ocean, Order, Push, Spec};

const N: usize = 8;
const DT: f64 = 0.1;
const DEPTH: f64 = 4.0;

fn spec(order: Order, checkpoint_bytes: usize) -> Spec {
    Spec {
        cells: [N, N],
        cell_size: 1.0,
        origin: [0.0, 0.0],
        dt: DT,
        order,
        moving_bed: true,
        bodies: true,
        body_owners: 2,
        body_push: true,
        boundary: Boundary::Periodic,
        max_work: 1 << 40,
        checkpoint_bytes,
        ..Default::default()
    }
}
fn basin(spec: Spec) -> Ocean {
    Ocean::new(spec, vec![DEPTH; N * N], vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N], vec![]).unwrap()
}
/// A push of `momentum` for body 0 at velocity `target` over the first `columns` columns, evenly.
fn push(owner: u32, target: [f64; 2], momentum: [f64; 2], columns: usize) -> Push {
    Push { owner, target, momentum, columns: (0..columns as u32).map(|c| (c, 1.0 / columns as f64)).collect() }
}
type Offers = Vec<(u64, Vec<BodySample>)>;
/// A driver that gives `pushes` in every step, over a flat bed with the body's occupancy marked.
fn gives(pushes: Vec<Push>, offers: &mut Offers) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> + '_ {
    move |_, f| {
        f.bed.fill(DEPTH);
        f.occupancy.fill(0.0);
        // the body is marked in four columns but leaves the bed flat, so that nothing else pushes the water
        for c in 0..4 {
            f.occupancy[c] = 1.0;
        }
        f.owner.fill(0);
        f.pushes = pushes.clone();
        if let Some((k, _)) = f.exchange {
            offers.push((k, f.bodies.clone()));
        }
        Ok(())
    }
}
fn momentum(ocean: &Ocean) -> [f64; 2] {
    ocean.frame().cells.iter().fold([0.0; 2], |m, c| [m[0] + c.depth * c.velocity[0], m[1] + c.depth * c.velocity[1]])
}

#[test]
fn the_water_gains_exactly_what_the_push_gives_and_the_body_is_credited_with_it() {
    for order in [Order::First, Order::Second] {
        let mut ocean = basin(spec(order, 0));
        let mut offers = Offers::new();
        // 10 per step spread over four columns; the body moves so fast that nothing caps it
        {
            let mut driver = gives(vec![push(0, [100.0, -50.0], [10.0, -4.0], 4)], &mut offers);
            ocean.at_driven(0.2, &mut driver).unwrap();
        }
        // two steps: all of it given, in the water's momentum (per unit density, times the cell area of 1)
        let gained = momentum(&ocean);
        let given = ocean.exchanged_impulse();
        assert!((given[0] - 10.0).abs() < 1e-12 && (given[1] + 4.0).abs() < 1e-12, "{order:?}: {given:?}");
        let (_, bodies) = offers.iter().find(|(k, _)| *k == 1).unwrap();
        assert!((bodies[0].impulse[0] - 10.0).abs() < 1e-12, "{order:?}: {:?}", bodies[0]);
        // the water gained it in total (periodic edges: nothing else changes the total momentum)
        let total = gained;
        assert!((total[0] - 20.0).abs() < 1e-9 && (total[1] + 8.0).abs() < 1e-9, "{order:?}: {total:?}");
    }
}

#[test]
fn a_column_is_never_pushed_past_the_velocity_of_the_body() {
    let mut ocean = basin(spec(Order::First, 0));
    let mut offers = Offers::new();
    // asks for 1000 over four columns at a body velocity of 0.5: each column may take 0.5 * 4 = 2
    {
        let mut driver = gives(vec![push(0, [0.5, 0.0], [1000.0, 0.0], 4)], &mut offers);
        ocean.at_driven(0.3, &mut driver).unwrap();
    }
    for c in 0..4 {
        let cell = ocean.frame().cells[c];
        assert!(cell.velocity[0] <= 0.5 + 1e-12 && cell.velocity[0] > 0.0, "column {c}: {:?}", cell.velocity);
    }
    let given = ocean.exchanged_impulse();
    assert!(given[0] > 0.0 && given[0] < 1000.0 * 0.5, "what was applied, not what was asked: {given:?}");
    let (_, bodies) = offers.iter().find(|(k, _)| *k == 2).unwrap();
    assert!((bodies[0].impulse[0] - given[0]).abs() < 1e-9 * given[0], "{:?} against {given:?}", bodies[0]);
    // a body that moves against the push's sign gives nothing: the water is already past it
    let mut ocean = basin(spec(Order::First, 0));
    let mut sink = Offers::new();
    let mut driver = gives(vec![push(0, [-1.0, 0.0], [5.0, 0.0], 4)], &mut sink);
    ocean.at_driven(0.2, &mut driver).unwrap();
    assert_eq!(ocean.exchanged_impulse(), [0.0; 2]);
}

#[test]
fn a_basin_with_no_water_takes_nothing() {
    // all dry: the pushes find no water to move and nothing is credited
    let mut ocean = Ocean::new(
        spec(Order::First, 0),
        vec![DEPTH; N * N],
        vec![Cell { depth: 0.0, velocity: [0.0; 2] }; N * N],
        vec![],
    )
    .unwrap();
    let mut sink = Offers::new();
    let mut driver = gives(vec![push(0, [100.0, 0.0], [8.0, 0.0], 4)], &mut sink);
    ocean.at_driven(0.2, &mut driver).unwrap();
    assert_eq!(ocean.exchanged_impulse(), [0.0; 2]);
}

#[test]
fn a_replay_gives_the_same_water_and_the_same_credit_bit_for_bit() {
    for (order, checkpoints) in [(Order::First, 0), (Order::Second, 0), (Order::Second, 1 << 20)] {
        let mut ocean = basin(spec(order, checkpoints));
        let mut first = Offers::new();
        ocean.at_driven(0.5, &mut gives(vec![push(0, [3.0, 1.0], [2.0, 0.5], 4)], &mut first)).unwrap();
        let cells = ocean.frame().cells.clone();
        let mut replay = Offers::new();
        ocean.at_driven(0.2, &mut gives(vec![push(0, [3.0, 1.0], [2.0, 0.5], 4)], &mut replay)).unwrap();
        ocean.at_driven(0.5, &mut gives(vec![push(0, [3.0, 1.0], [2.0, 0.5], 4)], &mut replay)).unwrap();
        assert_eq!(ocean.frame().cells, cells, "{order:?} checkpoints {checkpoints}");
        for (k, bodies) in &replay {
            assert_eq!(bodies, &first.iter().find(|(j, _)| j == k).unwrap().1, "{order:?}: step {k}");
        }
    }
}

#[test]
fn without_a_push_a_body_in_push_mode_gives_the_water_nothing() {
    let mut ocean = basin(spec(Order::First, 0));
    ocean.at_driven(0.3, &mut gives(vec![], &mut Offers::new())).unwrap();
    assert_eq!(ocean.exchanged_impulse(), [0.0; 2]);
    // the raised columns of the occupancy settle, but no momentum was handed over
    assert!(ocean.frame().cells.iter().all(|c| c.depth.is_finite()));
}

#[test]
fn bad_pushes_and_bad_settings_are_errors() {
    let bad = |p: Push| {
        let mut ocean = basin(spec(Order::First, 0));
        ocean.at_driven(0.2, &mut gives(vec![p], &mut Offers::new())).is_err()
    };
    assert!(bad(push(5, [1.0, 0.0], [1.0, 0.0], 4)), "an owner the solver does not know");
    assert!(
        bad(Push { columns: vec![(0, 0.4), (1, 0.4)], ..push(0, [1.0, 0.0], [1.0, 0.0], 4) }),
        "shares that do not add up"
    );
    assert!(bad(Push { columns: vec![(999, 1.0)], ..push(0, [1.0, 0.0], [1.0, 0.0], 4) }), "a column outside the grid");
    assert!(bad(push(0, [f64::NAN, 0.0], [1.0, 0.0], 4)));
    // pushes without the setting
    let mut off = basin(Spec { body_push: false, ..spec(Order::First, 0) });
    assert!(off.at_driven(0.2, &mut gives(vec![push(0, [1.0, 0.0], [1.0, 0.0], 4)], &mut Offers::new())).is_err());
    // the setting without owners
    let no_owners = Spec { body_owners: 0, ..spec(Order::First, 0) };
    assert!(Ocean::new(no_owners, vec![DEPTH; N * N], vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N], vec![])
        .is_err());
}
