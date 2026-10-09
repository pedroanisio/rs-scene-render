//! Sample source deformation and preserve the renderer's exterior attributes.
use crate::{FrameNode, Program};
use glam::{DMat4, DVec3, Vec3};
use sr_3d::{Model, Primitive};
use sr_model::element::Element;
use std::sync::Arc;

/// The source's surfaces as frozen at `n`'s frame. `notes` receives what is reported without failing: an
/// `animationClipTo` that names no clip (SREP 42: the pose blends toward the rest pose).
pub(crate) fn load(
    p: &Program,
    n: &FrameNode,
    scale: [f64; 3],
    budget: usize,
    notes: &mut Vec<String>,
) -> Result<(Vec<Primitive>, Option<Arc<Model>>), String> {
    let value = |key, default| {
        n.props.get(key).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(&*n.elem, key, default))
    };
    let text = |key| {
        n.props
            .get(key)
            .and_then(|v| if let crate::Value::Str(v) = v { Some(v.to_string()) } else { None })
            .or_else(|| crate::sim::text(&*n.elem, key))
    };
    let scale = DMat4::from_scale(DVec3::from(scale));
    if !scale.is_finite() || scale.determinant() == 0. {
        return Err("fracture source has singular scale".into());
    }
    if text("primitive").as_deref() == Some("mesh") {
        let model = if let Some(sequence) = crate::mesh_sequence::sample(p, n)? {
            sequence.frame.model.ok_or("fracture source mesh sequence is transparent")?
        } else {
            let key = n.asset.as_deref().ok_or("fracture mesh asset missing")?;
            let (path, format) = crate::sim3d::mesh_path(p, key)?;
            if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > budget as u64 {
                return Err("fracture mesh file exceeds budget".into());
            }
            let sr_3d::Asset::Model(m) = sr_3d::import::load(&path, format.as_deref())? else {
                return Err("fracture requires a triangle solid".into());
            };
            Arc::new(m)
        };
        if sr_3d::sequence::bytes(&model) > budget {
            return Err("fracture imported model exceeds budget".into());
        }
        let find = |name: &String| {
            model
                .animations
                .iter()
                .find(|c| &c.name == name)
                .or_else(|| name.parse::<usize>().ok().and_then(|i| model.animations.get(i)))
        };
        let mut clips = Vec::new();
        for attr in ["animationClip", "animationClipTo"] {
            let name = text(attr);
            let clip = name.as_ref().and_then(find);
            match (&name, clip) {
                // SREP 42 Semantics 4: reported, and the pose blends toward the rest pose (`None`)
                (Some(want), None) if attr == "animationClipTo" => {
                    notes.push(format!("animationClipTo: {}", sr_3d::anim::unknown_clip_to_rest(want)))
                }
                (Some(_), None) => return Err("fracture animation clip not found".into()),
                _ => {}
            }
            clips.push(clip);
        }
        let at = |duration: Option<f32>, offset: &'static str| {
            let time = (n.local_time * value("animationSpeed", 1.) + value(offset, 0.)) as f32;
            duration.map_or(0., |d| if d > 0. { time.rem_euclid(d) } else { 0. })
        };
        let blend = value("animationBlend", 0.).clamp(0., 1.) as f32;
        let (locals, weights) = sr_3d::anim::pose_blend(
            &model,
            clips[0],
            at(clips[0].map(|c| c.duration), "animationOffset"),
            clips[1],
            at(clips[1].map(|c| c.duration), "animationOffsetTo"),
            blend,
        );
        let morph = match n.props.get("morphWeights") {
            Some(crate::Value::List(v)) => Some(v.iter().map(|v| *v as f32).collect::<Vec<_>>()),
            _ => match n.elem.get_attr("morphWeights") {
                Some(sr_model::element::AttrValue::Numbers(v)) => Some(v.into_iter().map(|v| v as f32).collect()),
                _ => None,
            },
        };
        let mut result = Vec::new();
        let mut count = 0usize;
        let variant = text("materialVariant");
        for item in sr_3d::anim::draw_list(&model, &locals, &weights, morph.as_deref()) {
            let original = &model.primitives[item.prim];
            count = count
                .saturating_add(original.vertices.len().saturating_mul(512))
                .saturating_add(original.indices.len().saturating_mul(16));
            if count > budget {
                return Err("fracture frozen mesh exceeds budget".into());
            }
            let mut mesh = original.clone();
            if let Some(vertices) = item.vertices {
                mesh.vertices = vertices;
            }
            mesh.material = variant
                .as_ref()
                .and_then(|v| mesh.variants.iter().find(|(name, _)| name == v).map(|(_, k)| *k))
                .or(mesh.material);
            if text("material").is_none() {
                if let Some(material) = mesh.material.and_then(|k| model.materials.get(k)) {
                    material.apply_texture_coordinates(original, &mut mesh.vertices);
                }
            }
            mesh.morphs.clear();
            mesh.joints.clear();
            mesh.weights.clear();
            transform(&mut mesh, (model.basis * item.matrix).as_dmat4())?;
            deform(n, &mut mesh, budget)?;
            transform(&mut mesh, scale)?;
            result.push(mesh);
        }
        return Ok((result, Some(model)));
    }
    let globe = text("primitive").as_deref() == Some("globe");
    let r = value("radius", if globe { 200. } else { 50. }) as f32;
    let w = value("width", 2. * r as f64) as f32;
    let h = value("height", 2. * r as f64) as f32;
    let depth = value("depth", 10.) as f32;
    let segs = value("segments", if globe { 96. } else { 32. }).clamp(if globe { 24. } else { 3. }, 512.) as u32;
    let trigonometric =
        matches!(text("primitive").as_deref(), Some("sphere" | "globe" | "cylinder" | "cone" | "capsule" | "torus"));
    if trigonometric && (segs as usize + 1).saturating_pow(2).saturating_mul(1024) > budget {
        return Err("fracture primitive exceeds budget".into());
    }
    let mut mesh = match text("primitive").as_deref().unwrap_or("box") {
        "box" => sr_3d::prim::cuboid(w, h, depth),
        "clay" => crate::solid::clay_mesh(n, budget)?,
        "text" => crate::solid::text_mesh(p, n, budget)?,
        "globe" if text("terrain").is_some() => crate::terrain::globe_with_budget(p, n, budget)?.mesh.clone(),
        "globe" => {
            let mut mesh = sr_3d::prim::sphere(r, segs);
            transform(&mut mesh, DMat4::from_rotation_y(-std::f64::consts::FRAC_PI_2))?;
            mesh
        }
        "sphere" => sr_3d::prim::sphere(r, segs),
        "cylinder" => sr_3d::prim::cylinder(r, r, h, segs),
        "cone" => sr_3d::prim::cylinder(0., r, h, segs),
        "capsule" => sr_3d::prim::capsule(r, value("height", 4. * r as f64).max(2. * r as f64) as f32, segs),
        "torus" => sr_3d::prim::torus(r, (value("height", 0.7 * r as f64) * 0.5).min(r as f64) as f32, segs),
        "extrude" => sr_3d::prim::extrude(
            &sr_3d::prim::path_polygons(&text("path").ok_or("fracture extrude has no path")?, 0.25)?,
            depth,
            value("bevel", 0.) as f32,
        )?,
        other => return Err(format!("fracture source primitive {other} has no solid geometry")),
    };
    // Procedural trigonometric seams carry f32 roundoff. Canonicalize only
    // generated meshes, never weld unrelated nearby vertices of imported solids.
    let extent = mesh.vertices.iter().flat_map(|v| v.pos).map(f32::abs).fold(0., f32::max);
    let quantum = extent * 1e-6;
    if trigonometric && quantum > 0. {
        for v in &mut mesh.vertices {
            v.pos = v.pos.map(|x| (x / quantum).round() * quantum);
        }
        mesh.indices = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| {
                let [a, b, c] = t.map(|i| Vec3::from(mesh.vertices[i as usize].pos));
                (b - a).cross(c - a).length_squared() > 0.
            })
            .flatten()
            .copied()
            .collect();
    }
    deform(n, &mut mesh, budget)?;
    transform(&mut mesh, scale)?;
    Ok((vec![mesh], None))
}

fn transform(mesh: &mut Primitive, matrix: DMat4) -> Result<(), String> {
    let normal = matrix.inverse().transpose();
    let sign = matrix.determinant().signum() as f32;
    for v in &mut mesh.vertices {
        v.pos = matrix.transform_point3(DVec3::from(v.pos.map(f64::from))).as_vec3().to_array();
        v.normal =
            normal.transform_vector3(DVec3::from(v.normal.map(f64::from))).normalize_or_zero().as_vec3().to_array();
        let t = matrix.transform_vector3(DVec3::new(v.tangent[0] as f64, v.tangent[1] as f64, v.tangent[2] as f64));
        let n = DVec3::from(v.normal.map(f64::from));
        let t = (t - n * t.dot(n)).normalize_or_zero();
        v.tangent = [t.x as f32, t.y as f32, t.z as f32, v.tangent[3] * sign];
        if v.pos.iter().chain(&v.normal).chain(&v.tangent).any(|v| !v.is_finite()) {
            return Err("fracture source exceeds render precision".into());
        }
    }
    if sign < 0. {
        for t in mesh.indices.as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
    }
    Ok(())
}

fn deform(n: &FrameNode, mesh: &mut Primitive, budget: usize) -> Result<(), String> {
    if let Some(crater) = crate::crater::at(n)? {
        mesh.vertices = crater.kernel.deform(&mesh.vertices, crater.progress, budget.min(crater.max_bytes))?;
    }
    Ok(())
}
