//! A crater in ground that slopes, end to end through the evaluator: the ball comes along the normal of the slope, and the cut is the law's crater at the
//! surface of the cells on the slope.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{assert_every_frame_is_clean, evaluator, Dir};
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::voxel_crater::Rock;
use sr_eval::voxel_cut::{crater_axis, crater_cut_of, crater_spec, seed_of, Anchor, Settings};
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
    oblique_document(degrees, 0.0, false)
}

/// The slope of `degrees` and a ball that comes at `phi` degrees from its normal with the same speed along the normal, 86.6 m/s: the tangent it adds is down the
/// slope, or along z (across it). The ball goes to touch the surface first at x = 15 m, z = 15 m.
fn oblique_document(degrees: f64, phi: f64, across: bool) -> String {
    let theta = degrees.to_radians();
    // the outward normal of the surface (y down: out of the ground is toward minus y) and the point of the surface at x = 15 m, z = 15 m
    let normal = [theta.sin(), -theta.cos(), 0.0];
    let surface = [15.0, 12.0 + 15.0 * theta.tan(), 15.0];
    // the ball's velocity: the normal speed into the ground and the tangent that is added to it (down the slope: (cos, sin, 0); across: z)
    let tangent = if across { [0.0, 0.0, 1.0] } else { [theta.cos(), theta.sin(), 0.0] };
    let (vn, vt) = (86.6, 86.6 * phi.to_radians().tan());
    let velocity: [f64; 3] = std::array::from_fn(|i| -normal[i] * vn + tangent[i] * vt);
    let speed = velocity.iter().map(|c| c * c).sum::<f64>().sqrt();
    // it first touches at the surface point when its centre is at the point plus the radius along the normal, and it starts ten metres back along its velocity
    let touch: [f64; 3] = std::array::from_fn(|i| surface[i] + normal[i] * 2.0);
    let start: [f64; 3] = std::array::from_fn(|i| touch[i] - velocity[i] / speed * 10.0);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="slope.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="ball" primitive="sphere" radius="2" x="{bx}" y="{by}" z="{bz}">
            <rigidBody shape="sphere" mass="90478" velocityX="{vx}" velocityY="{vy}" velocityZ="{vz}" restitution="0" friction="0.5" linearDamping="0" angularDamping="0"/>
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
        bz = start[2],
        vx = velocity[0],
        vy = velocity[1],
        vz = velocity[2],
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
    hit_oblique("along", degrees, 0.0, false)
}

/// The same with the ball at `phi` degrees from the normal (down the slope or across it).
/// (`test` names the directory, which the tests that run together must not share.)
fn hit_oblique(test: &str, degrees: f64, phi: f64, across: bool) -> (sr_eval::FrameNode, Occupancy) {
    let dir = Dir::new(&format!("slope-{test}-{degrees}-{phi}-{across}"));
    let ground = slope(degrees);
    std::fs::write(dir.0.join("slope.srvol"), srvol::write(&ground, 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &oblique_document(degrees, phi, across));
    assert_every_frame_is_clean(&ev, &format!("the slope of {degrees} degrees at {phi} degrees from its normal"));
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
        // what is left over is the staircase, whose mean plane the kernel is put on: -1.0, -1.0 and -1.3 percent at 10, 20 and 30 degrees
        assert!((volume - law).abs() < 0.025 * law, "{degrees} degrees: {volume} m3 against the law's {law}");
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

#[test]
fn the_document_of_the_slope_evaluates_clean_through_its_last_frame() {
    let dir = Dir::new("slope-every-frame");
    std::fs::write(dir.0.join("slope.srvol"), srvol::write(&slope(20.0), 0.25).unwrap()).unwrap();
    assert_every_frame_is_clean(&evaluator(&dir, &document(20.0)), "the slope of 20 degrees");
}

/// Where the ball first touches the surface, in the object's frame: x = 15 m, z = 15 m, on the plane (the object is placed where the box of cells was).
fn touch(degrees: f64) -> [f64; 3] {
    let theta = degrees.to_radians();
    [15.0, 12.0 + 15.0 * theta.tan() - f64::from(top_key(degrees)) * 0.25, 15.0]
}

#[test]
fn a_ball_that_comes_at_an_angle_to_the_normal_of_a_slope_makes_the_crater_of_its_normal_speed_at_the_place_it_touched()
{
    // the law reads the speed along the normal, so the crater is the same whatever the tangent that the ball adds to it; the axis is the surface's, not the
    // velocity's; the centre is where the ball touched, to two cells (the contact point is inside the ground by the step the ball went on, along its
    // velocity); the cut is as the normal one is
    let flat = hit_oblique("oblique-flat", 0.0, 0.0, false).0.crater_impact.as_ref().expect("the impact").law().volume;
    let normal = {
        let theta = 20.0f64.to_radians();
        [theta.sin(), -theta.cos(), 0.0]
    };
    for (phi, across) in [(30.0, false), (60.0, false), (30.0, true), (60.0, true)] {
        let what =
            format!("20 degrees, {phi} degrees from the normal, {}", if across { "across" } else { "down the slope" });
        let (node, ground) = hit_oblique("oblique", 20.0, phi, across);
        let grown = node.crater_impact.as_ref().expect("the impact");
        let state = node.voxels.as_ref().expect("cells");
        assert_eq!(state.revision, 1, "{what}: one cut");
        let law = grown.law().volume;
        assert!((law - flat).abs() < 0.005 * flat, "{what}: the law's volume is {law} m3 and the flat ground's {flat}");
        let spec = crater_spec(grown, &ground, &settings());
        let angle = (0..3).map(|i| spec.outward[i] * normal[i]).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees();
        assert!(angle < 3.0, "{what}: the axis is {angle} degrees off the slope's normal");
        let at = touch(20.0);
        let off = (0..3).map(|i| (spec.center[i] - at[i]).powi(2)).sum::<f64>().sqrt();
        println!("OBLIQUE {what}: axis {angle:.2} degrees off the normal, the centre {off:.3} m from where the ball touched, law {law:.2} m3");
        // A RESULT, recorded and not hidden: the point that the world gives is the mean of the manifold's contacts at the end of the step in which the ball
        // touched, and it is off the place it touched by 0.25, 0.77, 0.77 and 1.04 m in the four cases here (the ball goes on by up to 0.6 m tangentially in a
        // step at sixty degrees, the impulse weights the later contacts of the step, and the contacts of the voxel collider are not a symmetric patch: the
        // sideways error is as large across the slope as down it, and about 0.4 m for a ball that comes along the normal). A crater's centre is therefore known
        // to about four cells, a sixth of its crest radius at the worst: the bound below is that, with a margin, and the volume of the cut is the plane's and not
        // the centre's.
        assert!(
            off < 1.2,
            "{what}: the centre of the crater is {off} m from the place the ball touched: {:?} against {:?}",
            spec.center,
            at
        );
        let cut = crater_cut_of(grown, &ground, &settings(), seed_of("ground")).unwrap();
        let volume = cut.cut.cut.destroyed.len() as f64 * 0.015625;
        println!(
            "OBLIQUE {what}: destroyed {} cells {volume:.2} m3, heaped {}",
            cut.cut.cut.destroyed.len(),
            cut.cut.cut.added.len()
        );
        assert!((volume - law).abs() < 0.025 * law, "{what}: {volume} m3 against the law's {law}");
        for c in &cut.cut.cut.destroyed {
            let p: [f64; 3] = std::array::from_fn(|i| (f64::from(c[i]) + 0.5) * 0.25 - spec.center[i]);
            let along: f64 = (0..3).map(|i| p[i] * spec.outward[i]).sum();
            let r = (0..3).map(|i| (p[i] - along * spec.outward[i]).powi(2)).sum::<f64>().sqrt();
            assert!(
                r < spec.radius + spec.rim_width + 0.5 && along <= spec.radius + 0.5,
                "{what}: {c:?} is outside the reach ({r}, {along})"
            );
        }
        // the model has no downrange bias: the cells thrown carry no net momentum along the surface (a stated limit: the law's own list of ejecta has some)
        let mut tangential = [0.0f64; 3];
        let mut total = 0.0;
        for t in &cut.excavation.thrown {
            let along: f64 = (0..3).map(|i| t.velocity[i] * spec.outward[i]).sum();
            for i in 0..3 {
                tangential[i] += t.velocity[i] - along * spec.outward[i];
            }
            total += t.velocity.iter().map(|c| c * c).sum::<f64>().sqrt();
        }
        let net = tangential.iter().map(|c| c * c).sum::<f64>().sqrt();
        println!("OBLIQUE {what}: net tangential speed of the thrown cells {net:.1} of {total:.1} m/s summed");
        assert!(net < 0.03 * total, "{what}: the thrown cells have a net tangential momentum of {net} for {total}");
    }
}
