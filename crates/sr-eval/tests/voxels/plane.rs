//! The mean plane of the surface of cells that a crater is put on, and what it refuses to fit.
use sr_3d::occupancy::Occupancy;
use sr_eval::voxel_cut::{mean_height_over, mean_plane_offset};

const CELL: f64 = 0.25;
/// Up is toward minus y: the outward axis of ground whose surface is y = const.
const UP: [f64; 3] = [0.0, -1.0, 0.0];

/// Ground of 20 m by 20 m in plan from the surface at cell row `top` down to row `bottom` (exclusive), without the cells for which `gone` says so.
fn ground(top: i32, bottom: i32, gone: impl Fn(i32, i32, i32) -> bool) -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..80 {
        for j in top..bottom {
            for i in 0..80 {
                if !gone(i, j, k) {
                    cells.push(([i, j, k], 1u8));
                }
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

/// The surface of `ground(top, ..)` in object units (y of the top face of the first row) and the point 10 m across it, at the surface.
fn at_surface(top: i32) -> [f64; 3] {
    [10.0, f64::from(top) * CELL, 10.0]
}

#[test]
fn flat_ground_of_a_thickness_has_its_plane_at_its_surface() {
    // the positive control of every refusal below: the same ground with nothing taken away, and the point 0.1 m under the surface
    let g = ground(8, 80, |_, _, _| false);
    let point = at_surface(8);
    let under = [point[0], point[1] + 0.1, point[2]];
    let ball = mean_plane_offset(&g, under, UP, 4.0, CELL).expect("a plane");
    assert!(
        (ball - 0.1).abs() < 0.08,
        "the ball's fit puts the plane {ball} m over the point under the surface by 0.1 m"
    );
    let disc = mean_height_over(&g, under, UP, 6.0, 4.0, CELL).expect("a plane");
    assert!(
        (disc - 0.1).abs() < 1e-9,
        "the disc's fit puts it {disc} m over the point (0.1 m): the share of a cell counts for its extent"
    );
}

#[test]
fn a_ball_of_a_few_cells_is_no_sample_of_a_plane() {
    let g = ground(8, 80, |_, _, _| false);
    let point = at_surface(8);
    assert!(mean_plane_offset(&g, point, UP, 4.0, CELL).is_some());
    // a radius of three cells holds under two hundred cells: the share jumps by a cell's worth with every cell
    assert!(mean_plane_offset(&g, point, UP, 0.75, CELL).is_none());
    assert!(mean_height_over(&g, point, UP, 6.0, 4.0, CELL).is_some());
    // a disc of 0.4 m (about eight cells across a layer) over sixteen layers
    assert!(mean_height_over(&g, point, UP, 0.4, 4.0, CELL).is_none());
}

#[test]
fn ground_thinner_than_half_the_radius_is_no_half_space_and_gets_no_plane() {
    let point = at_surface(8);
    // 1 m of ground under a ball of 4 m (it needs 2 m): a slab
    let slab = ground(8, 12, |_, _, _| false);
    assert!(mean_plane_offset(&slab, point, UP, 4.0, CELL).is_none());
    assert!(mean_height_over(&slab, point, UP, 6.0, 4.0, CELL).is_none());
}

#[test]
fn a_cliff_inside_the_reach_is_no_half_space_either_though_the_ground_is_thick_under_the_point() {
    let point = at_surface(8);
    // the ground stops 2 m to the side of the point, from the surface to the bottom: the ground under the point is thick, and the bottom of the sample is
    // half empty (the share would put the plane 1.5 m too low)
    let cliff = ground(8, 80, |i, _, _| f64::from(i) * CELL > 12.0);
    assert!(mean_plane_offset(&cliff, point, UP, 4.0, CELL).is_none());
    assert!(mean_height_over(&cliff, point, UP, 6.0, 4.0, CELL).is_none());
    // and a cliff outside the reach changes nothing
    let far = ground(8, 80, |i, _, _| f64::from(i) * CELL > 17.0);
    assert!(mean_plane_offset(&far, point, UP, 4.0, CELL).is_some());
    assert!(mean_height_over(&far, point, UP, 6.0, 4.0, CELL).is_some());
}

#[test]
fn a_surface_that_is_more_than_nine_tenths_of_a_radius_over_the_point_is_out_of_the_fit() {
    let g = ground(8, 120, |_, _, _| false);
    // the point is 3.8 m (0.95 of a radius of 4 m) under the surface: the ball is nearly all ground
    let deep = [10.0, 8.0 * CELL + 3.8, 10.0];
    assert!(mean_plane_offset(&g, deep, UP, 4.0, CELL).is_none());
    let shallow = [10.0, 8.0 * CELL + 2.0, 10.0];
    assert!(mean_plane_offset(&g, shallow, UP, 4.0, CELL).is_some());
}

#[test]
fn something_that_stands_in_the_reach_taller_than_half_the_depth_is_not_a_surface_to_fit_the_disc_to() {
    let point = at_surface(8);
    // a pillar 3 m tall, 1 m across, 5 m from the point
    let pillar = ground(8, 80, |_, _, _| false);
    let mut cells: Vec<([i32; 3], u8)> = pillar.cells().map(|c| (c, 1u8)).collect();
    for k in 38..42 {
        for i in 58..62 {
            for j in -4..8 {
                cells.push(([i, j, k], 1));
            }
        }
    }
    let with = Occupancy::from_cells(cells).unwrap();
    assert!(mean_height_over(&pillar, point, UP, 6.0, 4.0, CELL).is_some());
    assert!(mean_height_over(&with, point, UP, 6.0, 4.0, CELL).is_none());
}

#[test]
fn the_disc_of_the_reach_finds_the_mean_of_a_stair_that_the_ball_of_the_crest_sees_as_one_terrace() {
    // a step down of one cell from 4 m to the side of the point: the ball of 3.5 m sees one terrace and the disc of 6 m sees both, in the shares of their areas
    // (the part of a disc of 6 m that is more than 4 m from its centre is 0.1095 of it: the mean is 0.0274 m under the terrace)
    let step = ground(8, 80, |i, j, _| j == 8 && f64::from(i) * CELL > 14.0);
    let point = at_surface(8);
    let ball = mean_plane_offset(&step, point, UP, 3.5, CELL).expect("a plane");
    let disc = mean_height_over(&step, point, UP, 6.0, 4.0, CELL).expect("a plane");
    assert!(ball.abs() < 0.08, "the ball sees one terrace: {ball}");
    assert!((disc + 0.0274).abs() < 0.008, "the disc sees the step in the share of its area: {disc}");
}
