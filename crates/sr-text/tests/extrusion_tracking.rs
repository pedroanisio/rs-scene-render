//! Glyph outlines for 3D text carry the tracking of `object3D/@tracking`: extra space in thousandths of an em after each
//! glyph, applied after layout.

use sr_text::{extrusion, FontLib};

fn extent(polys: &[Vec<[f64; 2]>]) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in polys.iter().flatten() {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

#[test]
fn tracking_moves_each_glyph_by_its_index_in_thousandths_of_the_size() {
    let mut lib = FontLib::new(true);
    let plain = extrusion::outline_polygons_tracked(&mut lib, "III", None, 100.0, 0.0, 0.25, usize::MAX).unwrap();
    let open = extrusion::outline_polygons_tracked(&mut lib, "III", None, 100.0, 200.0, 0.25, usize::MAX).unwrap();
    assert!(!plain.is_empty(), "the test needs a font");
    let (a, b) = (extent(&plain), extent(&open));
    // glyph 0 stays, glyph 1 moves 20 px, glyph 2 moves 40 px: the block grows by 40 px on the right only
    assert!((a[0] - b[0]).abs() < 1e-6, "left edge {} {}", a[0], b[0]);
    assert!(((b[2] - a[2]) - 40.0).abs() < 1e-6, "right edge grew by {}", b[2] - a[2]);
    // the height and the baseline are untouched
    assert!((a[1] - b[1]).abs() < 1e-9 && (a[3] - b[3]).abs() < 1e-9);
    // negative tracking tightens; zero is the untracked layout, polygon for polygon
    let zero = extrusion::outline_polygons_tracked(&mut lib, "III", None, 100.0, 0.0, 0.25, usize::MAX).unwrap();
    assert_eq!(plain, zero);
    let tight = extrusion::outline_polygons_tracked(&mut lib, "III", None, 100.0, -100.0, 0.25, usize::MAX).unwrap();
    assert!(((extent(&tight)[2] - a[2]) + 20.0).abs() < 1e-6);
    // the old entry point is the untracked layout
    let old = extrusion::outline_polygons(&mut lib, "III", None, 100.0, 0.25, usize::MAX).unwrap();
    assert_eq!(plain, old);
}
