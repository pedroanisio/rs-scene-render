use sr_3d::{Primitive, Vertex};
use sr_sim::ocean::whitewater::{Frame, Kind};

/// Two batches rather than a separate draw and prototype for every tracer. The foam batch is empty when `foam` is false:
/// the foam is then the coverage of the surface, not triangles.
pub(super) fn meshes(frame: &Frame, available: usize, foam: bool) -> Result<[Primitive; 2], String> {
    // Eight-sided foam disc: 9 vertices, 24 indices. Spray octahedron:
    // 6 vertices, 24 indices. Include normal/tangent workspace and upload copy.
    // only the tracers that are drawn are charged and given room
    let drawn = |p: &&sr_sim::ocean::whitewater::Particle| foam || p.kind == Kind::Spray;
    let total = frame.particles.iter().filter(drawn).count();
    let bytes = total.checked_mul(9 * 256 + 24 * 8).ok_or("whitewater surface memory overflow")?;
    if bytes > available || total > u32::MAX as usize / 9 {
        return Err("whitewater surface exceeds ocean surface memory budget".into());
    }
    let mut meshes = [Primitive::default(), Primitive::default()];
    for (i, mesh) in meshes.iter_mut().enumerate() {
        let count = frame.particles.iter().filter(drawn).filter(|p| usize::from(p.kind == Kind::Spray) == i).count();
        mesh.vertices.reserve_exact(count * if i == 0 { 9 } else { 6 });
        mesh.indices.reserve_exact(count * 24);
    }
    for p in frame.particles.iter().filter(drawn) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use sr_sim::ocean::whitewater::Particle;

    fn foam(n: usize) -> Frame {
        let particles = (0..n)
            .map(|i| Particle {
                id: i as u64,
                kind: Kind::Foam,
                birth: 0.0,
                lifetime: 2.0,
                position: [i as f64 * 0.01, 0.0, 0.0],
                velocity: [0.0; 3],
                radius: 0.05,
            })
            .collect();
        Frame { time: 0.5, particles }
    }

    #[test]
    fn foam_that_is_not_drawn_takes_no_share_of_the_memory_budget() {
        let frame = foam(1000);
        // room for a hundred tracers: the thousand discs do not fit, and in the albedo mode there are none to fit
        let available = 100 * (9 * 256 + 24 * 8);
        assert!(meshes(&frame, available, true).is_err(), "the foam discs exceed the budget");
        let [foam_batch, spray_batch] = meshes(&frame, available, false).expect("no disc is built, so none is charged");
        assert!(
            foam_batch.vertices.capacity() == 0 && foam_batch.indices.capacity() == 0,
            "nothing is reserved for it"
        );
        assert!(spray_batch.vertices.is_empty());
    }
}
