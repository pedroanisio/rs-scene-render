//! SREP 26: the point generators of `repeat/points`, checked against the numeric tests of the SREP
//! (Conformance, "Numeric tests"). Counts and candidate indices are exact; coordinates to 1e-9 px.

use sr_eval::points::{self, FlatPath, Point};

const EPS: f64 = 1e-9;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPS
}

fn at(p: &Point, x: f64, y: f64) -> bool {
    close(p.x, x) && close(p.y, y)
}

fn flat(d: &str) -> FlatPath {
    FlatPath::parse(d).unwrap_or_else(|e| panic!("{d:?}: {e:?}"))
}

// ------------------------------------------------------------------ hash tests

#[test]
fn hash_chain_matches_the_srep() {
    assert_eq!(points::hash(0, 0, 0), 0x238275bc38fcbe91);
    assert_eq!(points::hash(7, 0, 0), 0xd3c8201d52fd2df4);
    assert_eq!(points::hash(7, 1, 0), 0x7a099dad0dd482f0);
    assert_eq!(points::hash(14325487974692486532, 0, 0), 0x0);
}

#[test]
fn unit_draw_matches_the_srep() {
    assert_eq!(points::unit(0, 0, 0), 0.13870941014555427);
    assert_eq!(points::unit(7, 0, 0), 0.8272724219886994);
    assert_eq!(points::unit(7, 1, 0), 0.47670922732307197);
    assert_eq!(points::unit(14325487974692486532, 0, 0), 0.0);
}

// ------------------------------------------------------------------ transform tests

#[test]
fn point_random_is_channel_two() {
    assert_eq!(points::unit(7, 2, 0), 0.5407366383275906);
    assert_eq!(points::unit(7, 2, 5), 0.356758657594873);
}

#[test]
fn grid_is_centred_on_the_origin() {
    // 3 x 2 at the default spacing of 100
    let want = [(-100.0, -50.0), (0.0, -50.0), (100.0, -50.0), (-100.0, 50.0), (0.0, 50.0), (100.0, 50.0)];
    for (i, (x, y)) in want.iter().enumerate() {
        let p = points::grid(3, 2, 100.0, 100.0, i as u64);
        assert!(at(&p, *x, *y), "point {i}: {p:?}");
        assert_eq!(p.theta, 0.0);
        assert_eq!(p.direction, 0.0);
    }
    let p = points::grid(1, 1, 37.0, 11.0, 0);
    assert!(at(&p, 0.0, 0.0));
}

#[test]
fn rectangle_scatter_point_zero() {
    let p = points::scatter_rect(7, 100.0, 100.0, 0);
    assert!(at(&p, (0.8272724219886994 - 0.5) * 100.0, (0.47670922732307197 - 0.5) * 100.0), "{p:?}");
    assert_eq!(p.theta, 0.0);
}

#[test]
fn edge_candidate_is_accepted() {
    let path = flat("M0,0 H100 V100 H0 Z");
    let (pts, js) = points::scatter_path(&path, 14325487974692486532, 3, false);
    assert_eq!(js, vec![0, 1, 2]);
    assert_eq!(pts.len(), 3);
    assert!(at(&pts[0], 0.0, 60.38343622198615), "{:?}", pts[0]);
}

#[test]
fn sliver_count_and_order() {
    let path = flat("M0,0 H100 V0.5 H0.5 V100 H0 Z");
    let (_, js) = points::scatter_path(&path, 7, 10, false);
    assert_eq!(js, vec![77, 81, 90, 149, 299, 490, 592]);
    assert_eq!(points::scatter_path(&path, 1, 10, false).1.len(), 4);
    assert_eq!(points::scatter_path(&path, 42, 10, false).1.len(), 5);
}

#[test]
fn candidate_bound_is_sixty_four_per_point() {
    let path = flat("M0,0 H100 V0.5 H0.5 V100 H0 Z");
    let (pts, js) = points::scatter_path(&path, 61, 10, false);
    assert_eq!(js, vec![40, 115, 164, 315, 437, 464]);
    assert_eq!(pts.len(), 6, "candidate 640 lies inside the region but must not be tried");
    // the candidate the bound excludes is inside the region
    let (cx, cy) = (points::unit(61, 0, 640) * 100.0, points::unit(61, 1, 640) * 100.0);
    assert!(close(cx, 0.42830310767921764) && close(cy, 10.420086475479973), "({cx}, {cy})");
}

#[test]
fn fill_rule_decides_the_nested_square() {
    let path = flat("M0,0 H100 V100 H0 Z M25,25 H75 V75 H25 Z");
    assert_eq!(points::scatter_path(&path, 7, 5, false).1, vec![0, 1, 2, 3, 4]);
    assert_eq!(points::scatter_path(&path, 7, 5, true).1, vec![0, 1, 3, 4, 5]);
}

#[test]
fn degenerate_paths() {
    for d in ["", "M50,50", "M1,1 M2,2"] {
        let p = flat(d);
        assert!(points::along_path(&p, 3, true).is_empty(), "along-path {d:?}");
        assert!(points::scatter_path(&p, 7, 3, false).0.is_empty(), "scatter {d:?}");
    }
    assert!(points::vertices(&flat("")).is_empty());
    // movetos only: one vertex per moveto
    assert_eq!(points::vertices(&flat("M10,10 M90,10")).len(), 2);
    // a zero-size box: no scatter, while the same line keeps its along-path and vertex points
    let vertical = flat("M100,-50 L100,0");
    assert!(points::scatter_path(&vertical, 7, 3, false).0.is_empty());
    assert_eq!(points::along_path(&vertical, 2, true).len(), 2);
    assert_eq!(points::vertices(&vertical).len(), 2);
    // segments of zero total length: every point at the start, direction 0
    let dot = points::along_path(&flat("M50,50 L50,50"), 3, true);
    assert_eq!(dot.len(), 3);
    for p in &dot {
        assert!(at(p, 50.0, 50.0) && p.theta == 0.0 && p.direction == 0.0, "{p:?}");
    }
    // a zero-length segment before a vertical one: the start takes the vertical direction
    let p = points::along_path(&flat("M0,0 L0,0 L0,10"), 2, true);
    assert!(close(p[0].direction, 90.0) && close(p[0].theta, 90.0), "{:?}", p[0]);
    assert_eq!(points::along_path(&flat("M0,0 L10,0"), 0, false).len(), 0);
}

// ------------------------------------------------------------------ geometry of the raster cases

#[test]
fn order_case_point_one_points_down() {
    // srep-0-order: along-path M100,-50 L100,0, count 2, orient
    let p = points::along_path(&flat("M100,-50 L100,0"), 2, true);
    assert!(at(&p[0], 100.0, -50.0));
    assert!(at(&p[1], 100.0, 0.0), "{:?}", p[1]);
    assert!(close(p[1].theta, 90.0) && close(p[1].direction, 90.0));
}

#[test]
fn open_path_spaces_points_end_to_end() {
    let p = points::along_path(&flat("M-100,0 L100,0"), 5, false);
    let xs: Vec<f64> = p.iter().map(|p| p.x).collect();
    assert_eq!(xs, vec![-100.0, -50.0, 0.0, 50.0, 100.0]);
    assert!(p.iter().all(|p| p.y == 0.0 && p.theta == 0.0));
    let one = points::along_path(&flat("M-100,0 L100,0"), 1, false);
    assert!(at(&one[0], -100.0, 0.0));
}

#[test]
fn closed_path_does_not_double_the_start() {
    let p = points::along_path(&flat("M-50,-50 H50 V50 H-50 Z"), 4, false);
    let want = [(-50.0, -50.0), (50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)];
    for (p, (x, y)) in p.iter().zip(want) {
        assert!(at(p, x, y), "{p:?}");
    }
    // direction is reported whatever orient says; theta only with orient
    let p = points::along_path(&flat("M-50,-50 H50 V50 H-50 Z"), 8, false);
    assert!(close(p[1].direction, 0.0) && close(p[3].direction, 90.0) && close(p[5].direction, 180.0));
    assert!(close(p[7].direction.abs(), 90.0) && p[7].direction < 0.0, "{:?}", p[7]);
    assert!(p.iter().all(|p| p.theta == 0.0));
}

#[test]
fn two_subpaths_are_open_and_jumps_take_no_length() {
    // two closed squares: not "exactly one closed subpath", so the last point is the path's end
    let p = points::along_path(&flat("M0,0 H10 V10 H0 Z M100,0 H110 V10 H100 Z"), 3, true);
    assert!(at(&p[0], 0.0, 0.0));
    // half of 80 px is 40: the end of the first square, which is the jump: the next subpath's start
    assert!(at(&p[1], 100.0, 0.0), "{:?}", p[1]);
    assert!(close(p[1].direction, 0.0));
    assert!(at(&p[2], 100.0, 0.0), "{:?}", p[2]);
    assert!(close(p[2].direction, -90.0), "the last segment goes up: {:?}", p[2]);
}

#[test]
fn curves_flatten_in_sixteen_parameter_steps() {
    let f = flat("M0,0 C0,100 100,100 100,0");
    let v = f.vertices();
    assert_eq!(v.len(), 17);
    for (k, q) in v.iter().enumerate() {
        let t = k as f64 / 16.0;
        let m = 1.0 - t;
        let x = 3.0 * m * t * t * 100.0 + t * t * t * 100.0;
        let y = 3.0 * m * m * t * 100.0 + 3.0 * m * t * t * 100.0;
        assert!(close(q[0], x) && close(q[1], y), "vertex {k}: {q:?} vs ({x}, {y})");
    }
    // the midpoint of a 2-point along-path on a symmetric curve is a flattened vertex: t = 1/2 exactly
    let p = points::along_path(&f, 3, false);
    assert!(at(&p[1], 50.0, 75.0), "{:?}", p[1]);
    // an arc is one curve command: 16 steps of its angle
    let a = flat("M0,0 A50,50 0 0 1 100,0");
    assert_eq!(a.vertices().len(), 17);
    let mid = a.vertices()[8];
    assert!(close(mid[0], 50.0) && close(mid[1], -50.0), "{mid:?}");
    // quadratic curves too
    assert_eq!(flat("M0,0 Q50,100 100,0").vertices().len(), 17);
}

#[test]
fn vertices_are_command_end_points() {
    // closed triangle with a curved side: three points, none for the closepath
    let v = points::vertices(&flat("M-100,-50 L100,-50 Q50,0 0,50 Z"));
    let want = [(-100.0, -50.0), (100.0, -50.0), (0.0, 50.0)];
    assert_eq!(v.len(), 3);
    for (p, (x, y)) in v.iter().zip(want) {
        assert!(at(p, x, y), "{p:?}");
    }
    // the last point of a closed subpath that returns to its start is dropped
    assert_eq!(points::vertices(&flat("M0,0 L10,0 L10,10 L0,0 Z")).len(), 3);
    // ... but not when the subpath is open
    assert_eq!(points::vertices(&flat("M0,0 L10,0 L10,10 L0,0")).len(), 4);
    // implicit linetos after a moveto are commands
    assert_eq!(points::vertices(&flat("M0,0 10,0 10,10")).len(), 3);
}

#[test]
fn list_parses_pairs() {
    let p = points::list(&[[-100.0, -50.0], [0.0, 0.0], [100.0, 50.0]]);
    assert_eq!(p.len(), 3);
    assert!(at(&p[2], 100.0, 50.0));
}

#[test]
fn point_u_runs_from_zero_to_one() {
    assert_eq!(points::point_u(0, 1), 0.0);
    assert_eq!(points::point_u(0, 5), 0.0);
    assert_eq!(points::point_u(4, 5), 1.0);
    assert_eq!(points::point_u(1, 5), 0.25);
}

#[test]
fn path_errors_are_reported() {
    assert!(FlatPath::parse("0 0 L 1 1").is_err());
    assert!(FlatPath::parse("M0 0 X 1 1").is_err());
}
