//! Objects that move the ocean's bed at every solver step: the crater of an
//! `object3D` deforms the seabed, and a closed body occupies part of the water
//! column. Both are sampled in the ocean's own frame.
use super::*;
use glam::{DMat4, DVec3};

pub(super) struct Colliders {
    beds: Vec<Bed>,
    bodies: Vec<Body>,
    /// Entries of the `colliders` list, which bound the tags.
    slots: usize,
    water_level: f64,
    rest_done: bool,
    top: Vec<f64>,
    hits: Vec<Hit>,
}
struct Bed {
    id: Arc<str>,
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    /// Topmost surface ordinate of every column at ocean time zero; infinite
    /// where the surface does not cover the column.
    rest: Vec<f64>,
}
struct Body {
    id: Arc<str>,
    /// Position in the ocean's `colliders` list, which is the tag of the body's columns.
    slot: usize,
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
}
/// Where a vertical line through a column meets a body's surface.
struct Hit {
    column: u32,
    y: f64,
    triangle: u32,
    /// Barycentric weights of the second and third vertices.
    weights: [f64; 2],
}

pub(super) fn ids(e: &sr_model::model::Ocean) -> Vec<String> {
    text(e, "colliders").unwrap_or_default().split_whitespace().map(str::to_string).collect()
}

impl Colliders {
    /// Resolves the referenced objects and tessellates their rest surface. An
    /// object with a crater deforms the bed; any other supported object is a
    /// closed body. Memory for the geometry and the per-column samples is charged
    /// to `budget`.
    pub(super) fn build(
        p: &Program,
        ids: &[String],
        spec: &Spec,
        water_level: f64,
        budget: usize,
    ) -> Result<Colliders, String> {
        if ids.len() > 4096 {
            return Err("at most 4096 ocean colliders".into());
        }
        let count = spec.cells[0] * spec.cells[1];
        let mut remaining = budget;
        let (mut beds, mut bodies) = (Vec::new(), Vec::new());
        for (slot, id) in ids.iter().enumerate() {
            let node =
                p.nodes.iter().find(|n| &*n.id == id).ok_or_else(|| format!("collider {id} is not instantiated"))?;
            let e = &*node.elem;
            let kind = text(e, "primitive");
            let bytes = |points: usize, triangles: usize| {
                points
                    .saturating_mul(24)
                    .saturating_add(triangles.saturating_mul(12))
                    .saturating_add(count.saturating_mul(8))
            };
            if crate::crater::from_element(e, 0.0)?.is_some() {
                let (points, triangles) = match kind.as_deref() {
                    Some("plane") => {
                        let r = num(e, "radius", 50.);
                        let segments = num(e, "segments", 32.).clamp(1., 1024.) as u32;
                        let mesh = sr_3d::prim::plane(
                            num(e, "width", 2. * r) as f32,
                            num(e, "height", 2. * r) as f32,
                            segments,
                        );
                        (
                            mesh.vertices.iter().map(|v| v.pos.map(f64::from)).collect::<Vec<_>>(),
                            mesh.indices.as_chunks::<3>().0.to_vec(),
                        )
                    }
                    Some("mesh") => {
                        let asset = node.asset.as_deref().ok_or_else(|| format!("collider {id} has no mesh asset"))?;
                        crate::sim3d::mesh_asset_triangles(p, asset, remaining)?
                    }
                    _ => return Err(format!("ocean collider {id} with a crater must be a plane or a mesh")),
                };
                remaining = remaining
                    .checked_sub(bytes(points.len(), triangles.len()))
                    .ok_or("ocean collider geometry exceeds memory budget")?;
                beds.push(Bed { id: node.id.clone(), points, triangles, rest: Vec::new() });
            } else {
                if kind.as_deref() == Some("plane") {
                    return Err(format!(
                        "ocean collider {id} is a plane without a crater: it is neither a body nor a deformable bed"
                    ));
                }
                let (points, triangles) = crate::pyro::colliders::geometry(p, node, 0., remaining)?;
                remaining = remaining
                    .checked_sub(bytes(points.len(), triangles.len()))
                    .ok_or("ocean collider geometry exceeds memory budget")?;
                bodies.push(Body { id: node.id.clone(), slot, points, triangles });
            }
        }
        remaining.checked_sub(count.saturating_mul(24)).ok_or("ocean collider geometry exceeds memory budget")?;
        Ok(Colliders {
            beds,
            bodies,
            slots: ids.len(),
            water_level,
            rest_done: false,
            top: Vec::new(),
            hits: Vec::new(),
        })
    }

    pub(super) fn has_bodies(&self) -> bool {
        !self.bodies.is_empty()
    }

    /// The bound of the tags the solver is given: a body tags its columns with its position in the
    /// ocean's `colliders` list (counting the craters' surfaces), so a list maps tags to ids.
    pub(super) fn body_count(&self) -> usize {
        if self.bodies.is_empty() {
            0
        } else {
            self.slots
        }
    }

    /// Fills `forcing` for ocean-local `time`. The bed is the bathymetry plus how
    /// far each crater's deformed surface has moved from where it was at time
    /// zero, raised by the thickness of the bodies in each column; with bodies it
    /// also holds that thickness and the bodies' horizontal velocity. `frame_at`
    /// maps ocean-local time to a scene frame.
    pub(super) fn sample(
        &mut self,
        spec: &Spec,
        ocean: &str,
        frame_at: &mut dyn FnMut(f64) -> Result<Arc<FrameGraph>, String>,
        time: f64,
        base: &[f64],
        forcing: &mut sim::Forcing,
    ) -> Result<(), String> {
        forcing.bed.copy_from_slice(base);
        if !self.rest_done {
            let frame = frame_at(0.)?;
            if let Some(inverse) = inverse_of(&frame, ocean)? {
                for i in 0..self.beds.len() {
                    let mut rest = Vec::new();
                    self.beds[i].top(spec, inverse, &frame, &mut rest)?;
                    self.beds[i].rest = rest;
                }
            }
            self.rest_done = true;
        }
        let frame = frame_at(time)?;
        let Some(inverse) = inverse_of(&frame, ocean)? else {
            forcing.occupancy.fill(0.);
            forcing.velocity.fill([0.; 2]);
            return Ok(());
        };
        for i in 0..self.beds.len() {
            let mut top = std::mem::take(&mut self.top);
            self.beds[i].top(spec, inverse, &frame, &mut top)?;
            for ((bed, now), rest) in forcing.bed.iter_mut().zip(&top).zip(&self.beds[i].rest) {
                if now.is_finite() && rest.is_finite() {
                    *bed += now - rest;
                }
            }
            self.top = top;
        }
        if self.bodies.is_empty() {
            return Ok(());
        }
        let n = forcing.bed.len();
        forcing.occupancy.clear();
        forcing.occupancy.resize(n, 0.);
        let mut momentum = vec![[0.; 2]; n];
        // the body that holds most of a column owns it; bodies come in index order, so the lowest wins a tie
        let mut held_most = vec![0.; if spec.body_owners > 0 { n } else { 0 }];
        forcing.owner.clear();
        forcing.owner.resize(if spec.body_owners > 0 { n } else { 0 }, 0);
        for i in 0..self.bodies.len() {
            self.occupy(i, spec, ocean, frame_at, time, &frame, inverse, forcing, &mut momentum, &mut held_most)?;
        }
        forcing.velocity.clear();
        forcing.velocity.resize(n, [0.; 2]);
        let level = self.water_level;
        for (((bed, occupancy), velocity), momentum) in
            forcing.bed.iter_mut().zip(&mut forcing.occupancy).zip(&mut forcing.velocity).zip(&momentum)
        {
            // A column cannot hold more body than it holds water.
            let held = occupancy.min((*bed - level).max(0.));
            if *occupancy > 0. {
                let scale = 1. / *occupancy;
                *velocity = [momentum[0] * scale, momentum[1] * scale];
            }
            *occupancy = held;
            *bed -= held;
        }
        Ok(())
    }

    /// Adds body `index`'s thickness over every column it covers, between the
    /// rest level and the bed, and its thickness-weighted horizontal velocity.
    #[allow(clippy::too_many_arguments)]
    fn occupy(
        &mut self,
        index: usize,
        spec: &Spec,
        ocean: &str,
        frame_at: &mut dyn FnMut(f64) -> Result<Arc<FrameGraph>, String>,
        time: f64,
        frame: &FrameGraph,
        inverse: DMat4,
        forcing: &mut sim::Forcing,
        momentum: &mut [[f64; 2]],
        held_most: &mut [f64],
    ) -> Result<(), String> {
        let body = &self.bodies[index];
        let Some(j) = frame.nodes.iter().position(|n| n.id == body.id) else { return Ok(()) };
        let now = inverse * crate::sim3d::world3(frame, j, 0);
        // The body's velocity is the displacement of a material point over one
        // canonical step, forward when the body is still there, otherwise backward.
        let pose_at = |frame_at: &mut dyn FnMut(f64) -> Result<Arc<FrameGraph>, String>,
                       t: f64|
         -> Result<Option<DMat4>, String> {
            let frame = frame_at(t)?;
            let (Some(inverse), Some(j)) =
                (inverse_of(&frame, ocean)?, frame.nodes.iter().position(|n| n.id == body.id))
            else {
                return Ok(None);
            };
            Ok(Some(inverse * crate::sim3d::world3(&frame, j, 0)))
        };
        let dt = spec.dt;
        let (from, to) = match pose_at(frame_at, time + dt)? {
            Some(next) => (now, next),
            None if time >= dt => match pose_at(frame_at, time - dt)? {
                Some(before) => (before, now),
                None => (now, now),
            },
            None => (now, now),
        };
        let moved: Vec<[f64; 3]> =
            body.points.iter().map(|&p| now.transform_point3(DVec3::from_array(p)).to_array()).collect();
        if moved.iter().flatten().any(|v| !v.is_finite()) {
            return Err("ocean body geometry is nonfinite".into());
        }
        self.hits.clear();
        raster_hits(spec, &moved, &body.triangles, &mut self.hits);
        self.hits.sort_unstable_by(|a, b| {
            a.column.cmp(&b.column).then(a.y.total_cmp(&b.y)).then(a.triangle.cmp(&b.triangle))
        });
        let mut start = 0;
        while start < self.hits.len() {
            let column = self.hits[start].column;
            let mut end = start;
            while end < self.hits.len() && self.hits[end].column == column {
                end += 1;
            }
            if (end - start) % 2 != 0 {
                return Err(format!("ocean body {} is not a closed surface", body.id));
            }
            let c = column as usize;
            let (low, high) = (self.water_level, forcing.bed[c]);
            let mut thickness = 0.;
            let mut entry = None;
            for pair in self.hits[start..end].chunks(2) {
                let held = (pair[1].y.min(high) - pair[0].y.max(low)).max(0.);
                if held > 0. && entry.is_none() {
                    entry = Some(&pair[0]);
                }
                thickness += held;
            }
            if let Some(hit) = entry {
                let [a, b, c3] = body.triangles[hit.triangle as usize].map(|i| body.points[i as usize]);
                let (v, w) = (hit.weights[0], hit.weights[1]);
                let u = 1. - v - w;
                let rest: DVec3 = DVec3::from_array(a) * u + DVec3::from_array(b) * v + DVec3::from_array(c3) * w;
                let velocity = (to.transform_point3(rest) - from.transform_point3(rest)) / dt;
                forcing.occupancy[c] += thickness;
                if let Some(most) = held_most.get_mut(c).filter(|most| thickness > **most) {
                    *most = thickness;
                    forcing.owner[c] = body.slot as u32;
                }
                momentum[c][0] += thickness * velocity.x;
                momentum[c][1] += thickness * velocity.z;
            }
            start = end;
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

/// Meets of the vertical lines through cell centres with a surface. Centres are
/// moved by a fixed irrational fraction of a cell so that no line runs exactly
/// along a shared edge, which would count one crossing twice.
fn raster_hits(spec: &Spec, points: &[[f64; 3]], triangles: &[[u32; 3]], hits: &mut Vec<Hit>) {
    let [nx, nz] = spec.cells;
    let dx = spec.cell_size;
    let jitter = [0.618_033_988_749_895e-7 * dx, 0.414_213_562_373_095e-7 * dx];
    for (index, tri) in triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|i| points[i as usize]);
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
            (((high[i] - spec.origin[i]) / dx - 0.5).ceil().clamp(-1., spec.cells[i] as f64 - 1.) as isize + 1) as usize
        });
        for iz in starts[1]..ends[1].min(nz) {
            for ix in starts[0]..ends[0].min(nx) {
                let px = spec.origin[0] + (ix as f64 + 0.5) * dx + jitter[0] - a[0];
                let pz = spec.origin[1] + (iz as f64 + 0.5) * dx + jitter[1] - a[2];
                let v = (px * cz - pz * cx) / det;
                let w = (bx * pz - bz * px) / det;
                let u = 1. - v - w;
                if u < 0. || v < 0. || w < 0. {
                    continue;
                }
                hits.push(Hit {
                    column: (iz * nx + ix) as u32,
                    y: u * a[1] + v * b[1] + w * c[1],
                    triangle: index as u32,
                    weights: [v, w],
                });
            }
        }
    }
}

impl Bed {
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
        let mut hits = Vec::new();
        raster_hits(spec, &moved, &self.triangles, &mut hits);
        for hit in hits {
            let cell = &mut out[hit.column as usize];
            *cell = cell.min(hit.y);
        }
        Ok(())
    }
}
