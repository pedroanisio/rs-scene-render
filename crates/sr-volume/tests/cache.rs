use sr_volume::{CacheLimits, SparseGrid, Transform, Volume};

fn example(reverse: bool) -> Volume {
    let mut g = SparseGrid::new(Transform::identity(), 0.0, 4).unwrap();
    let mut points = vec![([-8, 2, 9], 1.5), ([9, 0, -1], 3.0)];
    if reverse {
        points.reverse();
    }
    for (p, v) in points {
        g.set(p, v).unwrap();
    }
    let mut v = Volume::new();
    v.insert("density", g).unwrap();
    v
}

fn encode(v: &Volume) -> Vec<u8> {
    let mut bytes = Vec::new();
    v.write(&mut bytes).unwrap();
    bytes
}

#[test]
fn cache_bytes_are_deterministic_and_roundtrip_fields() {
    let a = encode(&example(false));
    assert_eq!(a, encode(&example(true)));
    let v = Volume::read(a.as_slice(), CacheLimits::default()).unwrap();
    assert_eq!(v.grid("density").unwrap().value([-8, 2, 9]), 1.5);
    assert_eq!(v.grid("density").unwrap().value([9, 0, -1]), 3.0);
    assert_eq!(encode(&v), a);
}

#[test]
fn cache_rejects_truncation_trailing_bytes_version_and_resource_exhaustion() {
    let bytes = encode(&example(false));
    for n in [0, 7, 8, 12, 20, 150, bytes.len() - 1] {
        assert!(Volume::read(&bytes[..n], CacheLimits::default()).is_err(), "truncated at {n}");
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(Volume::read(bad.as_slice(), CacheLimits::default()).is_err());
    bad = bytes.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Volume::read(bad.as_slice(), CacheLimits::default()).is_err());
    for limits in [
        CacheLimits { max_bytes: bytes.len() as u64 - 1, ..Default::default() },
        CacheLimits { max_bricks: 1, ..Default::default() },
        CacheLimits { max_grids: 0, ..Default::default() },
    ] {
        assert!(Volume::read(bytes.as_slice(), limits).is_err());
    }
    let exact = CacheLimits { max_bytes: bytes.len() as u64, max_bricks: 2, max_grids: 1 };
    assert!(Volume::read(bytes.as_slice(), exact).is_ok());
}

#[test]
fn channel_names_are_unique_and_invalid_names_leave_the_volume_unchanged() {
    let mut v = example(false);
    for name in ["density", "", "../density", "not a channel"] {
        assert!(v.insert(name, SparseGrid::new(Transform::identity(), 0.0, 0).unwrap()).is_err());
    }
    assert_eq!(v.grid("density").unwrap().value([-8, 2, 9]), 1.5);
}

// Independently authored wire fixture, not produced by Volume::write.
fn wire_fixture() -> Vec<u8> {
    let mut b = b"SRVOL\0\r\n".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(7u16.to_le_bytes());
    b.extend(b"density");
    b.extend(0f32.to_le_bytes());
    for i in 0..16 {
        b.extend(if i % 5 == 0 { 1f64 } else { 0f64 }.to_le_bytes());
    }
    b.extend(1u32.to_le_bytes());
    for v in [-1i32, 0, 1] {
        b.extend(v.to_le_bytes());
    }
    for i in 0..512 {
        b.extend(if i == 7 { 2f32 } else { 0f32 }.to_le_bytes());
    }
    b
}

#[test]
fn independent_wire_fixture_checks_layout_and_hostile_payloads() {
    let b = wire_fixture();
    let v = Volume::read(b.as_slice(), CacheLimits::default()).unwrap();
    assert_eq!(v.grid("density").unwrap().value([-1, 0, 8]), 2.0);
    assert_eq!(encode(&v), b);
    // Header 16, name 9, background 4, matrix 128, count 4, key 12, values 2048.
    for (offset, bytes) in [
        (0, b"WRONGMAG".to_vec()),
        (12, u32::MAX.to_le_bytes().to_vec()),
        (16, u16::MAX.to_le_bytes().to_vec()),
        (25, f32::NAN.to_le_bytes().to_vec()),
        (29, 0f64.to_le_bytes().to_vec()), // singular transform
        (157, u32::MAX.to_le_bytes().to_vec()),
        (161, i32::MAX.to_le_bytes().to_vec()),
        (173, f32::INFINITY.to_le_bytes().to_vec()),
        (173 + 7 * 4, 0f32.to_le_bytes().to_vec()), // empty brick
    ] {
        let mut bad = b.clone();
        bad[offset..offset + bytes.len()].copy_from_slice(&bytes);
        assert!(Volume::read(bad.as_slice(), CacheLimits::default()).is_err(), "offset {offset}");
    }
    let mut duplicate = b.clone();
    duplicate[157..161].copy_from_slice(&2u32.to_le_bytes());
    duplicate.extend(&b[161..]);
    assert!(Volume::read(duplicate.as_slice(), CacheLimits::default()).is_err());
    let mut duplicate_channel = b.clone();
    duplicate_channel[12..16].copy_from_slice(&2u32.to_le_bytes());
    duplicate_channel.extend(&b[16..]);
    assert!(Volume::read(duplicate_channel.as_slice(), CacheLimits::default()).is_err());
}

#[test]
fn cache_budget_is_shared_across_channels_and_transforms_survive_roundtrip() {
    let mut volume = Volume::new();
    let matrix = glam::DMat4::from_scale_rotation_translation(
        glam::DVec3::new(1.5, 2.5, 3.0),
        glam::DQuat::from_rotation_y(0.5),
        glam::DVec3::new(-8.0, 20.0, 7.0),
    )
    .to_cols_array();
    for name in ["velocity.x", "temperature"] {
        let mut g = SparseGrid::new(Transform::new(matrix).unwrap(), -1.0, 1).unwrap();
        g.set([0, 0, 0], 4.0).unwrap();
        volume.insert(name, g).unwrap();
    }
    let bytes = encode(&volume);
    assert!(Volume::read(bytes.as_slice(), CacheLimits { max_bricks: 1, ..Default::default() }).is_err());
    let decoded = Volume::read(bytes.as_slice(), CacheLimits::default()).unwrap();
    let g = decoded.grid("velocity.x").unwrap();
    assert_eq!(g.transform().columns(), matrix);
    assert_eq!(g.background(), -1.0);
    assert_eq!(g.sample_world([-8.0, 20.0, 7.0]), 4.0);
}

#[test]
fn arbitrary_small_inputs_never_panic_or_allocate_beyond_the_limit() {
    let mut seed = 1u64;
    let limits = CacheLimits { max_bytes: 4096, max_bricks: 1, max_grids: 2 };
    let good = wire_fixture();
    for i in 0..400 {
        let mut bytes = good.clone();
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let index = (seed as usize) % bytes.len();
        bytes[index] ^= ((seed >> 32) as u8).max(1);
        if i % 3 == 0 {
            bytes.truncate(index);
        }
        let _ = Volume::read(bytes.as_slice(), limits);
    }
}
