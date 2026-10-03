use sr_volume::{
    bake::{BakeLimits, BakeWriter, BakedSequence},
    sequence::Interpolation,
    SparseGrid, Transform, Volume,
};

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "sr-bake-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn field(value: f32) -> Volume {
    let mut grid = SparseGrid::new(Transform::identity(), 0., 1).unwrap();
    grid.set([0; 3], value).unwrap();
    let mut v = Volume::new();
    v.insert("density", grid).unwrap();
    v
}

#[test]
fn archive_round_trips_deduplicates_and_seeks_on_composition_time() {
    let dir = Temp::new();
    let root = dir.0.join("take");
    let mut writer = BakeWriter::new(&root, 2., 10., BakeLimits::default()).unwrap();
    writer.push(None).unwrap();
    writer.push(Some(&field(1.))).unwrap();
    writer.push(Some(&field(3.))).unwrap();
    writer.push(Some(&field(3.))).unwrap();
    let receipt = writer.finish().unwrap();
    assert_eq!(receipt.frames, 4);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3, "two unique fields and the manifest");
    let archive = BakedSequence::open(&receipt.manifest, Some(&receipt.sha256), BakeLimits::default()).unwrap();
    for (t, value) in [(2.2, 3.), (2.15, 2.), (2., 0.), (2.1, 1.)] {
        let pair = archive.load(t, Interpolation::Linear, 1 << 20).unwrap();
        assert!((pair.sample("density", [0.; 3]).unwrap() - value).abs() < 1e-10);
    }
    assert!(archive.load(1.9, Interpolation::Hold, 1 << 20).is_err());
    assert!(archive.load(2.4, Interpolation::Hold, 1 << 20).is_err());
    assert!(BakedSequence::open(&receipt.manifest, Some(&"0".repeat(64)), BakeLimits::default()).is_err());
    let frame = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "srvol"))
        .unwrap();
    let mut data = std::fs::read(&frame).unwrap();
    *data.last_mut().unwrap() ^= 1;
    std::fs::write(frame, data).unwrap();
    let errors = [2.1, 2.2].iter().filter(|&&t| archive.load(t, Interpolation::Hold, 1 << 20).is_err()).count();
    assert_eq!(errors, 1, "a structurally valid changed field must fail its digest");
}

#[test]
fn failed_bakes_leave_no_published_archive_and_never_replace_existing_output() {
    let dir = Temp::new();
    let root = dir.0.join("take");
    {
        let mut writer = BakeWriter::new(&root, 0., 10., BakeLimits { max_frames: 1, ..Default::default() }).unwrap();
        writer.push(Some(&field(1.))).unwrap();
        assert!(writer.push(None).is_err());
        assert!(writer.finish().is_err(), "failed append poisons the transaction");
    }
    assert!(!root.exists());
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("keep"), b"original").unwrap();
    assert!(BakeWriter::new(&root, 0., 10., BakeLimits::default()).is_err());
    assert_eq!(std::fs::read(root.join("keep")).unwrap(), b"original");
    let small = dir.0.join("small");
    {
        let mut writer =
            BakeWriter::new(&small, 0., 10., BakeLimits { max_total_bytes: 128, ..Default::default() }).unwrap();
        assert!(writer.push(Some(&field(1.))).is_err());
    }
    assert!(!small.exists());
}

#[test]
fn malformed_manifests_and_frame_limits_fail_before_loading_payloads() {
    let dir = Temp::new();
    let root = dir.0.join("take");
    let mut writer = BakeWriter::new(&root, 0., 10., BakeLimits::default()).unwrap();
    writer.push(Some(&field(1.))).unwrap();
    let receipt = writer.finish().unwrap();
    let good = std::fs::read(&receipt.manifest).unwrap();
    for length in [0, 7, 12, good.len() - 1] {
        std::fs::write(&receipt.manifest, &good[..length]).unwrap();
        assert!(BakedSequence::open(&receipt.manifest, None, BakeLimits::default()).is_err());
    }
    let mut trailing = good.clone();
    trailing.push(0);
    std::fs::write(&receipt.manifest, &trailing).unwrap();
    assert!(BakedSequence::open(&receipt.manifest, None, BakeLimits::default()).is_err());
    std::fs::write(&receipt.manifest, &good).unwrap();
    assert!(BakedSequence::open(&receipt.manifest, None, BakeLimits { max_frames: 0, ..Default::default() }).is_err());
    let mut limits = BakeLimits::default();
    limits.frame.max_bytes = 16;
    assert!(BakedSequence::open(&receipt.manifest, None, limits).is_err());
}

#[test]
fn partial_bakes_select_exact_samples_at_nonzero_start_and_fractional_fps() {
    let dir = Temp::new();
    for (index, fps) in [10., 24000. / 1001.].into_iter().enumerate() {
        let start = 101. / fps;
        let mut writer = BakeWriter::new(&dir.0.join(index.to_string()), start, fps, BakeLimits::default()).unwrap();
        for i in 0..12 {
            writer.push(Some(&field(i as f32 + 1.))).unwrap();
        }
        let receipt = writer.finish().unwrap();
        let bake = BakedSequence::open(&receipt.manifest, Some(&receipt.sha256), BakeLimits::default()).unwrap();
        for i in 0..12 {
            let time = (101 + i) as f64 / fps;
            for interpolation in [Interpolation::Hold, Interpolation::Linear] {
                let pair = bake.load(time, interpolation, 1 << 20).unwrap();
                assert_eq!(pair.sample("density", [0.; 3]).unwrap(), (i + 1) as f64, "fps={fps}, i={i}");
            }
        }
    }
}

#[test]
fn repeated_frame_digest_cannot_declare_conflicting_lengths() {
    let dir = Temp::new();
    let mut writer = BakeWriter::new(&dir.0.join("take"), 0., 10., BakeLimits::default()).unwrap();
    writer.push(Some(&field(1.))).unwrap();
    writer.push(Some(&field(1.))).unwrap();
    let receipt = writer.finish().unwrap();
    let mut bytes = std::fs::read(&receipt.manifest).unwrap();
    let n = bytes.len();
    bytes[n - 8] ^= 1;
    std::fs::write(&receipt.manifest, bytes).unwrap();
    assert!(BakedSequence::open(&receipt.manifest, None, BakeLimits::default()).is_err());
}

#[test]
fn baked_motion_times_use_relative_composition_clock_and_freeze_endpoints() {
    let dir = Temp::new();
    let root = dir.0.join("motion");
    let mut writer = BakeWriter::new(&root, 1_000_000., 4., BakeLimits::default()).unwrap();
    writer.push(Some(&field(1.))).unwrap();
    writer.push(None).unwrap();
    writer.push(Some(&field(2.))).unwrap();
    let receipt = writer.finish().unwrap();
    let archive = BakedSequence::open(&receipt.manifest, Some(&receipt.sha256), BakeLimits::default()).unwrap();
    for (offset, elapsed) in [(0., [0., 0.]), (0.125, [0.125, -0.125]), (0.375, [0.125, -0.125]), (0.625, [0., 0.])] {
        let timed = archive.load_timed(1_000_000. + offset, Interpolation::Linear, 1 << 20).unwrap();
        assert_eq!(timed.elapsed, elapsed);
        if offset == 0.125 {
            assert!(timed.frames.second.is_none());
        }
        if offset == 0.375 {
            assert!(timed.frames.first.is_none());
        }
    }
    let exact = archive.load_timed(1_000_000.5, Interpolation::Linear, 1 << 20).unwrap();
    assert_eq!(exact.elapsed, [0.; 2]);
    assert!(archive.load_timed(1_000_000.75, Interpolation::Linear, 1 << 20).is_err());
}
