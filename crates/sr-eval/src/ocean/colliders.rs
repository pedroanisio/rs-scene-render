//! Deformable seabeds that move the ocean's bed: the crater of an `object3D`,
//! sampled for every solver step in the ocean's own frame.
use super::*;
use glam::{DMat4, DVec3};

pub(super) struct Colliders {
    items: Vec<Item>,
    rest_done: bool,
    top: Vec<f64>,
}
struct Item {
    id: Arc<str>,
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    /// Topmost surface ordinate of every column at ocean time zero; infinite
    /// where the surface does not cover the column.
    rest: Vec<f64>,
}

pub(super) fn ids(e: &sr_model::model::Ocean) -> Vec<String> {
    text(e, "colliders").unwrap_or_default().split_whitespace().map(str::to_string).collect()
}

impl Colliders {
    /// Resolves the referenced objects and tessellates their rest surface. Memory
    /// for the geometry and the per-column samples is charged to `budget`.
    pub(super) fn build(p: &Program, ids: &[String], spec: &Spec, budget: usize) -> Result<Colliders, String> {
        if ids.len() > 4096 {
            return Err("at most 4096 ocean colliders".into());
        }
        let count = spec.cells[0] * spec.cells[1];
        let mut remaining = budget;
        let mut items = Vec::new();
        for id in ids {
            let node =
                p.nodes.iter().find(|n| &*n.id == id).ok_or_else(|| format!("collider {id} is not instantiated"))?;
            let e = &*node.elem;
            if crate::crater::from_element(e, 0.0)?.is_none() {
                return Err(format!("ocean collider {id} needs a <crater>: it moves the bed it deforms"));
            }
            let (points, triangles) = match text(e, "primitive").as_deref() {
                Some("plane") => {
                    let r = num(e, "radius", 50.);
                    let segments = num(e, "segments", 32.).clamp(1., 1024.) as u32;
                    let mesh =
                        sr_3d::prim::plane(num(e, "width", 2. * r) as f32, num(e, "height", 2. * r) as f32, segments);
                    (
                        mesh.vertices.iter().map(|v| v.pos.map(f64::from)).collect::<Vec<_>>(),
                        mesh.indices.as_chunks::<3>().0.to_vec(),
                    )
                }
                Some("mesh") => {
                    let asset = node.asset.as_deref().ok_or_else(|| format!("collider {id} has no mesh asset"))?;
                    crate::sim3d::mesh_asset_triangles(p, asset, remaining)?
                }
                _ => return Err(format!("ocean collider {id} requires a plane or mesh surface")),
            };
            let bytes = points
                .len()
                .saturating_mul(24)
                .saturating_add(triangles.len().saturating_mul(12))
                .saturating_add(count.saturating_mul(8));
            remaining = remaining.checked_sub(bytes).ok_or("ocean collider geometry exceeds memory budget")?;
            items.push(Item { id: node.id.clone(), points, triangles, rest: Vec::new() });
        }
        remaining.checked_sub(count.saturating_mul(8)).ok_or("ocean collider geometry exceeds memory budget")?;
        Ok(Colliders { items, rest_done: false, top: Vec::new() })
    }

    /// Fills `out` with the effective bed at ocean-local `time`: the base bed
    /// plus, for every crater, how far its deformed surface has moved from where it
    /// was at time zero. `frame_at` maps ocean-local time to a scene frame.
    pub(super) fn bed(
        &mut self,
        spec: &Spec,
        ocean: &str,
        frame_at: &mut dyn FnMut(f64) -> Result<Arc<FrameGraph>, String>,
        time: f64,
        base: &[f64],
        out: &mut [f64],
    ) -> Result<(), String> {
        out.copy_from_slice(base);
        if !self.rest_done {
            let frame = frame_at(0.)?;
            if let Some(inverse) = inverse_of(&frame, ocean)? {
                for i in 0..self.items.len() {
                    let mut rest = Vec::new();
                    self.items[i].top(spec, inverse, &frame, &mut rest)?;
                    self.items[i].rest = rest;
                }
            }
            self.rest_done = true;
        }
        let frame = frame_at(time)?;
        let Some(inverse) = inverse_of(&frame, ocean)? else { return Ok(()) };
        for i in 0..self.items.len() {
            let mut top = std::mem::take(&mut self.top);
            self.items[i].top(spec, inverse, &frame, &mut top)?;
            for ((bed, now), rest) in out.iter_mut().zip(&top).zip(&self.items[i].rest) {
                if now.is_finite() && rest.is_finite() {
                    *bed += now - rest;
                }
            }
            self.top = top;
        }
        Ok(())
    }
}

/// Inverse of the ocean's world matrix in `frame`, or `None` when it is absent.
fn inverse_of(frame: &FrameGraph, ocean: &str) -> Result<Option<DMat4>, String> {
    let Some(i) = frame.nodes.iter().position(|n| &*n.id == ocean) else { return Ok(None) };
    let world = crate::sim3d::world3(frame, i, 0);
    let inverse = world.inverse();
    if !inverse.to_cols_array().iter().all(|v| v.is_finite()) {
        return Err("ocean pose is singular".into());
    }
    Ok(Some(inverse))
}

impl Item {
    /// Topmost ordinate of this surface over every column, in the ocean's frame.
    fn top(&self, spec: &Spec, inverse: DMat4, frame: &FrameGraph, out: &mut Vec<f64>) -> Result<(), String> {
        let count = spec.cells[0] * spec.cells[1];
        out.clear();
        out.resize(count, f64::INFINITY);
        let Some(j) = frame.nodes.iter().position(|n| n.id == self.id) else { return Ok(()) };
        let matrix = inverse * crate::sim3d::world3(frame, j, 0);
        let crater = crate::crater::at(&frame.nodes[j])?.ok_or("ocean collider lost its crater")?;
        let mut moved = Vec::with_capacity(self.points.len());
        for &point in &self.points {
            let deformed = crater.kernel.map(point, crater.progress).map_err(|e| e.to_string())?.position;
            let p = matrix.transform_point3(DVec3::from_array(deformed));
            if !p.is_finite() {
                return Err("ocean collider geometry is nonfinite".into());
            }
            moved.push(p.to_array());
        }
        let [nx, nz] = spec.cells;
        let dx = spec.cell_size;
        for tri in &self.triangles {
            let [a, b, c] = tri.map(|i| moved[i as usize]);
            let (bx, bz, cx, cz) = (b[0] - a[0], b[2] - a[2], c[0] - a[0], c[2] - a[2]);
            let det = bx * cz - bz * cx;
            if !det.is_finite() || det == 0. {
                continue;
            }
            let low = [a[0].min(b[0]).min(c[0]), a[2].min(b[2]).min(c[2])];
            let high = [a[0].max(b[0]).max(c[0]), a[2].max(b[2]).max(c[2])];
            let starts: [usize; 2] = std::array::from_fn(|i| {
                ((low[i] - spec.origin[i]) / dx - 0.5).floor().clamp(0., spec.cells[i] as f64) as usize
            });
            let ends: [usize; 2] = std::array::from_fn(|i| {
                (((high[i] - spec.origin[i]) / dx - 0.5).ceil().clamp(-1., spec.cells[i] as f64 - 1.) as isize + 1)
                    as usize
            });
            for iz in starts[1]..ends[1].min(nz) {
                for ix in starts[0]..ends[0].min(nx) {
                    let px = spec.origin[0] + (ix as f64 + 0.5) * dx - a[0];
                    let pz = spec.origin[1] + (iz as f64 + 0.5) * dx - a[2];
                    let v = (px * cz - pz * cx) / det;
                    let w = (bx * pz - bz * px) / det;
                    let u = 1. - v - w;
                    if u < -1e-9 || v < -1e-9 || w < -1e-9 {
                        continue;
                    }
                    let y = u * a[1] + v * b[1] + w * c[1];
                    let cell = &mut out[iz * nx + ix];
                    *cell = cell.min(y);
                }
            }
        }
        Ok(())
    }
}
