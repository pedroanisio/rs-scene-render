//! The frame of the cells of an object of cells: the centre of a cell, the cell that holds a point, and the key of the file.
//! The values are worked out by hand from the statement of the frame, and the round trips are checked on every cell of a box.

use sr_3d::occupancy::KEY_LIMIT;
use sr_3d::voxel::{cell_to_object, file_key, object_to_cell};

#[test]
fn the_centre_of_a_cell_is_half_a_cell_in_from_its_lower_corner() {
    assert_eq!(cell_to_object([0, 0, 0], 2.0), [1.0, 1.0, 1.0]);
    assert_eq!(cell_to_object([-1, 3, 5], 0.5), [-0.25, 1.75, 2.75]);
    // the first cell fills [0, 2) in each axis for cells of 2: its centre is 1 and no other cell has the centre 1
    assert_eq!(cell_to_object([1, 0, 0], 2.0)[0], 3.0);
}

#[test]
fn the_cell_that_holds_a_point_has_the_face_between_two_cells_as_its_lower_face() {
    assert_eq!(object_to_cell([0.0, 0.0, 0.0], 1.0), Some([0, 0, 0]));
    assert_eq!(object_to_cell([2.0, -1.0, 3.999], 1.0), Some([2, -1, 3]));
    assert_eq!(object_to_cell([-0.0001, 0.9999, 1.0], 1.0), Some([-1, 0, 1]));
    assert_eq!(object_to_cell([5.0, 5.0, 5.0], 2.5), Some([2, 2, 2]));
    // the y axis points down in the object as in the scene: a point below the corner is in a cell of a positive j
    assert_eq!(object_to_cell([0.5, 10.5, 0.5], 1.0), Some([0, 10, 0]));
}

#[test]
fn a_point_on_a_face_that_the_division_rounds_to_just_below_it_is_still_in_the_cell_above() {
    // 0.3 / 0.1 is 2.9999999999999996 in a double, and the face between the cells 2 and 3 of cells of 0.1 is at 0.3: the point is on it
    assert_eq!(0.3f64 / 0.1, 2.999_999_999_999_999_6);
    assert_eq!(object_to_cell([0.3, 0.3, 0.3], 0.1), Some([3, 3, 3]));
    // every face of a lattice of cells of 0.1, 0.3 and 0.7, reached the way a caller reaches it (the key times the size), is in the cell above it
    for size in [0.1, 0.3, 0.7, 1.0 / 3.0, 2.5] {
        for k in -40..40 {
            let at = f64::from(k) * size;
            assert_eq!(object_to_cell([at, at, at], size), Some([k, k, k]), "face {k} of cells of {size}");
        }
    }
    // far from the origin the same: the rounding of the product grows with the number and so does the tolerance
    for k in [1_000_000, 123_456_789, -987_654_321] {
        let at = f64::from(k) * 0.1;
        assert_eq!(object_to_cell([at, 0.0, 0.0], 0.1).map(|c| c[0]), Some(k), "{k}");
    }
    // a point that is really below the face is below it: a nanometre of a cell of a metre, far over the tolerance (a few units in the last place)
    assert_eq!(object_to_cell([0.3 - 1e-9, 0.0, 0.0], 0.1), Some([2, 0, 0]));
    assert_eq!(object_to_cell([3.0 - 1e-9, 0.0, 0.0], 1.0), Some([2, 0, 0]));
    // and in the middle of a cell, nothing changed
    assert_eq!(object_to_cell([0.25, 0.0, 0.0], 0.1), Some([2, 0, 0]));
}

#[test]
fn a_point_that_is_nowhere_or_a_size_that_is_nothing_has_no_cell() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(object_to_cell([bad, 0.0, 0.0], 1.0), None);
    }
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(object_to_cell([0.0; 3], size), None, "{size}");
    }
    // the last key of an occupancy is in, the one after it is not (and neither is the first key before it)
    assert_eq!(object_to_cell([f64::from(KEY_LIMIT - 1) + 0.5, 0.0, 0.0], 1.0), Some([KEY_LIMIT - 1, 0, 0]));
    assert_eq!(object_to_cell([f64::from(KEY_LIMIT) + 0.5, 0.0, 0.0], 1.0), None);
    assert_eq!(object_to_cell([-f64::from(KEY_LIMIT), 0.0, 0.0], 1.0), Some([-KEY_LIMIT, 0, 0]));
    assert_eq!(object_to_cell([-f64::from(KEY_LIMIT) - 0.5, 0.0, 0.0], 1.0), None);
}

#[test]
fn every_cell_of_a_box_is_the_cell_of_its_own_centre_for_any_size() {
    for size in [1.0, 0.25, 3.5, 100.0, 1e-3] {
        for k in 0..6 {
            for j in -3..3 {
                for i in 0..5 {
                    let key = [i, j, k];
                    assert_eq!(object_to_cell(cell_to_object(key, size), size), Some(key), "size {size}");
                }
            }
        }
    }
}

#[test]
fn the_key_of_the_file_is_the_key_of_the_model_and_where_the_box_was() {
    assert_eq!(file_key([0, 0, 0], [5, -7, 100]), [5, -7, 100]);
    assert_eq!(file_key([3, 2, 1], [-5, 7, i64::from(KEY_LIMIT)]), [-2, 9, i64::from(KEY_LIMIT) + 1]);
}
