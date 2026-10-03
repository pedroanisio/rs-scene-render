//! Shared scene geometry and sampled rigid collider motion in scene axes.
use super::*;
use glam::{DMat4, DVec3};
use sr_sim::particles3d::collider::{Collider, Geometry};

pub(super) struct Colliders {
    surfaces: Vec<Surface>,
    // Original surfaces remain available for backward seeks. All additional
    // fragment acceleration structures share the unused component allowance.
    fracture_budget: usize,
}
impl Colliders {
    pub(super) fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}
struct Pieces {
    key: u64,
    shapes: Vec<Geometry>,
    bytes: usize,
}
struct Surface {
    id: Arc<str>,
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    budget: usize,
    deforms: bool,
    cached: Option<([f64; 3], Geometry)>,
    pieces: Option<Pieces>,
}
pub(super) fn build(p: &Program, n: &FrameNode, max_bytes: usize) -> Result<Colliders, String> {
    let list = text(&*n.elem, "colliders").unwrap_or_default();
    let mut remaining = max_bytes;
    let mut out = Vec::new();
    for id in list.split_whitespace() {
        let node = p.nodes.iter().find(|n| &*n.id == id).ok_or_else(|| format!("collider {id} is not instantiated"))?;
        let e = &*node.elem;
        let r = num(e, "radius", 50.);
        let w = num(e, "width", 2. * r);
        let h = num(e, "height", 2. * r);
        let depth = num(e, "depth", 10.);
        let kind = text(e, "primitive").ok_or("collider primitive missing")?;
        let crater = crate::crater::from_element(e, 0.)?;
        let deforms = crater.is_some();
        let allowance = crater.as_ref().map_or(remaining, |c| remaining.min(c.max_bytes));
        let (points, triangles) = if kind == "globe" && text(e, "terrain").is_some() {
            crate::terrain::collider_triangles(p, node, allowance)?
        } else if kind == "mesh" {
            crate::sim3d::mesh_asset_triangles(p, node.asset.as_deref().ok_or("collider mesh missing")?, allowance)?
        } else if matches!(kind.as_str(), "text" | "clay" | "extrude") {
            crate::solid::collider_triangles(p, node, allowance)?
        } else {
            let segments = if kind == "plane" {
                num(e, "segments", 32.).clamp(1., 1024.)
            } else {
                num(e, "segments", 32.).clamp(3., 512.)
            } as u32;
            if kind != "box"
                && (kind != "plane" || deforms)
                && u64::from(segments) * u64::from(segments) * 2048 > allowance as u64
            {
                return Err("collider primitive tessellation exceeds geometry budget".into());
            }
            let primitive = match kind.as_str() {
                "box" => sr_3d::prim::cuboid(w as f32, h as f32, depth as f32),
                "plane" => sr_3d::prim::plane(w as f32, h as f32, if deforms { segments } else { 1 }),
                "sphere" | "globe" => sr_3d::prim::sphere(r as f32, segments),
                "cylinder" => sr_3d::prim::cylinder(r as f32, r as f32, h as f32, segments),
                "cone" => sr_3d::prim::cylinder(0., r as f32, h as f32, segments),
                "capsule" => sr_3d::prim::capsule(r as f32, num(e, "height", 4. * r).max(2. * r) as f32, segments),
                "torus" => sr_3d::prim::torus(r as f32, (num(e, "height", 0.7 * r) * 0.5).min(r) as f32, segments),
                _ => return Err(format!("unsupported particle collider {id}")),
            };
            (
                primitive.vertices.iter().map(|v| v.pos.map(f64::from)).collect(),
                primitive.indices.as_chunks::<3>().0.to_vec(),
            )
        };
        let charge = points
            .len()
            .checked_mul(if deforms { 192 } else { 72 })
            .and_then(|b| triangles.len().checked_mul(524).and_then(|t| b.checked_add(t)))
            .and_then(|b| b.checked_add(4096))
            .ok_or("particle collider geometry memory overflow")?;
        if charge > allowance {
            return Err("particle collider geometry memory budget".into());
        }
        remaining = remaining.checked_sub(charge).ok_or("particle collider geometry memory budget")?;
        out.push(Surface {
            id: node.id.clone(),
            points,
            triangles,
            budget: charge,
            deforms,
            cached: None,
            pieces: None,
        });
    }
    Ok(Colliders { surfaces: out, fracture_budget: remaining })
}
fn pose(m: DMat4) -> Result<(DVec3, glam::DQuat, DVec3), Error> {
    if !m.is_finite() || !m.determinant().is_finite() || m.determinant() == 0. {
        return Err(Error::Invalid("particle collider affine"));
    }
    let (s, r, t) = m.to_scale_rotation_translation();
    let reconstructed = DMat4::from_scale_rotation_translation(s, r, t);
    if !s.is_finite()
        || !r.is_finite()
        || m.to_cols_array()
            .iter()
            .zip(reconstructed.to_cols_array())
            .any(|(a, b)| (a - b).abs() > 1e-8 * a.abs().max(1.))
    {
        return Err(Error::Invalid("particle rigid colliders require a pose without shear"));
    }
    Ok((s, r, t))
}
pub(super) fn sample(
    colliders: &mut Colliders,
    current: &FrameGraph,
    next: &FrameGraph,
    time: f64,
    dt: f64,
) -> Result<Vec<Collider>, Error> {
    let mut out = Vec::new();
    let mut remaining = colliders.fracture_budget;
    for surface in &mut colliders.surfaces {
        let key =
            current.nodes.iter().find(|n| n.id == surface.id).and_then(|n| n.fracture.as_ref()).map(|f| f.geometry.key);
        if surface.pieces.as_ref().is_some_and(|p| Some(p.key) != key) {
            surface.pieces = None;
        }
        if let Some(pieces) = &surface.pieces {
            remaining = remaining.checked_sub(pieces.bytes).ok_or(Error::Limit("fracture collider memory budget"))?;
        }
    }
    for surface in &mut colliders.surfaces {
        let Some(i) = current.nodes.iter().position(|n| n.id == surface.id) else {
            continue;
        };
        if let Some(fracture) = &current.nodes[i].fracture {
            if surface.pieces.is_none() {
                let mut shapes = Vec::new();
                let mut bytes = 0;
                for piece in &fracture.geometry.pieces {
                    let sr_sim::physics3d::Shape3::Decomposition(points, triangles) = &piece.shape else {
                        return Err(Error::Invalid("fracture collider surface"));
                    };
                    let shape = Geometry::new(points, triangles, remaining)?;
                    remaining -= shape.bytes();
                    bytes += shape.bytes();
                    shapes.push(shape);
                }
                surface.pieces = Some(Pieces { key: fracture.geometry.key, shapes, bytes });
            }
            let future = next
                .nodes
                .iter()
                .find(|n| n.id == surface.id)
                .and_then(|n| n.fracture.as_ref())
                .filter(|f| f.geometry.key == fracture.geometry.key);
            for (j, shape) in surface.pieces.as_ref().expect("fracture shapes").shapes.iter().enumerate() {
                if !fracture.enabled[j] {
                    continue;
                }
                let a = fracture.poses[j];
                let b = future.filter(|f| f.enabled[j]).map_or(a, |f| f.poses[j]);
                let velocity = (DVec3::from_array(b.pos) - DVec3::from_array(a.pos)) / dt;
                let rotation = glam::DQuat::from_array(a.rot);
                let next_rotation = glam::DQuat::from_array(b.rot);
                let omega = (next_rotation * rotation.conjugate()).normalize().to_scaled_axis() / dt;
                out.push(shape.moving(a, velocity.to_array(), omega.to_array().map(f64::to_degrees), time)?);
            }
            continue;
        }
        let m = crate::sim3d::world3(current, i, 0);
        let m1 =
            next.nodes.iter().position(|n| n.id == surface.id).map(|i| crate::sim3d::world3(next, i, 0)).unwrap_or(m);
        if surface.deforms {
            let j = next
                .nodes
                .iter()
                .position(|n| n.id == surface.id)
                .ok_or(Error::Invalid("missing crater collider endpoint"))?;
            let endpoint = |node: &FrameNode, matrix: DMat4| -> Result<Vec<[f64; 3]>, Error> {
                if !matrix.is_finite() || !matrix.determinant().is_finite() || matrix.determinant() == 0. {
                    return Err(Error::Invalid("crater collider transform"));
                }
                let crater = crate::crater::at(node)
                    .map_err(|_| Error::Invalid("crater collider configuration"))?
                    .ok_or(Error::Invalid("missing crater collider configuration"))?;
                surface
                    .points
                    .iter()
                    .map(|&p| {
                        let point = crater
                            .kernel
                            .map(p, crater.progress)
                            .map_err(|_| Error::Invalid("crater collider deformation"))?
                            .position;
                        Ok(matrix.transform_point3(DVec3::from_array(point)).to_array())
                    })
                    .collect()
            };
            let start = endpoint(&current.nodes[i], m)?;
            let end = endpoint(&next.nodes[j], m1)?;
            out.push(Collider::deforming(&start, &end, &surface.triangles, time, dt, surface.budget)?);
            continue;
        }
        let (scale, r, t) = pose(m)?;
        let (scale1, r1, t1) = pose(m1)?;
        if (scale - scale1).abs().max_element() > 1e-8 * scale.abs().max_element().max(1.) {
            return Err(Error::Invalid("particle rigid collider scale changes within a step"));
        }
        let scale = scale.to_array();
        if surface.cached.as_ref().is_none_or(|(s, _)| s != &scale) {
            // Release the old shape before allocating its replacement. Sweep
            // snapshots have already been evicted by the scene driver.
            surface.cached = None;
            let points: Vec<_> = surface.points.iter().map(|p| std::array::from_fn(|i| p[i] * scale[i])).collect();
            let mut indices = surface.triangles.clone();
            if scale.iter().product::<f64>() < 0. {
                for t in &mut indices {
                    t.swap(1, 2);
                }
            }
            surface.cached = Some((scale, Geometry::new(&points, &indices, surface.budget)?));
        }
        let velocity = (t1 - t) / dt;
        let omega = (r1 * r.conjugate()).normalize().to_scaled_axis() / dt;
        let geometry = &surface.cached.as_ref().expect("particle collider geometry").1;
        out.push(geometry.moving(
            sr_sim::physics3d::Pose3 { pos: t.to_array(), rot: r.to_array() },
            velocity.to_array(),
            omega.to_array().map(f64::to_degrees),
            time,
        )?);
    }
    Ok(out)
}
