//! What falls into the water in a canonical step arrives as a sparse event: volume taken from a cell and
//! given to its neighbours, momentum added to the cell, applied when the step reaches its end.
use sr_sim::ocean::{Boundary, Cell, Error, Forcing, Ocean, Order, Spec, SplashCell, CAVITY_SHARE};

const N: usize = 8;
const DT: f64 = 0.1;
const DEPTH: f64 = 4.0;

fn spec(order: Order, checkpoint_bytes: usize) -> Spec {
    Spec {
        cells: [N, N],
        cell_size: 0.5,
        dt: DT,
        order,
        moving_bed: true,
        boundary: Boundary::Closed,
        max_work: 1 << 40,
        checkpoint_bytes,
        ..Default::default()
    }
}
fn basin(spec: Spec) -> Ocean {
    Ocean::new(spec, vec![DEPTH; N * N], vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N], vec![]).unwrap()
}
/// Gives `cells` in the step that ends at `at`, over a flat bed.
fn drops(at: f64, cells: Vec<SplashCell>) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |time, f| {
        f.bed.fill(DEPTH);
        if (time - at).abs() < 1e-9 {
            f.splash = cells.clone();
        }
        Ok(())
    }
}
fn cell(c: usize, volume: f64, momentum: [f64; 2]) -> SplashCell {
    SplashCell { cell: c as u32, volume, momentum }
}
fn water(o: &Ocean) -> f64 {
    o.frame().cells.iter().map(|c| c.depth).sum::<f64>() * 0.25
}
fn momentum(o: &Ocean) -> [f64; 2] {
    o.frame()
        .cells
        .iter()
        .fold([0.0; 2], |m, c| [m[0] + c.depth * c.velocity[0] * 0.25, m[1] + c.depth * c.velocity[1] * 0.25])
}

#[test]
fn the_volume_goes_from_the_cell_to_its_neighbours_and_the_momentum_stays_in_it() {
    for order in [Order::First, Order::Second] {
        let mut ocean = basin(spec(order, 0));
        let before = water(&ocean);
        // one cell inside, 0.5 of volume: 2 of depth (cell area 0.25) taken, 0.25 to each of the eight
        let c = 3 * N + 3;
        ocean.at_driven(0.1, &mut drops(0.1, vec![cell(c, 0.5, [0.2, -0.1])])).unwrap();
        let f = ocean.frame();
        assert!((f.cells[c].depth - (DEPTH - 2.0)).abs() < 1e-12, "{order:?}: {}", f.cells[c].depth);
        for (dx, dz) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let n = (3 + dz) as usize * N + (3 + dx) as usize;
            assert!(
                (f.cells[n].depth - (DEPTH + 0.25)).abs() < 1e-12,
                "{order:?} neighbour ({dx},{dz}): {}",
                f.cells[n].depth
            );
        }
        assert!((water(&ocean) - before).abs() < 1e-12 * before);
        let m = momentum(&ocean);
        assert!((m[0] - 0.2 / 0.25 * 0.25).abs() < 1e-12 && (m[1] + 0.1).abs() < 1e-12, "{order:?}: {m:?}");
    }
}

#[test]
fn a_cell_gives_at_most_what_the_layer_allows_and_the_rest_is_dropped() {
    let mut ocean = basin(spec(Order::First, 0));
    let before = water(&ocean);
    let c = 3 * N + 3;
    // a thousand times the volume the cell holds
    ocean.at_driven(0.1, &mut drops(0.1, vec![cell(c, 1000.0, [0.0; 2])])).unwrap();
    let f = ocean.frame();
    assert!((f.cells[c].depth - (1.0 - CAVITY_SHARE) * DEPTH).abs() < 1e-12, "{}", f.cells[c].depth);
    assert!((water(&ocean) - before).abs() < 1e-12 * before, "what is taken is all given");
}

#[test]
fn at_the_edge_and_in_the_corner_the_neighbours_that_exist_share_it() {
    let mut ocean = basin(spec(Order::First, 0));
    let before = water(&ocean);
    // the corner has three neighbours: 2 of depth, 2/3 to each
    ocean.at_driven(0.1, &mut drops(0.1, vec![cell(0, 0.5, [0.0; 2]), cell(N * N - 1, 0.25, [0.0; 2])])).unwrap();
    let f = ocean.frame();
    assert!((f.cells[1].depth - (DEPTH + 2.0 / 3.0)).abs() < 1e-12);
    assert!((f.cells[N].depth - (DEPTH + 2.0 / 3.0)).abs() < 1e-12);
    assert!((f.cells[N + 1].depth - (DEPTH + 2.0 / 3.0)).abs() < 1e-12);
    assert!((water(&ocean) - before).abs() < 1e-12 * before);
}

#[test]
fn nothing_happens_before_the_end_of_the_step_and_a_replay_is_identical() {
    for (order, checkpoints) in [(Order::First, 0), (Order::Second, 0), (Order::Second, 1 << 20)] {
        let mut ocean = basin(spec(order, checkpoints));
        let c = 3 * N + 3;
        let mut d = drops(0.3, vec![cell(c, 0.5, [0.2, 0.0])]);
        // up to 0.2 s the sample of step 3 has not been used, and a time inside step 3 does not see it
        assert!(ocean.at_driven(0.25, &mut d).unwrap().cells.iter().all(|x| x.depth == DEPTH));
        assert_eq!(ocean.frame().cells[c].velocity, [0.0; 2]);
        let after = ocean.at_driven(0.5, &mut d).unwrap().clone();
        assert!(after.cells[c].depth != DEPTH);
        let early = ocean.at_driven(0.2, &mut d).unwrap().clone();
        assert!(early.cells.iter().all(|x| x.depth == DEPTH), "{order:?}");
        let again = ocean.at_driven(0.5, &mut d).unwrap().clone();
        assert_eq!(after.cells, again.cells, "{order:?} checkpoints {checkpoints}");
    }
}

#[test]
fn no_splash_changes_nothing_and_a_dry_cell_takes_none() {
    let mut a = basin(spec(Order::First, 0));
    let mut b = basin(spec(Order::First, 0));
    a.at_driven(0.4, &mut drops(9.0, vec![])).unwrap();
    b.at_driven(0.4, &mut |_: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(DEPTH);
        Ok(())
    })
    .unwrap();
    assert_eq!(a.frame().cells, b.frame().cells);
    // a basin that holds no water has none to give
    let mut dry = Ocean::new(
        spec(Order::First, 0),
        vec![DEPTH; N * N],
        vec![Cell { depth: 0.0, velocity: [0.0; 2] }; N * N],
        vec![],
    )
    .unwrap();
    dry.at_driven(0.2, &mut drops(0.1, vec![cell(10, 1.0, [1.0, 0.0])])).unwrap();
    assert!(dry.frame().cells.iter().all(|c| c.depth == 0.0 && c.velocity == [0.0; 2]));
}

#[test]
fn bad_splashes_are_errors() {
    let bad = |cells: Vec<SplashCell>| {
        let mut ocean = basin(spec(Order::First, 0));
        ocean.at_driven(0.2, &mut drops(0.1, cells)).is_err()
    };
    assert!(bad(vec![cell(N * N, 1.0, [0.0; 2])]), "a cell outside the grid");
    assert!(bad(vec![cell(5, 1.0, [0.0; 2]), cell(5, 1.0, [0.0; 2])]), "a cell twice");
    assert!(bad(vec![cell(6, 1.0, [0.0; 2]), cell(5, 1.0, [0.0; 2])]), "not sorted");
    assert!(bad(vec![cell(5, -1.0, [0.0; 2])]), "a negative volume");
    assert!(bad(vec![cell(5, f64::NAN, [0.0; 2])]));
    assert!(bad(vec![cell(5, 1.0, [f64::INFINITY, 0.0])]));
}
