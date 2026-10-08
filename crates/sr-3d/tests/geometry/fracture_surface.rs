use sr_3d::{
    fracture::{self, Spec, SurfaceMaterial},
    prim,
};

#[test]
fn cut_surfaces_keep_exterior_uvs_colors_and_distinct_interior_materials() {
    let mut source = prim::cuboid(2., 2., 2.);
    // Independently recoverable affine attributes on every exterior triangle.
    for v in &mut source.vertices {
        v.uv = [v.pos[0] * 0.25 + 0.5, v.pos[1] * 0.25 + 0.5];
        v.map_uv = [[v.pos[1] * 0.1, v.pos[2] * 0.1]; 4];
        v.color = [v.pos[0] * 0.2 + 0.5, v.pos[1] * 0.2 + 0.5, v.pos[2] * 0.2 + 0.5, 0.7];
    }
    source.material = Some(7);
    source.tex_coords.insert(2, source.vertices.iter().map(|v| [v.pos[2] * 0.3, v.pos[0] * 0.3]).collect());
    let vertices: Vec<_> = source.vertices.iter().map(|v| v.pos.map(f64::from)).collect();
    let triangles = source.indices.as_chunks::<3>().0.to_vec();
    let pieces = fracture::fracture(&vertices, &triangles, Spec { pieces: 5, seed: 23, ..Default::default() }).unwrap();
    for piece in &pieces {
        let surfaces = fracture::surface(piece, std::slice::from_ref(&source), 0.25, 1 << 20).unwrap();
        assert!(surfaces.iter().any(|s| s.material == SurfaceMaterial::Interior));
        assert!(surfaces.iter().any(|s| s.material == SurfaceMaterial::Exterior(0)));
        assert_eq!(surfaces.iter().map(|s| s.mesh.indices.len()).sum::<usize>(), piece.faces.len() * 3);
        for s in &surfaces {
            if s.material == SurfaceMaterial::Exterior(0) {
                assert_eq!(s.mesh.material, Some(7));
            }
            for (i, v) in s.mesh.vertices.iter().enumerate() {
                let p = std::array::from_fn::<_, 3, _>(|i| v.pos[i] as f64 + piece.center[i]);
                if s.material == SurfaceMaterial::Exterior(0) {
                    near(v.uv[0] as f64, p[0] * 0.25 + 0.5);
                    near(v.uv[1] as f64, p[1] * 0.25 + 0.5);
                    near(v.color[0] as f64, p[0] * 0.2 + 0.5);
                    near(v.color[3] as f64, 0.7);
                    for uv in v.map_uv {
                        near(uv[0] as f64, p[1] * 0.1);
                        near(uv[1] as f64, p[2] * 0.1);
                    }
                    let uv = s.mesh.tex_coords[&2][i];
                    near(uv[0] as f64, p[2] * 0.3);
                    near(uv[1] as f64, p[0] * 0.3);
                } else {
                    assert_eq!(v.color, [1.; 4]);
                }
                let n = glam::Vec3::from(v.normal);
                let t = glam::Vec3::from_slice(&v.tangent);
                near(n.length() as f64, 1.);
                near(t.length() as f64, 1.);
                near(n.dot(t) as f64, 0.);
            }
        }
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn provenance_splits_exterior_batches_without_merging_material_seams() {
    let cube = prim::cuboid(2., 2., 2.);
    let mut a = cube.clone();
    let mut b = cube.clone();
    a.indices = cube.indices[..18].to_vec();
    b.indices = cube.indices[18..].to_vec();
    a.material = Some(4);
    b.material = Some(8);
    let sources = [a, b];
    let v: Vec<_> = sources.iter().flat_map(|p| p.vertices.iter().map(|v| v.pos.map(f64::from))).collect();
    let t: Vec<_> = sources
        .iter()
        .enumerate()
        .flat_map(|(k, p)| p.indices.as_chunks::<3>().0.iter().map(move |t| t.map(|i| i + k as u32 * 24)))
        .collect();
    let piece = fracture::fracture(&v, &t, Spec { pieces: 1, ..Default::default() }).unwrap().remove(0);
    let surfaces = fracture::surface(&piece, &sources, 1., 1 << 20).unwrap();
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0].material, SurfaceMaterial::Exterior(0));
    assert_eq!(surfaces[1].material, SurfaceMaterial::Exterior(1));
    assert_eq!(surfaces[0].mesh.material, Some(4));
    assert_eq!(surfaces[1].mesh.material, Some(8));
}

#[test]
fn reconstruction_rejects_broken_provenance_attributes_and_budget() {
    let source = prim::cuboid(2., 2., 2.);
    let v: Vec<_> = source.vertices.iter().map(|v| v.pos.map(f64::from)).collect();
    let t = source.indices.as_chunks::<3>().0;
    let piece = fracture::fracture(&v, t, Spec { pieces: 1, ..Default::default() }).unwrap().remove(0);
    for (uv, budget) in [(0., 1 << 20), (f64::INFINITY, 1 << 20), (1., 1)] {
        assert!(fracture::surface(&piece, std::slice::from_ref(&source), uv, budget).is_err());
    }
    let mut invalid = piece.clone();
    invalid.faces[0].source = Some(usize::MAX);
    assert!(fracture::surface(&invalid, std::slice::from_ref(&source), 1., 1 << 20).is_err());
    let mut invalid = source.clone();
    invalid.vertices[0].uv[0] = f32::NAN;
    assert!(fracture::surface(&piece, &[invalid], 1., 1 << 20).is_err());
    let mut invalid = source.clone();
    invalid.tex_coords.insert(2, vec![[0., 0.]]);
    assert!(fracture::surface(&piece, &[invalid], 1., 1 << 20).is_err());
}

#[test]
fn opposite_cut_faces_share_uvs_and_keep_outward_shading_frames() {
    let source = prim::cuboid(2., 2., 2.);
    let v: Vec<_> = source.vertices.iter().map(|v| v.pos.map(f64::from)).collect();
    let t = source.indices.as_chunks::<3>().0;
    let pieces = fracture::fracture(&v, t, Spec { pieces: 2, seed: 11, ..Default::default() }).unwrap();
    let surfaces: Vec<_> =
        pieces.iter().map(|p| fracture::surface(p, std::slice::from_ref(&source), 0.3, 1 << 20).unwrap()).collect();
    let cap = |i: usize| surfaces[i].iter().find(|s| s.material == SurfaceMaterial::Interior).unwrap();
    let normal = glam::Vec3::from(cap(0).mesh.vertices[0].normal);
    assert!(normal.dot(glam::Vec3::from(cap(1).mesh.vertices[0].normal)) < -0.99999);
    for (i, piece) in pieces.iter().enumerate() {
        let mesh = &cap(i).mesh;
        for indices in mesh.indices.as_chunks::<3>().0 {
            let verts = indices.map(|k| mesh.vertices[k as usize]);
            let points = verts.map(|v| glam::Vec3::from(v.pos));
            let face = (points[1] - points[0]).cross(points[2] - points[0]).normalize();
            assert!(face.dot(glam::Vec3::from(verts[0].normal)) > 0.99999);
        }
        for v in &mesh.vertices {
            let global = glam::DVec3::from(v.pos.map(f64::from)) + glam::DVec3::from(piece.center);
            let other = cap(1 - i)
                .mesh
                .vertices
                .iter()
                .find(|w| {
                    let q = glam::DVec3::from(w.pos.map(f64::from)) + glam::DVec3::from(pieces[1 - i].center);
                    (global - q).length() < 1e-6
                })
                .expect("shared cap point");
            near(v.uv[0] as f64, other.uv[0] as f64);
            near(v.uv[1] as f64, other.uv[1] as f64);
            near(v.tangent[3] as f64, -(other.tangent[3] as f64));
        }
    }
}
