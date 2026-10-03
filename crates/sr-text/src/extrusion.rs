//! Shared glyph outlines for solid extrusion, in layout coordinates (y down).
use crate::{
    layout::{self, Run},
    FontLib, Opts, Para, Style,
};
use sr_vector::Xf;

pub fn outline_polygons(
    lib: &mut FontLib,
    text: &str,
    family: Option<&str>,
    size: f64,
    tolerance: f64,
    max_bytes: usize,
) -> Result<Vec<Vec<[f64; 2]>>, String> {
    if !size.is_finite() || size <= 0. || !tolerance.is_finite() || tolerance <= 0. {
        return Err("text extrusion requires a positive finite size and tolerance".into());
    }
    if text.len().saturating_mul(1024) > max_bytes {
        return Err("text extrusion layout exceeds memory budget".into());
    }
    let mut style = Style { size, ..Default::default() };
    if let Some(family) = family.filter(|f| !f.is_empty()) {
        style.families = family.split(',').map(|s| s.trim().trim_matches(['"', '\'']).to_string()).collect();
    }
    let para = Para {
        runs: vec![Run { text: text.to_string(), style: 0, role: None }],
        styles: vec![style],
        opts: Opts::default(),
    };
    let layout = layout::layout(lib, &para);
    let mut out = Vec::new();
    let mut charge = text.len().saturating_mul(1024);
    for glyph in &layout.glyphs {
        let variations = &layout.styles[glyph.style].variations;
        let outline = lib.outline(glyph.face, glyph.gid, variations);
        let scale = glyph.size / lib.upem(glyph.face).max(1.);
        let transform = Xf([scale, 0., 0., -scale, glyph.x, glyph.y]);
        for poly in outline.transform(&transform).flatten(tolerance) {
            if poly.pts.len() >= 3 {
                charge = charge.saturating_add(poly.pts.len().saturating_mul(256));
                if charge > max_bytes {
                    return Err("text extrusion outlines exceed memory budget".into());
                }
                out.push(poly.pts.iter().map(|q| [q.x, q.y]).collect());
            }
        }
    }
    Ok(out)
}
