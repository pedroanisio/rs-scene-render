//! The stress that a load makes in the joints of a cut: a pure function of the sections of the joints and the load that the rest of a body puts on one side
//! of them, with no state and no world.
//!
//! A *cut* is the set of joints that separate one side of a body from the rest. The load on the side is a force and a moment (the wrench that the rest puts
//! on it), and the joints of the cut carry it as one rigid section: the joints, with their own areas and their own second moments, are the faces of one
//! cross-section, and the stress in it is that of beam theory on that section. So
//!
//! * the normal stress is the pull over the area, `N / A`, `N` the force along the normal of the section (tension is positive: the rest pulls the side
//!   away);
//! * the shear stress is the mean of the force across the normal over the area (the parabola of a beam's shear, 1.5 times the mean at the middle of a
//!   rectangle, is not taken: a joint is a bond, and the mean is the load per area that it has to take);
//! * the bending stress is linear over the section, `g . r`, `r` the position from the centre of the section and `g = J^-1 (n x M)` with `J` the second
//!   moments of the section's faces about its centre (the section modulus `W = I / c` of a beam is the bending moment's stress at the fibre `c` away,
//!   which this is, with the exact `I`) and `M` the part of the moment that is in the plane of the section, the stress that a joint takes being the
//!   greatest `g . r` over the corners of its own box;
//! * the twist, the part of the moment along the normal, is a shear by the polar moment of the section, `T r / J_p`, `r` the farthest corner of the
//!   joint: the formula of a round section, which a rectangle exceeds (by an eighth for a square, more for a thin one).
//!
//! The joint breaks when the principal tension at its worst fibre, `s / 2 + sqrt(s^2 / 4 + t^2)` with `s` the normal and bending stress added and `t`
//! the shear and the twist added (their directions are not worked out, so the sum is a bound), reaches the strength: a brittle material under Rankine's
//! criterion. A pull makes it `s`, a shear alone makes it `t`, and a push alone makes it zero: compression does not break a joint by itself.
//!
//! What this does not do: it does not know where the load comes from (the world works that out from the motion of the body and its contacts), it does not
//! take a joint's stress concentrations (a corner, a hole) or its own softness (the joint is rigid until it breaks), and a section that is not flat is a
//! flat one with the mean normal of its faces: its faces' own tilt is in the second moments but not in the direction of the stress.

pub mod balance;

pub(crate) type V3 = [f64; 3];

pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f64) -> V3 {
    a.map(|v| v * s)
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The section of one joint, in the frame of the body and in metres: what `sr_3d::pieces::Section` gives for the faces of the pieces that it joins, with the
/// lattice's axes turned into the physics' and the lengths into metres.
#[derive(Clone, Debug, PartialEq)]
pub struct JointSection {
    /// Square metres.
    pub area: f64,
    /// The centre of the faces, weighted by their areas.
    pub centroid: V3,
    /// The integral of `r r^T` over the faces, `r` the position from `centroid`, in metres to the fourth.
    pub second: [V3; 3],
    /// The unit normal from the piece `a` of the joint to its piece `b`.
    pub normal: V3,
    /// The box that holds the faces: its corners are where the stress is read.
    pub lo: V3,
    pub hi: V3,
}

/// The load that the rest of a body puts on one side of a cut: a force (newtons) and a moment (newton metres) about `point`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrench {
    pub force: V3,
    pub moment: V3,
    pub point: V3,
}

/// The stresses at the worst fibre of one joint of a cut, in pascals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutStress {
    /// `N / A`: positive in tension.
    pub normal: f64,
    /// The mean shear of the force across the section.
    pub shear: f64,
    /// The greatest bending stress over the corners of the joint's box: positive in tension, and negative if the whole joint is in compression by bending.
    pub bending: f64,
    /// The shear of the twist at the farthest corner of the joint's box.
    pub torsion: f64,
    /// The principal tension of the stresses added (zero if they only compress).
    pub principal: f64,
}

/// The stresses in the joints of a cut under the load `load` that the rest puts on the side of it that has the piece `a` of joint `i` where `side_has_a[i]`
/// is true, and the piece `b` where it is false (the normal of a joint is from `a` to `b`, so from a side that has `a` it points to the rest). The joints
/// are one rigid section; the result has one entry for each joint, in order. A section that cannot carry the bending moment that it is asked for (a line,
/// or no area) has an infinite bending stress.
pub fn cut_stresses(joints: &[&JointSection], side_has_a: &[bool], load: &Wrench) -> Vec<CutStress> {
    assert_eq!(joints.len(), side_has_a.len(), "a side for every joint");
    let area: f64 = joints.iter().map(|j| j.area).sum();
    if joints.is_empty() || area.is_nan() || area <= 0.0 {
        return vec![CutStress { normal: 0.0, shear: 0.0, bending: 0.0, torsion: 0.0, principal: 0.0 }; joints.len()];
    }
    let centre: V3 = std::array::from_fn(|a| joints.iter().map(|j| j.area * j.centroid[a]).sum::<f64>() / area);
    // the second moments of the whole section about its centre: each joint's own and its area at its distance
    let mut second = [[0.0; 3]; 3];
    for j in joints {
        let d = sub(j.centroid, centre);
        for a in 0..3 {
            for b in 0..3 {
                second[a][b] += j.second[a][b] + j.area * d[a] * d[b];
            }
        }
    }
    // the normal of the section, from the side to the rest: the mean of the joints' normals with the signs of the side, weighted by their areas; if they cancel,
    // the direction of the force, and the first joint's if there is none
    let outward: Vec<V3> =
        joints.iter().zip(side_has_a).map(|(j, a)| scale(j.normal, if *a { 1.0 } else { -1.0 })).collect();
    let mean: V3 = std::array::from_fn(|a| joints.iter().zip(&outward).map(|(j, n)| j.area * n[a]).sum());
    let n = if norm(mean) > 1e-9 * area {
        mean.map(|c| c / norm(mean))
    } else if norm(load.force) > 0.0 {
        load.force.map(|c| c / norm(load.force))
    } else {
        outward[0]
    };
    let pull = dot(load.force, n);
    let across = sub(load.force, scale(n, pull));
    // the moment about the centre of the section
    let moment = {
        let lever = cross(sub(load.point, centre), load.force);
        [load.moment[0] + lever[0], load.moment[1] + lever[1], load.moment[2] + lever[2]]
    };
    let twist = dot(moment, n);
    let bending_moment = sub(moment, scale(n, twist));
    // a basis of the plane of the section
    let helper = if n[0].abs() < 0.6 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = {
        let t = sub(helper, scale(n, dot(helper, n)));
        t.map(|c| c / norm(t))
    };
    let v = cross(n, u);
    let apply = |m: &[V3; 3], x: V3| -> V3 { std::array::from_fn(|a| dot(m[a], x)) };
    let (juu, juv, jvv) = (dot(u, apply(&second, u)), dot(u, apply(&second, v)), dot(v, apply(&second, v)));
    // g such that J g = n x M in the plane: the eigenvectors of the 2 x 2 moment, each solved for, an infinite one where the section has no moment to carry it
    let w = cross(n, bending_moment);
    let (wu, wv) = (dot(w, u), dot(w, v));
    let mean_j = (juu + jvv) / 2.0;
    let radius = (((juu - jvv) / 2.0).powi(2) + juv * juv).sqrt();
    let (big, small) = (mean_j + radius, mean_j - radius);
    let angle = 0.5 * (2.0 * juv).atan2(juu - jvv);
    let (c, s) = (angle.cos(), angle.sin());
    // the eigenvectors in the (u, v) coordinates: (c, s) for the larger and (-s, c) for the smaller
    let (w_big, w_small) = (c * wu + s * wv, -s * wu + c * wv);
    let part = |component: f64, moment: f64| -> f64 {
        if component == 0.0 {
            0.0
        } else if moment > 1e-12 * big.max(f64::MIN_POSITIVE) {
            component / moment
        } else {
            f64::INFINITY.copysign(component)
        }
    };
    let (g_big, g_small) = (part(w_big, big), part(w_small, small));
    // g in the plane, as a vector: the infinite parts stay infinite through the products below
    let direction_big = [c * u[0] + s * v[0], c * u[1] + s * v[1], c * u[2] + s * v[2]];
    let direction_small = [-s * u[0] + c * v[0], -s * u[1] + c * v[1], -s * u[2] + c * v[2]];
    let polar = juu + jvv;
    joints
        .iter()
        .map(|j| {
            let mut bending = f64::NEG_INFINITY;
            let mut reach = 0.0f64;
            for corner in 0..8 {
                let p: V3 = std::array::from_fn(|a| if corner >> a & 1 == 1 { j.hi[a] } else { j.lo[a] });
                let r = sub(p, centre);
                // the infinity of a part that the section cannot carry is only felt by a corner that is off its axis
                let along = |g: f64, d: V3| {
                    let x = dot(d, r);
                    if g == 0.0 || x == 0.0 {
                        0.0
                    } else {
                        g * x
                    }
                };
                bending = bending.max(along(g_big, direction_big) + along(g_small, direction_small));
                let in_plane = sub(r, scale(n, dot(r, n)));
                reach = reach.max(norm(in_plane));
            }
            let normal = pull / area;
            let shear = norm(across) / area;
            let torsion = if twist == 0.0 {
                0.0
            } else if polar > 0.0 {
                twist.abs() * reach / polar
            } else {
                f64::INFINITY
            };
            let sigma = normal + bending;
            let tau = shear + torsion;
            let principal = (sigma / 2.0 + (sigma * sigma / 4.0 + tau * tau).sqrt()).max(0.0);
            CutStress { normal, shear, bending, torsion, principal }
        })
        .collect()
}
