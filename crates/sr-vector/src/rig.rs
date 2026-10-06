//! Skeletons: bone hierarchies, world poses, inverse kinematics (analytic
//! two-bone and FABRIK) and linear blend skinning.

use crate::geom::{p, Xf, P};

/// One bone's local pose relative to its parent bone's space (x along the parent bone).
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub id: String,
    pub parent: Option<usize>,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub length: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

impl Bone {
    /// Local transform: bone space → parent bone space.
    pub fn local(&self) -> Xf {
        Xf::translate(self.x, self.y).mul(&Xf::rotate(self.rotation)).mul(&Xf::scale(self.scale_x, self.scale_y))
    }
}

/// World transforms (bone space → world) of every bone; parents must precede children.
pub fn world_poses(bones: &[Bone], root: &Xf) -> Vec<Xf> {
    let mut out: Vec<Xf> = Vec::with_capacity(bones.len());
    for b in bones {
        let parent = b.parent.and_then(|i| out.get(i).copied()).unwrap_or(*root);
        out.push(parent.mul(&b.local()));
    }
    out
}

/// World position of a bone's tip.
pub fn tip(world: &Xf, length: f64) -> P {
    world.apply(p(length, 0.0))
}

/// Chain of bone indices from the root-most ancestor to `end`.
pub fn chain(bones: &[Bone], end: usize) -> Vec<usize> {
    let mut c = vec![end];
    let mut cur = end;
    while let Some(par) = bones[cur].parent {
        c.push(par);
        cur = par;
        if c.len() > bones.len() {
            break;
        }
    }
    c.reverse();
    c
}

/// Sets a bone's local rotation so its world x axis points at `angle` (radians, world).
fn aim(bones: &mut [Bone], worlds: &[Xf], root: &Xf, i: usize, angle: f64) {
    let parent = bones[i].parent.map(|k| worlds[k]).unwrap_or(*root);
    let parent_angle = libm::atan2(parent.0[1], parent.0[0]);
    bones[i].rotation = (angle - parent_angle).to_degrees();
}

/// Solves IK so the tip of bone `end` reaches `target` (world). Chains of
/// two bones use the analytic solution with the bend side from
/// `bend_positive`; longer chains use FABRIK; a single bone aims at the target.
pub fn solve_ik(bones: &mut [Bone], root: &Xf, end: usize, target: P, bend_positive: bool, influence: f64) {
    // The chain is solved in the skeleton's own space, where bone lengths are what the document says: under a
    // rotated, flipped, skewed or unevenly scaled ancestor the frame-space distances between joints are not the
    // lengths the analytic solution needs. The target is taken into that space, and the bones keep local values.
    if let Some(inv) = root.inverse() {
        return solve_ik_in(bones, &Xf::IDENTITY, end, inv.apply(target), bend_positive, influence);
    }
    solve_ik_in(bones, root, end, target, bend_positive, influence)
}

fn solve_ik_in(bones: &mut [Bone], root: &Xf, end: usize, target: P, bend_positive: bool, influence: f64) {
    let mut ch = chain(bones, end);
    let before = bones.to_vec();
    let worlds = world_poses(bones, root);
    let mut joints: Vec<P> =
        ch.iter().map(|&i| worlds[i].origin()).chain(std::iter::once(tip(&worlds[end], bones[end].length))).collect();
    let mut lens: Vec<f64> = joints.windows(2).map(|w| w[0].dist(w[1])).collect();
    // zero-length bones at the base cannot move the tip
    while ch.len() > 1 && lens[0] < 1e-9 {
        ch.remove(0);
        joints.remove(0);
        lens.remove(0);
    }
    match ch.len() {
        0 => return,
        1 => {
            let a = (target - joints[0]).angle();
            aim(bones, &worlds, root, ch[0], a);
        }
        2 => {
            let (a, l1, l2) = (joints[0], lens[0], lens[1]);
            let d = (target - a).len().clamp((l1 - l2).abs() + 1e-9, l1 + l2 - 1e-9);
            let base = (target - a).angle();
            let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
            let sgn = if bend_positive { 1.0 } else { -1.0 };
            let a1 = base - sgn * libm::acos(cos_a);
            aim(bones, &worlds, root, ch[0], a1);
            let w2 = world_poses(bones, root);
            let joint = a + p(libm::cos(a1), libm::sin(a1)) * l1;
            aim(bones, &w2, root, ch[1], (target - joint).angle());
        }
        _ => {
            let mut j = joints.clone();
            let total: f64 = lens.iter().sum();
            let origin = j[0];
            if origin.dist(target) >= total {
                let dir = (target - origin).norm();
                for k in 1..j.len() {
                    j[k] = j[k - 1] + dir * lens[k - 1];
                }
            } else {
                for _ in 0..32 {
                    let last = j.len() - 1;
                    j[last] = target;
                    for k in (0..last).rev() {
                        j[k] = j[k + 1] + (j[k] - j[k + 1]).norm() * lens[k];
                    }
                    j[0] = origin;
                    for k in 1..j.len() {
                        j[k] = j[k - 1] + (j[k] - j[k - 1]).norm() * lens[k - 1];
                    }
                    if j[last].dist(target) < 1e-6 {
                        break;
                    }
                }
            }
            for (k, &bi) in ch.iter().enumerate() {
                let w = world_poses(bones, root);
                aim(bones, &w, root, bi, (j[k + 1] - j[k]).angle());
            }
        }
    }
    if influence < 1.0 {
        for &i in &ch {
            let (r0, r1) = (before[i].rotation, bones[i].rotation);
            let mut d = (r1 - r0).rem_euclid(360.0);
            if d > 180.0 {
                d -= 360.0;
            }
            bones[i].rotation = r0 + d * influence;
        }
    }
}

/// Linear blend skinning over rest and current bone poses.
#[derive(Debug, Clone)]
pub struct Skin {
    /// Rest world transforms, inverted.
    rest_inv: Vec<Xf>,
    /// Current world transforms.
    now: Vec<Xf>,
    /// Bone segments at rest (joint, tip) for automatic weights.
    segs: Vec<(P, P)>,
    /// Explicit weights at sample points: (point, [(bone, weight)]).
    samples: Vec<(P, Vec<(usize, f64)>)>,
    /// World ↔ node-local transforms.
    to_world: Xf,
    to_local: Xf,
}

impl Skin {
    /// Builds a skin. `to_world` maps the deformed node's local space to the skeleton's world.
    pub fn new(rest: &[Xf], now: &[Xf], lengths: &[f64], to_world: Xf, samples: Vec<(P, Vec<(usize, f64)>)>) -> Skin {
        let rest_inv = rest.iter().map(|x| x.inverse().unwrap_or(Xf::IDENTITY)).collect();
        let segs = rest.iter().zip(lengths).map(|(x, &l)| (x.origin(), tip(x, l))).collect();
        Skin {
            rest_inv,
            now: now.to_vec(),
            segs,
            samples,
            to_local: to_world.inverse().unwrap_or(Xf::IDENTITY),
            to_world,
        }
    }

    /// Weights of a world-space rest point: explicit samples (nearest) or inverse-distance to bone segments.
    pub fn weights(&self, w: P) -> Vec<(usize, f64)> {
        if !self.samples.is_empty() {
            let best = self.samples.iter().min_by(|a, b| a.0.dist(w).total_cmp(&b.0.dist(w))).unwrap();
            return best.1.clone();
        }
        let mut ws: Vec<(usize, f64)> = self
            .segs
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| {
                let ab = b - a;
                let t = ((w - a).dot(ab) / ab.dot(ab).max(1e-12)).clamp(0.0, 1.0);
                let d = (a + ab * t).dist(w);
                (i, 1.0 / (d + 1.0).powi(4))
            })
            .collect();
        ws.sort_by(|a, b| b.1.total_cmp(&a.1));
        ws.truncate(4);
        let s: f64 = ws.iter().map(|x| x.1).sum();
        ws.iter().map(|&(i, x)| (i, x / s.max(1e-300))).collect()
    }

    /// Maps a node-local rest point to its skinned position.
    pub fn map(&self, q: P) -> P {
        let w = self.to_world.apply(q);
        let mut acc = p(0.0, 0.0);
        let mut total = 0.0;
        for (i, wt) in self.weights(w) {
            if let (Some(ri), Some(ni)) = (self.rest_inv.get(i), self.now.get(i)) {
                acc = acc + ni.apply(ri.apply(w)) * wt;
                total += wt;
            }
        }
        if total <= 0.0 {
            return q;
        }
        self.to_local.apply(acc * (1.0 / total))
    }
}
