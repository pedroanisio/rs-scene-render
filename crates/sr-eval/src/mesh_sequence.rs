//! Mesh-cache sampling at an object's source clock, with bounded retained models.
use crate::{FrameNode, Program};
use sr_3d::sequence::{Interpolation, Missing, Sequence};
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub struct Loaded {
    pub frame: sr_3d::sequence::Sample,
    pub key: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    format: Option<String>,
    len: u64,
    modified: Option<std::time::SystemTime>,
}
#[derive(Debug)]
struct Entry {
    key: Key,
    model: Arc<sr_3d::Model>,
    revision: u64,
    bytes: usize,
}
#[derive(Debug, Default)]
pub(crate) struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

fn load(
    cache: &Mutex<Cache>,
    path: &Path,
    format: Option<&str>,
    budget: usize,
) -> Result<Option<(Arc<sr_3d::Model>, u64)>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if metadata.len() > budget as u64 {
        return Err("mesh sequence input file exceeds budget".into());
    }
    let key = Key {
        path: path.to_owned(),
        format: format.map(str::to_owned),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };
    static NEXT: AtomicU64 = AtomicU64::new(1);
    if let Some(entry) = cache.lock().unwrap_or_else(|p| p.into_inner()).entries.iter().find(|e| e.key == key) {
        if entry.bytes > budget {
            return Err("mesh sequence decoded frame exceeds budget".into());
        }
        return Ok(Some((entry.model.clone(), entry.revision)));
    }
    // The shared importers currently own decoder/dependency admission. This
    // budget bounds source bytes and retained models, not importer peak RSS.
    let sr_3d::Asset::Model(model) = sr_3d::import::load(path, format)? else {
        return Err("mesh sequences require triangle models, not Gaussian splats".into());
    };
    let bytes = sr_3d::sequence::bytes(&model);
    if bytes > budget {
        return Err("mesh sequence decoded frame exceeds budget".into());
    }
    let model = Arc::new(model);
    let revision = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    const CAP: usize = 256 << 20;
    if bytes <= CAP {
        while !cache.entries.is_empty() && (cache.bytes.saturating_add(bytes) > CAP || cache.entries.len() >= 64) {
            cache.bytes -= cache.entries.pop_front().unwrap().bytes;
        }
        cache.bytes += bytes;
        cache.entries.push_back(Entry { key, model: model.clone(), revision, bytes });
    }
    Ok(Some((model, revision)))
}

/// Returns None for an ordinary mesh asset. Files and their external dependencies
/// are immutable inputs for a compiled Program. Recompilation starts a fresh cache.
/// Each Program retains at most 256 MiB / 64 frames; active snapshots may retain more.
pub fn sample(p: &Program, n: &FrameNode) -> Result<Option<Loaded>, String> {
    let Some(asset) = n.asset.as_deref() else { return Ok(None) };
    let Some((owner, id)) = p.assets.get(asset) else { return Ok(None) };
    let scene = if *owner == 0 {
        &p.scene
    } else {
        &p.includes.get(*owner as usize - 1).ok_or("mesh sequence include missing")?.1
    };
    let Some(sr_model::model::AssetsChild::MeshSequence(config)) =
        scene.assets.as_ref().and_then(|a| a.children.iter().find(|a| a.id() == Some(id.as_str())))
    else {
        return Ok(None);
    };
    let base = p.base_dirs.get(*owner as usize).cloned().unwrap_or_default();
    let e: &dyn sr_model::element::Element = config;
    let num = |k, d| crate::sim::num(e, k, d);
    let interpolation = if crate::sim::text(e, "interpolation").as_deref() == Some("linear") {
        Interpolation::Linear
    } else {
        Interpolation::Hold
    };
    let missing = match crate::sim::text(e, "missingFrame").as_deref() {
        Some("hold") => Missing::Hold,
        Some("transparent") => Missing::Transparent,
        _ => Missing::Error,
    };
    let seq =
        Sequence::new(num("first", 0.) as i64, num("last", 0.) as i64, config.fps.as_f64(), interpolation, missing)?;
    let budget = (num("maxMemoryMiB", 256.) as usize).checked_mul(1 << 20).ok_or("mesh sequence budget overflow")?;
    let value = |key, default| {
        n.props.get(key).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(&*n.elem, key, default))
    };
    let time = n.local_time * value("animationSpeed", 1.) + value("animationOffset", 0.);
    let format = config.format.map(|f| f.to_string());
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let frame = seq.sample(time, budget, |label| {
        let uri =
            sr_model::assets::sequence_frame(&config.src, label).ok_or("mesh sequence requires a numbered pattern")?;
        let path = match sr_model::assets::resolve(&uri, &base) {
            sr_model::assets::Resolved::Local(p) => p,
            _ => return Err("remote mesh sequence must be resolved before rendering".into()),
        };
        Ok(load(&p.mesh_sequence_cache, &path, format.as_deref(), budget)?.map(|(model, revision)| {
            revision.hash(&mut hash);
            model
        }))
    })?;
    frame.blend.to_bits().hash(&mut hash);
    frame.opacity.to_bits().hash(&mut hash);
    Ok(Some(Loaded { frame, key: hash.finish() }))
}
