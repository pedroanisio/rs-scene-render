//! Bounded sweep-and-prune validation of source surface intersections.
use super::{Budget, Error, Solid};
use glam::{DVec2, DVec3};
const EPS: f64 = 1e-12;

pub(super) fn intersections(s: &Solid, budget: &mut Budget) -> Result<(), Error> {
    let mut bounds: Vec<_> = s
        .faces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let p = f.indices.map(|i| s.vertices[i as usize]);
            (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]), i)
        })
        .collect();
    bounds.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.2.cmp(&b.2)));
    for i in 0..bounds.len() {
        let (lo, hi, first) = bounds[i];
        for &(other_lo, other_hi, second) in &bounds[i + 1..] {
            budget.work(1)?;
            if other_lo.x > hi.x + EPS {
                break;
            }
            if other_hi.y < lo.y - EPS || other_lo.y > hi.y + EPS || other_hi.z < lo.z - EPS || other_lo.z > hi.z + EPS
            {
                continue;
            }
            let fa = &s.faces[first];
            let fb = &s.faces[second];
            let a = fa.indices.map(|i| s.vertices[i as usize]);
            let b = fb.indices.map(|i| s.vertices[i as usize]);
            let common: Vec<_> =
                fa.indices.iter().filter(|i| fb.indices.contains(i)).map(|&i| s.vertices[i as usize]).collect();
            let allowed = |p: DVec3| {
                if common.iter().any(|q| p.distance_squared(*q) <= EPS * EPS) {
                    return true;
                }
                if common.len() == 2 {
                    let edge = common[1] - common[0];
                    let t = (p - common[0]).dot(edge) / edge.length_squared();
                    return (-EPS..=1. + EPS).contains(&t) && p.distance_squared(common[0] + t * edge) <= EPS * EPS;
                }
                false
            };
            let normal = (a[1] - a[0]).cross(a[2] - a[0]).normalize();
            if b.iter().all(|p| (*p - a[0]).dot(normal).abs() <= EPS) {
                if coplanar_overlap(a, b, normal) > 1e-22 {
                    return Err(Error::Invalid("overlapping coplanar source faces"));
                }
            } else if common.len() < 2 {
                // Two noncoplanar triangles sharing an edge intersect only on
                // that edge. Recomputing that known intersection divides by
                // a near-zero plane distance on almost planar tessellations.
                for (from, to) in [(a, b), (b, a)] {
                    for k in 0..3 {
                        for point in segment_hits(from[k], from[(k + 1) % 3], to) {
                            if !allowed(point) {
                                return Err(Error::Invalid("self-intersecting source surface"));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
fn project(p: DVec3, n: DVec3) -> DVec2 {
    let n = n.abs();
    if n.x >= n.y && n.x >= n.z {
        DVec2::new(p.y, p.z)
    } else if n.y >= n.z {
        DVec2::new(p.x, p.z)
    } else {
        DVec2::new(p.x, p.y)
    }
}
fn in_triangle(p: DVec3, t: [DVec3; 3], n: DVec3) -> bool {
    let q = project(p, n);
    let t = t.map(|p| project(p, n));
    let sign = (t[1] - t[0]).perp_dot(t[2] - t[0]).signum();
    (0..3).all(|i| sign * (t[(i + 1) % 3] - t[i]).perp_dot(q - t[i]) >= -EPS)
}
fn segment_hits(a: DVec3, b: DVec3, t: [DVec3; 3]) -> Vec<DVec3> {
    let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize();
    let da = (a - t[0]).dot(n);
    let db = (b - t[0]).dot(n);
    if da.abs() <= EPS && db.abs() <= EPS {
        let mut points = Vec::new();
        if in_triangle(a, t, n) {
            points.push(a)
        }
        if in_triangle(b, t, n) {
            points.push(b)
        }
        let p = project(a, n);
        let r = project(b, n) - p;
        for i in 0..3 {
            let q = project(t[i], n);
            let s = project(t[(i + 1) % 3], n) - q;
            let denom = r.perp_dot(s);
            if denom.abs() < 1e-24 {
                continue;
            }
            let u = (q - p).perp_dot(s) / denom;
            let v = (q - p).perp_dot(r) / denom;
            if (0. ..=1.).contains(&u) && (0. ..=1.).contains(&v) {
                points.push(a.lerp(b, u));
            }
        }
        points
    } else if (da > EPS && db > EPS) || (da < -EPS && db < -EPS) || da == db {
        Vec::new()
    } else {
        // Preserve exact incident vertices. Near-coplanar edges otherwise
        // amplify roundoff and invent intersections beside a shared vertex.
        let p = if da.abs() <= EPS {
            a
        } else if db.abs() <= EPS {
            b
        } else {
            a.lerp(b, (da / (da - db)).clamp(0., 1.))
        };
        if in_triangle(p, t, n) {
            vec![p]
        } else {
            Vec::new()
        }
    }
}
fn coplanar_overlap(a: [DVec3; 3], b: [DVec3; 3], normal: DVec3) -> f64 {
    let b = b.map(|p| project(p, normal));
    let mut polygon = a.map(|p| project(p, normal)).to_vec();
    let sign = (b[1] - b[0]).perp_dot(b[2] - b[0]).signum();
    for i in 0..3 {
        let mut next = Vec::new();
        let edge = b[(i + 1) % 3] - b[i];
        for j in 0..polygon.len() {
            let a = polygon[j];
            let c = polygon[(j + 1) % polygon.len()];
            let da = sign * edge.perp_dot(a - b[i]);
            let dc = sign * edge.perp_dot(c - b[i]);
            if da >= 0. {
                next.push(a)
            }
            if (da < 0. && dc > 0.) || (da > 0. && dc < 0.) {
                next.push(a.lerp(c, da / (da - dc)));
            }
        }
        polygon = next;
    }
    if polygon.len() < 3 {
        return 0.;
    }
    let origin = polygon[0];
    (1..polygon.len() - 1).map(|i| (polygon[i] - origin).perp_dot(polygon[i + 1] - origin)).sum::<f64>().abs() * 0.5
}

/// Connected boundary shells, grouped into physical solids. An inward shell is
/// a cavity of its nearest containing outward shell; disconnected outer shells
/// become distinct fragments. Reject nested shells with inconsistent winding.
pub(super) fn separate(s: Solid, budget: &mut Budget) -> Result<Vec<Solid>, Error> {
    budget.work(s.faces.len())?;
    let mut neighbors = vec![Vec::new(); s.faces.len()];
    let mut owners = std::collections::BTreeMap::<[u32; 2], usize>::new();
    for (i, f) in s.faces.iter().enumerate() {
        for k in 0..3 {
            let mut edge = [f.indices[k], f.indices[(k + 1) % 3]];
            edge.sort();
            if let Some(&j) = owners.get(&edge) {
                neighbors[i].push(j);
                neighbors[j].push(i);
            } else {
                owners.insert(edge, i);
            }
        }
    }
    let mut seen = vec![false; s.faces.len()];
    let mut shells = Vec::new();
    for first in 0..s.faces.len() {
        if seen[first] {
            continue;
        }
        seen[first] = true;
        let mut queue = vec![first];
        let mut faces = Vec::new();
        while let Some(i) = queue.pop() {
            faces.push(s.faces[i].clone());
            for &j in &neighbors[i] {
                if !seen[j] {
                    seen[j] = true;
                    queue.push(j);
                }
            }
        }
        shells.push(compact(&s.vertices, faces));
    }
    let volumes = shells.iter().map(|s| s.properties().map(|p| p.0)).collect::<Result<Vec<_>, _>>()?;
    let mut parents = vec![None; shells.len()];
    for (i, shell) in shells.iter().enumerate() {
        let probe = shell.faces[0].indices.iter().map(|&j| shell.vertices[j as usize]).sum::<DVec3>() / 3.;
        let mut depth = 0;
        let mut parent_volume = f64::INFINITY;
        for (j, other) in shells.iter().enumerate() {
            if i == j {
                continue;
            }
            budget.work(other.faces.len())?;
            if contains(other, probe) {
                depth += 1;
                if volumes[j].abs() < parent_volume {
                    parents[i] = Some(j);
                    parent_volume = volumes[j].abs();
                }
            }
        }
        if (volumes[i] > 0.) != (depth % 2 == 0) {
            return Err(Error::Invalid("nested source shells require alternating outward/cavity orientation"));
        }
    }
    let mut solids = Vec::new();
    for i in 0..shells.len() {
        if volumes[i] < 0. {
            continue;
        }
        let mut faces = shells[i].faces.clone();
        let mut vertices = shells[i].vertices.clone();
        for (j, shell) in shells.iter().enumerate() {
            if parents[j] != Some(i) || volumes[j] > 0. {
                continue;
            }
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&shell.vertices);
            faces.extend(
                shell.faces.iter().map(|f| super::Face { indices: f.indices.map(|i| i + base), source: f.source }),
            );
        }
        solids.push(Solid { vertices, faces });
    }
    Ok(solids)
}
fn compact(points: &[DVec3], mut faces: Vec<super::Face>) -> Solid {
    let mut map = std::collections::BTreeMap::new();
    let mut vertices = Vec::new();
    for f in &mut faces {
        for i in &mut f.indices {
            *i = *map.entry(*i).or_insert_with(|| {
                let id = vertices.len() as u32;
                vertices.push(points[*i as usize]);
                id
            });
        }
    }
    Solid { vertices, faces }
}
fn contains(s: &Solid, point: DVec3) -> bool {
    let (lo, hi) = s.bounds();
    if point.cmplt(lo).any() || point.cmpgt(hi).any() {
        return false;
    }
    let mut angle = 0.;
    for f in &s.faces {
        let [a, b, c] = f.indices.map(|i| s.vertices[i as usize] - point);
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
