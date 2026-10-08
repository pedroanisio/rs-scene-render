//! A bed of granular material on a ground: volume poured onto it piles at the angle of repose, whatever the
//! direction on the grid, and is never made or lost.
use sr_sim::granular::Bed;

const REPOSE: f64 = 35.0;

/// Pours the volume of a cone of `radius` cells at the middle of a flat floor in `increments` pours, relaxing after
/// each, and returns the bed and the volume.
fn pile(radius: f64, increments: usize) -> (Bed, f64) {
    pile_at(REPOSE, radius, increments)
}

/// The same at another angle of repose.
fn pile_at(repose: f64, radius: f64, increments: usize) -> (Bed, f64) {
    let n = 4 * radius as usize + 9;
    let mut bed = Bed::flat([n, n], 1.0, repose).unwrap();
    let volume = std::f64::consts::PI * radius.powi(3) * repose.to_radians().tan() / 3.0;
    let middle = (n / 2) as f64 + 0.5;
    for _ in 0..increments {
        bed.deposit([middle, middle], volume / increments as f64).unwrap();
        bed.relax(1e-9, 200_000).unwrap();
    }
    (bed, volume)
}

/// The angle of the flank, in degrees, in each of the 24 sectors of 15 degrees: the slope of the least-squares line of
/// the height against the distance from the axis over the cells between 0.3 and 0.7 of the pile's radius.
fn flanks(bed: &Bed, tan: f64) -> Vec<f64> {
    let [n, _] = bed.cells();
    let c = (n / 2) as f64;
    let top = (0..n * n).map(|i| bed.deposit_at(i % n, i / n)).fold(0.0, f64::max);
    let radius = top / tan;
    (0..24)
        .map(|k| {
            let centre = (k as f64 * 15.0).to_radians();
            let (mut sr, mut sh, mut srr, mut srh, mut m) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for iz in 0..n {
                for ix in 0..n {
                    let (dx, dz) = (ix as f64 - c, iz as f64 - c);
                    let r = dx.hypot(dz);
                    let mut da = (dz.atan2(dx) - centre).rem_euclid(std::f64::consts::TAU);
                    if da > std::f64::consts::PI {
                        da -= std::f64::consts::TAU;
                    }
                    if r > 0.3 * radius && r < 0.7 * radius && da.abs() <= 7.5f64.to_radians() {
                        let h = bed.deposit_at(ix, iz);
                        sr += r;
                        sh += h;
                        srr += r * r;
                        srh += r * h;
                        m += 1.0;
                    }
                }
            }
            let slope = (m * srh - sr * sh) / (m * srr - sr * sr);
            (-slope).atan().to_degrees()
        })
        .collect()
}

#[test]
fn a_volume_poured_at_one_point_piles_into_a_cone_of_the_angle_of_repose_and_none_of_it_is_lost() {
    let tan = REPOSE.to_radians().tan();
    let (bed, volume) = pile(10.0, 30);
    let kept = bed.volume() / volume - 1.0;
    let angles = flanks(&bed, tan);
    let (lo, hi) = (angles.iter().cloned().fold(f64::MAX, f64::min), angles.iter().cloned().fold(f64::MIN, f64::max));
    let axis = angles[0];
    let diagonal = angles[3];
    println!("CONE volume kept {kept:+.1e}; flank angle {lo:.2} to {hi:.2} (axis {axis:.2}, diagonal {diagonal:.2}), declared {REPOSE}");
    assert!(kept.abs() < 1e-9, "{kept}");
    for (k, a) in angles.iter().enumerate() {
        assert!((a - REPOSE).abs() < 2.0, "sector {k}: {a} against {REPOSE}");
    }
    // the radius of the base: the farthest cell that holds any deposit is within one cell of the ideal cone's
    let [n, _] = bed.cells();
    let c = (n / 2) as f64;
    let reach = (0..n * n)
        .filter(|&i| bed.deposit_at(i % n, i / n) > 1e-6)
        .map(|i| ((i % n) as f64 - c).hypot((i / n) as f64 - c))
        .fold(0.0, f64::max);
    println!("CONE radius of the base {reach:.2} cells against 10 for the ideal cone");
    assert!((reach - 10.0).abs() < 1.5, "{reach}");
    // nothing is steeper than the angle, to the tolerance of the relaxation
    assert!(bed.steepest_transferable() < 1e-6, "{}", bed.steepest_transferable());
}

#[test]
fn the_same_pours_give_the_same_bits() {
    let (a, _) = pile(6.0, 12);
    let (b, _) = pile(6.0, 12);
    assert_eq!(a, b);
}

#[test]
fn a_layer_on_a_slope_under_the_angle_stays_and_on_one_over_it_runs_down_and_nothing_is_lost() {
    // a ramp that rises along x with a slope of tan(angle) per unit of length
    let ramp = |angle: f64| {
        let (nx, nz) = (40usize, 8usize);
        let ground: Vec<f64> = (0..nx * nz).map(|i| (nx - 1 - i % nx) as f64 * angle.to_radians().tan()).collect();
        Bed::on(ground, [nx, nz], 1.0, REPOSE).unwrap()
    };
    let mut gentle = ramp(30.0);
    for iz in 0..8 {
        for ix in 0..40 {
            gentle.set_deposit(ix, iz, 0.5);
        }
    }
    let before = gentle.clone();
    assert_eq!(gentle.relax(1e-9, 1000).unwrap(), 0, "already at rest: no pass moves anything");
    assert_eq!(gentle, before);
    let mut steep = ramp(40.0);
    for iz in 0..8 {
        for ix in 0..40 {
            steep.set_deposit(ix, iz, 0.5);
        }
    }
    let volume = steep.volume();
    let passes = steep.relax(1e-9, 1_000_000).unwrap();
    println!("RAMP 40 degrees: {passes} passes, volume kept {:+.1e}", steep.volume() / volume - 1.0);
    assert!((steep.volume() / volume - 1.0).abs() < 1e-12);
    // the top of the ramp is bare and the foot has the deposit; none of it is negative
    assert!(steep.deposit_at(0, 3) < 0.5 && steep.deposit_at(39, 3) > 0.5);
    for iz in 0..8 {
        for ix in 0..40 {
            assert!(steep.deposit_at(ix, iz) >= 0.0);
        }
    }
    // across the ramp one row is the same as another, to what the order of the passes leaves
    let across = (0..40).map(|ix| (steep.deposit_at(ix, 0) - steep.deposit_at(ix, 7)).abs()).fold(0.0, f64::max);
    let tallest = (0..40).map(|ix| steep.deposit_at(ix, 0)).fold(0.0, f64::max);
    println!("RAMP largest difference between the first and the last row {across:.2e} of a deposit {tallest:.2} high");
    // the fixed order of the passes leaves a difference, a few thousandths of the heap at most
    assert!(across < 5e-3 * tallest, "{across}");
}

#[test]
fn a_deposit_outside_the_grid_or_a_bed_that_makes_no_sense_is_an_error() {
    assert!(Bed::flat([0, 4], 1.0, REPOSE).is_err());
    assert!(Bed::flat([4, 4], 0.0, REPOSE).is_err());
    assert!(Bed::flat([4, 4], 1.0, 0.0).is_err());
    assert!(Bed::flat([4, 4], 1.0, 90.0).is_err());
    assert!(Bed::on(vec![0.0; 3], [2, 2], 1.0, REPOSE).is_err());
    let mut bed = Bed::flat([4, 4], 1.0, REPOSE).unwrap();
    assert!(bed.deposit([-0.1, 2.0], 1.0).is_err());
    assert!(bed.deposit([2.0, 4.1], 1.0).is_err());
    assert!(bed.deposit([2.0, 2.0], -1.0).is_err());
    assert!(bed.deposit([f64::NAN, 2.0], 1.0).is_err());
    assert_eq!(bed.volume(), 0.0);
}

#[test]
fn the_cone_has_the_declared_angle_from_ten_cells_of_radius_up_and_less_below() {
    for (repose, radius) in [(30.0, 5.0), (30.0, 8.0), (30.0, 10.0), (25.0, 12.0), (40.0, 12.0), (35.0, 20.0)] {
        let (bed, volume) = pile_at(repose, radius, 20);
        let angles = flanks(&bed, repose.to_radians().tan());
        let (lo, hi) =
            (angles.iter().cloned().fold(f64::MAX, f64::min), angles.iter().cloned().fold(f64::MIN, f64::max));
        println!(
            "CONE {repose} degrees, {radius} cells: flank {lo:.2} to {hi:.2}, volume kept {:+.1e}",
            bed.volume() / volume - 1.0
        );
        assert!((bed.volume() / volume - 1.0).abs() < 1e-9);
        // a pile of ten cells' radius or more is within two degrees; a smaller one is not (5 degrees off at 5 and 8 cells)
        if radius >= 10.0 {
            assert!(lo > repose - 2.0 && hi < repose + 2.0, "{lo} {hi} against {repose}");
        }
    }
}
