//! Layout: node boxes, fit and crop placement of media, flex and grid
//! layout of group children, responsive alignment and safe areas.
//!
//! Every function here is pure geometry on local-space rectangles, so the
//! evaluator can apply it per frame and the renderer receives final
//! transforms and content rectangles.

use sr_model::model::{AlignItems, AlignTo, AlignX, AlignY, Fit, GroupLayout, Justify, SafeAreaPreset};
use sr_model::values::Length;

/// Media placement inside a layer's box, in the layer's local space.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Content {
    /// Destination rectangle x0, y0, x1, y1.
    pub dest: [f64; 4],
    /// Source rectangle in normalised texture coordinates u0, v0, u1, v1
    /// (flips swap the corners).
    pub uv: [f64; 4],
    /// For `contain-blur`: the blurred cover fill behind the content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur_fill: Option<([f64; 4], [f64; 4])>,
}

/// Static fit parameters of a layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitSpec {
    /// Fit mode.
    pub fit: Fit,
    /// Box width.
    pub box_w: Option<Length>,
    /// Box height.
    pub box_h: Option<Length>,
    /// Focus point for positioning and cover cropping.
    pub focus: [f64; 2],
    /// Crop fractions left, top, right, bottom.
    pub crop: [f64; 4],
    /// Horizontal and vertical flips.
    pub flip: [bool; 2],
}

/// Places a source of size `src` according to `spec`. Returns the node box
/// size and the content placement.
pub fn place(src: [f64; 2], spec: &FitSpec, box_size: Option<[f64; 2]>) -> ([f64; 2], Content) {
    let [cl, ct, cr, cb] = spec.crop;
    let (u0, v0, u1, v1) = (cl, ct, (1.0 - cr).max(cl), (1.0 - cb).max(ct));
    let cw = src[0] * (u1 - u0);
    let ch = src[1] * (v1 - v0);
    let flip = |uv: [f64; 4]| {
        let [a, b, c, d] = uv;
        let (a, c) = if spec.flip[0] { (c, a) } else { (a, c) };
        let (b, d) = if spec.flip[1] { (d, b) } else { (b, d) };
        [a, b, c, d]
    };
    let bx = match (spec.fit, box_size) {
        (Fit::None, _) | (_, None) => {
            let dest = [src[0] * u0, src[1] * v0, src[0] * u1, src[1] * v1];
            return (src, Content { dest, uv: flip([u0, v0, u1, v1]), blur_fill: None });
        }
        (_, Some(b)) => b,
    };
    let [fx, fy] = spec.focus;
    let (bw, bh) = (bx[0], bx[1]);
    let contain = |scale_down: bool| {
        let mut s = (bw / cw.max(1e-9)).min(bh / ch.max(1e-9));
        if scale_down {
            s = s.min(1.0);
        }
        let (dw, dh) = (cw * s, ch * s);
        let (x0, y0) = ((bw - dw) * fx, (bh - dh) * fy);
        [x0, y0, x0 + dw, y0 + dh]
    };
    let cover = || {
        let s = (bw / cw.max(1e-9)).max(bh / ch.max(1e-9));
        let (vw, vh) = (bw / s, bh / s);
        let (ou, ov) = ((cw - vw) * fx / src[0], (ch - vh) * fy / src[1]);
        let uv = [u0 + ou, v0 + ov, u0 + ou + vw / src[0], v0 + ov + vh / src[1]];
        ([0.0, 0.0, bw, bh], uv)
    };
    let c = match spec.fit {
        Fit::Contain | Fit::ScaleDown => {
            Content { dest: contain(spec.fit == Fit::ScaleDown), uv: flip([u0, v0, u1, v1]), blur_fill: None }
        }
        Fit::ContainBlur => {
            let (d, uv) = cover();
            Content { dest: contain(false), uv: flip([u0, v0, u1, v1]), blur_fill: Some((d, flip(uv))) }
        }
        Fit::Cover => {
            let (d, uv) = cover();
            Content { dest: d, uv: flip(uv), blur_fill: None }
        }
        Fit::Fill | Fit::None => Content { dest: [0.0, 0.0, bw, bh], uv: flip([u0, v0, u1, v1]), blur_fill: None },
    };
    (bx, c)
}

/// Layout parameters of a group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSpec {
    /// Layout kind.
    pub kind: GroupLayout,
    /// Gap between children.
    pub gap: Length,
    /// Inner padding.
    pub padding: Length,
    /// Main-axis distribution.
    pub justify: Justify,
    /// Cross-axis alignment.
    pub align_items: AlignItems,
    /// Grid columns.
    pub columns: u32,
}

/// Where a child goes: target top-left of its box and a stretch factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Target top-left in the container's space.
    pub at: [f64; 2],
    /// Scale applied around the box's top-left (stretch).
    pub stretch: [f64; 2],
}

fn justify(j: Justify, avail: f64, sizes: &[f64], gap: f64) -> (f64, f64) {
    let n = sizes.len() as f64;
    let total: f64 = sizes.iter().sum::<f64>() + gap * (n - 1.0).max(0.0);
    let free = avail - total;
    match j {
        Justify::Start => (0.0, gap),
        Justify::Center => (free / 2.0, gap),
        Justify::End => (free, gap),
        Justify::SpaceBetween if n > 1.0 => (0.0, gap + free / (n - 1.0)),
        Justify::SpaceBetween => (0.0, gap),
        Justify::SpaceAround => (free / n / 2.0, gap + free / n),
        Justify::SpaceEvenly => (free / (n + 1.0), gap + free / (n + 1.0)),
    }
}

fn cross(a: AlignItems, avail: f64, size: f64) -> (f64, f64) {
    match a {
        AlignItems::Start | AlignItems::Baseline => (0.0, 1.0),
        AlignItems::Center => ((avail - size) / 2.0, 1.0),
        AlignItems::End => (avail - size, 1.0),
        AlignItems::Stretch => (0.0, if size > 0.0 { avail / size } else { 1.0 }),
    }
}

/// Lays out children of sizes `sizes` (already scaled) inside a container
/// box (`None` when the group has no size: the box hugs the content).
pub fn arrange(spec: &LayoutSpec, container: Option<[f64; 2]>, gap: f64, pad: f64, sizes: &[[f64; 2]]) -> Vec<Slot> {
    if sizes.is_empty() {
        return Vec::new();
    }
    let content = |axis: usize| -> f64 {
        match spec.kind {
            GroupLayout::Row if axis == 0 => sizes.iter().map(|s| s[0]).sum::<f64>() + gap * (sizes.len() - 1) as f64,
            GroupLayout::Column if axis == 1 => {
                sizes.iter().map(|s| s[1]).sum::<f64>() + gap * (sizes.len() - 1) as f64
            }
            _ => sizes.iter().map(|s| s[axis]).fold(0.0, f64::max),
        }
    };
    let avail = [
        container.map(|c| c[0] - 2.0 * pad).unwrap_or_else(|| content(0)),
        container.map(|c| c[1] - 2.0 * pad).unwrap_or_else(|| content(1)),
    ];
    match spec.kind {
        GroupLayout::None => Vec::new(),
        GroupLayout::Row | GroupLayout::Column => {
            let (m, c) = if spec.kind == GroupLayout::Row { (0, 1) } else { (1, 0) };
            let mains: Vec<f64> = sizes.iter().map(|s| s[m]).collect();
            let (start, step) = justify(spec.justify, avail[m], &mains, gap);
            let mut cursor = pad + start;
            sizes
                .iter()
                .map(|s| {
                    let (off, k) = cross(spec.align_items, avail[c], s[c]);
                    let mut at = [0.0; 2];
                    let mut stretch = [1.0; 2];
                    at[m] = cursor;
                    at[c] = pad + off;
                    stretch[c] = k;
                    cursor += s[m] + step;
                    Slot { at, stretch }
                })
                .collect()
        }
        GroupLayout::Stack => sizes
            .iter()
            .map(|s| {
                let (x, _) = justify(spec.justify, avail[0], &[s[0]], 0.0);
                let (y, k) = cross(spec.align_items, avail[1], s[1]);
                Slot { at: [pad + x, pad + y], stretch: [1.0, k] }
            })
            .collect(),
        GroupLayout::Grid => {
            let cols = spec.columns.max(1) as usize;
            let rows = sizes.len().div_ceil(cols);
            let mut cw = vec![0.0f64; cols];
            let mut rh = vec![0.0f64; rows];
            for (i, s) in sizes.iter().enumerate() {
                cw[i % cols] = cw[i % cols].max(s[0]);
                rh[i / cols] = rh[i / cols].max(s[1]);
            }
            let (x0, xstep) = justify(spec.justify, avail[0], &cw, gap);
            let total_h: f64 = rh.iter().sum::<f64>() + gap * (rows - 1) as f64;
            let y0 = match spec.align_items {
                AlignItems::Center => (avail[1] - total_h) / 2.0,
                AlignItems::End => avail[1] - total_h,
                _ => 0.0,
            };
            sizes
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let (c, r) = (i % cols, i / cols);
                    let x = pad + x0 + cw[..c].iter().sum::<f64>() + xstep * c as f64;
                    let y = pad + y0 + rh[..r].iter().sum::<f64>() + gap * r as f64;
                    let (off, k) = cross(spec.align_items, rh[r], s[1]);
                    Slot {
                        at: [x, y + if spec.align_items == AlignItems::Stretch { 0.0 } else { off.max(0.0) }],
                        stretch: [1.0, k],
                    }
                })
                .collect()
        }
    }
}

/// Responsive alignment of one node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignSpec {
    /// Horizontal alignment.
    pub x: Option<AlignX>,
    /// Vertical alignment.
    pub y: Option<AlignY>,
    /// Reference rectangle.
    pub to: AlignTo,
    /// Margin.
    pub margin: Length,
}

/// Aligns a box (axis-aligned bounds `bounds` = x0, y0, x1, y1 in the
/// container's space) within `rect`. Returns the target top-left and stretch.
pub fn align(spec: &AlignSpec, bounds: [f64; 4], rect: [f64; 4], margin: f64) -> Slot {
    let (w, h) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let mut at = [bounds[0], bounds[1]];
    let mut stretch = [1.0, 1.0];
    match spec.x {
        Some(AlignX::Left) => at[0] = rect[0] + margin,
        Some(AlignX::Center) => at[0] = (rect[0] + rect[2] - w) / 2.0,
        Some(AlignX::Right) => at[0] = rect[2] - margin - w,
        Some(AlignX::Stretch) => {
            at[0] = rect[0] + margin;
            stretch[0] = if w > 0.0 { (rect[2] - rect[0] - 2.0 * margin) / w } else { 1.0 };
        }
        None => {}
    }
    match spec.y {
        Some(AlignY::Top) => at[1] = rect[1] + margin,
        Some(AlignY::Middle) => at[1] = (rect[1] + rect[3] - h) / 2.0,
        Some(AlignY::Bottom) => at[1] = rect[3] - margin - h,
        Some(AlignY::Stretch) => {
            at[1] = rect[1] + margin;
            stretch[1] = if h > 0.0 { (rect[3] - rect[1] - 2.0 * margin) / h } else { 1.0 };
        }
        None => {}
    }
    Slot { at, stretch }
}

/// Insets (top, right, bottom, left) as fractions of the frame for the
/// renderer's maintained safe-area presets.
pub fn preset_insets(p: SafeAreaPreset) -> [f64; 4] {
    match p {
        SafeAreaPreset::Custom => [0.0; 4],
        // EBU R 95: action-safe 93 %, title-safe 90 % of each dimension
        SafeAreaPreset::ActionSafe => [0.035; 4],
        SafeAreaPreset::TitleSafe => [0.05; 4],
        // vertical social formats: platform chrome at the top (account, close) and
        // bottom (caption, actions), action rail on the right
        SafeAreaPreset::InstagramReels => [0.14, 0.12, 0.35, 0.05],
        SafeAreaPreset::InstagramStories => [0.14, 0.05, 0.20, 0.05],
        SafeAreaPreset::InstagramFeed => [0.0, 0.0, 0.0, 0.0],
        SafeAreaPreset::FacebookReels => [0.14, 0.12, 0.35, 0.05],
        SafeAreaPreset::Tiktok => [0.10, 0.12, 0.28, 0.05],
        SafeAreaPreset::YoutubeShorts => [0.08, 0.12, 0.25, 0.05],
        SafeAreaPreset::Snapchat => [0.10, 0.05, 0.20, 0.05],
        SafeAreaPreset::PinterestIdea => [0.12, 0.05, 0.25, 0.05],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(fit: Fit) -> FitSpec {
        FitSpec { fit, box_w: None, box_h: None, focus: [0.5, 0.5], crop: [0.0; 4], flip: [false; 2] }
    }

    #[test]
    fn fit_modes() {
        let (_, c) = place([200.0, 100.0], &spec(Fit::Contain), Some([100.0, 100.0]));
        assert_eq!(c.dest, [0.0, 25.0, 100.0, 75.0]);
        let (_, c) = place([200.0, 100.0], &spec(Fit::Cover), Some([100.0, 100.0]));
        assert_eq!((c.dest, c.uv), ([0.0, 0.0, 100.0, 100.0], [0.25, 0.0, 0.75, 1.0]));
        let (_, c) = place([50.0, 50.0], &spec(Fit::ScaleDown), Some([100.0, 100.0]));
        assert_eq!(c.dest, [25.0, 25.0, 75.0, 75.0]);
        let (b, c) = place(
            [200.0, 100.0],
            &FitSpec { crop: [0.25, 0.0, 0.25, 0.0], flip: [true, false], ..spec(Fit::None) },
            None,
        );
        assert_eq!((b, c.dest, c.uv), ([200.0, 100.0], [50.0, 0.0, 150.0, 100.0], [0.75, 0.0, 0.25, 1.0]));
        assert!(place([200.0, 100.0], &spec(Fit::ContainBlur), Some([100.0, 100.0])).1.blur_fill.is_some());
    }

    #[test]
    fn flex_rows_columns_and_grids() {
        let l = |kind, justify, align_items| LayoutSpec {
            kind,
            gap: Length::px(10.0),
            padding: Length::px(5.0),
            justify,
            align_items,
            columns: 2,
        };
        let s = arrange(
            &l(GroupLayout::Row, Justify::Start, AlignItems::Center),
            Some([200.0, 100.0]),
            10.0,
            5.0,
            &[[20.0, 10.0], [30.0, 50.0]],
        );
        assert_eq!(s[0].at, [5.0, 5.0 + 40.0]);
        assert_eq!(s[1].at, [35.0, 5.0 + 20.0]);
        let s = arrange(
            &l(GroupLayout::Row, Justify::SpaceBetween, AlignItems::Stretch),
            Some([200.0, 100.0]),
            10.0,
            5.0,
            &[[20.0, 10.0], [30.0, 45.0]],
        );
        assert_eq!(s[1].at[0], 5.0 + 190.0 - 30.0);
        assert_eq!(s[0].stretch, [1.0, 9.0]);
        let s = arrange(
            &l(GroupLayout::Column, Justify::End, AlignItems::End),
            Some([100.0, 100.0]),
            10.0,
            5.0,
            &[[20.0, 10.0], [30.0, 20.0]],
        );
        assert_eq!(s[1].at, [5.0 + 90.0 - 30.0, 5.0 + 90.0 - 20.0]);
        let s = arrange(
            &l(GroupLayout::Grid, Justify::Start, AlignItems::Start),
            None,
            10.0,
            0.0,
            &[[20.0, 10.0], [30.0, 20.0], [10.0, 5.0]],
        );
        assert_eq!(s.iter().map(|x| x.at).collect::<Vec<_>>(), vec![[0.0, 0.0], [30.0, 0.0], [0.0, 30.0]]);
    }

    #[test]
    fn alignment() {
        let a =
            AlignSpec { x: Some(AlignX::Right), y: Some(AlignY::Middle), to: AlignTo::Frame, margin: Length::px(10.0) };
        let s = align(&a, [0.0, 0.0, 50.0, 20.0], [0.0, 0.0, 1000.0, 500.0], 10.0);
        assert_eq!(s.at, [940.0, 240.0]);
    }
}
