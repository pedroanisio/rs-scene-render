//! The stress that a load makes in the joints of a cut: normal, shear, bending and torsion from the exact section of the joints, the brittle criterion on
//! the principal tension, and the rigid section that distributes a cut's load over its joints. The oracles are the closed forms of the beam (a
//! cantilever with a load at its tip, a column, a couple on two joints) with the exact W and L, and the invariance of the answer under the move and the
//! turn of the whole configuration.
#![allow(clippy::needless_range_loop)]

use sr_sim::stress::{cut_stresses, JointSection, Wrench};

/// A rectangular joint of `b` (along y) by `h` (along z) at `x`, whose normal from the piece a to the piece b is +x.
fn rectangle(x: f64, b: f64, h: f64) -> JointSection {
    JointSection {
        area: b * h,
        centroid: [x, 0.0, 0.0],
        second: [[0.0; 3], [0.0, b * b * b * h / 12.0, 0.0], [0.0, 0.0, b * h * h * h / 12.0]],
        normal: [1.0, 0.0, 0.0],
        lo: [x, -b / 2.0, -h / 2.0],
        hi: [x, b / 2.0, h / 2.0],
    }
}

/// The load that the root puts on a cantilever of length `length` from the cut at `x` when its tip carries `tip` (a force on the beam, N, at x = length):
/// the rest of the beam, the root, pushes the tip's side with the opposite force, and a moment that balances the tip's about the centre of the cut.
fn cantilever_load(x: f64, length: f64, tip: [f64; 3]) -> Wrench {
    let force = tip.map(|f| -f);
    let lever = [length - x, 0.0, 0.0];
    let external = [
        lever[1] * tip[2] - lever[2] * tip[1],
        lever[2] * tip[0] - lever[0] * tip[2],
        lever[0] * tip[1] - lever[1] * tip[0],
    ];
    Wrench { force, moment: external.map(|m| -m), point: [x, 0.0, 0.0] }
}

#[test]
fn a_cantilever_with_a_load_at_its_tip_has_the_bending_stress_of_the_beam_with_the_exact_section_modulus() {
    let (b, h, length, p) = (0.4, 0.6, 3.0, 5000.0);
    let joint = rectangle(0.0, b, h);
    // the beam is held at x = 0 and the cut is at its root: the tip side is the side of the piece b, so the cut's normal from it points to -x (not to the root's)
    // a load P downward (-y) bends it down: the top fibre (+y) is in tension, to M c / I with M = P L, c = h/2, I = b h^3 / 12 about z
    let w = wrench_of(&joint, length, [0.0, -p, 0.0]);
    let s = cut_stresses(&[&joint], &[false], &w);
    // the section modulus about the neutral axis that the moment is about (the moment is about z, the fibre distance is along y, so the extent b is the one that
    // the load bends): I = h b^3 / 12 with c = b / 2
    let modulus = h * b * b / 6.0;
    let bending = p * length / modulus;
    assert!((s[0].bending - bending).abs() <= 1e-12 * bending, "{} against {bending}", s[0].bending);
    assert!((s[0].normal).abs() < 1e-9, "no axial load: {}", s[0].normal);
    assert!((s[0].shear - p / (b * h)).abs() <= 1e-12 * p / (b * h), "{} against {}", s[0].shear, p / (b * h));
}

/// The load on the tip's side of the cut `joint` of a cantilever of this length, from a tip load.
fn wrench_of(joint: &JointSection, length: f64, tip: [f64; 3]) -> Wrench {
    cantilever_load(joint.centroid[0], length, tip)
}

#[test]
fn the_load_that_breaks_a_cantilever_is_the_one_whose_principal_tension_is_the_strength_to_the_shear_it_makes_too() {
    let (b, h, length) = (0.2, 0.5, 4.0);
    let joint = rectangle(0.0, b, h);
    let strength = 2.0e6;
    let modulus = h * b * b / 6.0;
    let area = b * h;
    // sigma = P L / W and tau = P / A; the principal tension s1 = sigma / 2 + sqrt(sigma^2 / 4 + tau^2) is the strength when sigma = s - tau^2 / s, which is
    // P L / W + P^2 / (A^2 s) = s: a quadratic in P, solved exactly
    let (qa, qb, qc) = (1.0 / (area * area * strength), length / modulus, -strength);
    let exact = (-qb + (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
    // and it is the strength W / L to the second order of the depth over the length
    let plain = strength * modulus / length;
    assert!((exact / plain - 1.0).abs() < (h / length).powi(2), "{exact} against {plain}");
    let principal = |p: f64| {
        let w = wrench_of(&joint, length, [0.0, -p, 0.0]);
        cut_stresses(&[&joint], &[false], &w)[0].principal
    };
    assert!((principal(exact) - strength).abs() <= 1e-9 * strength, "{} against {strength}", principal(exact));
    assert!(principal(0.999 * exact) < strength && principal(1.001 * exact) > strength);
}

#[test]
fn a_pull_is_tension_a_push_is_not_and_a_shear_alone_is_the_principal_tension_that_it_is() {
    let joint = rectangle(1.0, 0.3, 0.3);
    let area = 0.09;
    // the side of the piece a (towards_b is true: the normal from it to the rest is +x): the rest pulls it with +x
    let pull = Wrench { force: [900.0, 0.0, 0.0], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &pull)[0];
    assert!((s.normal - 900.0 / area).abs() < 1e-9 && s.shear == 0.0 && s.bending == 0.0);
    assert!((s.principal - 900.0 / area).abs() < 1e-9);
    // a push (the rest pushes it, along -x): compression, which breaks nothing by itself: the principal tension is zero
    let push = Wrench { force: [-900.0, 0.0, 0.0], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &push)[0];
    assert!((s.normal + 900.0 / area).abs() < 1e-9);
    assert_eq!(s.principal, 0.0);
    // the same from the other side of the joint: the piece b, whose normal to the rest is -x, is pulled by a force along -x
    let other = Wrench { force: [-900.0, 0.0, 0.0], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[false], &other)[0];
    assert!((s.normal - 900.0 / area).abs() < 1e-9, "{}", s.normal);
    // a shear alone: the principal tension is the shear stress (Mohr: sigma 0, tau)
    let shear = Wrench { force: [0.0, 450.0, 0.0], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &shear)[0];
    assert!((s.shear - 450.0 / area).abs() < 1e-9 && (s.principal - 450.0 / area).abs() < 1e-9, "{s:?}");
    // a push with a shear: compression of a half and the shear: the principal tension is less than the shear, and positive
    let both = Wrench { force: [-900.0, 450.0, 0.0], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &both)[0];
    let (sigma, tau) = (-900.0 / area, 450.0 / area);
    assert!(
        (s.principal - (sigma / 2.0 + (sigma * sigma / 4.0 + tau * tau).sqrt())).abs() < 1e-9
            && s.principal < tau
            && s.principal > 0.0
    );
}

#[test]
fn a_twist_makes_a_shear_by_the_polar_moment_of_the_section() {
    let joint = rectangle(0.0, 0.2, 0.2);
    let twist = Wrench { force: [0.0; 3], moment: [30.0, 0.0, 0.0], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &twist)[0];
    // tau = T r / J with J the polar moment b h (b^2 + h^2) / 12 and r the half diagonal: the formula of a round section, which a square exceeds by about
    // an eighth (stated in the SREP)
    let polar = 0.2 * 0.2 * (0.2 * 0.2 + 0.2 * 0.2) / 12.0;
    let r = (0.1f64 * 0.1 + 0.1 * 0.1).sqrt();
    let tau = 30.0 * r / polar;
    assert!((s.torsion - tau).abs() <= 1e-9 * tau, "{} against {tau}", s.torsion);
    assert!((s.principal - tau).abs() <= 1e-9 * tau);
}

/// A rectangular joint of `b` (along y) by `h` (along z), its centre at `y`, with its normal along +x at x = 0.
fn at_y(y: f64, b: f64, h: f64) -> JointSection {
    let mut j = rectangle(0.0, b, h);
    j.centroid = [0.0, y, 0.0];
    j.lo = [0.0, y - b / 2.0, -h / 2.0];
    j.hi = [0.0, y + b / 2.0, h / 2.0];
    j
}

#[test]
fn two_joints_of_a_cut_share_a_pull_by_area() {
    // two joints, 0.1 and 0.3 m^2, side by side along y and cut by the same plane: a pull of 4000 N along +x through the centre of the cut is the same stress
    // in both (the forces are in the ratio of the areas), and no bending
    let (small, large) = (at_y(-0.2, 0.1, 1.0), at_y(0.0, 0.3, 1.0));
    let centre_y = (0.1 * -0.2 + 0.3 * 0.0) / 0.4;
    let pull = Wrench { force: [4000.0, 0.0, 0.0], moment: [0.0; 3], point: [0.0, centre_y, 0.0] };
    let s = cut_stresses(&[&small, &large], &[true, true], &pull);
    let sigma = 4000.0 / 0.4;
    assert!((s[0].normal - sigma).abs() < 1e-9 && (s[1].normal - sigma).abs() < 1e-9, "{s:?}");
    assert!(s[0].bending.abs() < 1e-9 && s[1].bending.abs() < 1e-9, "{s:?}");
    // the same pull through the centre of the small joint is a pull on the cut and a moment about its centre, which the rigid section carries as a stress that
    // is more on the near side: the near joint is pulled harder than the mean
    let off = Wrench { force: [4000.0, 0.0, 0.0], moment: [0.0; 3], point: [0.0, -0.2, 0.0] };
    let s = cut_stresses(&[&small, &large], &[true, true], &off);
    let (near, far) = (s[0].normal + s[0].bending, s[1].normal + s[1].bending);
    assert!(near > sigma && far < near, "{near} {far}");
}

#[test]
fn a_couple_on_two_separate_joints_is_carried_as_a_pair_of_forces_at_their_distance() {
    // two joints of 0.01 m^2 at y = +-0.5 (a section whose faces are far apart: a ring cut in two places) and a moment about z that has to be carried by them: the
    // forces are M / d each, the stress M / (A d), tension on one and compression on the other
    let (top, bottom) = (at_y(0.5, 0.1, 0.1), at_y(-0.5, 0.1, 0.1));
    let m = 40.0;
    // the side of the piece a: normal +x; the rest puts a moment about z: a force along +x at +y and -x at -y has the moment (y x f) = y * f * (y cross x) = -y f z...
    // so a moment of -m about z is a pull on the +y joint and a push on the -y one
    let w = Wrench { force: [0.0; 3], moment: [0.0, 0.0, -m], point: [0.0; 3] };
    let s = cut_stresses(&[&top, &bottom], &[true, true], &w);
    // the faces' own spread adds to the second moment (each face is 0.1 wide), so the stress at the centre of a face is M c / J with J = 2 (A d^2 / 4 + b^3 h / 12)
    let area = 0.01;
    let j = 2.0 * (area * 0.25 + 0.1 * 0.1 * 0.1 * 0.1 / 12.0);
    // the extreme fibre of the joint is the far edge: 0.55
    let far = m * 0.55 / j;
    assert!((s[0].normal).abs() < 1e-9);
    assert!((s[0].bending - far).abs() <= 1e-9 * far, "{} against {far}", s[0].bending);
    // the other joint is in compression along its centre, and its near edge (y = -0.45) is the least compressed: no tension anywhere in it
    assert!(s[1].bending <= 0.0, "{}", s[1].bending);
}

#[test]
fn the_stress_does_not_depend_on_the_point_the_moment_is_taken_about_or_on_where_or_how_the_section_is() {
    let joint = rectangle(2.0, 0.4, 0.6);
    let base = wrench_of(&joint, 5.0, [30.0, -4000.0, 700.0]);
    let a = &cut_stresses(&[&joint], &[false], &base)[0];
    // the same load with the moment taken about another point (the force acts through the first point): the moment about q is M_c + (c - q) x F
    let q = [2.0, 0.3, -0.2];
    let c = base.point;
    let d = [c[0] - q[0], c[1] - q[1], c[2] - q[2]];
    let f = base.force;
    let moved = Wrench {
        force: f,
        moment: [
            base.moment[0] + d[1] * f[2] - d[2] * f[1],
            base.moment[1] + d[2] * f[0] - d[0] * f[2],
            base.moment[2] + d[0] * f[1] - d[1] * f[0],
        ],
        point: q,
    };
    let b = &cut_stresses(&[&joint], &[false], &moved)[0];
    for (x, y) in [
        (a.normal, b.normal),
        (a.shear, b.shear),
        (a.bending, b.bending),
        (a.torsion, b.torsion),
        (a.principal, b.principal),
    ] {
        assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{a:?} against {b:?}");
    }
    // the same beam laid along y and z instead of x and y (a turn of the axes: x -> y, y -> z, z -> x): the same stresses
    let turn = |v: [f64; 3]| [v[2], v[0], v[1]];
    let turn_tensor = |t: [[f64; 3]; 3]| {
        let mut out = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                out[(i + 1) % 3][(j + 1) % 3] = t[i][j];
            }
        }
        out
    };
    let turned_joint = JointSection {
        area: joint.area,
        centroid: turn(joint.centroid),
        second: turn_tensor(joint.second),
        normal: turn(joint.normal),
        lo: turn(joint.lo),
        hi: turn(joint.hi),
    };
    let turned = Wrench { force: turn(base.force), moment: turn(base.moment), point: turn(base.point) };
    let t = &cut_stresses(&[&turned_joint], &[false], &turned)[0];
    for (x, y) in [(a.normal, t.normal), (a.shear, t.shear), (a.bending, t.bending), (a.principal, t.principal)] {
        assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{a:?} against {t:?}");
    }
}

#[test]
fn no_load_is_no_stress_and_a_line_that_is_asked_for_a_moment_is_an_infinite_one() {
    let joint = rectangle(0.0, 0.2, 0.2);
    let none = Wrench { force: [0.0; 3], moment: [0.0; 3], point: joint.centroid };
    let s = &cut_stresses(&[&joint], &[true], &none)[0];
    assert_eq!((s.normal, s.shear, s.bending, s.torsion, s.principal), (0.0, 0.0, 0.0, 0.0, 0.0));
    // a joint of no width (second moments of zero), asked to carry a bending moment: it cannot, and says so with an infinite stress
    let mut line = rectangle(0.0, 0.2, 0.2);
    line.second = [[0.0; 3]; 3];
    let bend = Wrench { force: [0.0; 3], moment: [0.0, 0.0, 5.0], point: line.centroid };
    assert!(cut_stresses(&[&line], &[true], &bend)[0].bending.is_infinite());
    // and the same line with a pull carries it
    let pull = Wrench { force: [10.0, 0.0, 0.0], moment: [0.0; 3], point: line.centroid };
    assert!((cut_stresses(&[&line], &[true], &pull)[0].normal - 10.0 / 0.04).abs() < 1e-9);
}
