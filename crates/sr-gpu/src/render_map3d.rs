//! Maps in 3D: `object3D primitive="map"` lays a map asset down as ground,
//! raised by elevation tiles and set with extruded buildings, and
//! `primitive="globe"` wraps a sphere in the map's content drawn over the whole
//! world. Both are draped with the map as the 2D renderer draws it, rasterised
//! once per view ([`crate::drape`]) and cached.

use super::*;
use crate::three::{Draw3, Maps, MeshSrc};
use crate::vector::Attrs;
use glam::{Mat4, Vec2, Vec3};
use sr_3d::{MaterialParams, Vertex};
use sr_eval::geo::{self as geo, DemEncoding, TileData};
use sr_geo::tiles::{self, Placer};
use sr_geo::view::{Kind, Map, View};

/// Earth's radius (metres), as Web Mercator uses it.
const R_EARTH: f64 = 6_378_137.0;

fn attrs(n: &sr_eval::FrameNode) -> Attrs<'_> {
    Attrs { e: &*n.elem, props: Some(&n.props) }
}

/// A key for the state a map's drawing depends on: its animated attributes and those of its
/// children, and the time when fly-to moves steer it.
fn map_state(g: &FrameGraph, mp: &sr_model::model::MapAsset) -> u64 {
    let mut s = String::new();
    for e in g.elements.iter().filter(|e| *e.key == *mp.id || e.key.starts_with(&format!("{}/", mp.id))) {
        s.push_str(&format!("{}{:?}", e.key, e.props));
    }
    if mp.children.iter().any(|c| matches!(c, sr_model::model::MapAssetChild::FlyTo(_))) {
        s.push_str(&format!("{:.4}", g.time));
    }
    sr_eval::rng::hash_str(&s)
}

impl Renderer {
    /// The draws of a `map` or `globe` object.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn map3d_draws(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        world: Mat4,
        opacity: f32,
        material: Option<(MaterialParams, Maps)>,
        cast: bool,
        receive: bool,
        draws: &mut Vec<Draw3>,
    ) {
        let n = &ctx.g.nodes[j];
        let a = attrs(n);
        let globe = a.str("primitive").as_deref() == Some("globe");
        let Some(id) = a.str("map") else {
            plan.stats.errors.push(format!(
                "{}: primitive=\"{}\" needs @map",
                n.id,
                if globe { "globe" } else { "map" }
            ));
            return;
        };
        let Some(mp) = geo::map_asset(ctx.p, &id) else {
            plan.stats.errors.push(format!("{}: map asset {id} not found", n.id));
            return;
        };
        let size = a.num("textureSize", 2048.0).clamp(64.0, 8192.0);
        let drape = match self.drape(plan, ctx, j, mp, globe, size) {
            Ok(t) => t,
            Err(e) => {
                plan.stats.errors.push(format!("{}: {e}", n.id));
                return;
            }
        };
        // the ground wears the drape under the object's material (its base colour multiplies the drape;
        // its roughness, metallic and unlit apply); the material dresses the buildings whole
        let params = match &material {
            Some((m, _)) => MaterialParams {
                base_color: m.base_color,
                roughness: m.roughness,
                metallic: m.metallic,
                unlit: m.unlit,
                ..Default::default()
            },
            None => MaterialParams { base_color: [1.0; 4], roughness: 0.9, ..Default::default() },
        };
        let mut maps = Maps::default();
        maps[0] = Some(drape);
        let (building_params, building_maps) = material.unwrap_or_else(|| {
            (
                MaterialParams { base_color: [0.86, 0.85, 0.83, 1.0], roughness: 0.8, ..Default::default() },
                Maps::default(),
            )
        });
        let surface = if globe {
            let r = a.num("radius", 200.0) as f32;
            let segs = a.num("segments", 96.0).clamp(24.0, 512.0) as u32;
            let key = format!("globe|{r}|{segs}");
            match self.three_engine().meshes.get(&key) {
                Some(m) => Some(m.clone()),
                None => {
                    // 0° longitude faces the camera (−z)
                    let mut p = sr_3d::prim::sphere(r, segs);
                    let rot = glam::Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
                    for v in &mut p.vertices {
                        v.pos = (rot * Vec3::from(v.pos)).into();
                        v.normal = (rot * Vec3::from(v.normal)).into();
                        let t = rot * Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]);
                        v.tangent = [t.x, t.y, t.z, v.tangent[3]];
                    }
                    let m = self.three_engine().upload_mesh(&p.vertices, &p.indices);
                    self.three_engine().meshes.insert(key, m.clone());
                    Some(m)
                }
            }
        } else {
            match self.ground(plan, ctx, j, mp) {
                Ok((surface, buildings)) => {
                    if let Some(b) = buildings {
                        draws.push(Draw3 {
                            mesh: MeshSrc::Cached(b),
                            model: world,
                            material: building_params.clone(),
                            maps: building_maps.clone(),
                            opacity,
                            cast_shadow: cast,
                            receive_shadow: receive,
                        });
                    }
                    Some(surface)
                }
                Err(e) => {
                    plan.stats.errors.push(format!("{}: {e}", n.id));
                    None
                }
            }
        };
        if let Some(mesh) = surface {
            draws.push(Draw3 {
                mesh: MeshSrc::Cached(mesh),
                model: world,
                material: params,
                maps,
                opacity,
                cast_shadow: cast,
                receive_shadow: receive,
            });
        }
    }

    /// The map drawn and rasterised: in its own frame, or (globe) over the whole world in
    /// equirectangular. Cached per map state and size.
    fn drape(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        mp: &sr_model::model::MapAsset,
        globe: bool,
        size: f64,
    ) -> Result<Arc<crate::three::TexGpu>, String> {
        // laid out in map pixels (a globe's whole world 2H × H, H the map's height, so labels and lines
        // keep their 2D size) and scaled to the texture: @textureSize sets only the resolution
        let (w, h) =
            if globe { (2.0 * mp.height as f64, mp.height as f64) } else { (mp.width as f64, mp.height as f64) };
        let scale = if globe { size.min(4096.0) / h } else { size / w.max(h) };
        let (tw, th) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
        let key = format!("drape|{}|{globe}|{tw}x{th}|{}", mp.id, map_state(ctx.g, mp));
        if let Some(t) = self.three_engine().textures.get(&key) {
            return Ok(t.clone());
        }
        let n = &ctx.g.nodes[j];
        let frame = globe.then(|| {
            let (m, c) = Map::new(Kind::Equirectangular, None, [w, h], &[], 0.0, Some([0.0, 0.0]));
            (m, View { lon: c[0], lat: c[1], zoom: 0.0, rotation: 0.0 })
        });
        let base = ctx.p.base_dirs.first().cloned().unwrap_or_default();
        let mut tc = std::mem::take(&mut self.text);
        let mut unsupported = Vec::new();
        let (p, g) = (ctx.p, ctx.g);
        let tokens = self.tokens.clone();
        let r = {
            let mut pf = |val: &Value, b: [f64; 4]| self.vector_paint(plan, p, g, val, b, &n.id);
            let mut cx = crate::text::Cx {
                p,
                g,
                n,
                base,
                tol: 0.25 / scale.max(1e-6),
                paint: &mut pf,
                tokens: &tokens,
                unsupported: &mut unsupported,
            };
            crate::text::map_drape(&mut tc, &mut cx, mp, frame)
        };
        self.text = tc;
        for u in unsupported {
            if !plan.stats.unsupported.contains(&u) {
                plan.stats.unsupported.push(u);
            }
        }
        let d = r?;
        let d = if scale != 1.0 { d.transformed(&sr_vector::geom::Xf::scale(scale, scale)) } else { d };
        let rgba = crate::drape::rasterize(&d, [tw, th]);
        let t = self.three_engine().upload_rgba8(tw, th, &rgba, true);
        // keep the drapes of recent states only
        let stale: Vec<String> = self
            .three_engine()
            .textures
            .keys()
            .filter(|k| k.starts_with(&format!("drape|{}|{globe}|", mp.id)))
            .cloned()
            .collect();
        for k in stale {
            self.three_engine().textures.remove(&k);
        }
        self.three_engine().textures.insert(key, t.clone());
        Ok(t)
    }

    /// The ground surface (raised by terrain) and the buildings of a `map` object.
    fn ground(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        j: usize,
        mp: &sr_model::model::MapAsset,
    ) -> Result<(Arc<crate::three::MeshGpu>, Option<Arc<crate::three::MeshGpu>>), String> {
        let n = &ctx.g.nodes[j];
        let a = attrs(n);
        let (w, h) = (mp.width as f64, mp.height as f64);
        let res = a.num("resolution", 64.0).clamp(8.0, 256.0) as u32;
        let exag = a.num("exaggeration", 1.0);
        let terrain = a.str("terrain");
        let with_buildings = a.num("buildings", 0.0) != 0.0;
        let enc = if a.str("terrainEncoding").as_deref() == Some("mapbox") {
            DemEncoding::Mapbox
        } else {
            DemEncoding::Terrarium
        };
        let key =
            format!("ground|{}|{res}|{exag}|{terrain:?}|{with_buildings}|{enc:?}|{}", mp.id, map_state(ctx.g, mp));
        if let Some(s) = self.three_engine().meshes.get(&key).cloned() {
            let b = self.three_engine().meshes.get(&format!("{key}|b")).cloned();
            return Ok((s, b));
        }
        let cam = geo::camera(ctx.p, mp)?;
        let at =
            crate::vector::Attrs { e: mp, props: ctx.g.elements.iter().find(|e| *e.key == *mp.id).map(|e| &e.props) };
        let animated = |name: &str| at.props.and_then(|p| p.get(name)).and_then(Value::as_num);
        let view = geo::view(&cam, mp, &animated, ctx.g.time);
        let proj = cam.map.projection(&view);
        let needs_mercator = terrain.is_some() || with_buildings;
        if needs_mercator && tiles::screen_lonlat(&proj, w / 2.0, h / 2.0).is_none() {
            plan.stats.unsupported.push(format!("{}: terrain and buildings need a web-mercator map; drawn flat", n.id));
        }
        // metres → scene units at the map's centre
        let centre = tiles::screen_lonlat(&proj, w / 2.0, h / 2.0);
        let ppm = centre.map(|c| proj.scale() / (R_EARTH * (c[1] * sr_geo::sphere::RAD).cos())).unwrap_or(0.0);
        // elevation sampler
        let dem = match (&terrain, centre) {
            (Some(tid), Some(_)) => {
                let (_, path) = geo::tiles_asset(ctx.p, tid)?;
                let arch = geo::archive(&path)?;
                // a DEM pixel per grid cell: 256-pixel tiles at the map zoom less log2(cell)
                let cell = w.max(h) / res as f64;
                let zd = (tiles::map_zoom(&proj) + 1.0 - cell.log2())
                    .round()
                    .clamp(arch.header.min_zoom as f64, arch.header.max_zoom as f64) as u8;
                Some((arch, zd))
            }
            _ => None,
        };
        let height = |x: f64, y: f64| -> f64 {
            let (Some((arch, zd)), Some(ll)) = (&dem, tiles::screen_lonlat(&proj, x, y)) else { return 0.0 };
            geo::elevation(arch, *zd, enc, ll[0], ll[1]).ok().flatten().unwrap_or(0.0)
        };
        let h0 = height(w / 2.0, h / 2.0);
        let z_at = |x: f64, y: f64| -> f32 { (-(height(x, y) - h0) * ppm * exag) as f32 };
        // the grid
        let (nx, ny) = if w >= h {
            (res, ((res as f64 * h / w).round() as u32).max(1))
        } else {
            (((res as f64 * w / h).round() as u32).max(1), res)
        };
        let mut zs = vec![0f32; ((nx + 1) * (ny + 1)) as usize];
        for jy in 0..=ny {
            for ix in 0..=nx {
                zs[(jy * (nx + 1) + ix) as usize] = z_at(w * ix as f64 / nx as f64, h * jy as f64 / ny as f64);
            }
        }
        let (dx, dy) = ((w / nx as f64) as f32, (h / ny as f64) as f32);
        let z =
            |i: i64, jj: i64| zs[(jj.clamp(0, ny as i64) as u32 * (nx + 1) + i.clamp(0, nx as i64) as u32) as usize];
        let mut vs = Vec::with_capacity(zs.len());
        for jy in 0..=ny as i64 {
            for ix in 0..=nx as i64 {
                let (u, v) = (ix as f32 / nx as f32, jy as f32 / ny as f32);
                // gradient by central differences; the surface faces the camera (−z)
                let gx = (z(ix + 1, jy) - z(ix - 1, jy)) / (2.0 * dx);
                let gy = (z(ix, jy + 1) - z(ix, jy - 1)) / (2.0 * dy);
                let nrm = Vec3::new(gx, gy, -1.0).normalize();
                let t = Vec3::X - nrm * nrm.x;
                vs.push(Vertex {
                    pos: [u * w as f32 - w as f32 / 2.0, v * h as f32 - h as f32 / 2.0, z(ix, jy)],
                    normal: nrm.into(),
                    uv: [u, v],
                    tangent: [t.x, t.y, t.z, 1.0],
                });
            }
        }
        let mut idx = Vec::with_capacity((nx * ny * 6) as usize);
        for jy in 0..ny {
            for ix in 0..nx {
                let a0 = jy * (nx + 1) + ix;
                let (b0, c0, d0) = (a0 + 1, a0 + nx + 1, a0 + nx + 2);
                idx.extend_from_slice(&[a0, c0, b0, b0, c0, d0]);
            }
        }
        let surface = self.three_engine().upload_mesh(&vs, &idx);
        // buildings from the first vector basemap
        let mut building_mesh = None;
        if with_buildings && centre.is_some() {
            let basemap = mp.children.iter().find_map(|c| match c {
                sr_model::model::MapAssetChild::Basemap(b) => Some(b.clone()),
                _ => None,
            });
            match basemap {
                None => plan.stats.errors.push(format!("{}: buildings need a basemap of vector tiles", n.id)),
                Some(b) => {
                    let (_, path) = geo::tiles_asset(ctx.p, &b.tiles)?;
                    let arch = geo::archive(&path)?;
                    let zt = geo::basemap_zoom(&proj, 512.0, b.detail, false).min(22);
                    let (mut bv, mut bi): (Vec<Vertex>, Vec<u32>) = (Vec::new(), Vec::new());
                    for t in tiles::visible(&proj, zt) {
                        let (src, window) = tiles::source(t, arch.header.max_zoom);
                        let Some(d) = geo::tile(&arch, src.z, src.x, src.y)? else { continue };
                        let TileData::Vector(layers) = &*d else { continue };
                        let Some(l) = layers.iter().find(|l| l.name == "buildings" || l.name == "building") else {
                            continue;
                        };
                        let placer = Placer::new(&proj, src, l.extent, window);
                        for f in l.features.iter().filter(|f| f.kind == sr_geo::mvt::GeomType::Polygon) {
                            let num = |k: &str| f.properties.get(k).and_then(|v| v.as_f64());
                            let top = num("height").or(num("render_height")).unwrap_or(8.0);
                            let bottom = num("min_height").or(num("render_min_height")).unwrap_or(0.0);
                            if top <= bottom {
                                continue;
                            }
                            for poly in f.polygons() {
                                let rings = placer.polygons(&[poly]);
                                if rings.is_empty() {
                                    continue;
                                }
                                let (cx0, cy0) = rings[0].iter().fold((0.0, 0.0), |s, q| (s.0 + q[0], s.1 + q[1]));
                                let cnt = rings[0].len() as f64;
                                let ground = z_at(cx0 / cnt, cy0 / cnt);
                                let depth = ((top - bottom) * ppm * exag) as f32;
                                let base_z = ground - (bottom * ppm * exag) as f32;
                                let polys: Vec<Vec<Vec2>> = rings
                                    .iter()
                                    .map(|r| {
                                        r.iter()
                                            .map(|q| {
                                                Vec2::new(q[0] as f32 - w as f32 / 2.0, q[1] as f32 - h as f32 / 2.0)
                                            })
                                            .collect()
                                    })
                                    .collect();
                                let Ok(p) = sr_3d::prim::extrude(&polys, depth.max(1e-3), 0.0) else { continue };
                                let off = bv.len() as u32;
                                // extrude spans z ∈ [−d/2, d/2]; the roof (−d/2) goes toward the camera
                                bv.extend(p.vertices.iter().map(|v| Vertex {
                                    pos: [v.pos[0], v.pos[1], base_z - depth / 2.0 + v.pos[2]],
                                    ..*v
                                }));
                                bi.extend(p.indices.iter().map(|i| i + off));
                            }
                        }
                    }
                    if !bi.is_empty() {
                        let m = self.three_engine().upload_mesh(&bv, &bi);
                        self.three_engine().meshes.insert(format!("{key}|b"), m.clone());
                        building_mesh = Some(m);
                    }
                }
            }
        }
        self.three_engine().meshes.insert(key, surface.clone());
        Ok((surface, building_mesh))
    }
}
