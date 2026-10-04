//! `sample` must stay bit-identical to the original trilinear loop: the
//! reference below is that loop, kept verbatim.

use super::*;

fn reference(values: &[f64], dims: [usize; 3], mut p: [f64; 3], boundary: Boundary, background: f64) -> f64 {
    if !finite3(p) {
        return background;
    }
    for a in 0..3 {
        if boundary == Boundary::Closed {
            p[a] = p[a].clamp(0.0, (dims[a] - 1) as f64);
        } else if p[a] < -1.0 || p[a] > dims[a] as f64 {
            return background;
        }
    }
    let lo = p.map(|v| v.floor() as i64);
    let f: [f64; 3] = std::array::from_fn(|a| p[a] - lo[a] as f64);
    let mut result = 0.0;
    for bit in 0..8 {
        let q: [i64; 3] = std::array::from_fn(|a| lo[a] + ((bit >> a) & 1));
        let weight: f64 = (0..3).map(|a| if (bit >> a) & 1 == 0 { 1.0 - f[a] } else { f[a] }).product();
        let v = if (0..3).all(|a| q[a] >= 0 && q[a] < dims[a] as i64) {
            values[index(q.map(|v| v as usize), dims)]
        } else {
            background
        };
        result += weight * v;
    }
    result
}

#[test]
fn sample_is_bit_identical_to_the_reference_loop() {
    let mut checked = 0u64;
    for dims in [[3, 3, 3], [5, 4, 3], [9, 2, 6], [2, 2, 2], [17, 13, 11]] {
        let n: usize = dims.iter().product();
        // Mixed signs, exact zeros and tiny/huge magnitudes exercise signed zeros.
        let values: Vec<f64> = (0..n)
            .map(|k| match k % 7 {
                0 => 0.0,
                1 => -0.0,
                2 => 1e-300 * crate::rng::signed(1, k as u64, 0),
                3 => 1e300 * crate::rng::signed(1, k as u64, 1),
                _ => 300.0 * crate::rng::signed(1, k as u64, 2),
            })
            .collect();
        for boundary in [Boundary::Open, Boundary::Closed] {
            for background in [0.0, 300.0, -0.0] {
                for i in 0..4000u64 {
                    // Random points beyond the array, plus lattice points and half cells.
                    let coordinate = |axis: u64| {
                        let span = dims[axis as usize] as f64 + 4.0;
                        match i % 5 {
                            0 => (crate::rng::unit(7, i, axis) * span - 2.0).round(),
                            1 => ((crate::rng::unit(7, i, axis) * span - 2.0) * 2.0).round() / 2.0,
                            _ => crate::rng::unit(7, i, axis) * span - 2.0,
                        }
                    };
                    let p = [coordinate(0), coordinate(1), coordinate(2)];
                    let (got, want) = (
                        sample(&values, dims, p, boundary, background),
                        reference(&values, dims, p, boundary, background),
                    );
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "{dims:?} {boundary:?} {background} {p:?}: {got} vs {want}"
                    );
                    checked += 1;
                }
            }
        }
        for p in [[f64::NAN, 0.5, 0.5], [0.5, f64::INFINITY, 0.5], [0.5, 0.5, f64::NEG_INFINITY]] {
            assert_eq!(
                sample(&values, dims, p, Boundary::Open, 7.0).to_bits(),
                reference(&values, dims, p, Boundary::Open, 7.0).to_bits()
            );
        }
    }
    assert!(checked > 100_000);
}

#[test]
fn floor_i64_matches_floor() {
    let mut values = vec![0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 2.999999999999999, -2.000000000000001, 1e15, -1e15];
    values.extend((0..100_000u64).map(|i| (crate::rng::unit(3, i, 0) - 0.5) * 4096.0));
    values.extend((-2000..2000).map(|i| f64::from(i) * 0.25));
    for v in values {
        assert_eq!(floor_i64(v), v.floor() as i64, "{v}");
    }
}
