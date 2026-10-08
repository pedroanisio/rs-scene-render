//! Water impulses the driver asks for, step by step, and the cavity impulse that cannot ask for
//! more water than there is.
use sr_sim::ocean::{Boundary, Cell, Error, Forcing, Impulse, ImpulseKind, Ocean, Order, Spec, CAVITY_SHARE};

const N: usize = 32;
const DT: f64 = 0.05;
const DEPTH: f64 = 4.0;

fn spec(order: Order, moving: bool, checkpoint_bytes: usize) -> Spec {
    Spec {
        cells: [N, N],
        cell_size: 0.5,
        origin: [0.0, 0.0],
        dt: DT,
        order,
        moving_bed: moving,
        boundary: Boundary::Closed,
        max_work: 1 << 40,
        checkpoint_bytes,
        ..Default::default()
    }
}
fn flat(spec: Spec, impulses: Vec<Impulse>) -> Ocean {
    Ocean::new(spec, vec![DEPTH; N * N], vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N], impulses).unwrap()
}
fn cavity(time: f64, amplitude: f64, radius: f64) -> Impulse {
    // a cell centre: (16 + 0.5) * 0.5
    Impulse { time, center: [8.25, 8.25], radius, amplitude, velocity: [0.0; 2], kind: ImpulseKind::Cavity }
}
/// A driver over the flat bed that asks for `events` in the step each belongs to.
fn asks(events: Vec<Impulse>) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> {
    move |time, f| {
        f.bed.fill(DEPTH);
        let from = time - DT;
        f.events = events.iter().filter(|e| e.time > from && e.time <= time && time >= DT).cloned().collect();
        Ok(())
    }
}
fn volume(ocean: &Ocean) -> f64 {
    ocean.frame().cells.iter().map(|c| c.depth).sum::<f64>() * 0.25
}

#[test]
fn a_cavity_moves_the_wanted_water_from_the_disc_to_the_annulus_and_keeps_all_of_it() {
    let mut ocean = flat(spec(Order::First, false, 0), vec![cavity(0.1, 1.0, 4.0)]);
    let before = volume(&ocean);
    ocean.at(0.1).unwrap();
    let centre = 16 * N + 16;
    let frame = ocean.frame();
    assert!((frame.cells[centre].depth - (DEPTH - 1.0)).abs() < 1e-12, "{}", frame.cells[centre].depth);
    // water went to the annulus (radius 2 to 4 from the centre: 4 to 8 cells), and nothing outside it
    let ring = 16 * N + 16 + 6;
    assert!(frame.cells[ring].depth > DEPTH, "{}", frame.cells[ring].depth);
    assert_eq!(frame.cells[16 * N + 16 + 10].depth, DEPTH);
    assert!((volume(&ocean) - before).abs() < 1e-9 * before, "{} against {before}", volume(&ocean));
}

#[test]
fn a_cavity_is_limited_by_the_water_it_finds_and_is_not_an_error() {
    // wanting 10 deep out of 4: it takes CAVITY_SHARE of what the disc holds
    let mut ocean = flat(spec(Order::First, false, 0), vec![cavity(0.1, 10.0, 4.0)]);
    let before = volume(&ocean);
    ocean.at(0.1).unwrap();
    let frame = ocean.frame();
    assert!(frame.cells.iter().all(|c| c.depth >= (1.0 - CAVITY_SHARE) * DEPTH - 1e-12), "no column is emptied");
    let centre = frame.cells[16 * N + 16].depth;
    assert!((centre - (1.0 - CAVITY_SHARE) * DEPTH).abs() < 1e-9, "the centre keeps its tenth: {centre}");
    assert!((volume(&ocean) - before).abs() < 1e-9 * before);
    // the same wish as a negative displacement is an error for lack of donor water
    let displace = Impulse { kind: ImpulseKind::Displace, amplitude: -10.0, ..cavity(0.1, 10.0, 4.0) };
    let bad = Ocean::new(
        spec(Order::First, false, 0),
        vec![DEPTH; N * N],
        vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N],
        vec![displace],
    )
    .and_then(|mut o| o.at(0.1).map(|_| ()));
    assert!(bad.is_err());
}

#[test]
fn a_cavity_in_a_dry_or_unresolved_place_does_nothing() {
    // centred outside the domain, and in a disc smaller than a cell
    let far = Impulse { center: [100.0, 100.0], ..cavity(0.1, 1.0, 4.0) };
    let tiny = cavity(0.1, 1.0, 0.1);
    let mut ocean = flat(spec(Order::First, false, 0), vec![far, tiny]);
    ocean.at(0.2).unwrap();
    assert!(ocean.frame().cells.iter().all(|c| c.depth == DEPTH));
    let negative = cavity(0.1, -1.0, 4.0);
    assert!(Ocean::new(spec(Order::First, false, 0), vec![DEPTH; N * N], vec![Cell::default(); N * N], vec![negative])
        .is_err());
}

#[test]
fn an_impulse_asked_by_the_driver_acts_like_the_authored_one_at_the_same_instant() {
    for order in [Order::First, Order::Second] {
        for time in [0.1, 0.12, 0.137] {
            let mut authored = flat(spec(order, false, 0), vec![cavity(time, 1.5, 4.0)]);
            let mut asked = flat(spec(order, true, 0), vec![]);
            let mut driver = asks(vec![cavity(time, 1.5, 4.0)]);
            for t in [0.05, 0.1, 0.2, 0.31, 0.4] {
                let a = authored.at(t).unwrap().clone();
                let b = asked.at_driven(t, &mut driver).unwrap().clone();
                assert_eq!(a.cells, b.cells, "{order:?} impulse at {time}, frame at {t}");
            }
        }
    }
}

#[test]
fn nothing_happens_before_the_instant_and_a_replay_gives_the_same_frames() {
    for (order, checkpoints) in [(Order::First, 0), (Order::Second, 0), (Order::Second, 1 << 20)] {
        let mut ocean = flat(spec(order, true, checkpoints), vec![]);
        let mut driver = asks(vec![cavity(0.237, 1.0, 4.0)]);
        assert!(ocean.at_driven(0.2, &mut driver).unwrap().cells.iter().all(|c| c.depth == DEPTH));
        let late = ocean.at_driven(0.5, &mut driver).unwrap().clone();
        assert!(late.cells.iter().any(|c| c.depth != DEPTH));
        let early = ocean.at_driven(0.23, &mut driver).unwrap().clone();
        assert!(early.cells.iter().all(|c| c.depth == DEPTH), "{order:?}: the impulse comes at 0.237");
        let again = ocean.at_driven(0.5, &mut driver).unwrap().clone();
        assert_eq!(late.cells, again.cells, "{order:?} checkpoints {checkpoints}");
        // a fractional time inside the step that holds the instant sees the impulse only after it
        let before = ocean.at_driven(0.236, &mut driver).unwrap().clone();
        let after = ocean.at_driven(0.238, &mut driver).unwrap().clone();
        assert!(before.cells.iter().all(|c| c.depth == DEPTH));
        assert!(after.cells.iter().any(|c| c.depth != DEPTH));
    }
}

#[test]
fn an_impulse_outside_the_step_it_was_asked_in_is_an_error() {
    let mut ocean = flat(spec(Order::First, true, 0), vec![]);
    // asked at every sample, whichever step it belongs to
    let mut wrong = |time: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(DEPTH);
        f.events = vec![cavity(0.12, 1.0, 4.0)];
        let _ = time;
        Ok(())
    };
    assert!(matches!(ocean.at_driven(0.3, &mut wrong), Err(Error::Invalid(_))));
    // and none may be given at the first sample, which ends no step
    let mut first = flat(spec(Order::First, true, 0), vec![]);
    let mut early = |time: f64, f: &mut Forcing| -> Result<(), Error> {
        f.bed.fill(DEPTH);
        if time == 0.0 {
            f.events = vec![cavity(0.0, 1.0, 4.0)];
        }
        Ok(())
    };
    assert!(first.at_driven(0.1, &mut early).is_err());
}

#[test]
fn a_driver_that_asks_for_nothing_changes_nothing() {
    for order in [Order::First, Order::Second] {
        let mut plain =
            flat(spec(order, true, 0), vec![Impulse { kind: ImpulseKind::AddWater, ..cavity(0.1, 0.3, 3.0) }]);
        let mut quiet = |_: f64, f: &mut Forcing| -> Result<(), Error> {
            f.bed.fill(DEPTH);
            Ok(())
        };
        let mut fixed =
            flat(spec(order, false, 0), vec![Impulse { kind: ImpulseKind::AddWater, ..cavity(0.1, 0.3, 3.0) }]);
        let a = plain.at_driven(0.4, &mut quiet).unwrap().clone();
        let b = fixed.at(0.4).unwrap().clone();
        assert_eq!(a.cells, b.cells, "{order:?}");
    }
}

fn part(time: f64, share: f64, amplitude: f64, radius: f64) -> Impulse {
    part_after(time, share, 0.0, amplitude, radius)
}
fn part_after(time: f64, share: f64, before: f64, amplitude: f64, radius: f64) -> Impulse {
    Impulse { kind: ImpulseKind::CavityPart { share, before }, ..cavity(time, amplitude * share, radius) }
}

#[test]
fn the_parts_of_a_cavity_take_what_the_whole_does_and_the_limit_of_the_whole_in_all() {
    // five parts of a fifth, at one instant, against the whole: the same volume moves, each part conserving water
    let mut whole = flat(spec(Order::First, false, 0), vec![cavity(0.1, 1.0, 4.0)]);
    let parts: Vec<Impulse> = (0..5).map(|_| part(0.1, 0.2, 1.0, 4.0)).collect();
    let mut split = flat(spec(Order::First, false, 0), parts);
    whole.at(0.1).unwrap();
    split.at(0.1).unwrap();
    let centre = 16 * N + 16;
    let (a, b) = (&whole.frame().cells, &split.frame().cells);
    assert!((volume(&whole) - volume(&split)).abs() < 1e-9, "all the water is kept");
    // the same wanted volume leaves the disc, spread a little differently as the disc empties
    let removed = |c: &[Cell]| c.iter().map(|x| (DEPTH - x.depth).max(0.0)).sum::<f64>() * 0.25;
    assert!((removed(a) - removed(b)).abs() < 1e-9 * removed(a), "{} against {}", removed(a), removed(b));
    assert!(
        (a[centre].depth - b[centre].depth).abs() < 0.1 * (DEPTH - a[centre].depth),
        "{} against {}",
        a[centre].depth,
        b[centre].depth
    );
    // wishing for far more than the water holds: each part takes its part of the limit, so the parts together take
    // what one whole cavity would (to a few per cent: the limit is measured on the water the disc holds), and no column is emptied
    let greedy: Vec<Impulse> = (0..5).map(|i| part_after(0.1, 0.2, 0.2 * i as f64, 10.0, 4.0)).collect();
    let mut limited = flat(spec(Order::First, false, 0), greedy);
    let mut single = flat(spec(Order::First, false, 0), vec![cavity(0.1, 10.0, 4.0)]);
    limited.at(0.1).unwrap();
    single.at(0.1).unwrap();
    assert!(limited.frame().cells.iter().all(|c| c.depth >= (1.0 - CAVITY_SHARE) * DEPTH - 1e-12));
    assert!(
        (removed(&limited.frame().cells) - removed(&single.frame().cells)).abs()
            < 0.03 * removed(&single.frame().cells),
        "{} against {}",
        removed(&limited.frame().cells),
        removed(&single.frame().cells)
    );
    assert!((volume(&limited) - volume(&single)).abs() < 1e-9);
}

#[test]
fn parts_asked_by_the_driver_step_by_step_act_like_the_same_parts_authored() {
    for order in [Order::First, Order::Second] {
        let parts = vec![part(0.07, 0.3, 1.5, 4.0), part(0.13, 0.5, 1.5, 4.0), part(0.18, 0.2, 1.5, 4.0)];
        let mut authored = flat(spec(order, false, 0), parts.clone());
        let mut asked = flat(spec(order, true, 0), vec![]);
        let mut driver = asks(parts);
        for t in [0.1, 0.2, 0.31, 0.5] {
            assert_eq!(
                authored.at(t).unwrap().cells,
                asked.at_driven(t, &mut driver).unwrap().cells,
                "{order:?} at {t}"
            );
        }
    }
}

#[test]
fn a_part_must_be_a_positive_share_not_above_the_whole() {
    for share in [0.0, -0.1, 1.5, f64::NAN] {
        let bad = Ocean::new(
            spec(Order::First, false, 0),
            vec![DEPTH; N * N],
            vec![Cell { depth: DEPTH, velocity: [0.0; 2] }; N * N],
            vec![part(0.1, share, 1.0, 4.0)],
        );
        assert!(bad.is_err(), "share {share}");
    }
}
