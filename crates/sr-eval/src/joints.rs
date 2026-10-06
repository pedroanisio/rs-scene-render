//! Joint sockets: the frame of a named node of an imported model, as the object's clips pose it, for elements that are
//! parented to that joint (`@parentJoint`, `transformConstraint/@targetJoint`).

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use glam::{DMat4, Mat4};
use sr_model::element::{children, Element};

use crate::{FrameGraph, FrameNode, Program};

/// Models loaded for joint lookups, by file and format; they live as long as their compiled scene.
pub(crate) type Cache = Mutex<HashMap<(PathBuf, Option<String>), Arc<Result<Arc<sr_3d::Model>, String>>>>;

/// Per object id, the joint names some element is attached to.
pub(crate) type Sockets = HashMap<String, Vec<String>>;

fn text(e: &dyn Element, name: &str) -> Option<String> {
    crate::sim::text(e, name).filter(|s| !s.is_empty())
}

/// The objects that are joint parents and the joints asked of each, from every element and light of the scene.
pub(crate) fn sockets(p: &Program) -> Sockets {
    let mut want: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut visit = |e: &dyn Element| {
        let kids = children(e);
        let constraints = || kids.iter().copied().filter(|c| c.element_name() == "transformConstraint");
        if let Some(joint) = text(e, "parentJoint") {
            let parent = text(e, "parent").or_else(|| {
                constraints().find_map(|c| (text(c, "type").as_deref() == Some("parent")).then(|| text(c, "target")).flatten())
            });
            if let Some(parent) = parent {
                want.entry(parent).or_default().insert(joint);
            }
        }
        for c in constraints() {
            if let (Some(joint), Some(target)) = (text(c, "targetJoint"), text(c, "target")) {
                want.entry(target).or_default().insert(joint);
            }
        }
    };
    for n in &p.nodes {
        visit(&*n.elem);
    }
    if let Some(lights) = &p.scene.lights {
        for l in &lights.lights {
            visit(l);
        }
    }
    want.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect()
}

fn load(p: &Program, n: &FrameNode) -> Result<Arc<sr_3d::Model>, String> {
    let key = n.asset.as_deref().map(str::to_string).or_else(|| text(&*n.elem, "mesh")).ok_or("no @mesh")?;
    let (path, format) = crate::sim3d::mesh_path(p, &key)?;
    let entry = p
        .joint_models
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry((path.clone(), format.clone()))
        .or_insert_with(|| {
            Arc::new(match sr_3d::import::load(&path, format.as_deref()) {
                Ok(sr_3d::Asset::Model(m)) => Ok(Arc::new(m)),
                Ok(_) => Err("not a model".to_string()),
                Err(e) => Err(e.to_string()),
            })
        })
        .clone();
    (*entry).clone()
}

/// The model-space-to-object frame matrices (axis conversion, node selection, pose) of the joints `names` of object `n`.
fn frames(p: &Program, n: &FrameNode, names: &[String], problems: &mut Vec<String>) -> Vec<(String, [f64; 16])> {
    let model = match load(p, n) {
        Ok(m) => m,
        Err(e) => {
            problems.push(format!("{}: parentJoint needs an object that draws an imported model ({e})", n.id));
            return Vec::new();
        }
    };
    let value = |key, default| {
        n.props.get(key).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(&*n.elem, key, default))
    };
    let find = |name: &str| {
        model
            .animations
            .iter()
            .find(|c| c.name == name)
            .or_else(|| name.parse::<usize>().ok().and_then(|k| model.animations.get(k)))
    };
    let mut clips = [None, None];
    for (slot, attr) in ["animationClip", "animationClipTo"].into_iter().enumerate() {
        if let Some(name) = text(&*n.elem, attr) {
            clips[slot] = find(&name);
        }
    }
    let at = |clip: Option<&sr_3d::Animation>, offset: &'static str| {
        let t = (n.local_time * value("animationSpeed", 1.) + value(offset, 0.)) as f32;
        clip.map_or(0., |c| if c.duration > 0. { t.rem_euclid(c.duration) } else { 0. })
    };
    let (mut locals, _) = sr_3d::anim::pose_blend(
        &model,
        clips[0],
        at(clips[0], "animationOffset"),
        clips[1],
        at(clips[1], "animationOffsetTo"),
        value("animationBlend", 0.).clamp(0., 1.) as f32,
    );
    // `<joint>` rotation offsets, as the renderer applies them (a look-at needs the 3D scene and is the renderer's)
    for (k, c) in children(&*n.elem).into_iter().filter(|c| c.element_name() == "joint").enumerate() {
        let Some(name) = text(c, "name") else { continue };
        let Some(node) = model.nodes.iter().position(|x| x.name == name) else { continue };
        let key = format!("{}/joint[{k}]", n.id);
        let props = n.parts.iter().find(|p| *p.key == key).map(|p| &p.props);
        let angle = |attr: &str| {
            props.and_then(|p| p.get(attr)).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(c, attr, 0.)) as f32
        };
        sr_3d::anim::pose_joint(&mut locals, node, sr_3d::anim::euler_degrees(angle("rotationX"), angle("rotationY"), angle("rotation")));
    }
    let world = model.world_matrices(&locals);
    let select = match text(&*n.elem, "node") {
        Some(name) => model.nodes.iter().position(|x| x.name == name).map(|k| world[k].inverse()),
        None => Some(Mat4::IDENTITY),
    }
    .unwrap_or(Mat4::IDENTITY);
    // the uniform scale of the source-to-scene conversion (scene units per model unit)
    let unit = model.basis.x_axis.truncate().length();
    let unit = if unit.is_finite() && unit > 0.0 { unit } else { 1.0 };
    let mut out = Vec::new();
    for name in names {
        match model.nodes.iter().position(|x| &x.name == name) {
            Some(k) => {
                // the model's unit conversion is not inherited: a child's own transform is in scene units
                let m = model.basis * select * world[k] * Mat4::from_scale(glam::Vec3::splat(unit.recip()));
                let m = DMat4::from_cols_array(&m.to_cols_array().map(f64::from));
                out.push((name.clone(), m.to_cols_array()));
            }
            None => problems.push(format!("{}: the model has no joint named {name}", n.id)),
        }
    }
    out
}

/// Fills `joints` of every object that is a joint parent.
pub(crate) fn attach(p: &Program, g: &mut FrameGraph) {
    let wanted = p.joint_sockets.get_or_init(|| sockets(p));
    if wanted.is_empty() {
        return;
    }
    let mut problems = Vec::new();
    for i in 0..g.nodes.len() {
        let Some(names) = wanted.get(&*g.nodes[i].id) else { continue };
        let found = frames(p, &g.nodes[i], names, &mut problems);
        g.nodes[i].joints = Some(Arc::new(found));
    }
    g.problems.extend(problems);
}
