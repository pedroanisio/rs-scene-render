//! A crater in ground that slopes, end to end through the evaluator: the ball comes along the normal of the slope, and the cut is the law's crater at the
//! surface of the cells on the slope.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{evaluator, Dir};
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::voxel_crater::Rock;
use sr_eval::voxel_cut::{crater_axis, crater_cut_of, seed_of, Anchor, Settings};
use sr_eval::voxels::{Overflow, Policy, Stay};

/// Ground whose surface falls toward +x at `degrees`: the cells under the plane y = 12 m + x tan(degrees) (y points down), 30 m by 30 m in plan and a
/// thickness of 12 m under the surface at the lowest. The cells are a quarter of a metre.
fn slope(degrees: f64) -> Occupancy {
    let tan = degrees.to_radians().tan();
    let top = top_key(degrees);
    let mut cells = Vec::new();
    for k in 0..120 {
        for i in 0..120 {
            let surface = ((12.0 + (f64::from(i) + 0.5) * 0.25 * tan) / 0.25).floor() as i32;
            let floor = ((12.0 + 30.0 * tan + 12.0) / 0.25).ceil() as i32;
            for j in surface..floor {
                cells.push(([i, j - top, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

/// The key of the highest cell of the ground (the loader moves the box of the cells to the origin, so the object is placed where the box was).
fn top_key(degrees: f64) -> i32 {
    ((12.0 + 0.125 * degrees.to_radians().tan()) / 0.25).floor() as i32
}

fn document(degrees: f64) -> String {
    let theta = degrees.to_radians();
    // the outward normal of the surface (y down: out of the ground is toward minus y) and the point of the surface at x = 15 m, z = 15 m
    let normal = [theta.sin(), -theta.cos()];
    let surface = [15.0, 12.0 + 15.0 * theta.tan()];
    // the ball starts 8 m clear of the surface along the normal and comes straight in along it at the speed of the law's normal
    let start = [surface[0] + normal[0] * 10.0, surface[1] + normal[1] * 10.0];
    let velocity = [-normal[0] * 86.6, -normal[1] * 86.6];
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="slope.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="ball" primitive="sphere" radius="2" x="{bx}" y="{by}" z="15">
            <rigidBody shape="sphere" mass="90478" velocityX="{vx}" velocityY="{vy}" restitution="0" friction="0.5" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="voxels" voxels="model" material="stone" y="{ground_y}">
            <rigidBody type="static" density="2700"/>
            <crater id="pit" source="ball" targetMaterial="softRock" gravity="9.80665"/>
          </object3D>
        </composition>
        <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##,
        ground_y = f64::from(top_key(degrees)) * 0.25,
        bx = start[0],
        by = start[1],
        vx = velocity[0],
        vy = velocity[1],
    )
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

/// The slope of `degrees` hit by the ball along its normal: the frame of the ground, the crater that the frame carries and the cells of the asset.
fn hit(degrees: f64) -> (sr_eval::FrameNode, Occupancy) {
    let dir = Dir::new(&format!("slope-{degrees}"));
    let ground = slope(degrees);
    std::fs::write(dir.0.join("slope.srvol"), srvol::write(&ground, 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &document(degrees));
    let frame = ev.evaluate(0.6);
    assert!(
        frame.failures.is_empty() && frame.problems.is_empty(),
        "{degrees} degrees: {:?} {:?}",
        frame.failures,
        frame.problems
    );
    (frame.nodes.iter().find(|n| &*n.id == "ground").unwrap().clone(), ground)
}

#[test]
fn the_ball_that_comes_along_the_normal_of_a_slope_cuts_the_law_s_crater_in_the_cells_with_its_axis_along_that_normal()
{
    // the ball comes at the same speed along the normal whatever the slope: the law's crater is the flat ground's
    let flat = hit(0.0).0.crater_impact.as_ref().expect("the impact").law().volume;
    for degrees in [10.0f64, 20.0, 30.0] {
        let (node, ground) = hit(degrees);
        let grown = node.crater_impact.as_ref().expect("the impact");
        let state = node.voxels.as_ref().expect("cells");
        assert_eq!(state.revision, 1, "{degrees} degrees: one cut");
        // the axis that the cut was made on: the normal of the surface of the cells, which is the slope's, and not the contact's, which the staircase of cells
        // turns by ten degrees and more
        let axis = crater_axis(grown, &ground, &settings());
        let theta = degrees.to_radians();
        let want = [theta.sin(), -theta.cos(), 0.0];
        let angle = (0..3).map(|i| axis[i] * want[i]).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees();
        let contact = grown.spec.outward;
        let contact_angle = (0..3).map(|i| contact[i] * want[i]).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees();
        println!("SLOPE {degrees}: the axis is {angle:.1} degrees off the slope's normal (the contact's is {contact_angle:.1})");
        assert!(angle < 5.0, "{degrees} degrees: the axis is {angle} degrees off the normal of the slope");
        // the cut: what it destroyed weighs about the law's volume (the stair-steps of a slope take a little more or less than a flat ground's 98 percent), and
        // every destroyed cell is inside the reach of the crater about that axis
        let cut = crater_cut_of(grown, &ground, &settings(), seed_of("ground")).unwrap();
        let volume = cut.cut.cut.destroyed.len() as f64 * 0.015625;
        let law = grown.law().volume;
        println!(
            "SLOPE {degrees}: destroyed {} cells {volume:.2} m3 for the law's {law:.2}; heaped {}",
            cut.cut.cut.destroyed.len(),
            cut.cut.cut.added.len()
        );
        assert!((law - flat).abs() < 0.005 * flat, "{degrees} degrees: the law's volume is {law} m3 and the flat ground's {flat}: it reads the speed along the normal");
        // what is left over is the staircase: the surface of the cells is a step of a quarter of a metre every few cells, the plane of the kernel is put where
        // the axis meets it, and the cells under the plane's mean are more or fewer than the plane's: +5.4, +2.0 and -1.3 percent at 10, 20 and 30 degrees
        assert!((volume - law).abs() < 0.065 * law, "{degrees} degrees: {volume} m3 against the law's {law}");
        let spec = grown.spec;
        for c in &cut.cut.cut.destroyed {
            let p: [f64; 3] = std::array::from_fn(|i| (f64::from(c[i]) + 0.5) * 0.25 - spec.center[i]);
            let along: f64 = (0..3).map(|i| p[i] * axis[i]).sum();
            let r = (0..3).map(|i| (p[i] - along * axis[i]).powi(2)).sum::<f64>().sqrt();
            assert!(
                r < spec.radius + spec.rim_width + 0.5,
                "{degrees} degrees: {c:?} is {r} m from the axis, the reach is {}",
                spec.radius + spec.rim_width
            );
            assert!(
                along <= spec.radius + 0.5,
                "{degrees} degrees: {c:?} is {along} m over the plane, the ceiling is {}",
                spec.radius
            );
        }
        // and the frame carries that cut: the cells that stay are the pure function's
        let mut have: Vec<[i32; 3]> = state.grid.cells().collect();
        have.sort_unstable();
        let mut stays = cut.cut.stays.clone();
        stays.sort_unstable();
        assert_eq!(have, stays, "{degrees} degrees");
    }
}
