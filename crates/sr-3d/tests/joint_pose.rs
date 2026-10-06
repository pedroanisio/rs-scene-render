//! Joint pose offsets and look-at on an imported model's local transforms.

use glam::{Quat, Vec3};
use sr_3d::{anim, Model, Node, Trs};

/// root (at the origin) -> arm (1 along x) -> hand (1 along x again).
fn chain() -> Model {
    let mut m = Model::default();
    let node = |name: &str, parent, x: f32| Node {
        name: name.into(),
        parent,
        local: Trs { t: Vec3::new(x, 0.0, 0.0), ..Default::default() },
        ..Default::default()
    };
    m.nodes = vec![node("root", None, 0.0), node("arm", Some(0), 1.0), node("hand", Some(1), 1.0)];
    m
}

fn world_pos(m: &Model, locals: &[Trs], k: usize) -> Vec3 {
    m.world_matrices(locals)[k].transform_point3(Vec3::ZERO)
}

fn rest(m: &Model) -> Vec<Trs> {
    m.nodes.iter().map(|n| n.local).collect()
}

#[test]
fn a_rotation_offset_turns_the_joint_in_its_own_axes_and_carries_its_children() {
    let m = chain();
    let mut l = rest(&m);
    anim::pose_joint(&mut l, 1, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    // the arm stays where it is; the hand, one unit along the arm's x, is now one unit along y
    assert!((world_pos(&m, &l, 1) - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    assert!((world_pos(&m, &l, 2) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-5);
}

#[test]
fn look_at_points_the_axis_at_the_target_without_moving_the_joint_or_its_parent() {
    let m = chain();
    let mut l = rest(&m);
    anim::look_at(&m, &mut l, 1, Vec3::new(1.0, 5.0, 0.0), Vec3::X, 1.0, std::f32::consts::PI);
    assert!((world_pos(&m, &l, 1) - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5, "the joint stays");
    assert!((world_pos(&m, &l, 2) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-5, "the hand is straight up from it");
}

#[test]
fn influence_scales_and_max_angle_limits_the_turn() {
    let m = chain();
    let turned = |influence: f32, max: f32| {
        let mut l = rest(&m);
        anim::look_at(&m, &mut l, 1, Vec3::new(1.0, 5.0, 0.0), Vec3::X, influence, max);
        let d = world_pos(&m, &l, 2) - world_pos(&m, &l, 1);
        d.y.atan2(d.x).to_degrees()
    };
    assert!((turned(1.0, std::f32::consts::PI) - 90.0).abs() < 1e-3);
    assert!((turned(0.5, std::f32::consts::PI) - 45.0).abs() < 1e-3);
    assert!((turned(1.0, 30f32.to_radians()) - 30.0).abs() < 1e-3);
    assert!(turned(0.0, std::f32::consts::PI).abs() < 1e-3);
}

#[test]
fn look_at_works_under_a_rotated_parent() {
    let m = chain();
    let mut l = rest(&m);
    // the root turns a quarter about z: the arm is at (0, 1, 0) and its x axis points along y
    anim::pose_joint(&mut l, 0, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    anim::look_at(&m, &mut l, 1, Vec3::new(5.0, 1.0, 0.0), Vec3::X, 1.0, std::f32::consts::PI);
    let d = world_pos(&m, &l, 2) - world_pos(&m, &l, 1);
    assert!((d.normalize() - Vec3::X).length() < 1e-4, "{d:?}");
    assert!((world_pos(&m, &l, 1) - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5, "the arm did not move");
}

#[test]
fn a_target_at_the_joint_or_an_aligned_axis_changes_nothing() {
    let m = chain();
    let mut l = rest(&m);
    anim::look_at(&m, &mut l, 1, Vec3::new(1.0, 0.0, 0.0), Vec3::X, 1.0, 3.0);
    anim::look_at(&m, &mut l, 1, Vec3::new(9.0, 0.0, 0.0), Vec3::X, 1.0, 3.0);
    assert_eq!(l, rest(&m));
}
