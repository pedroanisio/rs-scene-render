//! The image of a Schwarzschild black hole traced on the CPU: the size of its shadow, its symmetry, the redshift of
//! the disc, and its determinism.
use sr_sim::gr::image::{render, trace_pixel, Camera, Class, Disk};
use sr_sim::gr::oracle;

const M: f64 = 1.0;

/// A camera with a focal length of `focal` pixels.
fn camera(distance: f64, inclination: f64, focal: f64, size: [usize; 2]) -> Camera {
    let fov = 2.0 * (0.5 * size[1] as f64 / focal).atan();
    Camera::orbiting(distance, inclination, fov, size)
}

#[test]
fn the_camera_looks_at_the_hole_and_its_axes_are_orthonormal_and_right_handed() {
    for inclination in [0.0, 0.7, std::f64::consts::FRAC_PI_2, 2.5] {
        let c = camera(30.0 * M, inclination, 120.0, [257, 161]);
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for v in [c.fwd, c.right, c.down] {
            assert!((dot(v, v) - 1.0).abs() < 1e-14, "{inclination}: {v:?}");
        }
        assert!(
            dot(c.fwd, c.right).abs() < 1e-14 && dot(c.fwd, c.down).abs() < 1e-14 && dot(c.right, c.down).abs() < 1e-14
        );
        let cross = [
            c.right[1] * c.down[2] - c.right[2] * c.down[1],
            c.right[2] * c.down[0] - c.right[0] * c.down[2],
            c.right[0] * c.down[1] - c.right[1] * c.down[0],
        ];
        for i in 0..3 {
            assert!((cross[i] - c.fwd[i]).abs() < 1e-14, "right x down is forward");
        }
        // it looks at the hole from the distance asked, the centre pixel straight at it
        let distance = (c.eye[0].powi(2) + c.eye[1].powi(2) + c.eye[2].powi(2)).sqrt();
        assert!((distance - 30.0 * M).abs() < 1e-12);
        let centre = c.ray(128, 80);
        for i in 0..3 {
            assert!(
                (centre[i] - c.fwd[i]).abs() < 1e-15 && (c.eye[i] + 30.0 * c.fwd[i]).abs() < 1e-12,
                "{inclination}"
            );
        }
    }
    // an equatorial camera has the spin axis up in the image
    let c = camera(30.0 * M, std::f64::consts::FRAC_PI_2, 120.0, [257, 161]);
    assert!((c.down[1] + 1.0).abs() < 1e-14, "{:?}", c.down);
}

#[test]
fn the_shadow_in_pixels_is_the_critical_impact_parameter_seen_from_the_camera() {
    // an equatorial camera and no disc: the captured pixels are the disc of the shadow
    let (distance, focal, size) = (40.0 * M, 100.0, [256, 160]);
    let c = camera(distance, std::f64::consts::FRAC_PI_2, focal, size);
    let pixels = render(&c, M, None);
    let captured: Vec<usize> =
        pixels.iter().enumerate().filter(|(_, p)| p.class == Class::Captured).map(|(i, _)| i).collect();
    // the angle of the edge of the shadow seen by a static observer, and its radius on the image
    let alpha = (oracle::shadow_radius(M) * (1.0 - 2.0 * M / distance).sqrt() / distance).asin();
    let radius = focal * alpha.tan();
    let by_area = (captured.len() as f64 / std::f64::consts::PI).sqrt();
    println!(
        "SHADOW expected {radius:.3} px, by area {by_area:.3} px, {} captured of {}",
        captured.len(),
        pixels.len()
    );
    assert!((by_area - radius).abs() < 0.5, "{by_area} against {radius}");
    // along the middle row the captured run is the diameter, to a pixel
    let row = 80;
    let run = (0..size[0]).filter(|&x| pixels[row * size[0] + x].class == Class::Captured).count() as f64;
    assert!((0.5 * run - radius).abs() < 0.5, "{} against {radius}", 0.5 * run);
    // everything captured is inside, everything not captured is outside (a pixel's size apart)
    for (i, p) in pixels.iter().enumerate() {
        let (x, y) = ((i % size[0]) as f64 + 0.5 - 128.0, (i / size[0]) as f64 + 0.5 - 80.0);
        let rho = x.hypot(y);
        assert_eq!(p.class == Class::Captured, rho < radius, "pixel {i} at {rho:.2} of {radius:.2}");
    }
}

#[test]
fn the_image_is_symmetric_left_to_right_and_the_redshifts_of_the_two_sides_balance() {
    let c = camera(40.0 * M, 1.2, 100.0, [256, 160]);
    let disk = Disk::flat(6.0 * M, 20.0 * M);
    let pixels = render(&c, M, Some(&disk));
    let [w, h] = c.size;
    let mut discs = 0;
    for y in 0..h {
        for x in 0..w / 2 {
            let (a, b) = (&pixels[y * w + x], &pixels[y * w + (w - 1 - x)]);
            assert_eq!(a.class, b.class, "({x}, {y})");
            assert_eq!(a.order, b.order, "({x}, {y})");
            if a.class == Class::Disk {
                discs += 1;
                assert!((a.r - b.r).abs() < 1e-9 * a.r, "({x}, {y}): {} {}", a.r, b.r);
                // the disc turns one way: what the left side gains the right side loses, and the two redshifts
                // average to the gravitational one
                let gravitational = (1.0 - 3.0 * M / a.r).sqrt();
                assert!((0.5 * (1.0 / a.g + 1.0 / b.g) - 1.0 / gravitational).abs() < 1e-9, "({x}, {y})");
                assert!((a.g - b.g).abs() > 0.0 || x == w / 2);
            } else if a.class == Class::Background {
                // the sky is mirrored through the plane of the camera's axes
                assert!((a.direction[0] - b.direction[0]).abs() < 1e-9);
                assert!((a.direction[1] - b.direction[1]).abs() < 1e-9);
                assert!((a.direction[2] + b.direction[2]).abs() < 1e-9);
            }
        }
    }
    assert!(discs > 1000, "{discs} pixels of the disc on each side");
}

#[test]
fn where_the_ray_meets_the_disc_at_azimuth_zero_the_redshift_is_the_gravitational_one() {
    // an odd width puts a column of pixels in the plane of the observer, the hole and the axis of the disc, where
    // the motion of the disc is across the line of sight
    let c = camera(40.0 * M, 1.0, 100.0, [257, 161]);
    let disk = Disk::flat(6.0 * M, 20.0 * M);
    let mut seen = 0;
    for y in 0..161 {
        let p = trace_pixel(&c, M, Some(&disk), 128, y);
        if p.class == Class::Disk {
            seen += 1;
            let gravitational = (1.0 - 3.0 * M / p.r).sqrt();
            assert!((p.g - gravitational).abs() < 1e-12, "row {y}: {} against {gravitational}", p.g);
        }
    }
    assert!(seen > 20, "{seen} pixels of the disc in the column");
    // off that column the sides differ
    let left = trace_pixel(&c, M, Some(&disk), 100, 100);
    let right = trace_pixel(&c, M, Some(&disk), 156, 100);
    assert!(left.class == Class::Disk && right.class == Class::Disk);
    assert!((left.g - right.g).abs() > 0.05, "{} {}", left.g, right.g);
}

#[test]
fn a_disc_seen_from_above_is_a_ring_and_the_hole_is_black_in_the_middle() {
    let c = camera(60.0 * M, 0.0, 100.0, [201, 201]);
    let disk = Disk::flat(6.0 * M, 20.0 * M);
    let middle = trace_pixel(&c, M, Some(&disk), 100, 100);
    assert_eq!(middle.class, Class::Captured);
    // a ray at the shadow's edge and one far outside it: black, then the disc, then the sky
    let classes: Vec<Class> = (101..201).map(|x| trace_pixel(&c, M, Some(&disk), x, 100).class).collect();
    assert_eq!(classes[0], Class::Captured);
    assert!(classes.contains(&Class::Disk) && *classes.last().unwrap() == Class::Background);
    // the disc is one ring: the pixels of the disc along the radius are one run
    let first = classes.iter().position(|c| *c == Class::Disk).unwrap();
    let last = classes.iter().rposition(|c| *c == Class::Disk).unwrap();
    assert!(classes[first..=last].iter().all(|c| *c == Class::Disk), "{classes:?}");
}

#[test]
fn the_same_arguments_give_the_same_pixels() {
    let c = camera(40.0 * M, 1.3, 80.0, [64, 40]);
    let disk = Disk::flat(6.0 * M, 20.0 * M);
    let (a, b) = (render(&c, M, Some(&disk)), render(&c, M, Some(&disk)));
    assert_eq!(a, b);
    let bits = |p: &[sr_sim::gr::image::Pixel]| {
        p.iter().flat_map(|p| [p.r.to_bits(), p.g.to_bits(), p.psi.to_bits(), p.phi_inf.to_bits()]).collect::<Vec<_>>()
    };
    assert_eq!(bits(&a), bits(&b));
}
