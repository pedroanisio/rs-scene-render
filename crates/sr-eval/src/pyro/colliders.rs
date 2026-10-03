//! Rigid and crater collider geometry with substep boundary motion in domain axes.

use super::*;
use glam::DMat3;

pub(super) struct Colliders {
    surfaces: Vec<Collider>,
    fracture_budget: usize,
}
impl Colliders {
    pub(super) fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}
struct Pieces {
    key: u64,
    shapes: Vec<Arc<pyro::mesh::Mesh>>,
    bytes: usize,
}
struct Collider {
    id: Arc<str>,
    surface: Surface,
    pieces: Option<Pieces>,
}

enum Surface {
    Rigid(Shape),
    Crater { points: Vec<[f64; 3]>, triangles: Vec<[u32; 3]>, budget: usize },
}

pub(super) fn build(
    p: &Program,
    e: &sr_model::model::Pyro,
    remaining: &mut usize,
    assets: &mut HashMap<MeshRef, Arc<pyro::mesh::Mesh>>,
) -> Result<Colliders, String> {
    let list = text(e, "colliders").unwrap_or_default();
    if list.split_whitespace().count() > 4096 {
        return Err("at most 4096 pyro colliders".into());
    }
    let thickness = num(e, "colliderThickness", 2.0 * num(e, "voxelSize", 1.0));
    let mut result = Vec::new();
    for id in list.split_whitespace() {
        let node = p.nodes.iter().find(|n| &*n.id == id).ok_or_else(|| format!("collider {id} is not instantiated"))?;
        let n = &*node.elem;
        if let Some(crater) = crate::crater::from_element(n, 0.0)? {
            let allowance = (*remaining).min(crater.max_bytes);
            let (points, triangles) = geometry(p, node, thickness, allowance)?;
            let budget = geometry_bytes(points.len(), triangles.len())?;
            if budget > allowance {
                return Err("crater pyro geometry memory budget".into());
            }
            *remaining -= budget;
            result.push(Collider {
                id: node.id.clone(),
                surface: Surface::Crater { points, triangles, budget },
                pieces: None,
            });
            continue;
        }
        let r = num(n, "radius", 50.0);
        let w = num(n, "width", 2.0 * r);
        let h = num(n, "height", 2.0 * r);
        let shape = match text(n, "primitive").as_deref() {
            Some("box") => {
                let half = [w * 0.5, h * 0.5, num(n, "depth", 10.0) * 0.5];
                Shape::Box { min: half.map(|v| -v), max: half }
            }
            Some("plane") => {
                let half = [w * 0.5, h * 0.5, thickness * 0.5];
                Shape::Box { min: half.map(|v| -v), max: half }
            }
            Some("globe") if text(n, "terrain").is_some() => {
                Shape::Mesh(load_mesh(p, &MeshRef::Globe(node.id.clone()), remaining, assets)?)
            }
            Some("sphere" | "globe") => Shape::Sphere { center: [0.0; 3], radius: r },
            Some("cylinder") => Shape::Cylinder { radius: r, half_height: h * 0.5 },
            Some("cone") => Shape::Cone { radius: r, half_height: h * 0.5 },
            Some("capsule") => {
                Shape::Capsule { radius: r, half_segment: (num(n, "height", 4.0 * r) * 0.5 - r).max(0.0) }
            }
            Some("torus") => {
                Shape::Torus { major_radius: r, minor_radius: num(n, "height", 0.7 * r).min(2.0 * r) * 0.5 }
            }
            Some("mesh") => {
                let asset = node.asset.clone().ok_or_else(|| format!("collider {id} has no mesh asset"))?;
                Shape::Mesh(load_mesh(p, &MeshRef::Asset(asset), remaining, assets)?)
            }
            Some("text" | "clay" | "extrude") => {
                let (points, triangles) = crate::solid::collider_triangles(p, node, *remaining)?;
                let mesh = pyro::mesh::Mesh::new(&points, &triangles, *remaining).map_err(|e| e.to_string())?;
                *remaining = remaining.checked_sub(mesh.bytes()).ok_or("solid collider memory budget")?;
                Shape::Mesh(Arc::new(mesh))
            }
            _ => return Err(format!("collider {id} requires a supported rigid surface primitive")),
        };
        result.push(Collider { id: node.id.clone(), surface: Surface::Rigid(shape), pieces: None });
    }
    Ok(Colliders { surfaces: result, fracture_budget: *remaining })
}

pub(super) fn apply(
    input: &mut Inputs,
    colliders: &mut Colliders,
    frame: &FrameGraph,
    next: &FrameGraph,
    owner: &str,
    dt: f64,
) -> Result<(), pyro::Error> {
    let Some(domain) = frame.nodes.iter().position(|n| &*n.id == owner) else { return Ok(()) };
    let world = crate::sim3d::world3(frame, domain, 0);
    let inverse = inverse_transform(world)?;
    let next_inverse = next
        .nodes
        .iter()
        .position(|n| &*n.id == owner)
        .map(|i| inverse_transform(crate::sim3d::world3(next, i, 0)))
        .transpose()?
        .unwrap_or(inverse);
    let mut remaining = colliders.fracture_budget;
    for collider in &mut colliders.surfaces {
        let key =
            frame.nodes.iter().find(|n| n.id == collider.id).and_then(|n| n.fracture.as_ref()).map(|f| f.geometry.key);
        if collider.pieces.as_ref().is_some_and(|p| Some(p.key) != key) {
            collider.pieces = None;
        }
        if let Some(pieces) = &collider.pieces {
            remaining =
                remaining.checked_sub(pieces.bytes).ok_or(pyro::Error::Limit("fracture collider memory budget"))?;
        }
    }
    for collider in &mut colliders.surfaces {
        // Render visibility does not disable an explicitly referenced proxy;
        // start/end windows and conditions do, through presence in this frame.
        let Some(i) = frame.nodes.iter().position(|n| n.id == collider.id) else { continue };
        if let Some(fracture) = &frame.nodes[i].fracture {
            if collider.pieces.is_none() {
                let mut shapes = Vec::new();
                let mut bytes = 0;
                for piece in &fracture.geometry.pieces {
                    let sr_sim::physics3d::Shape3::Decomposition(points, triangles) = &piece.shape else {
                        return Err(pyro::Error::Invalid("fracture collider surface"));
                    };
                    let shape = pyro::mesh::Mesh::new(points, triangles, remaining)?;
                    remaining -= shape.bytes();
                    bytes += shape.bytes();
                    shapes.push(Arc::new(shape));
                }
                collider.pieces = Some(Pieces { key: fracture.geometry.key, shapes, bytes });
            }
            let future = next
                .nodes
                .iter()
                .find(|n| n.id == collider.id)
                .and_then(|n| n.fracture.as_ref())
                .filter(|f| f.geometry.key == fracture.geometry.key);
            let matrix = |pose: sr_sim::physics3d::Pose3| {
                DMat4::from_rotation_translation(glam::DQuat::from_array(pose.rot), DVec3::from_array(pose.pos))
            };
            for (j, shape) in collider.pieces.as_ref().expect("fracture shapes").shapes.iter().enumerate() {
                if !fracture.enabled[j] {
                    continue;
                }
                let a = fracture.poses[j];
                let b = future.filter(|f| f.enabled[j]).map_or(a, |f| f.poses[j]);
                input.obstacles.push(moving(
                    Shape::Mesh(shape.clone()),
                    inverse * matrix(a),
                    next_inverse * matrix(b),
                    dt,
                )?);
            }
            continue;
        }
        let current = inverse * crate::sim3d::world3(frame, i, 0);
        let future = next
            .nodes
            .iter()
            .position(|n| n.id == collider.id)
            .map(|i| next_inverse * crate::sim3d::world3(next, i, 0))
            .unwrap_or(current);
        match &collider.surface {
            Surface::Rigid(shape) => input.obstacles.push(moving(shape.clone(), current, future, dt)?),
            Surface::Crater { points, triangles, budget } => {
                let endpoint = |node: &FrameNode, matrix: DMat4| -> Result<Vec<[f64; 3]>, pyro::Error> {
                    sr_volume::Transform::new(matrix.to_cols_array())?;
                    let crater = crate::crater::at(node)
                        .map_err(|_| pyro::Error::Invalid("crater pyro configuration"))?
                        .ok_or(pyro::Error::Invalid("crater pyro configuration missing"))?;
                    points
                        .iter()
                        .map(|&p| {
                            let q = crater
                                .kernel
                                .map(p, crater.progress)
                                .map_err(|_| pyro::Error::Invalid("crater pyro deformation"))?
                                .position;
                            Ok(matrix.transform_point3(DVec3::from_array(q)).to_array())
                        })
                        .collect()
                };
                let next_node = next.nodes.iter().find(|n| n.id == collider.id).unwrap_or(&frame.nodes[i]);
                let start = endpoint(&frame.nodes[i], current)?;
                let end = endpoint(next_node, future)?;
                let mesh = pyro::mesh::Mesh::moving(&start, &end, triangles, dt, *budget)?;
                input.obstacles.push(pyro::Obstacle::stationary(Shape::Mesh(Arc::new(mesh))));
            }
        }
    }
    Ok(())
}

fn moving(shape: Shape, current: DMat4, future: DMat4, dt: f64) -> Result<pyro::Obstacle, pyro::Error> {
    let transform = sr_volume::Transform::new(current.to_cols_array())?;
    sr_volume::Transform::new(future.to_cols_array())?;
    // For a material point q, p0=A0*q+t0 and p1=A1*q+t1. Set the
    // midpoint velocity (p1-p0)/dt = G*((p0+p1)/2 - tm) + (t1-t0)/dt.
    // Thus G=(A1-A0)*inverse((A1+A0)/2)/dt. For a pure rotation G is
    // skew-symmetric, so it does not inject spurious volume expansion.
    let a0 = DMat3::from_mat4(current);
    let a1 = DMat3::from_mat4(future);
    let midpoint = a0 * 0.5 + a1 * 0.5;
    let determinant = midpoint.determinant();
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(pyro::Error::Invalid("collider midpoint motion is singular or nonfinite; reduce the fixed step"));
    }
    let gradient = (a1 - a0) * midpoint.inverse() / dt;
    let t0 = current.w_axis.truncate();
    let t1 = future.w_axis.truncate();
    let origin = t0 * 0.5 + t1 * 0.5;
    let velocity = (t1 - t0) / dt;
    if !gradient.is_finite() || !velocity.is_finite() || !origin.is_finite() {
        return Err(pyro::Error::Invalid("collider midpoint motion is singular or nonfinite; reduce the fixed step"));
    }
    Ok(pyro::Obstacle {
        shape: Shape::Transformed { shape: Box::new(shape), transform: Box::new(transform) },
        velocity: velocity.to_array(),
        velocity_origin: origin.to_array(),
        velocity_gradient: gradient.transpose().to_cols_array_2d(),
    })
}

/// Closed topology for voxel regions: seams share indices and polar vertices
/// are single points, avoiding degenerate render triangles at UV seams/poles.
fn geometry(
    p: &Program,
    node: &crate::program::InstNode,
    thickness: f64,
    budget: usize,
) -> Result<crate::sim3d::Triangles, String> {
    let e = &*node.elem;
    let kind = text(e, "primitive").ok_or("crater collider primitive missing")?;
    if matches!(kind.as_str(), "text" | "clay" | "extrude") {
        return crate::solid::collider_triangles(p, node, budget);
    }
    if kind == "mesh" {
        return crate::sim3d::mesh_asset_triangles(p, node.asset.as_deref().ok_or("collider mesh missing")?, budget);
    }
    if kind == "globe" && text(e, "terrain").is_some() {
        return crate::terrain::collider_triangles(p, node, budget);
    }
    let segments = num(e, "segments", 32.)
        .clamp(if kind == "plane" { 1. } else { 3. }, if kind == "plane" { 1024. } else { 512. })
        as u32;
    let r = num(e, "radius", 50.);
    let s = segments as usize;
    let rings = (s / 2).max(2);
    let (vertices, triangles) = match kind.as_str() {
        "plane" => (2 * (s + 1) * (s + 1), 4 * s * s + 8 * s),
        "box" => (24, 12),
        "sphere" | "globe" => (s * (rings - 1) + 2, 2 * s * (rings - 1)),
        "capsule" => {
            let rows = 2 * rings - usize::from(num(e, "height", 4. * r) <= 2. * r);
            (s * rows + 2, 2 * s * rows)
        }
        "cylinder" => (2 * s + 2, 4 * s),
        "cone" => (s + 2, 2 * s),
        "torus" => (s * rings.max(3), 2 * s * rings.max(3)),
        _ => return Err("unsupported crater pyro collider primitive".into()),
    };
    if geometry_bytes(vertices, triangles)? > budget {
        return Err("crater pyro tessellation exceeds geometry budget".into());
    }
    let w = num(e, "width", 2. * r);
    let h = num(e, "height", 2. * r);
    if kind == "box" {
        let mesh = sr_3d::prim::cuboid(w as f32, h as f32, num(e, "depth", 10.) as f32);
        return Ok((
            mesh.vertices.iter().map(|v| v.pos.map(f64::from)).collect(),
            mesh.indices.as_chunks::<3>().0.to_vec(),
        ));
    }
    if kind == "plane" {
        let mesh = sr_3d::prim::plane(w as f32, h as f32, segments);
        let mut points: Vec<_> = mesh
            .vertices
            .iter()
            .map(|v| {
                let p = v.pos.map(f64::from);
                [p[0], p[1], p[2] - 0.5 * thickness]
            })
            .collect();
        let count = points.len() as u32;
        for i in 0..count as usize {
            let p = points[i];
            points.push([p[0], p[1], p[2] + thickness]);
        }
        let mut triangles = mesh.indices.as_chunks::<3>().0.to_vec();
        let mut edges = std::collections::BTreeMap::new();
        for &[a, b, c] in &triangles {
            for (a, b) in [(a, b), (b, c), (c, a)] {
                if edges.remove(&(b, a)).is_none() {
                    edges.insert((a, b), ());
                }
            }
        }
        triangles.extend(mesh.indices.as_chunks::<3>().0.iter().map(|&[a, b, c]| [a + count, c + count, b + count]));
        for ((a, b), ()) in edges {
            triangles.extend([[b, a, a + count], [b, a + count, b + count]]);
        }
        return Ok((points, triangles));
    }
    let rings = (segments / 2).max(2);
    let mut profile = Vec::new();
    let cyclic = kind == "torus";
    match kind.as_str() {
        "sphere" | "globe" | "capsule" => {
            let half = if kind == "capsule" { (num(e, "height", 4. * r) * 0.5 - r).max(0.) } else { 0. };
            let steps = if kind == "capsule" { 2 * rings } else { rings };
            profile.push((0., -r - half));
            for j in 1..steps {
                let phi = std::f64::consts::PI * f64::from(j) / f64::from(steps);
                let y = -r * phi.cos();
                profile.push((r * phi.sin(), y + if j * 2 <= steps { -half } else { half }));
                if half > 0. && j * 2 == steps {
                    profile.push((r, half));
                }
            }
            profile.push((0., r + half));
        }
        "cylinder" => profile.extend([(0., -h * 0.5), (r, -h * 0.5), (r, h * 0.5), (0., h * 0.5)]),
        "cone" => profile.extend([(0., -h * 0.5), (r, h * 0.5), (0., h * 0.5)]),
        "torus" => {
            let tube = (num(e, "height", 0.7 * r) * 0.5).min(r);
            // A horn torus touches itself on its axis and cannot enclose a
            // manifold voxel solid at finite resolution.
            if tube >= r {
                return Err("crater pyro torus requires tube radius smaller than ring radius".into());
            }
            for j in 0..rings.max(3) {
                let angle = std::f64::consts::TAU * f64::from(j) / f64::from(rings.max(3));
                profile.push((r + tube * angle.cos(), tube * angle.sin()));
            }
        }
        _ => return Err("unsupported crater pyro collider primitive".into()),
    }
    let mut points = Vec::new();
    let mut rows = Vec::new();
    for (radius, y) in profile {
        let base = points.len() as u32;
        let count = if radius == 0. { 1 } else { segments };
        for i in 0..count {
            let angle = std::f64::consts::TAU * f64::from(i) / f64::from(segments);
            points.push([radius * angle.cos(), y, radius * angle.sin()]);
        }
        rows.push((base, count));
    }
    let mut triangles = Vec::new();
    for j in 0..(rows.len() - usize::from(!cyclic)) {
        let (first, n) = rows[j];
        let (second, m) = rows[(j + 1) % rows.len()];
        for i in 0..segments {
            let a = first + i % n;
            let b = first + (i + 1) % n;
            let c = second + i % m;
            let d = second + (i + 1) % m;
            if a != b {
                triangles.push([a, c, b]);
            }
            if c != d {
                triangles.push([b, c, d]);
            }
        }
    }
    Ok((points, triangles))
}

fn geometry_bytes(vertices: usize, triangles: usize) -> Result<usize, String> {
    vertices
        .checked_mul(512)
        .and_then(|v| triangles.checked_mul(800).and_then(|t| v.checked_add(t)))
        .and_then(|v| v.checked_add(8192))
        .ok_or_else(|| "crater pyro geometry size overflow".into())
}
