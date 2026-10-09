//! Shapes placed on regions of a pdf asset (SREP 17 §3).
//!
//! A shape with `@region` takes its box from the region, grown by `regionPadding` on every side, and is placed
//! through the layer that shows the page as a transform child would be: its world transform is
//! W_L · M · T(G.x, G.y) · M_s, with W_L the layer's world transform, M the layer's media-box transform for its
//! asset (crop, flips, fit: the placement the layer draws its picture with), G the grown region in cache pixels and
//! M_s the shape's own transform. The shape's containers still give it their opacity, clock and effects, not their
//! transforms. While the layer is not in the frame (outside its window or under a false condition) the shape is not
//! drawn: the words it marks are not on screen.

use std::collections::HashMap;
use std::sync::Arc;

use sr_model::element::{AttrValue, Element};

use crate::eval::{Affine, FrameNode};
use crate::program::Program;
use crate::value::Value;

/// A shape on a region: effective ids and the region's box in cache pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionInst {
    /// The shape's effective id.
    pub shape: Arc<str>,
    /// The effective id of the layer that shows the page.
    pub layer: Arc<str>,
    /// The region's x, y, width and height in the cache image's pixels.
    pub rect: [f64; 4],
    /// The page image's width and height (the pdf asset's).
    pub page: [f64; 2],
}

/// `regionPadding` of a shape: the animated value first, then the document's.
fn padding(n: &FrameNode) -> f64 {
    if let Some(v) = n.props.get("regionPadding").and_then(Value::as_num) {
        return v;
    }
    match n.elem.get_attr("regionPadding") {
        Some(AttrValue::Num(v)) => v,
        _ => 0.0,
    }
}

/// The transform from the page image's pixels into the layer's node space: the media box the layer draws with.
fn media_box(layer: &FrameNode, page: [f64; 2]) -> Affine {
    let Some(c) = layer.content.as_ref() else { return Affine::IDENTITY };
    let [x0, y0, x1, y1] = c.dest;
    let [u0, v0, u1, v1] = c.uv;
    let (du, dv) = (u1 - u0, v1 - v0);
    if du == 0.0 || dv == 0.0 || page[0] <= 0.0 || page[1] <= 0.0 {
        return Affine::IDENTITY;
    }
    let (sx, sy) = ((x1 - x0) / (du * page[0]), (y1 - y0) / (dv * page[1]));
    Affine([sx, 0.0, 0.0, sy, x0 - u0 * page[0] * sx, y0 - v0 * page[1] * sy])
}

/// Places every region shape of the frame on its layer. Run once every pose is final, before connectors read
/// them.
pub fn resolve(p: &Program, nodes: &mut [FrameNode]) {
    if p.regions.is_empty() {
        return;
    }
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (&*n.id, i)).collect();
    // (shape, its world transform and size on the page, or None while the page's layer is absent)
    type Placement = (usize, Option<(Affine, [f64; 2])>);
    let mut placed: Vec<Placement> = Vec::new();
    for r in &p.regions {
        let Some(&si) = index.get(&*r.shape) else { continue };
        let place = index.get(&*r.layer).map(|&li| {
            let layer = &nodes[li];
            let pad = padding(&nodes[si]);
            let g = [r.rect[0] - pad, r.rect[1] - pad, r.rect[2] + 2.0 * pad, r.rect[3] + 2.0 * pad];
            let world =
                layer.world.then(&media_box(layer, r.page)).then(&Affine::translate(g[0], g[1])).then(&nodes[si].local);
            (world, [g[2], g[3]])
        });
        placed.push((si, place));
    }
    for (si, place) in placed {
        match place {
            Some((world, size)) => {
                nodes[si].world = world;
                nodes[si].size = Some(size);
            }
            None => nodes[si].draw = false,
        }
    }
}
