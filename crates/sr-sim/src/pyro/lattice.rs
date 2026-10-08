//! The count of the cells of the infinite lattice that a sphere holds, against a count of every cell of a box with the very test that the window's cells are
//! counted by (the divergence of a blast is shared by that count, so that a centre exactly on the sphere must be in or out the same in both).

use super::*;

fn brute(center: [f64; 3], origin: [f64; 3], h: f64, reach: f64) -> usize {
    let span = (reach / h).ceil() as i64 + 3;
    let around: [i64; 3] = std::array::from_fn(|a| ((center[a] - origin[a]) / h).floor() as i64);
    let mut n = 0;
    for i in around[0] - span..=around[0] + span {
        for j in around[1] - span..=around[1] + span {
            for k in around[2] - span..=around[2] + span {
                let p = world_point(origin, h, [i as f64 + 0.5, j as f64 + 0.5, k as f64 + 0.5]);
                n += usize::from((0..3).map(|a| (p[a] - center[a]).powi(2)).sum::<f64>() <= reach * reach);
            }
        }
    }
    n
}

#[test]
fn the_lattice_count_is_the_count_of_the_cells_by_the_same_test_even_where_a_centre_is_on_the_sphere() {
    // cells of 0.1 and of 1/3 (which no float holds), spheres that put centres exactly or nearly on them, and centres off the lattice
    let mut cases = 0;
    for h in [0.1, 1.0 / 3.0, 0.25, 0.5, 1.0] {
        for origin in [[-8.0, -8.0, -8.0], [0.0, 0.0, 0.0], [-3.3, 1.7, 0.05]] {
            for center in [[0.0, 0.0, 0.0], [h * 0.5, h * 0.5, h * 0.5], [1.0, -2.0, 0.35], [0.3, 0.3, 0.3]] {
                for reach in [h, 1.5 * h, 2.0 * h, 0.5, 1.0, 2.5, 3.0 * h * 3f64.sqrt(), 4.0] {
                    assert_eq!(
                        lattice_count(center, origin, h, reach),
                        brute(center, origin, h, reach),
                        "h {h} origin {origin:?} centre {center:?} reach {reach}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert!(cases > 400);
}

#[test]
fn a_sphere_of_a_hundred_cells_has_the_volume_in_cells_to_a_few_percent() {
    let n = lattice_count([0.0; 3], [-40.0; 3], 0.5, 25.0);
    let volume = 4.0 / 3.0 * std::f64::consts::PI * 50f64.powi(3);
    assert!((n as f64 / volume - 1.0).abs() < 1e-3, "{n} against {volume}");
}
