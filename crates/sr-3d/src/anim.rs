//! Animation sampling, morph targets and skinning (on the CPU, per frame).

use glam::{Mat3, Mat4, Quat, Vec3};

use crate::{Animation, Interp, Model, Path, Trs, Vertex};

fn key_span(times: &[f32], t: f32) -> (usize, usize, f32, f32) {
    let n = times.len();
    if n == 0 || t.is_nan() || t <= times[0] {
        return (0, 0, 0.0, 0.0);
    }
    if t >= times[n - 1] || n < 2 {
        return (n - 1, n - 1, 0.0, 0.0);
    }
    // NaN or unsorted times (or a NaN `t`) leave the search anywhere: keep it inside the keys
    let i = times.partition_point(|&x| x <= t).clamp(1, n - 1) - 1;
    let dt = times[i + 1] - times[i];
    (i, i + 1, if dt > 0.0 { (t - times[i]) / dt } else { 0.0 }, dt)
}

/// Samples `width` floats of channel values at time `t`.
fn sample(interp: Interp, times: &[f32], values: &[f32], width: usize, t: f32, rotation: bool) -> Vec<f32> {
    let (a, b, u, dt) = key_span(times, t);
    let cubic = interp == Interp::CubicSpline;
    let stride = if cubic { width * 3 } else { width };
    let at = |k: usize, part: usize| -> &[f32] {
        let o = k * stride + if cubic { part * width } else { 0 };
        values.get(o..o + width).unwrap_or(&[])
    };
    let va = at(a, 1);
    let vb = at(b, 1);
    if va.len() < width || vb.len() < width {
        return vec![0.0; width];
    }
    let mut out: Vec<f32> = match interp {
        Interp::Step => va.to_vec(),
        Interp::Linear if rotation => {
            let q = Quat::from_slice(va).slerp(Quat::from_slice(vb), u);
            q.to_array().to_vec()
        }
        Interp::Linear => (0..width).map(|k| va[k] + (vb[k] - va[k]) * u).collect(),
        Interp::CubicSpline => {
            let (out_a, in_b) = (at(a, 2), at(b, 0));
            let (u2, u3) = (u * u, u * u * u);
            let (h00, h10, h01, h11) = (2.0 * u3 - 3.0 * u2 + 1.0, u3 - 2.0 * u2 + u, -2.0 * u3 + 3.0 * u2, u3 - u2);
            (0..width)
                .map(|k| {
                    h00 * va[k]
                        + h10 * dt * out_a.get(k).unwrap_or(&0.0)
                        + h01 * vb[k]
                        + h11 * dt * in_b.get(k).unwrap_or(&0.0)
                })
                .collect()
        }
    };
    if rotation {
        let q = Quat::from_slice(&out).normalize();
        out = q.to_array().to_vec();
    }
    out
}

/// Node transforms and morph weights of `model` at `t` seconds into `anim` (rest pose when `None`).
pub fn pose(model: &Model, anim: Option<&Animation>, t: f32) -> (Vec<Trs>, Vec<Vec<f32>>) {
    let mut locals: Vec<Trs> = model.nodes.iter().map(|n| n.local).collect();
    let mut weights: Vec<Vec<f32>> = model.nodes.iter().map(|n| n.weights.clone()).collect();
    if let Some(a) = anim {
        for ch in &a.channels {
            let Some(l) = locals.get_mut(ch.node) else { continue };
            match ch.path {
                Path::Translation => l.t = Vec3::from_slice(&sample(ch.interp, &ch.times, &ch.values, 3, t, false)),
                Path::Scale => l.s = Vec3::from_slice(&sample(ch.interp, &ch.times, &ch.values, 3, t, false)),
                Path::Rotation => l.r = Quat::from_slice(&sample(ch.interp, &ch.times, &ch.values, 4, t, true)),
                Path::Weights => {
                    let keys = ch.times.len().max(1);
                    let per = ch.values.len() / keys / if ch.interp == Interp::CubicSpline { 3 } else { 1 };
                    weights[ch.node] = sample(ch.interp, &ch.times, &ch.values, per, t, false);
                }
            }
        }
    }
    (locals, weights)
}

/// The pose of `a` at `ta` seconds blended with the pose of `b` at `tb` by `w` (0: all `a`, 1: all `b`). Translation, scale
/// and morph weights blend linearly and rotation along the shortest arc. A clip that does not animate a node leaves it at
/// its rest pose in that clip; `None` is the rest pose throughout.
pub fn pose_blend(
    model: &Model,
    a: Option<&Animation>,
    ta: f32,
    b: Option<&Animation>,
    tb: f32,
    w: f32,
) -> (Vec<Trs>, Vec<Vec<f32>>) {
    let (la, wa) = pose(model, a, ta);
    if w <= 0.0 || w.is_nan() {
        return (la, wa);
    }
    let (lb, wb) = pose(model, b, tb);
    if w >= 1.0 {
        return (lb, wb);
    }
    let locals = la
        .iter()
        .zip(&lb)
        .map(|(x, y)| Trs {
            t: x.t.lerp(y.t, w),
            r: x.r.slerp(y.r, w).normalize(),
            s: x.s.lerp(y.s, w),
        })
        .collect();
    let weights = wa
        .iter()
        .zip(&wb)
        .map(|(x, y)| {
            if x.len() == y.len() {
                x.iter().zip(y).map(|(p, q)| p + (q - p) * w).collect()
            } else {
                // clips that disagree on the morph count: the first clip's weights
                x.clone()
            }
        })
        .collect();
    (locals, weights)
}

/// The rotation of angles (degrees) about x, then y, then z of the frame being turned: `Rz · Ry · Rx`, as an object's own
/// `rotationX`, `rotationY` and `rotation` compose.
pub fn euler_degrees(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_rotation_z(z.to_radians()) * Quat::from_rotation_y(y.to_radians()) * Quat::from_rotation_x(x.to_radians())
}

/// A rotation added to a joint's local rotation, in the joint's own axes (`locals[node].r · extra`).
pub fn pose_joint(locals: &mut [Trs], node: usize, extra: Quat) {
    if let Some(l) = locals.get_mut(node) {
        l.r = (l.r * extra).normalize();
    }
}

/// Turns a joint so that its `axis` (a unit vector in the joint's own frame) points at `target` (a point in the model's
/// own space): the shortest rotation, scaled by `influence` (0 to 1) and limited to `max_angle` radians, applied in the
/// world about the joint and expressed back in the local rotation, so the joint's parents are not moved. A target at the
/// joint, or an axis already on target, leaves the joint as it is.
pub fn look_at(model: &Model, locals: &mut [Trs], node: usize, target: Vec3, axis: Vec3, influence: f32, max_angle: f32) {
    let world = model.world_matrices(locals);
    let Some(w) = world.get(node) else { return };
    let (_, world_rot, position) = w.to_scale_rotation_translation();
    let want = target - position;
    let forward = world_rot * axis.normalize_or_zero();
    if want.length_squared() < 1e-12 || forward.length_squared() < 0.5 {
        return;
    }
    let turn = Quat::from_rotation_arc(forward, want.normalize());
    let mut turn = Quat::IDENTITY.slerp(turn, influence.clamp(0.0, 1.0));
    let (turn_axis, angle) = turn.to_axis_angle();
    if angle > max_angle {
        turn = Quat::from_axis_angle(turn_axis, max_angle.max(0.0));
    }
    let parent_rot = match model.nodes[node].parent {
        Some(p) => world[p].to_scale_rotation_translation().1,
        None => Quat::IDENTITY,
    };
    // world rotation turn · (parent · local) = parent · local' → local' = parent⁻¹ · turn · parent · local
    let local = &mut locals[node];
    local.r = (parent_rot.inverse() * turn * parent_rot * local.r).normalize();
}

/// One primitive to draw: its model-space matrix and, when deformed, its vertices.
pub struct DrawItem {
    pub node: usize,
    pub prim: usize,
    /// Model space (before `Model::basis`); identity for skinned vertices, which are already in model space.
    pub matrix: Mat4,
    pub vertices: Option<Vec<Vertex>>,
}

/// Every primitive instance of the model at a pose, with morphs and skinning applied.
pub fn draw_list(model: &Model, locals: &[Trs], weights: &[Vec<f32>], morph_override: Option<&[f32]>) -> Vec<DrawItem> {
    let world = model.world_matrices(locals);
    let mut out = Vec::new();
    for (ni, node) in model.nodes.iter().enumerate() {
        for &pi in &node.primitives {
            let prim = &model.primitives[pi];
            let w: &[f32] = morph_override.unwrap_or(&weights[ni]);
            let morphed = !prim.morphs.is_empty() && w.iter().any(|x| x.abs() > 1e-6);
            let skinned = node.skin.is_some()
                && prim.joints.len() == prim.vertices.len()
                && prim.weights.len() == prim.vertices.len();
            if !morphed && !skinned {
                out.push(DrawItem { node: ni, prim: pi, matrix: world[ni], vertices: None });
                continue;
            }
            let mut vs = prim.vertices.clone();
            if morphed {
                for (k, target) in prim.morphs.iter().enumerate() {
                    let wk = w.get(k).copied().unwrap_or(0.0);
                    if wk == 0.0 {
                        continue;
                    }
                    for (i, v) in vs.iter_mut().enumerate() {
                        if let Some(d) = target.dpos.get(i) {
                            v.pos.iter_mut().zip(d).for_each(|(p, dp)| *p += dp * wk);
                        }
                        if let Some(d) = target.dnormal.get(i) {
                            v.normal.iter_mut().zip(d).for_each(|(n, dn)| *n += dn * wk);
                        }
                    }
                }
            }
            let mut matrix = world[ni];
            if skinned {
                let skin = &model.skins[node.skin.unwrap_or(0)];
                let joints: Vec<Mat4> = skin
                    .joints
                    .iter()
                    .enumerate()
                    .map(|(k, &j)| {
                        world.get(j).copied().unwrap_or(Mat4::IDENTITY)
                            * skin.inverse_bind.get(k).copied().unwrap_or(Mat4::IDENTITY)
                    })
                    .collect();
                for (i, v) in vs.iter_mut().enumerate() {
                    let (js, ws) = (prim.joints[i], prim.weights[i]);
                    let sum: f32 = ws.iter().sum();
                    let mut m = Mat4::ZERO;
                    for k in 0..4 {
                        if ws[k] > 0.0 {
                            m +=
                                joints.get(js[k] as usize).copied().unwrap_or(Mat4::IDENTITY) * (ws[k] / sum.max(1e-6));
                        }
                    }
                    if sum <= 0.0 {
                        m = Mat4::IDENTITY;
                    }
                    v.pos = m.transform_point3(Vec3::from(v.pos)).into();
                    // Cofactors give det(M) * inverse-transpose(M), without
                    // dividing by zero for a collapsed joint scale.
                    let linear = Mat3::from_mat4(m);
                    let cofactor = Mat3::from_cols(
                        linear.y_axis.cross(linear.z_axis),
                        linear.z_axis.cross(linear.x_axis),
                        linear.x_axis.cross(linear.y_axis),
                    );
                    let sign = if linear.determinant() < 0.0 { -1.0 } else { 1.0 };
                    v.normal = (cofactor * Vec3::from(v.normal) * sign).normalize_or_zero().into();
                    let t =
                        m.transform_vector3(Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2])).normalize_or_zero();
                    v.tangent = [t.x, t.y, t.z, v.tangent[3]];
                }
                matrix = Mat4::IDENTITY;
            } else {
                for v in vs.iter_mut() {
                    v.normal = Vec3::from(v.normal).normalize_or_zero().into();
                }
            }
            out.push(DrawItem { node: ni, prim: pi, matrix, vertices: Some(vs) });
        }
    }
    out
}
