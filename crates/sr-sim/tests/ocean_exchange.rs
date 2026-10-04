//! The momentum bodies give the water in a canonical step is offered to the driver as soon as
//! the step completes, and a replay of the step offers exactly the same.
use sr_sim::ocean::{Boundary, Cell, Error, Forcing, Ocean, Order, Spec};

fn ocean(order: Order, checkpoint_bytes: usize) -> Ocean {
    let spec = Spec {
        cells: [8, 8],
        cell_size: 1.0,
        origin: [0.0, 0.0],
        dt: 0.1,
        order,
        moving_bed: true,
        bodies: true,
        boundary: Boundary::Periodic,
        max_work: 1 << 40,
        checkpoint_bytes,
        ..Default::default()
    };
    Ocean::new(spec, vec![4.0; 64], vec![Cell { depth: 4.0, velocity: [0.0; 2] }; 64], vec![]).unwrap()
}

/// A body thickness 2 moving at (1.5, -0.5) over a flat bed of 4, which also lists what it was offered.
fn body(seen: &mut Vec<(u64, [f64; 2])>) -> impl FnMut(f64, &mut Forcing) -> Result<(), Error> + '_ {
    move |_, f| {
        f.bed.fill(2.0);
        f.occupancy.fill(2.0);
        f.velocity.fill([1.5, -0.5]);
        if let Some(offer) = f.exchange {
            seen.push(offer);
        }
        Ok(())
    }
}

#[test]
fn each_completed_step_offers_its_momentum_once_and_a_replay_offers_it_again_unchanged() {
    // without checkpoints a backward seek replays from the start: every step is offered again
    for (order, checkpoints) in [(Order::First, 0), (Order::Second, 0), (Order::First, 1 << 20)] {
        let mut solver = ocean(order, checkpoints);
        let mut first = Vec::new();
        let mut per_step = Vec::new();
        for k in 1..=4 {
            solver.at_driven(k as f64 * 0.1, &mut body(&mut first)).unwrap();
            per_step.push(solver.exchanged_impulse());
        }
        // the first sample after a step completes is the one that carries it
        let steps: Vec<u64> = first.iter().map(|(k, _)| *k).collect();
        assert_eq!(steps, [0, 1, 2, 3, 4], "{order:?}: one offer per canonical step, from step 0");
        assert_eq!(first[0].1, [0.0; 2], "nothing has been given before the first step");
        for k in 1..=4 {
            assert_eq!(first[k].1.map(f64::to_bits), per_step[k - 1].map(f64::to_bits), "{order:?} step {k}");
        }
        assert!(first[1].1[0] > 0.0, "the body gives the water momentum");
        // go back to the start and forward again: the same steps are offered the same values
        let mut replay = Vec::new();
        solver.at_driven(0.1, &mut body(&mut replay)).unwrap();
        solver.at_driven(0.4, &mut body(&mut replay)).unwrap();
        for (k, momentum) in &replay {
            let kept = first.iter().find(|(j, _)| j == k).unwrap().1;
            assert_eq!(momentum.map(f64::to_bits), kept.map(f64::to_bits), "{order:?} replay of step {k}");
        }
        if checkpoints == 0 {
            assert!(replay.len() >= 4, "the replay offered {} steps", replay.len());
        }
    }
}
