//! A fracture by stress, end to end through the evaluator: a cantilever of cells welded to a static block, with a weight on its tip. The oracle is the beam's: the
//! principal tension in each joint from the weight of what is beyond it and of the load, and the load that makes the root joint's tension the strength.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{assert_every_frame_is_clean, evaluator, position, Dir};
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::{Evaluator, FrameNode};
use std::collections::BTreeSet;

const G: f64 = 9.80665;
const DENSITY: f64 = 2400.0;
const N: usize = 5;
const EDGE: f64 = 0.4;
const STRENGTH: f64 = 8.0e5;

/// The principal tension of the joint after the piece `j` of a cantilever of `N` cubes that carries `load` newtons on its tip piece: the weight of the pieces
/// beyond the joint and the load make its bending moment and its shear, and the section is a square of the edge.
fn beam_principal(j: usize, load: f64) -> f64 {
    let mass = DENSITY * EDGE.powi(3);
    let beyond = (N - 1 - j) as f64;
    let weight = mass * G * beyond;
    let joint = (j + 1) as f64 * EDGE;
    let moment = weight * (beyond / 2.0) * EDGE + load * (N as f64 - 0.5) * EDGE - load * joint;
    let shear = (weight + load) / (EDGE * EDGE);
    let sigma = moment / (EDGE.powi(3) / 6.0);
    sigma / 2.0 + (sigma * sigma / 4.0 + shear * shear).sqrt()
}

/// The load on the tip that makes the principal tension of joint `j` the strength: found by halving, since the tension grows with the load.
fn breaking_load(j: usize) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    while beam_principal(j, hi) < STRENGTH {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if beam_principal(j, mid) < STRENGTH {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

fn beam() -> Occupancy {
    Occupancy::from_cells((0..N as i32).map(|i| ([i, 0, 0], 1u8))).unwrap()
}

/// The cantilever: the beam along x from the static block (a wall on the side of its root) and a weight of `load` newtons resting on its tip, which breaks by
/// stress at `STRENGTH` pascals between its cubes (planes at every edge). `rigid` and `fracture` add to the beam's attributes.
fn document(load: f64, rigid: &str, fracture: &str) -> String {
    let planes: Vec<String> = (1..N).map(|i| format!("1 0 0 {}", f64::from(i as u32) * EDGE)).collect();
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="beam.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="wall" primitive="box" x="-0.5" y="{wall_y}" z="{half}" width="1" height="{wall_h}" depth="{wall_d}"><rigidBody type="static"/></object3D>
          <object3D id="beam" primitive="voxels" voxels="model" material="stone" x="0" y="-5" z="0">
            <rigidBody density="{DENSITY}" friction="0.5" restitution="0" linearDamping="0" angularDamping="0" maxFragments="8" {rigid}/>
            <fracture mode="stress" strength="{STRENGTH}" partition="planes" planes="{planes}" {fracture}/>
          </object3D>
          <object3D id="load" primitive="box" x="{lx}" y="{ly}" z="{half}" width="{EDGE}" height="{EDGE}" depth="{EDGE}">
            <rigidBody mass="{mass}" friction="0.5" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
        </composition>
        <physics gravityY="-{G}" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none">
          <constraint id="root" type="weld" a="beam" b="wall" x="0" y="{anchor_y}" z="{half}"/>
        </physics></scene>"##,
        planes = planes.join(" "),
        half = EDGE / 2.0,
        wall_y = -5.0 + EDGE / 2.0,
        wall_h = 3.0 * EDGE,
        wall_d = 3.0 * EDGE,
        anchor_y = -5.0 + EDGE / 2.0,
        lx = (N as f64 - 0.5) * EDGE,
        ly = -5.0 - EDGE / 2.0 - 1e-6,
        mass = load / G,
    )
}

fn setup(name: &str, xml: &str) -> (Dir, Evaluator) {
    let dir = Dir::new(name);
    std::fs::write(dir.0.join("beam.srvol"), srvol::write(&beam(), EDGE).unwrap()).unwrap();
    let ev = evaluator(&dir, xml);
    (dir, ev)
}

fn node(ev: &Evaluator, id: &str, t: f64) -> FrameNode {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}")).clone()
}

fn cells_of(grid: &Occupancy) -> BTreeSet<[i32; 3]> {
    grid.cells().collect()
}

#[test]
fn the_root_joint_is_the_one_that_has_the_most_load_and_the_others_have_room() {
    // the premise of the oracle below, from the beam's formula and not from the engine: the root's tension is the greatest
    let target = breaking_load(0);
    for j in 1..N - 1 {
        assert!(breaking_load(j) > 1.3 * target, "joint {j}");
    }
}

#[test]
fn a_cantilever_loaded_to_995_thousandths_of_the_load_that_breaks_its_root_breaks_nothing_in_two_seconds() {
    let (_dir, ev) = setup("stress-under", &document(0.995 * breaking_load(0), "", ""));
    assert_every_frame_is_clean(&ev, "the cantilever under its breaking load");
    let held = node(&ev, "beam", 2.0);
    let state = held.voxels.as_ref().expect("a body of cells that breaks by stress says what it is");
    assert_eq!(state.revision, 0, "no cut");
    assert!(state.enabled && state.pieces.is_empty());
    assert_eq!(state.grid.count(), N as u64);
    // it is where it was
    let (start, end) = (position(&ev, "beam", 0.0), position(&ev, "beam", 2.0));
    let moved = (0..3).map(|i| (end[i] - start[i]).powi(2)).sum::<f64>().sqrt();
    assert!(moved < 5e-3, "the beam moved {moved} m under a load that does not break it");
}

#[test]
fn one_over_the_load_that_breaks_it_breaks_the_root_joint_and_only_it_and_the_part_beyond_falls() {
    let (_dir, ev) = setup("stress-over", &document(1.005 * breaking_load(0), "", ""));
    assert_every_frame_is_clean(&ev, "the cantilever over its breaking load");
    let broken = node(&ev, "beam", 2.0);
    let state = broken.voxels.as_ref().expect("cells");
    // the root piece stays the body (the weld holds it); the four beyond the joint are the one piece that fell
    assert_eq!(cells_of(&state.grid), BTreeSet::from([[0, 0, 0]]), "the part that the weld holds is the body");
    assert!(state.revision > 0, "the body was cut");
    assert_eq!(state.pieces.len(), 1, "the four pieces beyond the root joint come away together, being joined");
    let piece = &state.pieces[0];
    assert!(piece.enabled);
    assert_eq!(cells_of(&piece.grid), (1..N as i32).map(|i| [i, 0, 0]).collect::<BTreeSet<_>>());
    // fragments and dust are the source, each cell once: nothing is dust here (fragmentMinCells is 1)
    assert_eq!(state.grid.count() + piece.grid.count(), N as u64);
    // and the piece fell: it is well below where it was (y is down in the scene), and the root did not move
    let (root0, root) = (position(&ev, "beam", 0.0), position(&ev, "beam", 2.0));
    assert!((root[1] - root0[1]).abs() < 5e-3, "the root piece stayed in the weld: {root0:?} to {root:?}");
    assert!(piece.pose3[13] > root[1] + 5.0, "the part beyond the joint fell: {:?} against {root:?}", piece.pose3);
}

#[test]
fn the_part_that_fell_falls_with_the_acceleration_of_gravity() {
    // momentum is the mass of the piece times a velocity that gains G a second: the second difference of its height in free fall is G, to the pose's rounding
    let (_dir, ev) = setup("stress-fall", &document(1.005 * breaking_load(0), "", ""));
    let y = |t: f64| node(&ev, "beam", t).voxels.as_ref().expect("cells").pieces[0].pose3[13];
    let (a, b, c) = (y(1.5), y(1.5 + 1.0 / 24.0), y(1.5 + 2.0 / 24.0));
    let g = (a - 2.0 * b + c) * 24.0 * 24.0;
    assert!((g - G).abs() < 0.03 * G, "the second difference gives {g} against {G}");
}

#[test]
fn a_body_that_does_not_break_is_the_body_it_would_be_without_the_fracture_to_the_bit() {
    // the strength is out of reach: the same cantilever with the load that breaks the root at 1.005, and no fracture at all, move alike
    let load = 1.005 * breaking_load(0);
    let held = document(load, "", "").replace(&format!("strength=\"{STRENGTH}\""), "strength=\"1e18\"");
    let plain = {
        let xml = document(load, "", "");
        let a = xml.find("<fracture").unwrap();
        let b = xml[a..].find("/>").unwrap() + a + 2;
        // (the slots that take the pieces belong to a body that can break, and the schema refuses them on one that cannot)
        format!("{}{}", &xml[..a], &xml[b..]).replace(r#" maxFragments="8""#, "")
    };
    let (_d1, with) = setup("stress-bits-with", &held);
    let (_d2, without) = setup("stress-bits-without", &plain);
    assert_every_frame_is_clean(&with, "the beam of a strength out of reach");
    for n in 0..with.frame_count() {
        let (a, b) = (with.evaluate_frame(n), without.evaluate_frame(n));
        for id in ["beam", "load"] {
            let pose = |f: &sr_eval::FrameGraph| {
                f.nodes.iter().find(|x| &*x.id == id).unwrap().pose3.unwrap().map(f64::to_bits)
            };
            assert_eq!(pose(&a), pose(&b), "{id} at frame {n}");
        }
    }
}

#[test]
fn a_piece_that_is_too_small_is_dust_and_what_is_left_is_the_body_and_the_dust() {
    // the four pieces that fall are 4 cells and fragmentMinCells is 5: they are dust, and the body keeps its root cell
    let (_dir, ev) = setup("stress-dust", &document(1.005 * breaking_load(0), r#"fragmentMinCells="5""#, ""));
    assert_every_frame_is_clean(&ev, "the cantilever whose fallen part is dust");
    let state = node(&ev, "beam", 2.0).voxels.expect("cells");
    assert_eq!(cells_of(&state.grid), BTreeSet::from([[0, 0, 0]]));
    assert!(state.pieces.is_empty(), "four cells are under fragmentMinCells=5");
}
