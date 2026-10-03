//! Restore frozen render attributes from each fragment's source-triangle identity.
use super::{Error, Piece};
use crate::{Primitive, Vertex};
use glam::DVec3;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurfaceMaterial {
    /// Source primitive index; preserves its material and material variants.
    Exterior(usize),
    /// A caller-selected interior material, independent of exterior overrides.
    Interior,
}
#[derive(Clone, Debug)]
pub struct Surface {
    pub material: SurfaceMaterial,
    pub mesh: Primitive,
}

/// Reconstruct render batches for a fragment of the concatenated `sources`.
/// Triangle IDs count source primitives in order. Sources must already be in
/// the kernel's coordinate frame with skinning/morphing frozen at release.
/// Exterior UV sets, separate map UVs and colors interpolate within their
/// original triangle; vertex sharing never erases a material or normal seam.
/// Interior UVs use a deterministic plane basis in original object coordinates,
/// with `uv_scale` texture repeats per scene unit. Opposite caps share UVs.
/// The conservative temporary/output allowance excludes caller-owned inputs.
pub fn surface(piece: &Piece, sources: &[Primitive], uv_scale: f64, max_bytes: usize) -> Result<Vec<Surface>, Error> {
    if !uv_scale.is_finite() || uv_scale <= 0. || piece.center.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("surface requires finite centroid and positive UV scale"));
    }
    let mut triangles = 0usize;
    let mut vertices = 0usize;
    let mut uv_sets = 0usize;
    let mut metadata = 0usize;
    for source in sources {
        vertices = vertices.checked_add(source.vertices.len()).ok_or(Error::Limit("surface input size"))?;
        triangles = triangles.checked_add(source.indices.len() / 3).ok_or(Error::Limit("surface input size"))?;
        uv_sets = uv_sets.max(source.tex_coords.len());
        let strings = source
            .variants
            .iter()
            .try_fold(0usize, |n, (name, _)| n.checked_add(name.len() + 64))
            .ok_or(Error::Limit("surface material metadata"))?;
        metadata = metadata
            .checked_add(strings)
            .and_then(|n| n.checked_add(source.tex_coords.len().checked_mul(128)?))
            .ok_or(Error::Limit("surface material metadata"))?;
    }
    if vertices > 1_000_000 || triangles > 1_000_000 || piece.faces.len() > u32::MAX as usize / 3 {
        return Err(Error::Limit("surface element count"));
    }
    let per_vertex = uv_sets
        .checked_mul(8)
        .and_then(|n| n.checked_add(std::mem::size_of::<Vertex>() + 4))
        .ok_or(Error::Limit("surface output size"))?;
    let bytes = piece
        .faces
        .len()
        .checked_mul(3)
        .and_then(|n| n.checked_mul(per_vertex))
        .and_then(|n| n.checked_mul(2)) // vector growth and simultaneous completed batches
        .and_then(|n| n.checked_add(triangles.checked_mul(32)?))
        .and_then(|n| n.checked_add(sources.len().checked_add(1)?.checked_mul(1024)?))
        .and_then(|n| n.checked_add(metadata))
        .ok_or(Error::Limit("surface output size"))?;
    if bytes > max_bytes {
        return Err(Error::Limit("fracture surface memory allowance"));
    }
    let mut provenance = Vec::with_capacity(triangles);
    for (k, s) in sources.iter().enumerate() {
        if s.indices.len() % 3 != 0
            || s.indices.iter().any(|&i| i as usize >= s.vertices.len())
            || bytemuck::cast_slice::<Vertex, f32>(&s.vertices).iter().any(|v| !v.is_finite())
            || s.tex_coords
                .values()
                .any(|uv| uv.len() != s.vertices.len() || uv.iter().flatten().any(|v| !v.is_finite()))
        {
            return Err(Error::Invalid("fracture source attributes or indices"));
        }
        provenance.extend(s.indices.as_chunks::<3>().0.iter().map(|t| (k, *t)));
    }
    let mut batches = BTreeMap::<SurfaceMaterial, Primitive>::new();
    for face in &piece.faces {
        let mut points = [DVec3::ZERO; 3];
        for (p, &i) in points.iter_mut().zip(&face.indices) {
            *p = DVec3::from(*piece.vertices.get(i as usize).ok_or(Error::Invalid("fragment index"))?);
            if !p.is_finite() {
                return Err(Error::Invalid("fragment position"));
            }
        }
        let normal = (points[1] - points[0])
            .cross(points[2] - points[0])
            .try_normalize()
            .ok_or(Error::Geometry("degenerate fragment render face"))?;
        let original = points.map(|p| p + DVec3::from(piece.center));
        let origin = face
            .source
            .map(|id| provenance.get(id).copied().ok_or(Error::Invalid("source triangle identity")))
            .transpose()?;
        let material = origin.map_or(SurfaceMaterial::Interior, |(k, _)| SurfaceMaterial::Exterior(k));
        let mesh = batches.entry(material).or_insert_with(|| {
            if let Some((k, _)) = origin {
                let source = &sources[k];
                Primitive {
                    material: source.material,
                    variants: source.variants.clone(),
                    tex_coords: source.tex_coords.keys().map(|&key| (key, Vec::new())).collect(),
                    ..Default::default()
                }
            } else {
                Primitive::default()
            }
        });
        // Canonicalize the cap normal so matching opposite faces use identical
        // planar UV axes while preserving opposite outward shading normals.
        let axis = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
            0
        } else if normal.y.abs() >= normal.z.abs() {
            1
        } else {
            2
        };
        let canonical = if normal[axis] < 0. { -normal } else { normal };
        let u = canonical.any_orthonormal_vector();
        let v = canonical.cross(u);
        for j in 0..3 {
            let mut vertex = if let Some((k, indices)) = origin {
                let source = &sources[k];
                let attrs = indices.map(|i| source.vertices[i as usize]);
                let weights = barycentric(original[j], attrs.map(|v| DVec3::from(v.pos.map(f64::from))))?;
                let blend = |values: [f32; 3]| -> f32 {
                    values.into_iter().zip(weights).map(|(v, w)| v as f64 * w).sum::<f64>() as f32
                };
                let n = DVec3::from(std::array::from_fn::<_, 3, _>(|a| blend(attrs.map(|v| v.normal[a])) as f64))
                    .try_normalize()
                    .unwrap_or(normal);
                let tangent =
                    DVec3::from(std::array::from_fn::<_, 3, _>(|a| blend(attrs.map(|v| v.tangent[a])) as f64));
                let tangent =
                    (tangent - n * n.dot(tangent)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
                for (&key, output) in &mut mesh.tex_coords {
                    let values = &source.tex_coords[&key];
                    output.push(std::array::from_fn(|a| blend(indices.map(|i| values[i as usize][a]))));
                }
                Vertex {
                    normal: n.as_vec3().to_array(),
                    tangent: [
                        tangent.x as f32,
                        tangent.y as f32,
                        tangent.z as f32,
                        if blend(attrs.map(|v| v.tangent[3])) < 0. { -1. } else { 1. },
                    ],
                    uv: std::array::from_fn(|a| blend(attrs.map(|v| v.uv[a]))),
                    map_uv: std::array::from_fn(|m| std::array::from_fn(|a| blend(attrs.map(|v| v.map_uv[m][a])))),
                    color: std::array::from_fn(|a| blend(attrs.map(|v| v.color[a]))),
                    ..Default::default()
                }
            } else {
                let uv = [(original[j].dot(u) * uv_scale) as f32, (original[j].dot(v) * uv_scale) as f32];
                Vertex {
                    normal: normal.as_vec3().to_array(),
                    uv,
                    map_uv: [uv; 4],
                    tangent: [u.x as f32, u.y as f32, u.z as f32, if normal.dot(canonical) < 0. { -1. } else { 1. }],
                    ..Default::default()
                }
            };
            vertex.pos = points[j].as_vec3().to_array();
            if bytemuck::cast_slice::<Vertex, f32>(std::slice::from_ref(&vertex)).iter().any(|v| !v.is_finite())
                || points[j].to_array().into_iter().zip(vertex.pos).any(|(a, b)| a != 0. && b == 0.)
            {
                return Err(Error::Geometry("fragment render attributes exceed f32 range"));
            }
            mesh.indices.push(mesh.vertices.len() as u32);
            mesh.vertices.push(vertex);
        }
    }
    Ok(batches.into_iter().map(|(material, mesh)| Surface { material, mesh }).collect())
}

fn barycentric(p: DVec3, triangle: [DVec3; 3]) -> Result<[f64; 3], Error> {
    let [a, b, c] = triangle;
    let scale = (b - a).abs().max((c - a).abs()).max_element();
    if !scale.is_finite() || scale == 0. {
        return Err(Error::Geometry("degenerate source face"));
    }
    let (b, c, q) = ((b - a) / scale, (c - a) / scale, (p - a) / scale);
    let n = b.cross(c);
    let d = n.length_squared();
    if !d.is_finite() || d == 0. {
        return Err(Error::Geometry("degenerate source face"));
    }
    let u = q.cross(c).dot(n) / d;
    let v = b.cross(q).dot(n) / d;
    let w = [1. - u - v, u, v];
    if w.iter().any(|w| !w.is_finite() || *w < -1e-6 || *w > 1. + 1e-6) || q.dot(n).abs() > d.sqrt() * 1e-6 {
        return Err(Error::Geometry("fragment face is outside its source triangle"));
    }
    let w = w.map(|v| v.clamp(0., 1.));
    let sum = w.iter().sum::<f64>();
    Ok(w.map(|v| v / sum))
}
