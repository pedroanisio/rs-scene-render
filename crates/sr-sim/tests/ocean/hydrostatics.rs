//! The part of a body below the water's surface: its volume and the centroid of that volume,
//! for buoyancy. Scene axes: y points down, so the water is where y is greater than the
//! surface's.

use sr_sim::hydrostatics::{submerged_mesh, submerged_sphere, Submerged, Surface};
use std::f64::consts::PI;

fn level(y: f64) -> Surface {
    Surface { offset: y, slope: [0.0, 0.0] }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs().max(1e-12)
}

/// An axis-aligned box as an outward-facing closed mesh.
fn cuboid(centre: [f64; 3], half: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let points: Vec<[f64; 3]> = (0..8)
        .map(|k| {
            let s = |bit: usize, c: f64, h: f64| c + if (k >> bit) & 1 == 1 { h } else { -h };
            [s(0, centre[0], half[0]), s(1, centre[1], half[1]), s(2, centre[2], half[2])]
        })
        .collect();
    let faces = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    (points, faces.to_vec())
}

/// A UV sphere as an outward-facing closed mesh.
fn ball(centre: [f64; 3], r: f64, rings: usize, around: usize) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut points = vec![[centre[0], centre[1] - r, centre[2]]];
    for i in 1..rings {
        let phi = PI * i as f64 / rings as f64;
        for j in 0..around {
            let theta = 2.0 * PI * j as f64 / around as f64;
            points.push([
                centre[0] + r * phi.sin() * theta.cos(),
                centre[1] - r * phi.cos(),
                centre[2] + r * phi.sin() * theta.sin(),
            ]);
        }
    }
    points.push([centre[0], centre[1] + r, centre[2]]);
    let ring = |i: usize, j: usize| (1 + (i - 1) * around + j % around) as u32;
    let mut triangles = Vec::new();
    for j in 0..around {
        triangles.push([0, ring(1, j + 1), ring(1, j)]);
    }
    for i in 1..rings - 1 {
        for j in 0..around {
            let (a, b, c, d) = (ring(i, j), ring(i, j + 1), ring(i + 1, j), ring(i + 1, j + 1));
            triangles.push([a, b, c]);
            triangles.push([b, d, c]);
        }
    }
    let last = (points.len() - 1) as u32;
    for j in 0..around {
        triangles.push([last, ring(rings - 1, j), ring(rings - 1, j + 1)]);
    }
    (points, triangles)
}

fn mesh(m: &(Vec<[f64; 3]>, Vec<[u32; 3]>), s: &Surface) -> Submerged {
    submerged_mesh(&m.0, &m.1, s).unwrap()
}

#[test]
fn a_box_below_a_level_surface_holds_the_base_times_the_depth() {
    // 4 x 2 x 6 box centred at y = 10, whose top is at y = 9 and bottom at y = 11
    let b = cuboid([1.0, 10.0, -2.0], [2.0, 1.0, 3.0]);
    // the water is where y is greater than the surface's: a surface above the box's top (y = 9) has it all
    // under water and one below its bottom (y = 11) has none of it
    for surface in [8.0, 9.0, 9.5, 10.0, 10.9, 11.0, 30.0] {
        let depth = (11.0f64 - surface).clamp(0.0, 2.0);
        let s = mesh(&b, &level(surface));
        assert!(
            close(s.volume, 4.0 * 6.0 * depth, 1e-12) || (s.volume == 0.0 && depth == 0.0),
            "{surface}: {}",
            s.volume
        );
        if depth > 0.0 {
            // the submerged slab's centroid: middle of the base, half the depth above the bottom
            assert!(close(s.centroid[0], 1.0, 1e-9) && close(s.centroid[2], -2.0, 1e-9), "{:?}", s.centroid);
            assert!(close(s.centroid[1], 11.0 - depth / 2.0, 1e-9), "{surface}: {:?}", s.centroid);
        }
    }
}

#[test]
fn a_tilted_surface_cuts_a_box_in_a_wedge() {
    // surface y = 10 + 0.5 x through the centre of a 2 x 2 x 2 box at (0, 10, 0): half the box
    let b = cuboid([0.0, 10.0, 0.0], [1.0, 1.0, 1.0]);
    let s = mesh(&b, &Surface { offset: 10.0, slope: [0.5, 0.0] });
    assert!(close(s.volume, 4.0, 1e-12), "{}", s.volume);
    // a plane through the centre: the submerged half's centroid is below it, on the downhill side
    assert!(s.centroid[1] > 10.0 && s.centroid[0] < 0.0, "{:?}", s.centroid);
    // the wedge cut off at the corner: surface through (1, 9, *) and (-1, 11, *) is y = 10 - x
    let wedge = mesh(&b, &Surface { offset: 10.0, slope: [-1.0, 0.0] });
    assert!(close(wedge.volume, 4.0, 1e-12), "{}", wedge.volume);
    let corner = mesh(&b, &Surface { offset: 9.5, slope: [-1.0, 0.0] });
    // y > 9.5 - x: heights 1.5 + x up to x = 0.5 and then the whole 2, so 1.875 + 1 of area, times 2 on z
    assert!(close(corner.volume, 2.0 * 2.875, 1e-12), "{}", corner.volume);
}

#[test]
fn the_part_below_and_the_part_above_make_the_whole() {
    let shapes = [cuboid([0.3, 10.0, 0.1], [1.2, 0.8, 1.5]), ball([0.0, 10.0, 0.0], 2.0, 24, 32)];
    for m in &shapes {
        let total = mesh(m, &level(-1.0e6)).volume;
        assert!(total > 0.0);
        for surface in
            [Surface { offset: 9.2, slope: [0.1, -0.3] }, Surface { offset: 10.7, slope: [0.0, 0.2] }, level(9.9)]
        {
            // the same plane seen from the other side: y < surface is y' > -surface for mirrored axes
            let below = mesh(m, &surface).volume;
            let mirrored: Vec<[f64; 3]> = m.0.iter().map(|p| [p[0], -p[1], p[2]]).collect();
            // mirroring y flips the orientation, so swap two corners of every triangle
            let flipped: Vec<[u32; 3]> = m.1.iter().map(|t| [t[0], t[2], t[1]]).collect();
            let above = submerged_mesh(
                &mirrored,
                &flipped,
                &Surface { offset: -surface.offset, slope: [-surface.slope[0], -surface.slope[1]] },
            )
            .unwrap()
            .volume;
            assert!(close(below + above, total, 1e-9), "{surface:?}: {below} + {above} against {total}");
        }
    }
}

#[test]
fn a_ball_matches_the_spherical_cap_it_approximates() {
    let m = ball([0.0, 10.0, 0.0], 2.0, 48, 64);
    for surface in [8.0, 9.0, 10.0, 11.0, 11.9, 12.0] {
        let analytic = submerged_sphere([0.0, 10.0, 0.0], 2.0, &level(surface));
        let found = mesh(&m, &level(surface));
        // a tessellated sphere is inside the true one by about 0.1 %
        let whole = 4.0 / 3.0 * PI * 8.0;
        assert!(
            (found.volume - analytic.volume).abs() < 5e-3 * whole,
            "{surface}: {} vs {}",
            found.volume,
            analytic.volume
        );
        if analytic.volume > 1e-3 {
            assert!(
                (found.centroid[1] - analytic.centroid[1]).abs() < 2e-2,
                "{surface}: {:?} vs {:?}",
                found.centroid,
                analytic.centroid
            );
        }
    }
}

#[test]
fn the_cap_of_a_sphere_follows_the_closed_forms() {
    let r = 1.5;
    // surface at the centre: half the ball, centroid 3r/8 below the centre
    let half = submerged_sphere([2.0, 5.0, -1.0], r, &level(5.0));
    assert!(close(half.volume, 2.0 / 3.0 * PI * r.powi(3), 1e-12));
    assert!(close(half.centroid[1], 5.0 + 3.0 * r / 8.0, 1e-12), "{:?}", half.centroid);
    // wholly in the water, wholly out of it
    assert!(close(submerged_sphere([0.0; 3], r, &level(-10.0)).volume, 4.0 / 3.0 * PI * r.powi(3), 1e-12));
    assert_eq!(submerged_sphere([0.0; 3], r, &level(10.0)).volume, 0.0);
    // a cap of height h: V = pi h^2 (3r - h) / 3
    for h in [0.1, 0.7, 1.5, 2.2, 2.9] {
        let v = submerged_sphere([0.0, 0.0, 0.0], r, &level(r - h));
        assert!(close(v.volume, PI * h * h * (3.0 * r - h) / 3.0, 1e-12), "h = {h}");
    }
    // a tilted surface through the centre still cuts it in half, whatever its slope
    for slope in [[0.0, 0.0], [0.5, 0.0], [1.5, -2.0]] {
        let v = submerged_sphere([1.0, 2.0, 3.0], r, &Surface { offset: 2.0 - slope[0] - 3.0 * slope[1], slope });
        assert!(close(v.volume, 2.0 / 3.0 * PI * r.powi(3), 1e-12), "{slope:?}");
    }
}

#[test]
fn the_submerged_volume_grows_as_the_body_sinks_and_is_never_negative() {
    let mut last = -1.0;
    for k in 0..40 {
        let y = 8.0 + 0.1 * k as f64;
        let s = mesh(&ball([0.0, y, 0.0], 1.0, 16, 24), &level(10.0));
        assert!(s.volume >= 0.0 && s.volume >= last - 1e-12, "{y}: {} after {last}", s.volume);
        last = s.volume;
    }
}

#[test]
fn a_mesh_that_is_not_closed_enough_to_measure_is_an_error() {
    assert!(submerged_mesh(&[[0.0; 3]], &[[0, 1, 2]], &level(0.0)).is_err(), "a triangle names a missing vertex");
    assert!(submerged_mesh(&[[0.0, f64::NAN, 0.0], [1.0; 3], [2.0; 3]], &[[0, 1, 2]], &level(0.0)).is_err());
    assert!(submerged_mesh(&[[0.0; 3]; 3], &[[0, 1, 2]], &Surface { offset: f64::NAN, slope: [0.0; 2] }).is_err());
}

#[test]
fn the_waterline_and_the_area_seen_from_above_follow_the_shape() {
    // a box 4 x 6 in plan, half under: the waterline is the whole plan and so is what it presents from above
    let b = cuboid([1.0, 10.0, -2.0], [2.0, 1.0, 3.0]);
    let half = mesh(&b, &level(10.0));
    assert!(close(half.waterline, 24.0, 1e-12) && close(half.projected, 24.0, 1e-12), "{half:?}");
    // wholly under the surface there is no waterline, and it still presents its plan
    let under = mesh(&b, &level(0.0));
    assert!(under.waterline < 1e-12 && close(under.projected, 24.0, 1e-12), "{under:?}");
    // a ball, against the closed forms: a disc of the waterline's circle when shallow, the ball's disc when deep
    let m = ball([0.0, 10.0, 0.0], 2.0, 48, 64);
    for surface in [11.5, 10.0, 8.5] {
        let analytic = submerged_sphere([0.0, 10.0, 0.0], 2.0, &level(surface));
        let found = mesh(&m, &level(surface));
        assert!(
            close(found.waterline, analytic.waterline, 2e-2),
            "{surface}: {} vs {}",
            found.waterline,
            analytic.waterline
        );
        assert!(
            close(found.projected, analytic.projected, 2e-2),
            "{surface}: {} vs {}",
            found.projected,
            analytic.projected
        );
    }
    // dry, it presents nothing
    assert_eq!(mesh(&b, &level(30.0)).projected, 0.0);
}

#[test]
fn the_primitive_shapes_tessellate_into_closed_hulls_of_the_right_volume() {
    use sr_sim::hydrostatics::hull_mesh;
    use sr_sim::physics3d::{shape_volume, Shape3};
    for shape in
        [Shape3::Box([1.0, 2.0, 3.0]), Shape3::Cylinder(1.5, 0.8), Shape3::Cone(1.2, 1.0), Shape3::Capsule(1.0, 0.6)]
    {
        let (points, triangles) = hull_mesh(&shape).unwrap();
        // far below the surface the whole hull is submerged, and that is its volume
        let whole = submerged_mesh(&points, &triangles, &level(-1.0e6)).unwrap();
        let want = shape_volume(&shape).unwrap();
        assert!(close(whole.volume, want, 2e-2), "{shape:?}: {} against {want}", whole.volume);
        // and cut at its middle, about half (a cone is not symmetric)
        let half = submerged_mesh(&points, &triangles, &level(0.0)).unwrap();
        assert!(half.volume > 0.0 && half.volume < whole.volume, "{shape:?}");
    }
    assert!(hull_mesh(&Shape3::Sphere(1.0)).is_err());
    assert!(hull_mesh(&Shape3::Convex(vec![[0.0; 3]; 4])).is_err());
}

#[test]
fn the_waterline_of_a_ball_seen_from_above_does_not_depend_on_the_slope_of_the_surface() {
    // the waterline seen from above is the cut across a tilted surface times the cosine of the tilt, which the mesh
    // measures and the closed form of the sphere has to agree with
    let r = 2.0;
    let m = ball([0.0, 10.0, 0.0], r, 96, 128);
    for slope in [[0.0f64, 0.0], [0.5, 0.0], [0.0, -1.0], [1.5, 1.0]] {
        for h in [0.4f64, 1.0, 1.6] {
            // the cap of the ball below the surface is `h` high, measured along the surface's normal
            let steep = (1.0 + slope[0] * slope[0] + slope[1] * slope[1]).sqrt();
            let offset = 10.0 + (r - h) * steep;
            let surface = Surface { offset, slope };
            let analytic = submerged_sphere([0.0, 10.0, 0.0], r, &surface);
            let found = mesh(&m, &surface);
            let whole = PI * r * r;
            assert!(
                (found.waterline - analytic.waterline).abs() < 2e-2 * whole,
                "slope {slope:?}, h {h}: waterline {} against {}",
                found.waterline,
                analytic.waterline
            );
        }
    }
}
