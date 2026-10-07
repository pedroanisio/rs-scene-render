//! A crater in an object of cells, end to end through the evaluator: the ball hits the ground, the world cuts it, and the frames say what the cut was.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{evaluator, Dir};
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::voxel_crater::Rock;
use sr_eval::voxel_cut::{crater_cut_of, seed_of, Anchor, Settings};
use sr_eval::voxels::{Overflow, Policy, Stay};
use sr_eval::{Evaluator, FrameNode};
use std::collections::BTreeSet;

/// The ground: a slab of 120 by 40 by 120 cells (30 by 10 by 30 m of a quarter of a metre) with the top at the key 60, and a pillar of 2 by 2 cells
/// that stands 15 m over it, 4 m from the middle of the slab, where the ball does not fall.
fn ground() -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..120 {
        for j in 60..100 {
            for i in 0..120 {
                cells.push(([i, j, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    for k in 60..62 {
        for j in 0..60 {
            for i in 76..78 {
                cells.push(([i, j, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

const DOCUMENT: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
    <assets><voxelAsset id="model" src="ground.srvol"/></assets>
    <materials><material id="stone" baseColor="#808080"/></materials>
    <composition>
      <object3D id="ball" primitive="sphere" radius="2" x="15" y="2" z="15">
        <rigidBody shape="sphere" mass="90478" velocityY="86.6" restitution="0" friction="0.5" linearDamping="0" angularDamping="0"/>
      </object3D>
      <object3D id="ground" primitive="voxels" voxels="model" material="stone">
        <rigidBody type="static" density="2700"/>
        <crater id="pit" source="ball" targetMaterial="softRock" gravity="9.80665"/>
      </object3D>
    </composition>
    <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##;

fn setup(name: &str) -> (Dir, Evaluator) {
    let dir = Dir::new(name);
    std::fs::write(dir.0.join("ground.srvol"), srvol::write(&ground(), 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, DOCUMENT);
    (dir, ev)
}

fn ground_node(ev: &Evaluator, t: f64) -> FrameNode {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "ground").expect("the ground").clone()
}

fn settings() -> Settings {
    Settings {
        rock: Rock {
            size: [0.25; 3],
            density: 2700.0,
            pixels_per_meter: 1.0,
            policy: Policy { stay: Stay::Anchored, min_cells: 1, max_fragments: 64, overflow: Overflow::Error },
        },
        anchor: Anchor::Base,
    }
}

fn bricks(cells: impl IntoIterator<Item = [i32; 3]>) -> Vec<[i32; 3]> {
    cells.into_iter().map(|c| c.map(|k| k.div_euclid(8))).collect::<BTreeSet<_>>().into_iter().collect()
}

#[test]
fn the_ground_is_what_it_was_until_the_ball_hits_it() {
    let (_dir, ev) = setup("before");
    for t in [0.0, 0.05, 0.1] {
        let node = ground_node(&ev, t);
        let voxels = node.voxels.as_ref().expect("an object of cells that a crater cuts says what it is");
        assert_eq!(voxels.revision, 0, "t = {t}");
        assert_eq!(voxels.grid.count(), ground().count());
        assert!(voxels.changed_bricks.is_empty() && voxels.pieces.is_empty());
    }
}

#[test]
fn the_ball_cuts_the_ground_as_the_cut_of_the_crater_it_makes_says() {
    let (_dir, ev) = setup("cut");
    let asset = ground();
    let node = ground_node(&ev, 0.6);
    let grown = node.crater_impact.as_ref().expect("the impact has happened");
    let voxels = node.voxels.as_ref().expect("cells");
    assert_eq!(voxels.revision, 1, "one cut, one edit");
    // the cut that the pure function makes of the same impact in the same cells
    let wanted = crater_cut_of(grown, &asset, &settings(), seed_of("ground")).unwrap();
    let cut = &wanted.cut;
    println!(
        "VOXEL CRATER IN A DOCUMENT: {} added, {} destroyed ({} thrown), {} pieces {:?}, {} stay",
        cut.cut.added.len(),
        cut.cut.destroyed.len(),
        wanted.excavation.thrown.len(),
        cut.cut.pieces.len(),
        cut.cut.pieces.iter().map(|p| p.cells.len()).collect::<Vec<_>>(),
        cut.stays.len()
    );
    let mut stays = cut.stays.clone();
    stays.sort_unstable();
    let mut have: Vec<[i32; 3]> = voxels.grid.cells().collect();
    have.sort_unstable();
    assert_eq!(have, stays, "what stays of the ground is what the cut says");
    // the pieces are the cut's, in the slots, with their cells
    assert_eq!(voxels.pieces.len(), cut.cut.pieces.len());
    assert!(!voxels.pieces.is_empty(), "the top of the pillar is loose");
    for (piece, wanted) in voxels.pieces.iter().zip(&cut.cut.pieces) {
        let mut cells: Vec<[i32; 3]> = piece.grid.cells().collect();
        cells.sort_unstable();
        let mut expected = wanted.cells.clone();
        expected.sort_unstable();
        assert_eq!(cells, expected);
        assert_eq!(piece.revision, 1, "a piece is born with its cut");
    }
    // the palette: the ground's own cells keep theirs and the rim has those of the cells it was heaped from
    for c in voxels.grid.cells() {
        let expected = if asset.get(c) != 0 {
            asset.get(c)
        } else {
            wanted.excavation.rim.iter().find(|r| r.0 == c).map_or(0, |r| r.1)
        };
        assert_ne!(expected, 0);
        assert_eq!(voxels.grid.get(c), expected, "palette of {c:?}");
    }
    // and what the renderer has to look at again: the bricks that the cut touched
    let touched = bricks(
        cut.cut
            .destroyed
            .iter()
            .chain(&cut.cut.added)
            .chain(cut.cut.pieces.iter().flat_map(|p| p.cells.iter()))
            .copied(),
    );
    assert_eq!(voxels.changed_bricks, touched);
    // by edit, for a surface cache that read an earlier revision: one cut, one step, and it is the bricks the cut touched
    assert_eq!(voxels.steps, vec![(1, touched.clone())]);
    // pinned from the run (the numbers of the pure function, which they equal)
    assert_eq!(
        (cut.cut.added.len(), cut.cut.destroyed.len(), cut.cut.pieces.len(), wanted.excavation.thrown.len()),
        PINNED,
        "the counts that the document's crater makes"
    );
}

/// (added, destroyed, pieces, thrown) of the crater of the document, pinned from the run: a fifth of the cells taken out is heaped on the rim (1 416 of 7 080) and the
/// rest thrown (5 664), and the top of the pillar (136 cells) is the one loose part.
const PINNED: (usize, usize, usize, usize) = (1416, 7080, 1, 5664);

#[test]
fn the_same_frame_whoever_asks_and_in_whatever_order_and_a_second_impact_is_not_a_second_cut() {
    let (_dir, ev) = setup("ways");
    let a = ground_node(&ev, 0.6);
    let later = ground_node(&ev, 1.9);
    let again = ground_node(&ev, 0.6);
    let (_dir2, fresh_ev) = setup("ways-fresh");
    let fresh = ground_node(&fresh_ev, 0.6);
    for other in [&again, &fresh] {
        let (x, y) = (a.voxels.as_ref().unwrap(), other.voxels.as_ref().unwrap());
        assert_eq!(
            (x.revision, x.grid.fingerprint(), &x.changed_bricks),
            (y.revision, y.grid.fingerprint(), &y.changed_bricks)
        );
        assert_eq!(x.pieces.len(), y.pieces.len());
        for (p, q) in x.pieces.iter().zip(&y.pieces) {
            assert_eq!(
                (p.grid.fingerprint(), p.pose3.map(f64::to_bits)),
                (q.grid.fingerprint(), q.pose3.map(f64::to_bits))
            );
        }
    }
    // the ball is long since at rest on the ground, and what was cut was cut once
    assert_eq!(later.voxels.as_ref().unwrap().revision, 1);
    assert_eq!(later.voxels.as_ref().unwrap().grid.fingerprint(), a.voxels.as_ref().unwrap().grid.fingerprint());
}
