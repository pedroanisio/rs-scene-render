//! A crater in a body of cells as one cut for the rigid world: the cells it takes out leave, the rim is heaped on, the parts it leaves loose are
//! pieces, and the world that is given the cut has the body that the cells say.
#![allow(clippy::needless_range_loop)]

use super::crater::{ejection, ground, law, occupancy, H};
use sr_eval::voxel_crater::{crater_cut, Rock};
use sr_eval::voxels::{Overflow, Policy, Stay};
use sr_sim::fields::Field;
use sr_sim::physics3d::*;
use std::collections::BTreeSet;

const DENSITY: f64 = 2700.0;

fn rock(max_fragments: usize) -> Rock {
    Rock {
        size: [H; 3],
        density: DENSITY,
        pixels_per_meter: 1.0,
        policy: Policy { stay: Stay::Anchored, min_cells: 4, max_fragments, overflow: Overflow::Dust },
    }
}

/// What holds the ground: its lowest layer.
fn anchored(c: &[i32; 3]) -> bool {
    c[1] >= 39
}

fn set(cells: &[[i32; 3]]) -> BTreeSet<[i32; 3]> {
    cells.iter().copied().collect()
}

#[test]
fn the_cut_of_a_crater_heaps_its_rim_and_divides_every_cell_and_every_kilogram_once() {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = crater_cut(&before, &law.kernel, 1.0, &ejection(&law), 1, &rock(4), &anchored).unwrap();
    let cut = &x.cut.cut;
    assert_eq!(cut.revision, 1);
    // the rim is what the excavation heaps, none of it on a cell the ground has
    assert_eq!(set(&cut.added), x.excavation.rim.iter().map(|r| r.0).collect::<BTreeSet<_>>());
    assert_eq!(cut.added.len(), x.excavation.rim.len());
    assert!(!cut.added.is_empty() && cut.added.iter().all(|c| before.get(*c) == 0));
    // every cell of the ground and of the rim is in exactly one place: destroyed (taken out or dust), staying, or in a piece
    let mut places: Vec<[i32; 3]> = cut.destroyed.clone();
    places.extend(&x.cut.stays);
    places.extend(cut.pieces.iter().flat_map(|p| p.cells.iter().copied()));
    let mut all: BTreeSet<[i32; 3]> = before.cells().collect();
    all.extend(&cut.added);
    assert_eq!(places.len(), all.len(), "no cell twice");
    assert_eq!(set(&places), all, "no cell lost, none made up");
    assert!(set(&x.excavation.removed).is_subset(&set(&cut.destroyed)));
    // and every kilogram: what stays and what the pieces take is the cells that are not destroyed, at the mass of a cell
    let one = DENSITY * H * H * H;
    let kept = x.cut.stays.len() + cut.pieces.iter().map(|p| p.cells.len()).sum::<usize>();
    assert_eq!(kept + cut.destroyed.len(), all.len());
    let mass = cut.parent_mass + cut.pieces.iter().map(|p| p.mass).sum::<f64>();
    assert!((mass - kept as f64 * one).abs() < 1e-6 * mass, "{mass} against {}", kept as f64 * one);
}

/// Cuts the ground at 0.2 s with what the crater says and has the world tell what the body is, without a force on it.
struct Crater(Option<sr_sim::physics3d::VoxelCut3>, Vec<(u64, BodyState)>);

impl Driver3 for Crater {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        _: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        Ok(self.0.clone().filter(|_| parent == 0 && revision.is_none() && t + 1e-9 >= 0.2))
    }
    fn load(&mut self, step: u64, _: f64, body: usize, state: &BodyState) -> Result<Option<Load3>, String> {
        if body == 0 {
            self.1.push((step, *state));
        }
        Ok(None)
    }
}

#[test]
fn the_world_given_the_cut_of_a_crater_is_a_body_with_the_cells_that_are_left_and_the_rim() {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = crater_cut(&before, &law.kernel, 1.0, &ejection(&law), 1, &rock(2), &anchored).unwrap();
    let ground_body = |cells: Vec<[i32; 3]>, mass: f64| Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Voxels { size: [H; 3], cells },
        mass,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.; 3],
        angular_velocity: [0.; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3::default(),
    };
    let one = DENSITY * H * H * H;
    let mut bodies = vec![ground_body(before.cells().collect(), before.count() as f64 * one)];
    for _ in 0..2 {
        bodies.push(ground_body(vec![[0, 0, 0]], one));
    }
    let mut w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1, 2] }])
    .unwrap();
    let mut d = Crater(Some(x.cut.cut.clone()), vec![]);
    let frame = w.frame_at(0.3, &mut d);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    println!(
        "CRATER CUT: {} cells added, {} destroyed ({} of them dust), {} pieces ({:?} cells), {} stay",
        x.cut.cut.added.len(),
        x.cut.cut.destroyed.len(),
        x.cut.dust.len(),
        x.cut.cut.pieces.len(),
        x.cut.cut.pieces.iter().map(|p| p.cells.len()).collect::<Vec<_>>(),
        x.cut.stays.len()
    );
    // pinned from the run: the rim is a fifth of the cells taken out, and what the crater leaves loose is the top of the pillar
    assert_eq!((x.cut.cut.added.len(), x.cut.cut.pieces.len(), x.cut.cut.pieces[0].cells.len()), (1292, 1, 132));
    // the pieces took their slots
    assert_eq!(frame.enabled.iter().filter(|e| **e).count(), 1 + x.cut.cut.pieces.len());
    // the body has the centre of mass of the cells that stay
    let state = d.1.last().unwrap().1;
    let wanted =
        shape_mass_properties(&Shape3::Voxels { size: [H; 3], cells: x.cut.stays.clone() }, x.cut.cut.parent_mass, 1.0)
            .unwrap()
            .centre;
    for k in 0..3 {
        assert!(
            (state.centre[k] - wanted[k]).abs() < 1e-6,
            "axis {k}: the centre of mass of the ground is {} and its cells' is {}",
            state.centre[k],
            wanted[k]
        );
    }
}

#[test]
fn a_crater_cut_is_a_function_of_its_arguments_and_refuses_what_cannot_be_cut() {
    let law = law();
    let before = occupancy(&ground());
    let e = ejection(&law);
    let a = crater_cut(&before, &law.kernel, 1.0, &e, 1, &rock(4), &anchored).unwrap();
    assert_eq!(a, crater_cut(&before, &law.kernel, 1.0, &e, 1, &rock(4), &anchored).unwrap());
    // cells that are not cubes, a unit that is not one and a ground with nothing in it are errors
    let flat = Rock { size: [H, 2.0 * H, H], ..rock(4) };
    assert!(crater_cut(&before, &law.kernel, 1.0, &e, 1, &flat, &anchored).is_err());
    assert!(crater_cut(&before, &law.kernel, 0.0, &e, 1, &rock(4), &anchored).is_err());
    let empty = occupancy(&[]);
    assert!(crater_cut(&empty, &law.kernel, 1.0, &e, 1, &rock(4), &anchored).is_err());
}
