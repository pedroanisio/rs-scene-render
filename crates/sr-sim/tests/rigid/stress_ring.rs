//! A joint in a cycle: the load of its cut is shared over the joints that cross the plane as over one rigid section. A ring of four pieces (a square, each joined to its two
//! neighbours) with joints of different areas, pulled apart: the stress is the pull over the sum of the areas in the two joints that the plane crosses, whatever their areas,
//! which is the rule that says by area and is what an exact tree would not have; and a pull that is not through the centre of the section bends it.

use sr_sim::stress::balance::{Located, MassSum, Rigid, Step};
use sr_sim::stress::plan::{plan, JointGeom, PieceGeom};
use sr_sim::stress::{cut_stresses, JointSection};

const EYE: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A joint of `w` (along y) by `d` (along z) in the plane x = `x` at height `y`, from the piece `a` to the piece `b`, normal +x or +y by `along`.
fn joint(a: u32, b: u32, centre: [f64; 3], along: usize, width: f64, depth: f64) -> (JointGeom, JointSection) {
    let mut normal = [0.0; 3];
    normal[along] = 1.0;
    // the face is across the axis `along`: its extents are the two other axes
    let others: Vec<usize> = (0..3).filter(|k| *k != along).collect();
    let (u, v) = (others[0], others[1]);
    let size = [width, depth];
    let mut second = [[0.0; 3]; 3];
    second[u][u] = size[0].powi(3) * size[1] / 12.0;
    second[v][v] = size[1].powi(3) * size[0] / 12.0;
    let mut lo = centre;
    let mut hi = centre;
    lo[u] -= size[0] / 2.0;
    hi[u] += size[0] / 2.0;
    lo[v] -= size[1] / 2.0;
    hi[v] += size[1] / 2.0;
    (
        JointGeom { a, b, centroid: centre, normal },
        JointSection { area: width * depth, centroid: centre, second, normal, lo, hi },
    )
}

fn cube(c: [f64; 3]) -> PieceGeom {
    let m = 10.0;
    let mut second = [[0.0; 3]; 3];
    for a in 0..3 {
        for b in 0..3 {
            second[a][b] = m * c[a] * c[b];
        }
    }
    PieceGeom { centre: c, mass: MassSum { mass: m, first: c.map(|v| v * m), second } }
}

#[test]
fn a_pull_across_a_ring_is_shared_by_the_areas_of_the_joints_that_the_plane_crosses() {
    // the pieces 0 (0, 0), 1 (1, 0), 2 (0, 1) and 3 (1, 1) in the plane z = 0; the joints of the pairs 0-1 (area 0.01) and 2-3 (area 0.03) across the plane x = 0.5, and 0-2 and 1-3
    // across y = 0.5, of area 0.02
    let pieces = [cube([0.0, 0.0, 0.0]), cube([1.0, 0.0, 0.0]), cube([0.0, 1.0, 0.0]), cube([1.0, 1.0, 0.0])];
    let (g01, s01) = joint(0, 1, [0.5, 0.0, 0.0], 0, 0.1, 0.1);
    let (g02, s02) = joint(0, 2, [0.0, 0.5, 0.0], 1, 0.1, 0.2);
    let (g13, s13) = joint(1, 3, [1.0, 0.5, 0.0], 1, 0.1, 0.2);
    let (g23, s23) = joint(2, 3, [0.5, 1.0, 0.0], 0, 0.1, 0.3);
    let geoms = [g01, g02, g13, g23];
    let sections = [s01, s02, s13, s23];
    let held: Vec<u32> = (0..4).collect();
    let cuts = plan(&held, &[false; 4], &pieces, &geoms);
    // two cuts, one across x for the joints 0 and 3, one across y for the joints 1 and 2
    assert_eq!(cuts.len(), 2);
    let across = &cuts[0];
    assert_eq!(across.members, [0, 3]);
    // the body at rest in space; the left pieces (0 and 2) are pulled along -x, the right ones along +x, by loads that make no moment on the body (the resultant of each
    // side acts through the centre of the section across x, which is at y = (0.01 * 0 + 0.03 * 1) / 0.04 = 0.75): 1/4 on the row y = 0 and 3/4 on the row y = 1
    let f = 4000.0;
    let dt = 1.0 / 240.0;
    let whole = {
        let mut m = MassSum::default();
        for p in &pieces {
            m.add(&p.mass);
        }
        m
    };
    let rest = Rigid { position: [0.0; 3], rotation: EYE, linear: [0.0; 3], angular: [0.0; 3], centre: whole.centre() };
    let load = |at: [f64; 3], force: f64, piece: usize| Located { at, impulse: [force * dt, 0.0, 0.0], piece };
    let contacts = [
        load(pieces[0].centre, -0.25 * f, 0),
        load(pieces[2].centre, -0.75 * f, 2),
        load(pieces[1].centre, 0.25 * f, 1),
        load(pieces[3].centre, 0.75 * f, 3),
    ];
    let step = Step { whole, before: rest, after: rest, dt, accel: [0.0; 3], contacts: &contacts, anchor: None };
    let side = &across.side;
    let in_part = |i: usize| side.contains(i as u32, pieces[i].centre);
    let q = [0.5, 0.75, 0.0];
    let wrench = step.on_part(&across.mass, &in_part, q);
    // the side is the left (x < 0.5): the rest pulls it along +x with the whole load
    assert!((wrench.force[0] - f).abs() < 1e-9 * f && wrench.force[1].abs() < 1e-9 * f, "{:?}", wrench.force);
    assert!(wrench.moment.iter().all(|m| m.abs() < 1e-9 * f), "{:?}", wrench.moment);
    let members: Vec<&JointSection> = across.members.iter().map(|&j| &sections[j]).collect();
    let stresses = cut_stresses(&members, &across.side_has_a, &wrench);
    // the same stress in both, the pull over the sum of the areas, and the force that each takes in the ratio of the areas (1 to 3)
    let sigma = f / 0.04;
    for s in &stresses {
        assert!((s.normal - sigma).abs() < 1e-9 * sigma && s.bending.abs() < 1e-9 * sigma, "{s:?}");
        assert!((s.principal - sigma).abs() < 1e-9 * sigma, "{s:?}");
    }
    assert!((stresses[0].normal * 0.01 / (stresses[1].normal * 0.03) - 1.0 / 3.0).abs() < 1e-12);
    // the same pull taken by the joint alone (a tree, with the joint 0-1 the only way across) would make f / 0.01: four times the stress, which the rule of the plane does not
    assert!(stresses[0].principal < f / 0.01 / 3.9);
    // and a pull that is not through the centre of the section bends it: all of it on the row y = 0 (the pieces 0 and 1), the moment about the centre of the section
    // that it makes is carried by the section, and the joint that is nearer to it (0-1) is pulled harder than the other
    let off = [load(pieces[0].centre, -f, 0), load(pieces[1].centre, f, 1)];
    let step = Step { contacts: &off, ..step };
    let wrench = step.on_part(&across.mass, &in_part, q);
    let stresses = cut_stresses(&members, &across.side_has_a, &wrench);
    assert!(stresses[0].principal > stresses[1].principal && stresses[0].principal > sigma, "{stresses:?}");
}
