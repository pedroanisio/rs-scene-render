use sr_3d::{Primitive, Vertex};
use sr_sim::ocean::{Frame, Spec};
pub(super) fn memory_cost(spec: &Spec) -> Result<usize, String> {
    let [nx, nz] = spec.cells;
    let count = (nx + 1).checked_mul(nz + 1).ok_or("ocean surface size overflow")?;
    // Includes vertices, normals/tangent accumulation, indices and the upload
    // copy. Renderer device limits apply independently before GPU allocation.
    count
        .checked_mul(256)
        .and_then(|b| nx.checked_mul(nz).and_then(|n| n.checked_mul(48)).and_then(|i| b.checked_add(i)))
        .ok_or("ocean surface memory overflow".into())
}
pub(super) fn mesh(spec: &Spec, bed: &[f64], frame: &Frame, max_bytes: usize) -> Result<Primitive, String> {
    let [nx, nz] = spec.cells;
    let count = (nx + 1).checked_mul(nz + 1).ok_or("ocean surface size overflow")?;
    let bytes = memory_cost(spec)?;
    if bytes > max_bytes || count > u32::MAX as usize {
        return Err("ocean surface exceeds memory budget".into());
    }
    if !frame.cells.iter().any(|c| c.depth >= spec.dry_tolerance) {
        return Ok(Primitive::default());
    }
    let mut vertices = Vec::with_capacity(count);
    for z in 0..=nz {
        for x in 0..=nx {
            let (mut sum, mut n) = (0., 0);
            for iz in z.saturating_sub(1)..=z.min(nz - 1) {
                for ix in x.saturating_sub(1)..=x.min(nx - 1) {
                    let i = iz * nx + ix;
                    if frame.cells[i].depth >= spec.dry_tolerance {
                        sum += bed[i] - frame.cells[i].depth;
                        n += 1;
                    }
                }
            }
            let pos = [
                spec.origin[0] + x as f64 * spec.cell_size,
                if n > 0 { sum / n as f64 } else { 0. },
                spec.origin[1] + z as f64 * spec.cell_size,
            ]
            .map(|v| v as f32);
            if pos.iter().any(|v| !v.is_finite()) {
                return Err("ocean surface coordinates exceed device precision".into());
            }
            let uv = [x as f32 / nx as f32, z as f32 / nz as f32];
            vertices.push(Vertex { pos, uv, map_uv: [uv; 4], ..Default::default() });
        }
    }
    let mut indices = Vec::with_capacity(nx * nz * 6);
    for z in 0..nz {
        for x in 0..nx {
            if frame.cells[z * nx + x].depth < spec.dry_tolerance {
                continue;
            }
            let a = (z * (nx + 1) + x) as u32;
            let b = a + 1;
            let c = a + (nx + 1) as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    sr_3d::compute_normals(&mut vertices, &indices);
    sr_3d::compute_tangents(&mut vertices, &indices);
    Ok(Primitive { vertices, indices, ..Default::default() })
}
