//! `State::volume` must export exactly the grids the original serial,
//! per-voxel implementation produced. The reference below is that code, kept
//! verbatim.

use super::*;

fn reference(state: &State, max_bytes: usize) -> Result<Volume, Error> {
    let bricks = state.cells.iter().map(|n| n.div_ceil(8)).product::<usize>();
    let needed = bricks
        .checked_mul(5 * (512 * 4 + 128))
        .and_then(|v| v.checked_add(4096))
        .ok_or(Error::Limit("volume export size overflow"))?;
    if needed > max_bytes {
        return Err(Error::Limit("volume export memory budget"));
    }
    let h = state.h;
    let o = state.origin.map(|v| v + h * 0.5);
    let transform = Transform::new([h, 0.0, 0.0, 0.0, 0.0, h, 0.0, 0.0, 0.0, 0.0, h, 0.0, o[0], o[1], o[2], 1.0])?;
    let mut volume = Volume::new();
    for (channel, name) in ["density", "temperature", "velocity.x", "velocity.y", "velocity.z"].iter().enumerate() {
        let bg = if channel == 1 { state.ambient as f32 } else { 0.0 };
        let mut grid = SparseGrid::new(transform, bg, bricks)?;
        for k in 0..state.density.len() {
            let value = match channel {
                0 => state.density[k],
                1 => state.temperature[k],
                _ => state.velocity_at(state.cell_world(k))[channel - 2],
            } as f32;
            if !value.is_finite() {
                return Err(Error::Invalid("exported field exceeds f32 representation"));
            }
            grid.set(coords(k, state.cells).map(|v| v as i32), value)?;
        }
        volume.insert(name, grid)?;
    }
    Ok(volume)
}

/// Identical names, transforms, backgrounds, brick keys and every sample bit.
pub(super) fn assert_same(a: &Volume, b: &Volume, what: &str) {
    let names = |v: &Volume| v.grids().map(|(n, _)| n.to_owned()).collect::<Vec<_>>();
    assert_eq!(names(a), names(b), "{what}: grid names");
    for ((name, x), (_, y)) in a.grids().zip(b.grids()) {
        assert_eq!(
            x.transform().columns().map(f64::to_bits),
            y.transform().columns().map(f64::to_bits),
            "{what} {name}"
        );
        assert_eq!(x.background().to_bits(), y.background().to_bits(), "{what} {name}: background");
        assert_eq!(x.brick_count(), y.brick_count(), "{what} {name}: brick count");
        for ((ka, va), (kb, vb)) in x.bricks().zip(y.bricks()) {
            assert_eq!(ka, kb, "{what} {name}: brick key");
            assert!(va.iter().zip(vb).all(|(p, q)| p.to_bits() == q.to_bits()), "{what} {name}: brick {ka:?} values");
        }
    }
}

#[test]
fn exported_volumes_match_the_original_implementation() {
    // Cases: odd domain with colliders, closed domain with colliders, cinematic
    // plume; ambient 300 (the default) and a different nonzero background.
    for (index, steps) in [(0, 6), (1, 6), (2, 5), (3, 6), (5, 5)] {
        for ambient in [300.0, 273.15] {
            let sim = simulate_case(index, steps, ambient);
            let state = sim.state();
            let expected = reference(state, usize::MAX / 2).unwrap();
            let got = state.volume(usize::MAX / 2).unwrap();
            assert_same(&got, &expected, &format!("case {index} ambient {ambient}"));
            let bricks: usize = got.grids().map(|(_, g)| g.brick_count()).sum();
            assert!(bricks > 5, "case {index}: the export should contain smoke");
        }
    }
}

#[test]
fn an_empty_state_and_budget_errors_match_too() {
    let sim = simulate_case(0, 0, 300.0);
    let state = sim.state();
    assert_same(&state.volume(usize::MAX / 2).unwrap(), &reference(state, usize::MAX / 2).unwrap(), "empty");
    for budget in [0, 100, 100_000] {
        let (a, b) = (state.volume(budget), reference(state, budget));
        assert_eq!(a.is_ok(), b.is_ok(), "budget {budget}");
        if let (Err(a), Err(b)) = (a, b) {
            assert_eq!(a.to_string(), b.to_string());
        }
    }
}

fn simulate_case(index: usize, steps: u64, ambient: f64) -> Simulation {
    determinism::simulate(index, steps, ambient)
}
