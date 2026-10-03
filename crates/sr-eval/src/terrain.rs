//! Shared globe elevation geometry and bounded numeric terrain tile sampling.
mod dem;
use crate::{
    program::{InstNode, Program},
    FrameNode, Props,
};
pub use dem::{Dem, Missing};
use sr_model::element::Element;
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Debug)]
pub struct Surface {
    pub mesh: sr_3d::Primitive,
    /// Cache identity includes source metadata and all geometry/sampling inputs.
    pub key: u64,
}
impl std::ops::Deref for Surface {
    type Target = sr_3d::Primitive;
    fn deref(&self) -> &Self::Target {
        &self.mesh
    }
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    numbers: [u64; 4],
    segments: u32,
    zoom: u8,
    tile_size: u32,
    encoding: crate::geo::DemEncoding,
    missing: bool,
    budget: usize,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<(Key, Arc<Surface>, usize)>,
    bytes: usize,
}

/// Surface at a frame's authored geometry values, shared with static collider
/// construction. A process cache retains at most 128 MiB / 64 surfaces.
pub fn globe(p: &Program, n: &FrameNode) -> Result<Arc<Surface>, String> {
    globe_with_budget(p, n, usize::MAX)
}
pub(crate) fn globe_with_budget(p: &Program, n: &FrameNode, budget: usize) -> Result<Arc<Surface>, String> {
    let doc = p.nodes.iter().find(|v| v.id == n.id).map_or(0, |v| v.doc);
    build(p, doc, &*n.elem, Some(&n.props), budget)
}
pub(crate) fn globe_static(p: &Program, n: &InstNode, budget: usize) -> Result<Arc<Surface>, String> {
    build(p, n.doc, &*n.elem, None, budget)
}
pub(crate) fn collider_triangles(p: &Program, n: &InstNode, budget: usize) -> Result<crate::sim3d::Triangles, String> {
    let surface = globe_static(p, n, budget)?;
    if surface.vertices.len().saturating_mul(24).saturating_add(surface.indices.len().saturating_mul(4)) > budget {
        return Err("globe collider copy exceeds memory budget".into());
    }
    Ok((surface.vertices.iter().map(|v| v.pos.map(f64::from)).collect(), surface.indices.as_chunks::<3>().0.to_vec()))
}
fn build(p: &Program, doc: u16, e: &dyn Element, props: Option<&Props>, budget: usize) -> Result<Arc<Surface>, String> {
    let value =
        |k, d| props.and_then(|p| p.get(k)).and_then(crate::Value::as_num).unwrap_or_else(|| crate::sim::num(e, k, d));
    let spec = sr_3d::terrain::Globe {
        radius: value("radius", 50.),
        planet_radius: value("planetRadius", 6_378_137.),
        exaggeration: value("exaggeration", 1.),
        segments: value("segments", 32.).clamp(24., 512.) as u32,
        max_bytes: (value("terrainMemoryMiB", 128.) as usize)
            .checked_mul(1 << 20)
            .ok_or("terrain memory overflow")?
            .min(budget),
    };
    let geometry_bytes = spec.memory_cost()?;
    let id = crate::sim::text(e, "terrain").ok_or("globe elevation requires terrain")?;
    let namespace = doc.checked_sub(1).and_then(|d| p.includes.get(d as usize)).map(|d| &*d.0).unwrap_or("");
    let asset = crate::sim::asset_key(&p.assets, namespace, &id);
    let (owner, id) = p.assets.get(&asset).ok_or("terrain asset is missing")?;
    let scene =
        if *owner == 0 { &p.scene } else { &p.includes.get(*owner as usize - 1).ok_or("terrain include missing")?.1 };
    let asset = scene
        .assets
        .as_ref()
        .and_then(|a| a.children.iter().find(|a| a.id() == Some(id.as_str())))
        .ok_or("terrain asset is missing")?;
    let sr_model::model::AssetsChild::Tiles(tiles) = asset else {
        return Err("terrain must reference a tiles asset".into());
    };
    let src =
        tiles.src.as_deref().or(tiles.cache.as_deref()).ok_or("terrain tiles need local src or resolved cache")?;
    let base = p.base_dirs.get(*owner as usize).cloned().unwrap_or_default();
    let path = match sr_model::assets::resolve(src, &base) {
        sr_model::assets::Resolved::Local(p) => p,
        _ => return Err("remote terrain must be resolved before rendering".into()),
    };
    let metadata = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut header = [0; sr_geo::pmtiles::HEADER_LEN];
    std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut header)).map_err(|e| e.to_string())?;
    let header = sr_geo::pmtiles::Header::parse(&header)?;
    if header.min_zoom > header.max_zoom || header.min_zoom > 22 || header.max_zoom > 31 {
        return Err("terrain archive has an unsupported zoom range".into());
    }
    let tile_size = value("terrainTileSize", 256.) as u32;
    if tile_size == 0 || tile_size > 4096 || !tile_size.is_power_of_two() {
        return Err("invalid terrain tile dimensions".into());
    }
    let auto = (f64::from(spec.segments) / f64::from(tile_size))
        .log2()
        .ceil()
        .clamp(f64::from(header.min_zoom), f64::from(header.max_zoom.min(22))) as u8;
    let zoom = crate::sim::text(e, "terrainZoom").map_or(auto, |_| value("terrainZoom", f64::from(auto)) as u8);
    let encoding = if crate::sim::text(e, "terrainEncoding").as_deref() == Some("mapbox") {
        crate::geo::DemEncoding::Mapbox
    } else {
        crate::geo::DemEncoding::Terrarium
    };
    let missing =
        if crate::sim::text(e, "terrainMissing").as_deref() == Some("zero") { Missing::Zero } else { Missing::Error };
    let key = Key {
        path: path.clone(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
        numbers: [
            spec.radius.to_bits(),
            spec.planet_radius.to_bits(),
            spec.exaggeration.to_bits(),
            geometry_bytes as u64,
        ],
        segments: spec.segments,
        zoom,
        tile_size,
        encoding,
        missing: missing == Missing::Zero,
        budget: spec.max_bytes,
    };
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, surface, _)) =
        cache.lock().unwrap_or_else(|p| p.into_inner()).entries.iter().find(|(k, _, _)| k == &key)
    {
        return Ok(surface.clone());
    }
    let available = spec.max_bytes.checked_sub(geometry_bytes).ok_or("terrain geometry exceeds memory budget")?;
    let mut dem = Dem::open(&path, zoom, encoding, tile_size, available)?;
    let mesh = sr_3d::terrain::globe(&spec, |lon, lat| dem.sample(lon, lat, missing))?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hash);
    let bytes = mesh.vertices.capacity() * std::mem::size_of::<sr_3d::Vertex>() + mesh.indices.capacity() * 4 + 512;
    let result = Arc::new(Surface { mesh, key: hash.finish() });
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    while !cache.entries.is_empty() && (cache.bytes + bytes > 128 << 20 || cache.entries.len() >= 64) {
        let (_, _, b) = cache.entries.pop_front().unwrap();
        cache.bytes -= b;
    }
    cache.bytes += bytes;
    cache.entries.push_back((key, result.clone(), bytes));
    Ok(result)
}
