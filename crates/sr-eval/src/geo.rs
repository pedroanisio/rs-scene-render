//! Map cameras, shared by the renderer (which draws `<map>` assets) and by
//! expressions (`geo()` and `geoVisible()` place any layer on a map).
//!
//! A camera is fitted once from the map's unanimated attributes and its fit
//! data; the view at a time then follows the animated centre, zoom and
//! rotation, and the `<flyTo>` moves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use sr_geo::data::{self, Feature, Format, Geometry};
use sr_geo::view::{Fly, Kind, Map, View};
use sr_model::model as m;

use crate::curve::{self, KeyParams};
use crate::Program;

/// A fitted map: the projection setup at zoom 0 and the centre it looks at by default.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub map: Map,
    pub center: [f64; 2],
}

type FeatureCache = HashMap<String, Result<Arc<Vec<Feature>>, String>>;

/// Features of a geo file, read once per process (keyed by path, size and modification time).
pub fn load(path: &Path, format: &str, object: Option<&str>) -> Result<Arc<Vec<Feature>>, String> {
    static CACHE: OnceLock<Mutex<FeatureCache>> = OnceLock::new();
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let key = format!("{}|{}|{:?}|{format}|{}", path.display(), meta.len(), meta.modified().ok(), object.unwrap_or(""));
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    cache.entry(key).or_insert_with(|| data::load(path, Format::parse(format), object).map(Arc::new)).clone()
}

/// The geo asset `id` of the main document and its file.
pub fn geo_asset<'p>(p: &'p Program, id: &str) -> Result<(&'p m::GeoAsset, PathBuf), String> {
    let a = p
        .scene
        .assets
        .as_ref()
        .and_then(|a| a.children.iter().find(|c| c.id() == Some(id)))
        .ok_or_else(|| format!("no geo asset {id:?}"))?;
    let m::AssetsChild::Geo(g) = a else { return Err(format!("{id} is not a geo asset")) };
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    match sr_model::assets::resolve(&g.src, &base) {
        sr_model::assets::Resolved::Local(path) => Ok((g, path)),
        sr_model::assets::Resolved::Remote(u) => Err(format!("{u}: remote geo data is not fetched while rendering")),
    }
}

/// The features of the geo asset `id`.
pub fn features(p: &Program, id: &str) -> Result<Arc<Vec<Feature>>, String> {
    let (g, path) = geo_asset(p, id)?;
    load(&path, &g.format.to_string(), g.object.as_deref())
}

/// The map asset `id` of the main document.
pub fn map_asset<'p>(p: &'p Program, id: &str) -> Option<&'p m::MapAsset> {
    match p.scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id))? {
        m::AssetsChild::Map(mp) => Some(mp),
        _ => None,
    }
}

/// The fitted camera of a map asset (cached per process by its setup).
pub fn camera(p: &Program, mp: &m::MapAsset) -> Result<Arc<Camera>, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Camera>>>> = OnceLock::new();
    let kind = Kind::parse(&mp.projection.to_string()).ok_or("unknown projection")?;
    let parallels = mp.parallels.as_ref().filter(|p| p.len() >= 2).map(|p| [p[0], p[1]]);
    let fit_ids = mp.fit.clone().unwrap_or_default();
    let mut sig = format!(
        "{}|{parallels:?}|{}x{}|{:?}|{:?}|{}|{}",
        mp.projection,
        mp.width,
        mp.height,
        mp.center_lon,
        mp.center_lat,
        mp.fit_padding.get(),
        mp.precision.get()
    );
    for id in &fit_ids {
        let (g, path) = geo_asset(p, id)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        sig.push_str(&format!("|{}|{}|{:?}|{:?}", path.display(), meta.len(), meta.modified().ok(), g.object));
    }
    let cache = CACHE.get_or_init(Default::default);
    if let Some(c) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&sig) {
        return Ok(c.clone());
    }
    let mut geoms: Vec<Geometry> = Vec::new();
    for id in &fit_ids {
        geoms.extend(features(p, id)?.iter().map(|f| f.geometry.clone()));
    }
    let refs: Vec<&Geometry> = geoms.iter().collect();
    let size = [mp.width as f64, mp.height as f64];
    let center = mp.center_lon.zip(mp.center_lat).map(|(a, b)| [a, b]);
    let (mut map, mut c) = Map::new(kind, parallels, size, &refs, mp.fit_padding.get(), center);
    if let Some(lon) = mp.center_lon {
        c[0] = lon;
    }
    if let Some(lat) = mp.center_lat {
        c[1] = lat;
    }
    map.precision = mp.precision.get();
    // room round the frame for strokes, pins and labels reaching in from outside
    map.margin = 64.0;
    let cam = Arc::new(Camera { map, center: c });
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(sig, cam.clone());
    Ok(cam)
}

/// The view of a map at composition time `t`. `animated` gives the evaluated value of an
/// animated map attribute (`centerLon`, `centerLat`, `zoom`, `rotation`) when it has one.
pub fn view(cam: &Camera, mp: &m::MapAsset, animated: &dyn Fn(&str) -> Option<f64>, t: f64) -> View {
    let base = View {
        lon: animated("centerLon").or(mp.center_lon).unwrap_or(cam.center[0]),
        lat: animated("centerLat").or(mp.center_lat).unwrap_or(cam.center[1]),
        zoom: animated("zoom").unwrap_or(mp.zoom),
        rotation: animated("rotation").unwrap_or(mp.rotation),
    };
    let rest = View {
        lon: mp.center_lon.unwrap_or(cam.center[0]),
        lat: mp.center_lat.unwrap_or(cam.center[1]),
        zoom: mp.zoom,
        rotation: base.rotation,
    };
    let mut flies = Vec::new();
    let mut eases = Vec::new();
    for c in &mp.children {
        if let m::MapAssetChild::FlyTo(f) = c {
            flies.push(Fly {
                begin: f.begin,
                duration: f.duration.map(|d| d.get()),
                lon: f.lon,
                lat: f.lat,
                zoom: f.zoom,
                rho: f.rho.get(),
            });
            eases.push((f.begin, curve::resolve(f.easing, &KeyParams::default())));
        }
    }
    // each move eases by its own curve: the latest to have begun
    let active = eases.iter().filter(|(b, _)| *b <= t).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, e)| *e);
    let ease = |u: f64| active.map_or(u, |e| e.apply(u));
    cam.map.view_at(t, base, rest, &flies, &ease)
}

/// Where a longitude/latitude lands on a map (map pixels) and whether it is visible there
/// (on the near side of a globe and inside the frame).
pub fn locate(cam: &Camera, v: &View, lon: f64, lat: f64) -> ([f64; 2], bool) {
    let proj = cam.map.projection(v);
    let q = proj.point_unclipped(lon, lat);
    let inside = q[0] >= 0.0 && q[1] >= 0.0 && q[0] <= cam.map.size[0] && q[1] <= cam.map.size[1];
    (q, proj.visible(lon, lat) && inside)
}
