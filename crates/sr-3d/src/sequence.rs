//! Stateless mesh-cache time selection and topology-checked interpolation.
//! Decode errors are never treated as missing files. Loaders must independently
//! bound decoding; the budget here covers resident selected models and blending.
use crate::{Model, Vertex};
use glam::Vec3;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Hold,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    Error,
    Hold,
    Transparent,
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub model: Option<Arc<Model>>,
    pub opacity: f32,
    pub blend: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Sequence {
    first: i64,
    last: i64,
    fps: f64,
    interpolation: Interpolation,
    missing: Missing,
}
impl Sequence {
    pub fn new(
        first: i64,
        last: i64,
        fps: f64,
        interpolation: Interpolation,
        missing: Missing,
    ) -> Result<Self, String> {
        if !fps.is_finite()
            || fps <= 0.
            || !last.checked_sub(first).and_then(|n| n.checked_add(1)).is_some_and(|n| (1..=1_000_000).contains(&n))
        {
            return Err("mesh sequence requires finite positive fps and 1..1000000 ordered frames".into());
        }
        Ok(Self { first, last, fps, interpolation, missing })
    }
    pub fn sample(
        &self,
        time: f64,
        max_bytes: usize,
        mut loader: impl FnMut(i64) -> Result<Option<Arc<Model>>, String>,
    ) -> Result<Sample, String> {
        if !time.is_finite() {
            return Err("mesh sequence time must be finite".into());
        }
        let offset = (time * self.fps).clamp(0., (self.last - self.first) as f64);
        let first = self.first + offset.floor() as i64;
        let blend = if self.interpolation == Interpolation::Hold { 0. } else { offset.fract() };
        let a = self.resolve(first, &mut loader, None)?;
        let a_bytes = a.as_ref().map_or(0, |m| bytes(m));
        if a_bytes > max_bytes {
            return Err("mesh sequence exceeds resident budget".into());
        }
        if let Some(m) = &a {
            validate(m)?;
        }
        if blend == 0. || first == self.last {
            return Ok(Sample { opacity: if a.is_some() { 1. } else { 0. }, model: a, blend: 0. });
        }
        let b = self.resolve(first + 1, &mut loader, Some((first, &a)))?;
        let b_bytes = b.as_ref().map_or(0, |m| bytes(m));
        if let Some(m) = &b {
            validate(m)?;
        }
        match (a, b) {
            (None, None) => Ok(Sample { model: None, opacity: 0., blend }),
            (Some(a), None) => Ok(Sample { model: Some(a), opacity: (1. - blend) as f32, blend }),
            (None, Some(b)) if b_bytes <= max_bytes => Ok(Sample { model: Some(b), opacity: blend as f32, blend }),
            (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => Ok(Sample { model: Some(a), opacity: 1., blend }),
            (Some(a), Some(b)) if a_bytes.saturating_mul(2).saturating_add(b_bytes) <= max_bytes => {
                Ok(Sample { model: Some(Arc::new(interpolate(&a, &b, blend as f32)?)), opacity: 1., blend })
            }
            _ => Err("mesh sequence blend exceeds resident budget".into()),
        }
    }
    fn resolve(
        &self,
        mut label: i64,
        loader: &mut impl FnMut(i64) -> Result<Option<Arc<Model>>, String>,
        prior: Option<(i64, &Option<Arc<Model>>)>,
    ) -> Result<Option<Arc<Model>>, String> {
        loop {
            if let Some((previous, model)) = prior {
                if label == previous {
                    return Ok(model.clone());
                }
            }
            if let Some(model) = loader(label)? {
                return Ok(Some(model));
            }
            match self.missing {
                Missing::Error => return Err(format!("missing mesh sequence frame {label}")),
                Missing::Transparent => return Ok(None),
                Missing::Hold if label > self.first => label -= 1,
                Missing::Hold => return Err("missing mesh sequence has no preceding frame to hold".into()),
            }
        }
    }
}

/// Conservative retained allocation accounting, including importer metadata.
pub fn bytes(m: &Model) -> usize {
    fn vec<T>(v: &Vec<T>) -> usize {
        v.capacity().saturating_mul(std::mem::size_of::<T>())
    }
    let mut total = 4096usize;
    let mut add = |n: usize| total = total.saturating_add(n);
    add(vec(&m.nodes));
    add(vec(&m.primitives));
    add(vec(&m.materials));
    add(vec(&m.textures));
    add(vec(&m.skins));
    add(vec(&m.animations));
    add(vec(&m.warnings));
    for n in &m.nodes {
        add(n.name.capacity());
        add(vec(&n.primitives));
        add(vec(&n.weights));
    }
    for p in &m.primitives {
        add(vec(&p.vertices));
        add(vec(&p.indices));
        add(vec(&p.joints));
        add(vec(&p.weights));
        add(vec(&p.morphs));
        add(vec(&p.variants));
        for uv in p.tex_coords.values() {
            add(128);
            add(vec(uv));
        }
        for t in &p.morphs {
            add(vec(&t.dpos));
            add(vec(&t.dnormal));
        }
        for (name, _) in &p.variants {
            add(name.capacity());
        }
    }
    for t in &m.textures {
        add(vec(&t.rgba));
    }
    for t in &m.materials {
        add(t.name.capacity());
    }
    for s in &m.skins {
        add(vec(&s.joints));
        add(vec(&s.inverse_bind));
    }
    for a in &m.animations {
        add(a.name.capacity());
        add(vec(&a.channels));
        for c in &a.channels {
            add(vec(&c.times));
            add(vec(&c.values));
        }
    }
    for w in &m.warnings {
        add(w.capacity());
    }
    total
}

fn validate(m: &Model) -> Result<(), String> {
    if !m.basis.is_finite() {
        return Err("nonfinite mesh basis".into());
    }
    for n in &m.nodes {
        if !n.local.matrix().is_finite()
            || n.primitives.iter().any(|&p| p >= m.primitives.len())
            || n.parent.is_some_and(|p| p >= m.nodes.len())
            || n.skin.is_some_and(|s| s >= m.skins.len())
            || (n.local.r.length_squared() - 1.).abs() > 1e-3
        {
            return Err("invalid mesh node transform or reference".into());
        }
    }
    // Validate before the shared pose evaluator traverses the hierarchy.
    let mut state = vec![0u8; m.nodes.len()];
    let mut depth = vec![0u16; m.nodes.len()];
    let mut chain = Vec::new();
    for start in 0..m.nodes.len() {
        let mut at = Some(start);
        while let Some(i) = at {
            if state[i] == 2 {
                break;
            }
            if state[i] == 1 {
                return Err("mesh node hierarchy is cyclic".into());
            }
            if chain.len() >= 512 {
                return Err("mesh node hierarchy exceeds 512 levels".into());
            }
            state[i] = 1;
            chain.push(i);
            at = m.nodes[i].parent;
        }
        let mut levels = at.map_or(0, |i| depth[i]);
        for i in chain.drain(..).rev() {
            levels += 1;
            if levels > 512 {
                return Err("mesh node hierarchy exceeds 512 levels".into());
            }
            depth[i] = levels;
            state[i] = 2;
        }
    }
    for skin in &m.skins {
        if skin.joints.iter().any(|&j| j >= m.nodes.len()) || skin.inverse_bind.iter().any(|m| !m.is_finite()) {
            return Err("invalid mesh skin reference or transform".into());
        }
    }
    for material in &m.materials {
        let maps = &material.maps;
        if [maps.base_color, maps.normal, maps.metallic_roughness, maps.occlusion, maps.emissive]
            .into_iter()
            .flatten()
            .any(|i| i >= m.textures.len())
        {
            return Err("invalid mesh material texture reference".into());
        }
    }
    for t in &m.textures {
        if t.width == 0
            || t.height == 0
            || (t.width as usize).checked_mul(t.height as usize).and_then(|n| n.checked_mul(4)) != Some(t.rgba.len())
        {
            return Err("invalid mesh texture dimensions or pixels".into());
        }
    }
    for p in &m.primitives {
        if p.indices.len() % 3 != 0 || p.indices.iter().any(|&i| i as usize >= p.vertices.len()) {
            return Err("invalid mesh triangle indices".into());
        }
        if p.vertices
            .iter()
            .any(|v| bytemuck::cast_slice::<Vertex, f32>(std::slice::from_ref(v)).iter().any(|v| !v.is_finite()))
        {
            return Err("nonfinite mesh vertex attribute".into());
        }
    }
    Ok(())
}

fn mix<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|i| (f64::from(a[i]) * (1. - f64::from(t)) + f64::from(b[i]) * f64::from(t)) as f32)
}
fn direction(v: [f32; 3]) -> Result<Vec3, String> {
    Vec3::from(v).try_normalize().ok_or("mesh interpolation has an unresolved direction".into())
}
fn interpolate(a: &Model, b: &Model, t: f32) -> Result<Model, String> {
    let mismatch = || {
        "mesh sequence linear interpolation requires matching topology, attribute layouts and material/skin/clip definitions".to_string()
    };
    if a.primitives.len() != b.primitives.len()
        || a.nodes.len() != b.nodes.len()
        || a.basis != b.basis
        || a.materials.len() != b.materials.len()
        || a.materials
            .iter()
            .zip(&b.materials)
            .any(|(a, b)| a.params != b.params || a.maps != b.maps || a.texture_transforms != b.texture_transforms)
        || a.textures != b.textures
        || a.skins != b.skins
        || a.animations != b.animations
    {
        return Err(mismatch());
    }
    for (a, b) in a.nodes.iter().zip(&b.nodes) {
        if a.parent != b.parent
            || a.primitives != b.primitives
            || a.skin != b.skin
            || a.weights.len() != b.weights.len()
        {
            return Err(mismatch());
        }
    }
    for (a, b) in a.primitives.iter().zip(&b.primitives) {
        if a.indices != b.indices
            || a.vertices.len() != b.vertices.len()
            || a.material != b.material
            || a.variants != b.variants
            || a.joints != b.joints
            || a.weights.len() != b.weights.len()
            || a.morphs.len() != b.morphs.len()
            || !a.tex_coords.keys().eq(b.tex_coords.keys())
        {
            return Err(mismatch());
        }
        for (key, uv) in &a.tex_coords {
            if uv.len() != b.tex_coords[key].len() {
                return Err(mismatch());
            }
        }
        for (a, b) in a.morphs.iter().zip(&b.morphs) {
            if a.dpos.len() != b.dpos.len() || a.dnormal.len() != b.dnormal.len() {
                return Err(mismatch());
            }
        }
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            if a.tangent[3] != b.tangent[3] {
                return Err(mismatch());
            }
        }
    }
    let mut out = a.clone();
    for (n, b) in out.nodes.iter_mut().zip(&b.nodes) {
        n.local.t = n.local.t.lerp(b.local.t, t);
        n.local.s = n.local.s.lerp(b.local.s, t);
        n.local.r = n.local.r.slerp(b.local.r, t).normalize();
        for (a, b) in n.weights.iter_mut().zip(&b.weights) {
            *a = mix([*a], [*b], t)[0];
        }
    }
    for (p, b) in out.primitives.iter_mut().zip(&b.primitives) {
        for (v, b) in p.vertices.iter_mut().zip(&b.vertices) {
            v.pos = mix(v.pos, b.pos, t);
            v.uv = mix(v.uv, b.uv, t);
            v.color = mix(v.color, b.color, t);
            for (uv, b) in v.map_uv.iter_mut().zip(b.map_uv) {
                *uv = mix(*uv, b, t);
            }
            let n = direction(mix(v.normal, b.normal, t))?;
            let tangent = Vec3::from_slice(&mix(v.tangent, b.tangent, t)[..3]);
            let tangent = direction((tangent - n * n.dot(tangent)).to_array())?;
            v.normal = n.to_array();
            v.tangent = [tangent.x, tangent.y, tangent.z, v.tangent[3]];
        }
        for (key, uv) in &mut p.tex_coords {
            for (a, b) in uv.iter_mut().zip(&b.tex_coords[key]) {
                *a = mix(*a, *b, t);
            }
        }
        for (a, b) in p.weights.iter_mut().zip(&b.weights) {
            *a = mix(*a, *b, t);
        }
        for (a, b) in p.morphs.iter_mut().zip(&b.morphs) {
            for (a, b) in a.dpos.iter_mut().zip(&b.dpos) {
                *a = mix(*a, *b, t);
            }
            for (a, b) in a.dnormal.iter_mut().zip(&b.dnormal) {
                *a = mix(*a, *b, t);
            }
        }
    }
    validate(&out)?;
    Ok(out)
}
