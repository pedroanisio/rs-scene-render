use sr_3d::fracture::{fracture, Piece, Spec};
use std::collections::BTreeMap;

fn cube() -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let p = sr_3d::prim::cuboid(2., 2., 2.);
    (
        p.vertices.iter().map(|v| v.pos.map(f64::from)).collect(),
        p.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
    )
}
fn closed(piece: &Piece) {
    let mut edges = BTreeMap::<[u32; 2], (usize, i32)>::new();
    for face in &piece.faces {
        for i in 0..3 {
            let a = face.indices[i];
            let b = face.indices[(i + 1) % 3];
            let (key, sign) = if a < b { ([a, b], 1) } else { ([b, a], -1) };
            let e = edges.entry(key).or_default();
            e.0 += 1;
            e.1 += sign;
        }
    }
    assert!(edges.values().all(|v| *v == (2, 0)), "open or inconsistently oriented surface");
}
#[test]
fn partitions_a_solid_without_inventing_volume_and_preserves_mass_and_centroid() {
    let (v, t) = cube();
    let pieces = fracture(&v, &t, Spec { pieces: 13, seed: 47, mass: 19., ..Default::default() }).unwrap();
    assert_eq!(pieces.len(), 13);
    assert!((pieces.iter().map(|p| p.volume).sum::<f64>() - 8.).abs() < 1e-9);
    assert!((pieces.iter().map(|p| p.mass).sum::<f64>() - 19.).abs() < 1e-12);
    for axis in 0..3 {
        assert!(pieces.iter().map(|p| p.center[axis] * p.mass).sum::<f64>().abs() < 1e-9);
    }
    for p in &pieces {
        closed(p);
        assert!(p.mass > 0. && p.volume > 0.);
        assert!(p.faces.iter().any(|f| f.source.is_none()), "cut surfaces need explicit interior provenance");
        for f in &p.faces {
            if let Some(source) = f.source {
                assert!(source < t.len());
            }
        }
        for v in &p.vertices {
            for (coordinate, center) in v.iter().zip(p.center) {
                assert!((coordinate + center).abs() <= 1. + 1e-10);
            }
        }
    }
    assert_eq!(pieces, fracture(&v, &t, Spec { pieces: 13, seed: 47, mass: 19., ..Default::default() }).unwrap());
    assert_ne!(pieces, fracture(&v, &t, Spec { pieces: 13, seed: 48, mass: 19., ..Default::default() }).unwrap());
}

// A 3x3x1 solid with the central cell removed: concave, with a through hole.
fn ring() -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let faces = [
        ([0, -1, 0], [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]]),
        ([0, 1, 0], [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]]),
        ([-1, 0, 0], [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]]),
        ([1, 0, 0], [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]]),
        ([0, 0, -1], [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]]),
        ([0, 0, 1], [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]),
    ];
    let occupied = |x: i32, y: i32, z: i32| (0..3).contains(&x) && (0..3).contains(&y) && z == 0 && (x != 1 || y != 1);
    for x in 0..3 {
        for y in 0..3 {
            if !occupied(x, y, 0) {
                continue;
            }
            for (dir, quad) in faces {
                if occupied(x + dir[0], y + dir[1], dir[2]) {
                    continue;
                }
                let base = vertices.len() as u32;
                for q in quad {
                    vertices.push([(x + q[0]) as f64, (y + q[1]) as f64, q[2] as f64]);
                }
                triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            }
        }
    }
    (vertices, triangles)
}
fn inside(p: &Piece, point: [f64; 3]) -> bool {
    use glam::DVec3;
    let x = DVec3::from(point) - DVec3::from(p.center);
    let mut angle = 0.;
    for f in &p.faces {
        let [a, b, c] = f.indices.map(|i| DVec3::from(p.vertices[i as usize]) - x);
        angle += 2.
            * a.dot(b.cross(c)).atan2(
                a.length() * b.length() * c.length()
                    + a.dot(b) * c.length()
                    + b.dot(c) * a.length()
                    + c.dot(a) * b.length(),
            );
    }
    angle.abs() > std::f64::consts::TAU
}
#[test]
fn concave_fragments_keep_the_hole_and_partition_sampled_occupancy() {
    let (v, t) = ring();
    for seed in [0, 42, 1234] {
        let pieces = fracture(&v, &t, Spec { pieces: 9, seed, ..Default::default() }).unwrap();
        assert!((pieces.iter().map(|p| p.volume).sum::<f64>() - 8.).abs() < 1e-8);
        for p in &pieces {
            closed(p);
        }
        for x in 0..12 {
            for y in 0..12 {
                for z in 0..3 {
                    let point = [(x as f64 + 0.371) / 4., (y as f64 + 0.183) / 4., (z as f64 + 0.237) / 3.];
                    let expected = usize::from(!(1.0..2.0).contains(&point[0]) || !(1.0..2.0).contains(&point[1]));
                    assert_eq!(
                        pieces.iter().filter(|p| inside(p, point)).count(),
                        expected,
                        "seed {seed}, point {point:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn rejects_open_nonmanifold_nonfinite_and_over_budget_inputs() {
    let (mut v, t) = cube();
    assert!(fracture(&v, &t[..t.len() - 1], Spec::default()).is_err());
    let mut duplicate = t.clone();
    duplicate.push(t[0]);
    assert!(fracture(&v, &duplicate, Spec::default()).is_err());
    for spec in [
        Spec { pieces: 0, ..Default::default() },
        Spec { pieces: 4097, ..Default::default() },
        Spec { mass: f64::NAN, ..Default::default() },
        Spec { max_bytes: 32, ..Default::default() },
        Spec { max_work: 1, ..Default::default() },
    ] {
        assert!(fracture(&v, &t, spec).is_err());
    }
    v[0][0] = f64::INFINITY;
    assert!(fracture(&v, &t, Spec::default()).is_err());
}

#[test]
fn source_overlaps_are_rejected_instead_of_counting_the_same_material_twice() {
    let (mut v, mut t) = cube();
    let offset = v.len() as u32;
    let other = v.iter().map(|p| [p[0] + 0.5, p[1] + 0.3, p[2] + 0.2]).collect::<Vec<_>>();
    let extra = t.iter().map(|f| f.map(|i| i + offset)).collect::<Vec<_>>();
    v.extend(other);
    t.extend(extra);
    assert!(
        fracture(&v, &t, Spec { pieces: 1, ..Default::default() }).is_err(),
        "overlapping closed shells are not one valid source solid"
    );
}

#[test]
fn normalized_geometry_preserves_scaled_translated_and_reversed_solids() {
    let (v, mut t) = cube();
    for tri in &mut t {
        tri.swap(1, 2);
    }
    for (scale, shift) in [(1e-6, 0.), (1e6, 1e9)] {
        let input: Vec<_> = v.iter().map(|p| p.map(|v| v * scale + shift)).collect();
        let pieces = fracture(&input, &t, Spec { pieces: 5, ..Default::default() }).unwrap();
        assert!((pieces.iter().map(|p| p.volume).sum::<f64>() / (8. * scale.powi(3)) - 1.).abs() < 1e-9);
        for p in &pieces {
            closed(p);
            assert!(p.vertices.iter().flatten().all(|v| v.is_finite()));
        }
        for axis in 0..3 {
            assert!((pieces.iter().map(|p| p.center[axis] * p.mass).sum::<f64>() - shift).abs() < scale * 1e-8);
        }
    }
}

#[test]
fn nested_shells_require_cavity_orientation_and_keep_empty_interiors() {
    let (v, t) = cube();
    let mut vertices = v.clone();
    let offset = v.len() as u32;
    vertices.extend(v.iter().map(|p| p.map(|x| x * 0.5)));
    let mut triangles = t.clone();
    triangles.extend(t.iter().map(|f| f.map(|i| i + offset)));
    assert!(
        fracture(&vertices, &triangles, Spec { pieces: 1, ..Default::default() }).is_err(),
        "nested outward shells count material twice"
    );
    for tri in &mut triangles[t.len()..] {
        tri.swap(1, 2);
    }
    let pieces = fracture(&vertices, &triangles, Spec { pieces: 7, ..Default::default() }).unwrap();
    assert!((pieces.iter().map(|p| p.volume).sum::<f64>() - 7.).abs() < 1e-9);
    for p in &pieces {
        closed(p);
        assert!(!inside(p, [0.017, 0.029, 0.041]));
    }
}

#[test]
fn disconnected_material_is_not_joined_into_one_rigid_fragment() {
    let (mut v, mut t) = cube();
    let offset = v.len() as u32;
    let other = v.iter().map(|p| [p[0] + 5., p[1], p[2]]).collect::<Vec<_>>();
    let extra = t.iter().map(|f| f.map(|i| i + offset)).collect::<Vec<_>>();
    v.extend(other);
    t.extend(extra);
    assert!(fracture(&v, &t, Spec { pieces: 1, ..Default::default() }).is_err());
    let pieces = fracture(&v, &t, Spec { pieces: 2, mass: 40., ..Default::default() }).unwrap();
    assert_eq!(pieces.len(), 2);
    for p in &pieces {
        closed(p);
        assert!((p.mass - 20.).abs() < 1e-10);
        assert!((p.volume - 8.).abs() < 1e-10);
    }
    let mut centers = pieces.iter().map(|p| p.center[0]).collect::<Vec<_>>();
    centers.sort_by(f64::total_cmp);
    assert!(centers[0].abs() < 1e-10 && (centers[1] - 5.).abs() < 1e-10);
}

#[test]
fn many_fragments_remain_closed_across_seeds() {
    let (v, t) = cube();
    for seed in 0..12 {
        let pieces = fracture(&v, &t, Spec { pieces: 64, seed, ..Default::default() })
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(pieces.len(), 64);
        for p in &pieces {
            closed(p);
        }
        assert!((pieces.iter().map(|p| p.volume).sum::<f64>() - 8.).abs() < 1e-9);
    }
}

#[test]
fn fragments_never_publish_underflowed_zero_volume() {
    let (mut v, t) = cube();
    for p in &mut v {
        for x in p {
            *x *= 1e-108;
        }
    }
    let result = fracture(&v, &t, Spec { pieces: 8, ..Default::default() });
    assert!(result.is_err(), "world-space fragment volume cannot be represented: {result:?}");
}
