//! Timed crater configuration shared by rendering and simulation.
use crate::{
    sim::{curve_of, num},
    FrameNode,
};
use sr_model::element::{children, Element};

pub struct Deformation {
    pub kernel: sr_3d::crater::Crater,
    pub progress: f64,
    pub max_bytes: usize,
}

pub fn at(node: &FrameNode) -> Result<Option<Deformation>, String> {
    from_element(&*node.elem, node.local_time)
}

pub(crate) fn from_element(owner: &dyn Element, time: f64) -> Result<Option<Deformation>, String> {
    let Some(c) = children(owner).into_iter().find(|c| c.element_name() == "crater") else { return Ok(None) };
    let radius = num(c, "radius", 50.);
    let depth = num(c, "depth", 10.);
    let rim_height = num(c, "rimHeight", 2.);
    let kernel = sr_3d::crater::Crater::new(sr_3d::crater::Spec {
        center: [num(c, "centerX", 0.), num(c, "centerY", 0.), num(c, "centerZ", 0.)],
        outward: [num(c, "normalX", 0.), num(c, "normalY", 0.), num(c, "normalZ", -1.)],
        radius,
        depth,
        rim_height,
        rim_width: num(c, "rimWidth", 10.),
        influence_depth: num(c, "influenceDepth", 2. * radius.max(depth).max(rim_height)),
    })?;
    let start = num(c, "start", 0.);
    let end = num(c, "end", 1.);
    if !time.is_finite() || !start.is_finite() || !end.is_finite() || end <= start {
        return Err("crater requires finite local time and an ordered growth interval".into());
    }
    let progress = if time <= start {
        0.
    } else if time >= end {
        1.
    } else {
        curve_of(c, "curve").apply((time - start) / (end - start))
    };
    let max_bytes =
        (num(c, "maxMemoryMiB", 128.) as usize).checked_mul(1 << 20).ok_or("crater memory budget overflow")?;
    Ok(Some(Deformation { kernel, progress, max_bytes }))
}
