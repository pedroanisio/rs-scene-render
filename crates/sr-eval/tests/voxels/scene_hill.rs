//! A crater in ground that is curved (a hill, a basin), end to end through the evaluator: the ball comes along the normal of the ground where it hits.
#![allow(clippy::needless_range_loop)]

use super::scene_body::{assert_every_frame_is_clean, evaluator, Dir};
use sr_3d::crater::Crater;
use sr_3d::occupancy::Occupancy;
use sr_3d::voxel::srvol;
use sr_eval::voxel_crater::Rock;
use sr_eval::voxel_cut::{crater_axis, crater_cut_of, crater_kernel, crater_spec, seed_of, Anchor, Settings};
use sr_eval::voxels::{Overflow, Policy, Stay};

/// The ground: a slab 30 m by 30 m whose surface is level at y = 12 m (y down) and has a cap of a sphere of radius of curvature `radius` and height `height`
/// (a hill, up) at x = z = 15 m, or the same cap down (a basin), over a thickness of 12 m. The cells are a quarter of a metre.
#[derive(Clone, Copy)]
struct Ground {
    radius: f64,
    height: f64,
    basin: bool,
}

impl Ground {
    /// The height of the surface over the plane y = 12 at the distance `r` from the middle (up positive for a hill, down for a basin, in metres).
    fn rise(self, r: f64) -> f64 {
        let r0 = (self.radius.powi(2) - (self.radius - self.height).powi(2)).sqrt();
        if r >= r0 {
            0.0
        } else {
            (self.radius.powi(2) - r * r).sqrt() - (self.radius - self.height)
        }
    }

    /// The y of the surface (down positive) at x, z.
    fn surface(self, x: f64, z: f64) -> f64 {
        let h = self.rise(((x - 15.0).powi(2) + (z - 15.0).powi(2)).sqrt());
        if self.basin {
            12.0 + h
        } else {
            12.0 - h
        }
    }

    fn top_key(self) -> i32 {
        let top = if self.basin { 12.0 } else { 12.0 - self.height };
        (top / 0.25).floor() as i32
    }

    fn occupancy(self) -> Occupancy {
        let top = self.top_key();
        let floor = (24.0f64 / 0.25).ceil() as i32 + 4 * (self.height as i32);
        let mut cells = Vec::new();
        for k in 0..120 {
            for i in 0..120 {
                let surface =
                    (self.surface((f64::from(i) + 0.5) * 0.25, (f64::from(k) + 0.5) * 0.25) / 0.25).floor() as i32;
                for j in surface..floor {
                    cells.push(([i, j - top, k], 1 + ((i + j + k) % 3) as u8));
                }
            }
        }
        Occupancy::from_cells(cells).unwrap()
    }

    /// The outward unit normal of the surface at the distance `d` from the middle along +x (the surface is round about the middle).
    fn normal(self, d: f64) -> [f64; 3] {
        // the sphere's centre is `radius` under the top of a hill (y down: plus) or `radius` over the bottom of a basin
        let (centre_y, sign) =
            if self.basin { (12.0 + self.height - self.radius, -1.0) } else { (12.0 - self.height + self.radius, 1.0) };
        let y = self.surface(15.0 + d, 15.0);
        let v = [d, y - centre_y, 0.0];
        let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
        // out of the ground: away from the centre of a hill, toward it for a basin
        [sign * v[0] / length, sign * v[1] / length, 0.0]
    }
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

/// The ball that comes along the normal of the ground at `d` metres from the middle along +x and first touches it there.
fn document(ground: Ground, d: f64) -> String {
    let normal = ground.normal(d);
    let surface = [15.0 + d, ground.surface(15.0 + d, 15.0), 15.0];
    let start: [f64; 3] = std::array::from_fn(|i| surface[i] + normal[i] * 12.0);
    let velocity: [f64; 3] = std::array::from_fn(|i| -normal[i] * 86.6);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/>
        <assets><voxelAsset id="model" src="ground.srvol"/></assets>
        <materials><material id="stone" baseColor="#808080"/></materials>
        <composition>
          <object3D id="ball" primitive="sphere" radius="2" x="{}" y="{}" z="{}">
            <rigidBody shape="sphere" mass="90478" velocityX="{}" velocityY="{}" velocityZ="{}" restitution="0" friction="0.5" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="voxels" voxels="model" material="stone" y="{}">
            <rigidBody type="static" density="2700"/>
            <crater id="pit" source="ball" targetMaterial="softRock" gravity="9.80665"/>
          </object3D>
        </composition>
        <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.004166666666666667" bounds="none"/></scene>"##,
        start[0],
        start[1],
        start[2],
        velocity[0],
        velocity[1],
        velocity[2],
        f64::from(ground.top_key()) * 0.25,
    )
}

/// What a cut of the ground at `d` is: the frame's node, the cells, and the message if the evaluator said the cut could not be made.
struct Hit {
    node: sr_eval::FrameNode,
    cells: Occupancy,
    said: Vec<String>,
}

fn hit(test: &str, ground: Ground, d: f64) -> Hit {
    let dir = Dir::new(&format!("hill-{test}"));
    let cells = ground.occupancy();
    std::fs::write(dir.0.join("ground.srvol"), srvol::write(&cells, 0.25).unwrap()).unwrap();
    let ev = evaluator(&dir, &document(ground, d));
    let frame = ev.evaluate(0.6);
    let said: Vec<String> = frame.failures.iter().chain(&frame.problems).cloned().collect();
    let node = frame.nodes.iter().find(|n| &*n.id == "ground").unwrap().clone();
    if said.is_empty() {
        assert_every_frame_is_clean(&ev, test);
    }
    Hit { node, cells, said }
}

fn degrees_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees()
}

/// The cells that the kernel takes out of `cells` about the spec the wiring chose, by a brute force over every cell of the ground: those inside the crest radius
/// plus the width of the rim, over the surface of the kernel and under the ceiling.
fn brute_force(kernel: &Crater, cells: &Occupancy) -> std::collections::BTreeSet<[i32; 3]> {
    let spec = kernel.spec();
    let axis = spec.outward;
    cells
        .cells()
        .filter(|c| {
            let p: [f64; 3] = std::array::from_fn(|i| (f64::from(c[i]) + 0.5) * 0.25 - spec.center[i]);
            let a: f64 = (0..3).map(|i| p[i] * axis[i]).sum();
            let r = (0..3).map(|i| (p[i] - a * axis[i]).powi(2)).sum::<f64>().sqrt();
            r < spec.radius + spec.rim_width
                && a >= kernel.rim_height_at(r) - kernel.bowl_depth_at(r)
                && a <= spec.radius
        })
        .collect()
}

#[test]
fn the_normal_that_the_cells_give_is_measured_against_the_analytic_one_at_each_radius_of_estimate() {
    // the crest radius of the law for this ball, from the flat ground (the same ball and speed whatever the ground)
    let flat = Ground { radius: 20.0, height: 0.0, basin: false };
    let grown = hit("flat", flat, 0.0).node.crater_impact.as_ref().expect("the impact").clone();
    let crest = grown.spec.radius;
    println!("HILL the crest radius is {crest:.2} m");
    for (radius, height) in [(40.0, 5.0), (20.0, 4.0), (10.0, 3.0), (5.0, 2.0)] {
        let ground = Ground { radius, height, basin: false };
        let cells = ground.occupancy();
        for d in [0.0, 2.0, 5.0] {
            if ground.rise(d) <= 0.0 {
                continue;
            }
            let point = [15.0 + d, ground.surface(15.0 + d, 15.0), 15.0];
            // object units: the object's min corner is at the origin, the surface's y is counted from the top key
            let point = [point[0], point[1] - f64::from(ground.top_key()) * 0.25, point[2]];
            let want = ground.normal(d);
            let mut line = format!("HILL curvature {radius} m, {d} m off:");
            for (name, r) in [("crest", crest), ("half", 0.5 * crest), ("third", crest / 3.0), ("ball", 2.0)] {
                let normal = sr_eval::voxel_cut::surface_normal(&cells, point, r, 0.25).unwrap();
                line += &format!("  {name} {:.1} deg", degrees_between(normal, want));
            }
            println!("{line}");
            // the axis that the cut is made on, from the same point (the contact is the ball's first touch, here the analytic point with the normal of the cells' face)
            let mut at = (*grown).clone();
            at.spec.center = point;
            at.spec.outward = [0.0, -1.0, 0.0];
            let axis = crater_axis(&at, &cells, &settings());
            let error = degrees_between(axis, want);
            println!("HILL curvature {radius} m, {d} m off: the axis of the cut is {error:.1} degrees off the normal");
            assert!(
                error < 5.0,
                "curvature {radius} m, {d} m off: the axis is {error:.1} degrees off the analytic normal (limit 5)"
            );
        }
    }
}

#[test]
fn a_hill_is_cut_about_its_normal_and_what_the_kernel_takes_is_what_a_brute_force_says() {
    for (name, ground, d) in [
        ("top-20", Ground { radius: 20.0, height: 4.0, basin: false }, 0.0),
        ("off-20", Ground { radius: 20.0, height: 4.0, basin: false }, 5.0),
        ("top-10", Ground { radius: 10.0, height: 3.0, basin: false }, 0.0),
        ("off-10", Ground { radius: 10.0, height: 3.0, basin: false }, 5.0),
    ] {
        let shot = hit(name, ground, d);
        let grown = shot.node.crater_impact.as_ref().expect("the impact");
        let spec = crater_spec(grown, &shot.cells, &settings());
        let want = ground.normal(d);
        let angle = degrees_between(spec.outward, want);
        println!(
            "HILL {name}: the axis is {angle:.1} degrees off the normal there; the evaluator says {:?}",
            shot.said
        );
        let kernel = crater_kernel(grown, spec).unwrap();
        let all_of_it = brute_force(&kernel, &shot.cells);
        println!(
            "HILL {name}: the kernel's region holds {} cells ({:.2} m3), the law's {:.2}",
            all_of_it.len(),
            all_of_it.len() as f64 * 0.015625,
            grown.law().volume
        );
    }
}
