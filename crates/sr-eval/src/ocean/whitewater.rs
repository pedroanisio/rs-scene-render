use sr_3d::{Primitive, Vertex};
use sr_sim::ocean::whitewater::{Frame, Kind};

/// Two batches rather than a separate draw and prototype for every tracer. The foam batch is empty when `foam` is false:
/// the foam is then the coverage of the surface, not triangles.
pub(super) fn meshes(frame: &Frame, available: usize, foam: bool) -> Result<[Primitive; 2], String> {
    // Eight-sided foam disc: 9 vertices, 24 indices. Spray octahedron:
    // 6 vertices, 24 indices. Include normal/tangent workspace and upload copy.
    let bytes = frame.particles.len().checked_mul(9 * 256 + 24 * 8).ok_or("whitewater surface memory overflow")?;
    if bytes > available || frame.particles.len() > u32::MAX as usize / 9 {
        return Err("whitewater surface exceeds ocean surface memory budget".into());
    }
    let mut meshes = [Primitive::default(), Primitive::default()];
    for (i, mesh) in meshes.iter_mut().enumerate() {
        let count = frame.particles.iter().filter(|p| usize::from(p.kind == Kind::Spray) == i).count();
        mesh.vertices.reserve_exact(count * if i == 0 { 9 } else { 6 });
        mesh.indices.reserve_exact(count * 24);
    }
    for p in frame.particles.iter().filter(|p| foam || p.kind == Kind::Spray) {
        let mesh = &mut meshes[usize::from(p.kind == Kind::Spray)];
        let radius = p.radius * (1. - (frame.time - p.birth) / p.lifetime).clamp(0., 1.);
        let base = mesh.vertices.len() as u32;
        let mut vertex = |offset: [f64; 3], uv: [f32; 2]| -> Result<(), String> {
            let pos = std::array::from_fn(|a| (p.position[a] + offset[a] * radius) as f32);
            if pos.iter().any(|v| !v.is_finite()) {
                return Err("whitewater surface exceeds device precision".into());
            }
            mesh.vertices.push(Vertex { pos, uv, map_uv: [uv; 4], ..Default::default() });
            Ok(())
        };
        if p.kind == Kind::Foam {
            vertex([0.; 3], [0.5; 2])?;
            for i in 0..8 {
                let a = std::f64::consts::TAU * i as f64 / 8.;
                vertex([a.cos(), 0., a.sin()], [(a.cos() * 0.5 + 0.5) as f32, (a.sin() * 0.5 + 0.5) as f32])?;
            }
            for i in 0..8 {
                mesh.indices.extend_from_slice(&[base, base + 1 + i, base + 1 + (i + 1) % 8]);
            }
        } else {
            for v in [[1., 0., 0.], [-1., 0., 0.], [0., 1., 0.], [0., -1., 0.], [0., 0., 1.], [0., 0., -1.]] {
                vertex(v, [0.5; 2])?;
            }
            for i in [0, 2, 4, 4, 2, 1, 1, 2, 5, 5, 2, 0, 0, 4, 3, 4, 1, 3, 1, 5, 3, 5, 0, 3] {
                mesh.indices.push(base + i);
            }
        }
    }
    for mesh in &mut meshes {
        sr_3d::compute_normals(&mut mesh.vertices, &mesh.indices);
        sr_3d::compute_tangents(&mut mesh.vertices, &mesh.indices);
    }
    Ok(meshes)
}
