use super::{Budget, Error, Face, Solid};
use glam::{DVec2, DVec3};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::collections::BTreeMap;

pub(super) fn split(s: &Solid, normal: DVec3, bias: f64, budget: &mut Budget) -> Result<Option<[Solid; 2]>, Error> {
    let n = s.faces.len();
    budget.admit(
        s.vertices
            .len()
            .checked_add(n.checked_mul(3).ok_or(Error::Limit("cut size"))?)
            .ok_or(Error::Limit("cut size"))?,
        n.checked_mul(9).ok_or(Error::Limit("cut size"))?,
    )?;
    budget.work(n)?;
    let mut positions = s.vertices.clone();
    let mut distance: Vec<_> = positions.iter().map(|p| p.dot(normal) - bias).collect();
    for (p, d) in positions.iter_mut().zip(&mut distance) {
        if d.abs() < 1e-12 {
            *p -= normal * *d;
            *d = 0.;
        }
    }
    if !distance.iter().any(|d| *d < 0.) || !distance.iter().any(|d| *d > 0.) {
        return Ok(None);
    }
    let mut halves = [Vec::<Face>::new(), Vec::new()];
    let mut cuts = BTreeMap::<[u32; 2], u32>::new();
    for f in &s.faces {
        if f.indices.iter().all(|&i| distance[i as usize] == 0.) {
            let [a, b, c] = f.indices.map(|i| positions[i as usize]);
            let side = usize::from((b - a).cross(c - a).dot(normal) < 0.);
            halves[side].push(f.clone());
            continue;
        }
        for (side, faces) in halves.iter_mut().enumerate() {
            let mut polygon = Vec::with_capacity(4);
            for i in 0..3 {
                let a = f.indices[i];
                let b = f.indices[(i + 1) % 3];
                let da = distance[a as usize];
                let db = distance[b as usize];
                if (side == 0 && da <= 0.) || (side == 1 && da >= 0.) {
                    polygon.push(a)
                }
                if (da < 0. && db > 0.) || (da > 0. && db < 0.) {
                    let key = if a < b { [a, b] } else { [b, a] };
                    let id = *cuts.entry(key).or_insert_with(|| {
                        // Always interpolate in canonical edge order so shared
                        // edges have bit-identical intersections across faces.
                        let [a, b] = key.map(|i| i as usize);
                        let t = distance[a] / (distance[a] - distance[b]);
                        let p = positions[a].lerp(positions[b], t);
                        let id = positions.len() as u32;
                        positions.push(p);
                        distance.push(0.);
                        id
                    });
                    polygon.push(id);
                }
            }
            for i in 1..polygon.len().saturating_sub(1) {
                faces.push(Face { indices: [polygon[0], polygon[i], polygon[i + 1]], source: f.source });
            }
        }
    }
    // Boundary edges of the negative half define the cut domain. Their winding
    // includes holes; constraints prevent triangulation across concave corners.
    let mut edges = BTreeMap::<[u32; 2], (usize, [u32; 2])>::new();
    for face in &halves[0] {
        for i in 0..3 {
            let a = face.indices[i];
            let b = face.indices[(i + 1) % 3];
            if distance[a as usize] != 0. || distance[b as usize] != 0. {
                continue;
            }
            let key = if a < b { [a, b] } else { [b, a] };
            let e = edges.entry(key).or_insert((0, [a, b]));
            e.0 += 1;
        }
    }
    let boundary: Vec<_> = edges.values().filter_map(|&(count, edge)| (count == 1).then_some(edge)).collect();
    if !boundary.is_empty() {
        let axis = if normal.x.abs() < 0.8 { DVec3::X } else { DVec3::Y };
        let u = normal.cross(axis).normalize();
        let v = normal.cross(u);
        let project = |p: DVec3| DVec2::new(p.dot(u), p.dot(v));
        let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
        let mut handles = BTreeMap::new();
        let mut ids = BTreeMap::new();
        for edge in &boundary {
            for &id in edge {
                if handles.contains_key(&id) {
                    continue;
                }
                let p = project(positions[id as usize]);
                let h = cdt.insert(Point2::new(p.x, p.y)).map_err(|_| Error::Geometry("cut projection"))?;
                if ids.insert(h.index(), id).is_some() {
                    return Err(Error::Geometry("coincident cut vertices"));
                }
                handles.insert(id, h);
            }
        }
        for &[a, b] in &boundary {
            let (a, b) = (handles[&a], handles[&b]);
            if !cdt.can_add_constraint(a, b) {
                return Err(Error::Geometry("self-intersecting cut boundary"));
            }
            cdt.add_constraint(a, b);
        }
        let segments: Vec<_> = boundary.iter().map(|e| e.map(|i| project(positions[i as usize]))).collect();
        let mut cap: Vec<_> = cdt.inner_faces().map(|face| face.vertices().map(|h| ids[&h.fix().index()])).collect();
        repair_collinear(&mut cap, &positions, budget)?;
        for indices in cap {
            budget.work(segments.len())?;
            let p = project(indices.iter().map(|&i| positions[i as usize]).sum::<DVec3>() / 3.);
            let mut winding = 0;
            for &[a, b] in &segments {
                let side = (b - a).perp_dot(p - a);
                if a.y <= p.y && b.y > p.y && side > 0. {
                    winding += 1;
                }
                if b.y <= p.y && a.y > p.y && side < 0. {
                    winding -= 1;
                }
            }
            if winding != 0 {
                halves[0].push(Face { indices, source: None });
                halves[1].push(Face { indices: [indices[0], indices[2], indices[1]], source: None });
            }
        }
    }
    let mut result = Vec::with_capacity(2);
    for mut faces in halves {
        let mut vertices = Vec::new();
        let mut remap = BTreeMap::new();
        for face in &mut faces {
            for i in &mut face.indices {
                *i = *remap.entry(*i).or_insert_with(|| {
                    let n = vertices.len() as u32;
                    vertices.push(positions[*i as usize]);
                    n
                });
            }
        }
        let solid = Solid { vertices, faces };
        solid.check(budget)?;
        if solid.properties()?.0 <= 0. {
            return Err(Error::Geometry("nonpositive fragment volume"));
        }
        result.push(solid);
    }
    let right = result.pop().unwrap();
    let left = result.pop().unwrap();
    Ok(Some([left, right]))
}

// Projection roundoff can make three collinear boundary vertices appear as a
// tiny CDT face. Flip its longest interior edge so the middle boundary vertex
// subdivides the neighboring face instead; never discard a required boundary.
fn repair_collinear(faces: &mut Vec<[u32; 3]>, positions: &[DVec3], budget: &mut Budget) -> Result<(), Error> {
    for _ in 0..faces.len().saturating_mul(2) {
        budget.work(faces.len())?;
        let Some(i) = faces.iter().position(|f| {
            let [a, b, c] = f.map(|j| positions[j as usize]);
            (b - a).cross(c - a).length_squared() <= 1e-28
        }) else {
            return Ok(());
        };
        let f = faces[i];
        let edge = (0..3)
            .max_by(|&a, &b| {
                (positions[f[a] as usize] - positions[f[(a + 1) % 3] as usize])
                    .length_squared()
                    .total_cmp(&(positions[f[b] as usize] - positions[f[(b + 1) % 3] as usize]).length_squared())
            })
            .unwrap();
        let (a, b, c) = (f[edge], f[(edge + 1) % 3], f[(edge + 2) % 3]);
        budget.work(faces.len())?;
        let neighbor = faces.iter().enumerate().find_map(|(j, g)| {
            if j == i {
                return None;
            }
            (0..3).find_map(|k| (g[k] == b && g[(k + 1) % 3] == a).then_some((j, g[(k + 2) % 3])))
        });
        let Some((j, d)) = neighbor else {
            // A numerical sliver on the CDT's convex hull has no face across
            // its longest edge. The two shorter edges belong to the interior
            // triangulation and retain the original boundary subdivision.
            faces.remove(i);
            continue;
        };
        faces[i] = [a, d, c];
        faces[j] = [c, d, b];
    }
    Err(Error::Geometry("cap triangulation did not converge"))
}
