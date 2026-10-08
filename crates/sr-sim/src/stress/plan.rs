//! The cuts of a body made of pieces that are joined: for every intact joint, the side of the body that its load is worked out on, and the joints that the
//! load is shared over. A pure function of the graph and the geometry of the joints, worked out once and again when a joint breaks.
//!
//! The load on a joint is the load on one side of a cut that has it. For a joint that is the only way from one part of the body to another (a *bridge*: a tree
//! has nothing else) the cut is that joint alone and its side is the part of the body that it leaves on the side of its piece `a`: the load is exact, by
//! the balance of that part, whatever the loads are. For a joint that is in a cycle the load is statically indeterminate (a body of pieces that are
//! all joined to their neighbours is a mesh, and a mesh can be loaded in more than one way for the same forces), and the rule is the one of a beam: the
//! section is the plane of the joint (through its centre, normal to its mean normal), the side is the pieces whose centres are on the `a` side of it, the
//! cut is the joints that the side has with the rest, and the load of the side is shared over them as over one rigid section (by area for a pull, by
//! the distance from the centre of the section for a bending moment). A cut of the least area was the other thing to take, and it isolates the weakest piece
//! of a block (the cut of two neighbouring pieces in the middle of a block is the cut round the smaller of them), whose load is its own weight and
//! says nothing of the load that the block carries across: the plane is where a load is carried.
//!
//! If the pieces `a` and `b` of a joint are not on the two sides of its plane (a joint of pieces that are tangled) the side is the piece `a` alone.

use super::balance::MassSum;
use super::{dot, sub, V3};

/// The geometry of a piece that a cut needs: where it is and its mass sums, in the frame of the body.
#[derive(Clone, Copy, Debug)]
pub struct PieceGeom {
    pub centre: V3,
    pub mass: MassSum,
}

/// The geometry of a joint that a cut needs: the pieces it joins (`a < b`), its centre and its normal from `a` to `b`.
#[derive(Clone, Copy, Debug)]
pub struct JointGeom {
    pub a: u32,
    pub b: u32,
    pub centroid: V3,
    pub normal: V3,
}

/// The side of a cut: the pieces on one side of a plane (their centres), or a list of them.
#[derive(Clone, Debug, PartialEq)]
pub enum Side {
    Plane { point: V3, normal: V3 },
    List(Vec<u32>),
}

impl Side {
    /// Whether the piece `i`, whose centre is `centre`, is on the side.
    pub fn contains(&self, i: u32, centre: V3) -> bool {
        match self {
            Side::Plane { point, normal } => dot(sub(centre, *point), *normal) <= 0.0,
            Side::List(list) => list.binary_search(&i).is_ok(),
        }
    }
}

/// A cut: a side, the joints that the side has with the rest of the body (intact ones) and which side of each is the `a` of it, the joints whose load is
/// the one of this cut, and the mass sums of the side.
#[derive(Clone, Debug, PartialEq)]
pub struct CutPlan {
    pub side: Side,
    pub members: Vec<usize>,
    pub side_has_a: Vec<bool>,
    pub targets: Vec<usize>,
    pub mass: MassSum,
}

/// The joints that are intact among those the held pieces have: both ends held and not broken.
pub fn intact_joints(held: &[u32], broken: &[bool], joints: &[JointGeom]) -> Vec<usize> {
    let max = joints
        .iter()
        .map(|j| j.a.max(j.b))
        .max()
        .map_or(0, |m| m as usize + 1)
        .max(held.last().map_or(0, |h| *h as usize + 1));
    let mut is_held = vec![false; max];
    for h in held {
        is_held[*h as usize] = true;
    }
    (0..joints.len())
        .filter(|&k| !broken[k] && is_held[joints[k].a as usize] && is_held[joints[k].b as usize])
        .collect()
}

/// The connected components of the held pieces by their intact joints, each sorted, in the order of their lowest piece.
pub fn components(held: &[u32], broken: &[bool], joints: &[JointGeom]) -> Vec<Vec<u32>> {
    let intact = intact_joints(held, broken, joints);
    let n = held
        .last()
        .map_or(0, |h| *h as usize + 1)
        .max(joints.iter().map(|j| j.a.max(j.b) as usize + 1).max().unwrap_or(0));
    let mut adjacent: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &k in &intact {
        adjacent[joints[k].a as usize].push(joints[k].b);
        adjacent[joints[k].b as usize].push(joints[k].a);
    }
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    for &start in held {
        if seen[start as usize] {
            continue;
        }
        let mut component = vec![start];
        seen[start as usize] = true;
        let mut at = 0;
        while at < component.len() {
            let piece = component[at];
            at += 1;
            for &next in &adjacent[piece as usize] {
                if !seen[next as usize] {
                    seen[next as usize] = true;
                    component.push(next);
                }
            }
        }
        component.sort_unstable();
        out.push(component);
    }
    out
}

/// The bridges among the intact joints (a joint whose removal leaves its two pieces apart), by the low-link search of Tarjan, without recursion.
fn bridges(held: &[u32], intact: &[usize], joints: &[JointGeom]) -> Vec<bool> {
    let n = held
        .last()
        .map_or(0, |h| *h as usize + 1)
        .max(joints.iter().map(|j| j.a.max(j.b) as usize + 1).max().unwrap_or(0));
    let mut adjacent: Vec<Vec<(u32, usize)>> = vec![Vec::new(); n];
    for &k in intact {
        adjacent[joints[k].a as usize].push((joints[k].b, k));
        adjacent[joints[k].b as usize].push((joints[k].a, k));
    }
    let mut order = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut bridge = vec![false; joints.len()];
    let mut counter = 0u32;
    for &root in held {
        if order[root as usize] != u32::MAX {
            continue;
        }
        // (piece, the joint it was reached by, the next neighbour to look at)
        let mut stack: Vec<(u32, Option<usize>, usize)> = vec![(root, None, 0)];
        order[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        while let Some(top) = stack.last_mut() {
            let (piece, via, next) = *top;
            if next < adjacent[piece as usize].len() {
                top.2 += 1;
                let (to, k) = adjacent[piece as usize][next];
                if Some(k) == via {
                    continue;
                }
                if order[to as usize] == u32::MAX {
                    order[to as usize] = counter;
                    low[to as usize] = counter;
                    counter += 1;
                    stack.push((to, Some(k), 0));
                } else {
                    low[piece as usize] = low[piece as usize].min(order[to as usize]);
                }
            } else {
                stack.pop();
                if let (Some(parent), Some(k)) = (stack.last(), via) {
                    let parent = parent.0 as usize;
                    low[parent] = low[parent].min(low[piece as usize]);
                    if low[piece as usize] > order[parent] {
                        bridge[k] = true;
                    }
                }
            }
        }
    }
    bridge
}

/// The cuts of the body that has the pieces `held` joined by the joints that are not `broken`: one for each distinct cut, in the order of the first joint
/// whose cut it is. `pieces` and `joints` are indexed by piece and joint number; `broken` has an entry for every joint.
pub fn plan(held: &[u32], broken: &[bool], pieces: &[PieceGeom], joints: &[JointGeom]) -> Vec<CutPlan> {
    let intact = intact_joints(held, broken, joints);
    let bridge = bridges(held, &intact, joints);
    let mut adjacent: Vec<Vec<(u32, usize)>> = vec![Vec::new(); pieces.len()];
    for &k in &intact {
        adjacent[joints[k].a as usize].push((joints[k].b, k));
        adjacent[joints[k].b as usize].push((joints[k].a, k));
    }
    let mut plans: Vec<CutPlan> = Vec::new();
    for &j in &intact {
        let joint = &joints[j];
        let side = if bridge[j] {
            // the part that the piece `a` has left once the joint is gone
            let mut seen = vec![false; pieces.len()];
            let mut list = vec![joint.a];
            seen[joint.a as usize] = true;
            let mut at = 0;
            while at < list.len() {
                let piece = list[at];
                at += 1;
                for &(next, k) in &adjacent[piece as usize] {
                    if k != j && !seen[next as usize] {
                        seen[next as usize] = true;
                        list.push(next);
                    }
                }
            }
            list.sort_unstable();
            Side::List(list)
        } else {
            let plane = Side::Plane { point: joint.centroid, normal: joint.normal };
            if plane.contains(joint.a, pieces[joint.a as usize].centre)
                && !plane.contains(joint.b, pieces[joint.b as usize].centre)
            {
                plane
            } else {
                Side::List(vec![joint.a])
            }
        };
        let in_side = |i: u32| side.contains(i, pieces[i as usize].centre);
        let mut members = Vec::new();
        let mut side_has_a = Vec::new();
        for &k in &intact {
            let (a, b) = (in_side(joints[k].a), in_side(joints[k].b));
            if a != b {
                members.push(k);
                side_has_a.push(a);
            }
        }
        if let Some(existing) = plans.iter_mut().find(|p| p.members == members && p.side_has_a == side_has_a) {
            existing.targets.push(j);
            continue;
        }
        let mut mass = MassSum::default();
        for &i in held {
            if in_side(i) {
                mass.add(&pieces[i as usize].mass);
            }
        }
        plans.push(CutPlan { side, members, side_has_a, targets: vec![j], mass });
    }
    plans
}
