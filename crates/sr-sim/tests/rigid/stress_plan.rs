//! The cuts of a body of joined pieces: a joint that is the only way between two parts of the body is cut by itself and its side is exact, a joint in a cycle is cut
//! by its plane and shares its cut with the joints that cross the same plane, and a joint that breaks changes the cuts of the others.

use sr_sim::stress::balance::MassSum;
use sr_sim::stress::plan::{components, intact_joints, plan, JointGeom, PieceGeom, Side};

fn piece(c: [f64; 3]) -> PieceGeom {
    PieceGeom { centre: c, mass: MassSum { mass: 1.0, first: c, second: [[0.0; 3]; 3] } }
}

fn joint(a: u32, b: u32, centroid: [f64; 3], normal: [f64; 3]) -> JointGeom {
    JointGeom { a, b, centroid, normal }
}

/// A row of `n` pieces along x, 1 m apart, joined one to the next.
fn row(n: usize) -> (Vec<PieceGeom>, Vec<JointGeom>) {
    let pieces = (0..n).map(|i| piece([i as f64, 0.0, 0.0])).collect();
    let joints =
        (0..n - 1).map(|i| joint(i as u32, i as u32 + 1, [i as f64 + 0.5, 0.0, 0.0], [1.0, 0.0, 0.0])).collect();
    (pieces, joints)
}

/// Four pieces in a square, the pieces 0 and 1 below and 2 and 3 above, each joined to its two neighbours: a ring.
fn ring() -> (Vec<PieceGeom>, Vec<JointGeom>) {
    let pieces = vec![piece([0.0, 0.0, 0.0]), piece([1.0, 0.0, 0.0]), piece([0.0, 1.0, 0.0]), piece([1.0, 1.0, 0.0])];
    let joints = vec![
        joint(0, 1, [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]),
        joint(0, 2, [0.0, 0.5, 0.0], [0.0, 1.0, 0.0]),
        joint(1, 3, [1.0, 0.5, 0.0], [0.0, 1.0, 0.0]),
        joint(2, 3, [0.5, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    (pieces, joints)
}

#[test]
fn every_joint_of_a_row_is_a_bridge_cut_by_itself_with_the_side_of_its_piece_a() {
    let (pieces, joints) = row(5);
    let held: Vec<u32> = (0..5).collect();
    let cuts = plan(&held, &[false; 4], &pieces, &joints);
    assert_eq!(cuts.len(), 4);
    for (k, cut) in cuts.iter().enumerate() {
        assert_eq!(
            (cut.members.as_slice(), cut.targets.as_slice(), cut.side_has_a.as_slice()),
            (&[k][..], &[k][..], &[true][..])
        );
        // the part that the piece a has left once the joint is gone: the pieces 0 to k
        assert_eq!(cut.side, Side::List((0..=k as u32).collect()), "joint {k}");
        assert_eq!(cut.mass.mass, (k + 1) as f64);
    }
}

#[test]
fn a_ring_is_cut_by_the_plane_of_each_joint_and_the_joints_across_the_same_plane_share_the_cut() {
    let (pieces, joints) = ring();
    let held: Vec<u32> = (0..4).collect();
    let cuts = plan(&held, &[false; 4], &pieces, &joints);
    // two cuts, not four: the plane x = 0.5 is crossed by the joints 0 and 3, the plane y = 0.5 by the joints 1 and 2
    assert_eq!(cuts.len(), 2);
    let across_x = &cuts[0];
    assert_eq!((across_x.members.as_slice(), across_x.targets.as_slice()), (&[0, 3][..], &[0, 3][..]));
    // the side of the plane is the pieces 0 and 2 (x < 0.5): the joint 0 has its piece a on it, and so has the joint 3 (its a is the piece 2)
    assert_eq!(across_x.side_has_a, [true, true]);
    assert_eq!(across_x.mass.mass, 2.0);
    let across_y = &cuts[1];
    assert_eq!((across_y.members.as_slice(), across_y.targets.as_slice()), (&[1, 2][..], &[1, 2][..]));
    assert_eq!(across_y.side_has_a, [true, true]);
    // the side is a plane, and tells the pieces that are on it from their centres: 0 and 2 for x = 0.5, 0 and 1 for y = 0.5
    let on = |cut: &sr_sim::stress::plan::CutPlan| -> Vec<u32> {
        (0..4u32).filter(|&i| cut.side.contains(i, pieces[i as usize].centre)).collect()
    };
    assert_eq!((on(across_x), on(across_y)), (vec![0, 2], vec![0, 1]));
}

#[test]
fn a_joint_that_breaks_turns_the_ring_into_a_row_of_bridges() {
    let (pieces, joints) = ring();
    let held: Vec<u32> = (0..4).collect();
    // the joint 0 (between the pieces 0 and 1) gone: the path 0 - 2 - 3 - 1, all bridges
    let cuts = plan(&held, &[true, false, false, false], &pieces, &joints);
    assert_eq!(cuts.len(), 3);
    assert!(cuts.iter().all(|c| c.members.len() == 1 && matches!(c.side, Side::List(_))));
    // the joint 1 (0-2): the part with the piece 0 is the piece 0 alone
    let one = cuts.iter().find(|c| c.targets == [1]).unwrap();
    assert_eq!(one.side, Side::List(vec![0]));
    // the joint 3 (2-3): the part with the piece 2 is the pieces 0 and 2
    let three = cuts.iter().find(|c| c.targets == [3]).unwrap();
    assert_eq!(three.side, Side::List(vec![0, 2]));
    // and the joints of pieces that the body does not hold are not in it
    let held = [0u32, 1];
    let cuts = plan(&held, &[false; 4], &pieces, &joints);
    assert_eq!(cuts.len(), 1);
    assert_eq!(cuts[0].targets, [0]);
    assert_eq!(intact_joints(&held, &[false; 4], &joints), [0]);
}

#[test]
fn the_components_of_what_is_held_are_the_pieces_that_are_still_joined_in_the_order_of_their_lowest() {
    let (_, joints) = row(6);
    let held: Vec<u32> = (0..6).collect();
    let mut broken = vec![false; 5];
    assert_eq!(components(&held, &broken, &joints), vec![vec![0, 1, 2, 3, 4, 5]]);
    broken[1] = true;
    broken[3] = true;
    assert_eq!(components(&held, &broken, &joints), vec![vec![0, 1], vec![2, 3], vec![4, 5]]);
    // a body that holds only some of the pieces
    assert_eq!(components(&[1, 2, 4], &[false; 5], &joints), vec![vec![1, 2], vec![4]]);
    // a ring with one joint broken is still one body: nothing separates
    let (_, joints) = ring();
    assert_eq!(components(&[0, 1, 2, 3], &[true, false, false, false], &joints), vec![vec![0, 1, 2, 3]]);
}

#[test]
fn a_joint_whose_pieces_are_not_on_the_two_sides_of_its_plane_is_cut_with_its_piece_a_alone() {
    // a ring of three pieces whose joint 0 has a normal that is across the pieces (the plane through it leaves both pieces on one side)
    let pieces = vec![piece([0.0, 0.0, 0.0]), piece([1.0, 0.0, 0.0]), piece([0.5, 1.0, 0.0])];
    let joints = vec![
        joint(0, 1, [0.5, 0.0, 0.0], [0.0, 1.0, 0.0]),
        joint(0, 2, [0.25, 0.5, 0.0], [0.0, 1.0, 0.0]),
        joint(1, 2, [0.75, 0.5, 0.0], [0.0, 1.0, 0.0]),
    ];
    let cuts = plan(&[0, 1, 2], &[false; 3], &pieces, &joints);
    let first = cuts.iter().find(|c| c.targets.contains(&0)).unwrap();
    assert_eq!(first.side, Side::List(vec![0]));
    assert_eq!(first.members, [0, 1]);
    // the same plan, every time
    assert_eq!(cuts, plan(&[0, 1, 2], &[false; 3], &pieces, &joints));
}
