//! Two clips blended by a weight: translation and scale linearly, rotation along the shortest arc, morph weights
//! linearly; a node a clip does not animate keeps its rest pose in that clip.

use glam::{Quat, Vec3};
use sr_3d::{anim, Animation, Channel, Interp, Model, Node, Path, Trs};

fn channel(node: usize, path: Path, times: &[f32], values: &[f32]) -> Channel {
    Channel { node, path, interp: Interp::Linear, times: times.to_vec(), values: values.to_vec() }
}

fn model() -> Model {
    let node = |name: &str, parent| Node { name: name.into(), parent, weights: vec![0.0, 0.0], ..Default::default() };
    let mut m = Model::default();
    m.nodes = vec![node("root", None), node("arm", Some(0))];
    m.nodes[1].local = Trs { t: Vec3::new(1.0, 0.0, 0.0), ..Default::default() };
    m
}

fn quat(deg: f32) -> [f32; 4] {
    Quat::from_rotation_z(deg.to_radians()).to_array()
}

/// "a": the root turns 0 -> 90 degrees and the arm slides x 1 -> 11 over one second; "b": the root turns 0 -> -60 and
/// the arm's two morph weights go 0 -> 1, and nothing moves the arm's translation.
fn clips() -> (Animation, Animation) {
    let a = Animation {
        name: "a".into(),
        duration: 1.0,
        channels: vec![
            channel(0, Path::Rotation, &[0.0, 1.0], &[quat(0.0), quat(90.0)].concat()),
            channel(1, Path::Translation, &[0.0, 1.0], &[1.0, 0.0, 0.0, 11.0, 0.0, 0.0]),
        ],
    };
    let b = Animation {
        name: "b".into(),
        duration: 1.0,
        channels: vec![
            channel(0, Path::Rotation, &[0.0, 1.0], &[quat(0.0), quat(-60.0)].concat()),
            channel(1, Path::Weights, &[0.0, 1.0], &[0.0, 0.0, 1.0, 1.0]),
        ],
    };
    (a, b)
}

fn angle(q: Quat) -> f32 {
    q.to_euler(glam::EulerRot::ZYX).0.to_degrees()
}

#[test]
fn weight_zero_is_the_first_clip_and_one_is_the_second() {
    let (m, (a, b)) = (model(), clips());
    let (la, wa) = anim::pose(&m, Some(&a), 0.5);
    let (lb, wb) = anim::pose(&m, Some(&b), 0.25);
    let zero = anim::pose_blend(&m, Some(&a), 0.5, Some(&b), 0.25, 0.0);
    let one = anim::pose_blend(&m, Some(&a), 0.5, Some(&b), 0.25, 1.0);
    assert_eq!((zero.0.as_slice(), &zero.1), (la.as_slice(), &wa));
    assert_eq!((one.0.as_slice(), &one.1), (lb.as_slice(), &wb));
}

#[test]
fn halfway_blends_translation_linearly_and_rotation_along_the_arc() {
    let (m, (a, b)) = (model(), clips());
    let (l, w) = anim::pose_blend(&m, Some(&a), 1.0, Some(&b), 1.0, 0.5);
    // root: halfway between +90 and -60 degrees is +15
    assert!((angle(l[0].r) - 15.0).abs() < 0.01, "{}", angle(l[0].r));
    // arm translation: clip a gives x = 11, clip b leaves the rest pose x = 1: halfway 6
    assert!((l[1].t.x - 6.0).abs() < 1e-4, "{:?}", l[1].t);
    // morph weights: clip a leaves the rest weights (0, 0), clip b reaches (1, 1)
    assert_eq!(w[1], vec![0.5, 0.5]);
}

#[test]
fn rotation_takes_the_shortest_arc_when_the_quaternions_have_opposite_signs() {
    let (m, (a, b)) = (model(), clips());
    // negating a quaternion is the same rotation: blending 80 degrees with the negated 100 degrees must give 90
    let mut flipped = b.clone();
    flipped.channels[0] = channel(
        0,
        Path::Rotation,
        &[0.0, 1.0],
        &[(-Quat::from_rotation_z(1.7453292)).to_array(), (-Quat::from_rotation_z(1.7453292)).to_array()].concat(),
    );
    let mut first = a.clone();
    first.channels[0] = channel(0, Path::Rotation, &[0.0, 1.0], &[quat(80.0), quat(80.0)].concat());
    let (l, _) = anim::pose_blend(&m, Some(&first), 0.0, Some(&flipped), 0.0, 0.5);
    assert!((angle(l[0].r) - 90.0).abs() < 0.05, "{}", angle(l[0].r));
}

#[test]
fn no_second_clip_blends_with_the_rest_pose() {
    let m = model();
    let (a, _) = clips();
    let (l, _) = anim::pose_blend(&m, Some(&a), 1.0, None, 0.0, 0.5);
    assert!((angle(l[0].r) - 45.0).abs() < 0.01, "{}", angle(l[0].r));
}
