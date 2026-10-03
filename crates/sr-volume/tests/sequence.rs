use std::sync::Arc;

use sr_volume::sequence::{Interpolation, MissingFrame, Sequence};
use sr_volume::{Error, SparseGrid, Transform, Volume};

fn frame(value: f32) -> Arc<Volume> {
    let mut frame = Volume::new();
    frame.insert("density", SparseGrid::new(Transform::identity(), value, 0).unwrap()).unwrap();
    Arc::new(frame)
}

#[test]
fn sequence_time_clamps_at_endpoints_and_uses_fractional_frames() {
    let seq = Sequence::new(-2, 2, 24.0, Interpolation::Linear, MissingFrame::Error).unwrap();
    for (time, frames, blend) in [
        (-1.0, [-2, -2], 0.0),
        (0.0, [-2, -2], 0.0),
        (0.5 / 24.0, [-2, -1], 0.5),
        (2.0 / 24.0, [0, 0], 0.0),
        (1.0, [2, 2], 0.0),
    ] {
        let selection = seq.position(time).unwrap();
        assert_eq!([selection.first, selection.second], frames);
        assert!((selection.blend - blend).abs() < 1e-12);
    }
    let hold = Sequence::new(100, 104, 24.0, Interpolation::Hold, MissingFrame::Error).unwrap();
    let p = hold.position(1.75 / 24.0).unwrap();
    assert_eq!([p.first, p.second], [101, 101]);
    assert_eq!(p.blend, 0.0);
}

#[test]
fn missing_hold_is_a_predecessor_search_independent_of_seek_history() {
    let seq = Sequence::new(1, 5, 1.0, Interpolation::Linear, MissingFrame::Hold).unwrap();
    let loader = |n| {
        Ok(match n {
            1 | 4 => Some(frame(n as f32)),
            _ => None,
        })
    };
    for time in [2.5, 0.0, 4.0, 2.5, 1.0] {
        let pair = seq.load(time, 4096, loader).unwrap();
        let (a, b) = if time >= 3.0 {
            (4.0, 4.0)
        } else if time > 2.0 {
            (1.0, 4.0)
        } else {
            (1.0, 1.0)
        };
        assert_eq!(pair.first.unwrap().grid("density").unwrap().background(), a);
        assert_eq!(pair.second.unwrap().grid("density").unwrap().background(), b);
    }
    assert!(seq.load(0.0, 4096, |_| Ok(None)).is_err(), "hold must not invent a future frame");
}

#[test]
fn transparent_missing_frames_and_corrupt_frames_are_distinct() {
    let seq = Sequence::new(0, 1, 1.0, Interpolation::Linear, MissingFrame::Transparent).unwrap();
    let pair = seq.load(0.5, 4096, |n| Ok((n == 0).then(|| frame(1.0)))).unwrap();
    assert!(pair.first.is_some() && pair.second.is_none());
    assert_eq!(pair.blend, 0.5);
    for missing in [MissingFrame::Hold, MissingFrame::Transparent, MissingFrame::Error] {
        let seq = Sequence::new(0, 1, 1.0, Interpolation::Linear, missing).unwrap();
        assert!(matches!(seq.load(0.0, 4096, |_| Err(Error::Invalid("corrupt"))), Err(Error::Invalid("corrupt"))));
    }
    let strict = Sequence::new(0, 1, 1.0, Interpolation::Linear, MissingFrame::Error).unwrap();
    assert!(strict.load(0.5, 4096, |_| Ok(None)).is_err());
}

#[test]
fn frame_pair_loading_is_bounded_and_deduplicates_references() {
    let seq = Sequence::new(0, 1, 1.0, Interpolation::Linear, MissingFrame::Error).unwrap();
    let mut calls = Vec::new();
    let pair = seq
        .load(0.0, 4096, |n| {
            calls.push(n);
            Ok(Some(frame(1.0)))
        })
        .unwrap();
    assert_eq!(calls, [0]);
    assert!(Arc::ptr_eq(pair.first.as_ref().unwrap(), pair.second.as_ref().unwrap()));
    assert!(seq.load(0.0, 1, |_| Ok(Some(frame(1.0)))).is_err());
    let one = frame(1.0);
    let size = one.bytes();
    assert!(seq.load(0.5, size, |_| Ok(Some(one.clone()))).is_ok());
    assert!(seq.load(0.5, size, |_| Ok(Some(frame(1.0)))).is_err());
}

#[test]
fn sequence_rejects_unbounded_ranges_and_invalid_time_before_loading() {
    for (first, last, fps) in
        [(2, 1, 1.0), (0, 1_000_000, 1.0), (i64::MIN, i64::MAX, 1.0), (0, 1, 0.0), (0, 1, -1.0), (0, 1, f64::INFINITY)]
    {
        assert!(Sequence::new(first, last, fps, Interpolation::Hold, MissingFrame::Error).is_err());
    }
    // Large integral frame labels must not lose precision through a conversion to f64.
    let large = Sequence::new(i64::MAX - 1, i64::MAX, 1.0, Interpolation::Linear, MissingFrame::Error).unwrap();
    assert_eq!(large.position(0.5).unwrap().first, i64::MAX - 1);
    assert_eq!(large.position(0.5).unwrap().second, i64::MAX);
    for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(large.load(t, 4096, |_| panic!("invalid time must not read files")).is_err());
    }
}

#[test]
fn sequence_blends_fields_in_world_space_with_different_transforms() {
    let mut a = SparseGrid::new(Transform::identity(), 0.0, 1).unwrap();
    a.set([0, 0, 0], 2.0).unwrap();
    let transform =
        Transform::new(glam::DMat4::from_translation(glam::DVec3::new(1.0, 0.0, 0.0)).to_cols_array()).unwrap();
    let mut b = SparseGrid::new(transform, 0.0, 1).unwrap();
    b.set([0, 0, 0], 8.0).unwrap();
    let mut first = Volume::new();
    first.insert("density", a).unwrap();
    let mut second = Volume::new();
    second.insert("density", b).unwrap();
    let frames = [Arc::new(first), Arc::new(second)];
    let seq = Sequence::new(0, 1, 1.0, Interpolation::Linear, MissingFrame::Error).unwrap();
    let pair = seq.load(0.25, 16384, |n| Ok(Some(frames[n as usize].clone()))).unwrap();
    assert_eq!(pair.sample("density", [0.0; 3]).unwrap(), 1.5);
    assert_eq!(pair.sample("density", [1.0, 0.0, 0.0]).unwrap(), 2.0);
    assert_eq!(pair.sample("density", [0.5, 0.0, 0.0]).unwrap(), 1.75);
    assert!(pair.sample("missing", [0.0; 3]).is_err());
    assert!(pair.sample("density", [f64::NAN; 3]).is_err());
    let seq = Sequence::new(0, 1, 1.0, Interpolation::Linear, MissingFrame::Transparent).unwrap();
    let pair = seq.load(0.25, 16384, |n| Ok((n == 0).then(|| frames[0].clone()))).unwrap();
    assert_eq!(pair.sample("density", [0.0; 3]).unwrap(), 1.5);
}

#[test]
fn medium_interpolates_density_and_temperature_before_nonlinear_transport() {
    use sr_volume::medium::{Bounds, Medium, Optical};
    let grid = |v| Arc::new(SparseGrid::new(Transform::identity(), v, 0).unwrap());
    let first = Medium::new(
        grid(1.0),
        Some(Bounds::new([0.0; 3], [2.0; 3]).unwrap()),
        Transform::identity(),
        Optical::default(),
    )
    .unwrap()
    .with_temperature(grid(2000.0), 1.0, 0.5)
    .unwrap();
    let mixed = first.clone().with_next_frame(grid(3.0), Some(grid(8000.0)), 0.5).unwrap();
    assert_eq!(mixed.sample_density([1.0; 3]), 2.0);
    assert_eq!(mixed.sample_density([3.0; 3]), 0.0);
    let expected = sr_volume::thermal::blackbody_rgb(5000.0).unwrap().map(|v| v * 0.5);
    assert_eq!(mixed.sample_emission([1.0; 3]), expected);
    for fraction in [-0.1, 1.1, f64::NAN] {
        assert!(first.clone().with_next_frame(grid(3.0), Some(grid(8000.0)), fraction).is_err());
    }
    assert!(first.clone().with_next_frame(grid(-1.0), Some(grid(8000.0)), 0.5).is_err());
    assert!(first.clone().with_next_frame(grid(1.0), None, 0.5).is_err());
    assert!(first.with_next_frame(grid(1.0), Some(grid(-10.0)), 0.5).is_err());
}

#[test]
fn interpolated_domains_cover_both_transformed_sparse_supports() {
    use sr_volume::medium::{Medium, Optical};
    let mut first = SparseGrid::new(Transform::identity(), 0.0, 1).unwrap();
    first.set([0, 0, 0], 2.0).unwrap();
    let matrix = glam::DMat4::from_translation(glam::DVec3::new(10.0, 0.0, 0.0));
    let mut next = SparseGrid::new(Transform::new(matrix.to_cols_array()).unwrap(), 0.0, 1).unwrap();
    next.set([0, 0, 0], 6.0).unwrap();
    let first = Medium::new(Arc::new(first), None, Transform::identity(), Optical::default()).unwrap();
    let mixed = first.with_next_frame(Arc::new(next), None, 0.25).unwrap();
    assert_eq!(mixed.bounds().unwrap().min(), [-1.0; 3]);
    assert_eq!(mixed.bounds().unwrap().max(), [11.0, 1.0, 1.0]);
    assert_eq!(mixed.sample_density([0.0; 3]), 1.5);
    assert_eq!(mixed.sample_density([10.0, 0.0, 0.0]), 1.5);
    assert_eq!(mixed.sample_density([5.0, 0.0, 0.0]), 0.0);
}

#[test]
fn timed_frames_preserve_actual_held_labels_without_large_label_precision_loss() {
    for first in [0, i64::MAX - 4, i64::MIN] {
        let seq = Sequence::new(first, first + 4, 2., Interpolation::Linear, MissingFrame::Hold).unwrap();
        let mut calls = Vec::new();
        let timed = seq
            .load_timed(1.25, 4096, |label| {
                calls.push(label - first);
                Ok(([0, 3].contains(&(label - first))).then(|| frame(1.)))
            })
            .unwrap();
        assert_eq!(timed.elapsed, [1.25, -0.25]);
        assert_eq!(timed.frames.blend, 0.5);
        assert_eq!(calls, [2, 1, 0, 3]);
        let held = seq.load_timed(0.75, 4096, |label| Ok((label == first).then(|| frame(1.)))).unwrap();
        // A missing pair resolving to the same source freezes instead of extrapolating forever.
        assert_eq!(held.elapsed, [0.; 2]);
        let end = seq.load_timed(100., 4096, |_| Ok(Some(frame(1.)))).unwrap();
        assert_eq!(end.elapsed, [0.; 2]);
    }
}

#[test]
fn timed_transparent_and_hold_selection_have_explicit_motion_intervals() {
    let seq = Sequence::new(10, 11, 2., Interpolation::Linear, MissingFrame::Transparent).unwrap();
    let pair = seq.load_timed(0.125, 4096, |n| Ok((n == 10).then(|| frame(1.)))).unwrap();
    assert_eq!(pair.elapsed, [0.125, -0.375]);
    assert!(pair.frames.second.is_none());
    let hold = Sequence::new(10, 11, 2., Interpolation::Hold, MissingFrame::Error).unwrap();
    assert_eq!(hold.load_timed(0.125, 4096, |_| Ok(Some(frame(1.)))).unwrap().elapsed, [0.; 2]);
}
