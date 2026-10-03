//! Static bathymetry from numeric raster samples or topmost mesh intersections.
use super::*;
use sr_model::model::AssetsChild;
use std::io::{Cursor, Read};

pub(super) fn load(p: &Program, n: &FrameNode, spec: &Spec, flat: f64, budget: usize) -> Result<Vec<f64>, String> {
    let count = spec.cells[0] * spec.cells[1];
    if !flat.is_finite() {
        return Err("ocean bottom depth overflows".into());
    }
    let Some(key) = n.asset.as_deref() else {
        return Ok(vec![flat; count]);
    };
    let (doc, id) = p.assets.get(key).ok_or_else(|| format!("bathymetry {key} not found"))?;
    let scene =
        if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("bathymetry include missing")?.1 };
    let asset = scene
        .assets
        .as_ref()
        .and_then(|a| a.children.iter().find(|a| a.id() == Some(id.as_str())))
        .ok_or("bathymetry asset missing")?;
    let scale = num(&*n.elem, "bathymetryScale", 1.);
    let offset = num(&*n.elem, "bathymetryOffset", 0.);
    if !scale.is_finite() || !offset.is_finite() {
        return Err("nonfinite bathymetry transform".into());
    }
    match asset {
        AssetsChild::Mesh(_) => {
            let (points, triangles) = crate::sim3d::mesh_asset_triangles(p, key, budget)?;
            if points
                .len()
                .saturating_mul(24)
                .saturating_add(triangles.len().saturating_mul(12))
                .saturating_add(count.saturating_mul(8))
                > budget
            {
                return Err("bathymetry geometry exceeds memory budget".into());
            }
            let mut bed = vec![f64::INFINITY; count];
            let mut work = spec.max_work;
            let [nx, nz] = spec.cells;
            let dx = spec.cell_size;
            for tri in triangles {
                let [a, b, c] = tri.map(|i| points[i as usize]);
                if a.iter().chain(&b).chain(&c).any(|v| !v.is_finite()) {
                    return Err("nonfinite bathymetry mesh".into());
                }
                let (bx, bz, cx, cz) = (b[0] - a[0], b[2] - a[2], c[0] - a[0], c[2] - a[2]);
                let det = bx * cz - bz * cx;
                if !det.is_finite() {
                    return Err("bathymetry triangle precision overflow".into());
                }
                if det == 0. {
                    continue;
                }
                let low = [a[0].min(b[0]).min(c[0]), a[2].min(b[2]).min(c[2])];
                let high = [a[0].max(b[0]).max(c[0]), a[2].max(b[2]).max(c[2])];
                let starts: [usize; 2] = std::array::from_fn(|i| {
                    ((low[i] - spec.origin[i]) / dx - 0.5).floor().clamp(0., spec.cells[i] as f64) as usize
                });
                let ends: [usize; 2] = std::array::from_fn(|i| {
                    ((high[i] - spec.origin[i]) / dx - 0.5).ceil().clamp(-1., spec.cells[i] as f64 - 1.) as isize + 1
                })
                .map(|v| v as usize);
                for iz in starts[1]..ends[1].min(nz) {
                    for ix in starts[0]..ends[0].min(nx) {
                        work = work.checked_sub(1).ok_or("bathymetry sampling work budget")?;
                        let px = spec.origin[0] + (ix as f64 + 0.5) * dx - a[0];
                        let pz = spec.origin[1] + (iz as f64 + 0.5) * dx - a[2];
                        let v = (px * cz - pz * cx) / det;
                        let w = (bx * pz - bz * px) / det;
                        let u = 1. - v - w;
                        if u < -1e-9 || v < -1e-9 || w < -1e-9 {
                            continue;
                        }
                        let y = (u * a[1] + v * b[1] + w * c[1]) * scale + offset;
                        if !y.is_finite() {
                            return Err("bathymetry height overflow".into());
                        }
                        let cell = &mut bed[iz * nx + ix];
                        *cell = cell.min(y);
                    }
                }
            }
            if bed.iter().any(|v| !v.is_finite()) {
                return Err("bathymetry mesh does not cover every ocean cell centre".into());
            }
            Ok(bed)
        }
        AssetsChild::Image(image) => {
            let base = p.base_dirs.get(*doc as usize).cloned().unwrap_or_default();
            let path = match sr_model::assets::resolve(&image.src, &base) {
                sr_model::assets::Resolved::Local(p) => p,
                sr_model::assets::Resolved::Remote(_) => {
                    return Err("bathymetry image must be resolved to a local file".into())
                }
            };
            let mut file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            if len > budget.min(128 << 20) as u64 {
                return Err("bathymetry image file exceeds memory budget".into());
            }
            let mut bytes = vec![0; len as usize];
            file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            if file.read(&mut [0]).map_err(|e| e.to_string())? != 0 {
                return Err("bathymetry image changed while reading".into());
            }
            let reader =
                || image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string());
            let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
            let pixels = u64::from(w) * u64::from(h);
            if w == 0
                || h == 0
                || pixels.saturating_mul(32).saturating_add(len).saturating_add(count as u64 * 8) > budget as u64
            {
                return Err("bathymetry decoded image exceeds memory budget".into());
            }
            let mut r = reader()?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some((budget as u64 - len) / 2);
            r.limits(limits);
            let decoded = r.decode().map_err(|e| e.to_string())?;
            let encoding = text(&*n.elem, "bathymetryEncoding").unwrap_or_else(|| "red".into());
            let values: Vec<f64> = if encoding == "red" {
                decoded.into_rgb32f().pixels().map(|p| f64::from(p[0])).collect()
            } else {
                decoded
                    .into_rgb8()
                    .pixels()
                    .map(|p| {
                        let [r, g, b] = p.0.map(f64::from);
                        match encoding.as_str() {
                            "terrarium" => -(r * 256. + g + b / 256. - 32768.),
                            "mapbox" => -(-10000. + (r * 65536. + g * 256. + b) * 0.1),
                            _ => unreachable!("bathymetry encoding"),
                        }
                    })
                    .collect()
            };
            if values.iter().any(|v| !v.is_finite()) {
                return Err("nonfinite bathymetry raster samples".into());
            }
            let mut bed = Vec::with_capacity(count);
            for z in 0..spec.cells[1] {
                for x in 0..spec.cells[0] {
                    let u =
                        (((x as f64 + 0.5) / spec.cells[0] as f64) * f64::from(w) - 0.5).clamp(0., f64::from(w - 1));
                    let v =
                        (((z as f64 + 0.5) / spec.cells[1] as f64) * f64::from(h) - 0.5).clamp(0., f64::from(h - 1));
                    let (ix, iz) = (u.floor() as u32, v.floor() as u32);
                    let (tx, tz) = (u - u.floor(), v - v.floor());
                    let at = |x: u32, z: u32| values[(z as usize) * w as usize + x as usize];
                    let a = at(ix, iz) * (1. - tx) + at((ix + 1).min(w - 1), iz) * tx;
                    let b = at(ix, (iz + 1).min(h - 1)) * (1. - tx) + at((ix + 1).min(w - 1), (iz + 1).min(h - 1)) * tx;
                    let y = (a * (1. - tz) + b * tz) * scale + offset;
                    if !y.is_finite() {
                        return Err("bathymetry height overflow".into());
                    }
                    bed.push(y);
                }
            }
            Ok(bed)
        }
        _ => Err("bathymetry asset must be an image or mesh".into()),
    }
}
