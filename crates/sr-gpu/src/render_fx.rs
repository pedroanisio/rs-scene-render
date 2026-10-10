//! Effects, adjustment layers, transitions, motion blur and colour
//! finishing inside the compositor. The passes themselves live in `fx.rs`.
//!
//! * **Effects on nodes.** The node draws "bare" (without its own opacity,
//!   blend, masks and matte) into an offscreen covering its on-screen box
//!   plus the effects' reach; the chain runs; the result composites with the
//!   node's opacity, blend, masks and matte. The result is cached by the
//!   subtree's content, the effects' animated values and, for time-varying
//!   effects, the frame time.
//! * **Adjustment layers** snapshot the target at their place in the draw
//!   order, run their effects on the snapshot and draw the result over the
//!   target through their own opacity, masks, matte and blend.
//! * **Transitions** draw both sides into full-target offscreens at the
//!   incoming node's place in paint order and combine them in one pass.
//! * **Motion blur, echo, posterize-time and pixel-motion-blur** read the
//!   scene at other times from the frame's sub-frame provider.

use super::*;
use crate::fx::{Builder, Cx};
use crate::vector::Attrs;

/// Effect types whose output changes with time on its own.
pub(super) const TIME_VARYING: &[&str] = &[
    "film-grain",
    "noise",
    "light-leak",
    "light-sweep",
    "fractal-noise",
    "turbulent-displace",
    "wave-warp",
    "ripple",
    "heat-haze",
    "glitch",
    "vhs",
    "shader",
    "echo",
    "posterize-time",
    "pixel-motion-blur",
];

/// Effect types whose result turns and scales with the node they are on: the same at every
/// pixel (colour operations), the same in every direction, or directed in the node's own axes.
const TURNING: &[&str] = &[
    "blur",
    "glow",
    "bloom",
    "halation",
    "unsharp-mask",
    "inner-glow",
    "stroke",
    "outline",
    "matte-choke",
    "directional-blur",
    "color-grade",
    "lift-gamma-gain",
    "cdl",
    "lut",
    "curves",
    "levels",
    "white-balance",
    "exposure",
    "hue-saturation",
    "tonemap",
    "tint",
    "tritone",
    "color-overlay",
    "fill",
    "grayscale",
    "sepia",
    "invert",
    "posterize",
    "threshold",
    "selective-color",
    "chroma-key",
    "luma-key",
    "spill-suppress",
];

/// Video memory one frame's motion blur may hold (bytes). A frame over it draws the remaining moving nodes
/// sharp rather than failing to allocate.
const MOTION_BLUR_BUDGET: u64 = 8 << 30;

/// Retained accumulations are separate from the transient budget of a frame.
pub(super) const MOTION_BLUR_CACHE_BUDGET: u64 = 256 << 20;

pub(super) struct MotionBlurCache {
    hash: u64,
    pub(super) texture: Arc<Tex>,
    charge: u64,
    pub(super) used: u64,
}

/// Effects whose reach is the whole frame (they draw light or pull pixels from far away).
const UNBOUNDED: &[&str] = &[
    "lens-flare",
    "god-rays",
    "light-leak",
    "radial-blur",
    "zoom-blur",
    "twirl",
    "spherize",
    "bulge",
    "lens-distortion",
    "mirror",
    "kaleidoscope",
    "tile",
    "displacement-map",
    "letterbox",
    "vignette",
    "light-sweep",
    "fractal-noise",
];

pub(super) fn effect_ids(e: &dyn Element) -> Vec<String> {
    match e.get_attr("effects") {
        Some(AttrValue::Tokens(t)) => t,
        _ => Vec::new(),
    }
}

pub(super) fn find_effect<'p>(p: &'p Program, id: &str) -> Option<&'p m::Effect> {
    p.scene.effects.as_ref()?.effects.iter().find(|e| e.id == id)
}

fn eid(e: &dyn Element) -> Option<String> {
    match e.get_attr("id") {
        Some(AttrValue::Str(s)) => Some(s),
        _ => None,
    }
}

fn fps_of(p: &Program) -> f64 {
    let f = &p.scene.project.fps;
    (f.num as f64 / f.den.max(1) as f64).max(1e-6)
}

pub(super) fn element_props<'g>(g: &'g FrameGraph, key: &str) -> Option<&'g sr_eval::Props> {
    g.elements.iter().find(|e| &*e.key == key).map(|e| &e.props)
}

/// The time posterize-time shows at frame time `t`: the start of its step.
pub(super) fn posterized(t: f64, a: &Attrs) -> f64 {
    let rate = a.num("frequency", 1.0).max(1e-3);
    (t * rate + 1e-9).floor() / rate
}

/// Passes labelled with what they belong to, for GPU timings.
fn labelled(mut passes: Vec<fx::Pass>, who: &str) -> Vec<fx::Pass> {
    for p in &mut passes {
        if p.label.is_empty() {
            p.label = who.to_string();
        }
    }
    passes
}

/// Effect targets that follow moving content round their size up to a multiple of this, so
/// the texture sizes they ask the pool for repeat from frame to frame.
const TARGET_STEP: f64 = 32.0;

/// Rounds the span `a..b` up to a multiple of `TARGET_STEP` within `lo..hi`, growing towards
/// `hi` first and then towards `lo`. A wider target covers the same content.
fn round_span(a: f64, b: f64, lo: f64, hi: f64) -> (f64, f64) {
    let q = (((b - a) / TARGET_STEP).ceil() * TARGET_STEP).min(hi - lo);
    if a + q <= hi {
        (a, a + q)
    } else {
        ((hi - q).max(lo), hi)
    }
}

/// `round_span` on both axes of a rectangle, within the target's bounds.
fn round_rect(r: [f64; 4], lo: [f64; 2], hi: [f64; 2]) -> [f64; 4] {
    let (x0, x1) = round_span(r[0], r[2], lo[0], hi[0]);
    let (y0, y1) = round_span(r[1], r[3], lo[1], hi[1]);
    [x0, y0, x1, y1]
}

/// Children that move a node's geometry outside its box.
const RESHAPING: &[&str] = &["shapeModifier", "deform", "modifier", "softBody"];

/// Local-unit reach of a shape's stroke beyond its box: half the width centred, the whole width
/// outside, lengthened at corners by the join (√2 at a rectangle's right angles, up to the miter
/// limit where paths, polygons and stars can turn sharply; nothing for round outlines and joins).
fn stroke_reach(n: &sr_eval::FrameNode) -> f64 {
    if n.kind != "shape" {
        return 0.0;
    }
    let a = Attrs { e: &*n.elem, props: Some(&n.props) };
    let stroked = n.props.get("stroke").is_some() || n.elem.get_attr("stroke").is_some();
    if !stroked {
        return 0.0;
    }
    let w = a.num("strokeWidth", 1.0).max(0.0);
    let outside = matches!(a.str("strokePosition").as_deref(), Some("outside"));
    let base = if outside { w } else { w * 0.5 };
    let join = match (a.str("shape").as_deref(), a.str("strokeJoin").as_deref()) {
        (_, Some("round" | "bevel")) | (Some("ellipse" | "rounded-rect"), _) => 1.0,
        (Some("rect"), _) => std::f64::consts::SQRT_2,
        _ => a.num("miterLimit", 4.0).max(1.0),
    };
    base * join + 1.0
}

/// Target-space bounds of everything node `i` draws (its sized layers and shapes, their strokes
/// and the reach of every effect on the way), not clipped to the target. `None` when something
/// in it has no box to bound it (particles, 3D, reshaping modifiers, whole-frame effects).
fn drawn_bounds(ctx: &Ctx, i: usize, space: &Space) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    // (node, reach of the effects of its ancestors within the subtree, in target pixels)
    let mut stack = vec![(i, 0.0f64)];
    while let Some((k, outer)) = stack.pop() {
        let n = &ctx.g.nodes[k];
        if n.three_d.is_some() || sr_eval::draws_in_3d(n.kind) || n.particles.is_some() {
            return None;
        }
        if sr_model::element::children(&*n.elem).iter().any(|c| RESHAPING.contains(&c.element_name())) {
            return None;
        }
        let w = space.xform.then(&n.world);
        let scale = Xf(w.0).max_scale().max(1e-6);
        let own: f64 = effect_ids(&*n.elem)
            .iter()
            .filter_map(|id| find_effect(ctx.p, id))
            .filter(|e| e.enabled)
            .map(|e| reach(e, &Attrs { e: e as &dyn Element, props: element_props(ctx.g, &e.id) }))
            .sum::<f64>()
            * scale;
        if !own.is_finite() {
            return None;
        }
        let reach_px = outer + own;
        if ctx.kids[k].is_empty() {
            let s = n.size?;
            let m = reach_px + stroke_reach(n) * scale + 2.0;
            for q in [[0.0, 0.0], [s[0], 0.0], [s[0], s[1]], [0.0, s[1]]].map(|p| w.apply(p)) {
                b = [b[0].min(q[0] - m), b[1].min(q[1] - m), b[2].max(q[0] + m), b[3].max(q[1] + m)];
            }
        }
        stack.extend(ctx.kids[k].iter().map(|&c| (c, reach_px)));
    }
    (b[2] > b[0] && b[3] > b[1]).then_some(b)
}

/// Document-pixel reach of an effect beyond the node's box (infinite: whole frame).
fn reach(e: &m::Effect, a: &Attrs) -> f64 {
    let kind = e.r#type.as_str();
    if UNBOUNDED.contains(&kind) {
        return f64::INFINITY;
    }
    // read for every type; what a type uses is in its declared attributes (sr_model::effect_attrs)
    let (r, sz) = crate::vector::quiet(|| (a.num("radius", 4.0), a.num("size", 1.0)));
    match kind {
        // radius is the standard deviation: reach four of them
        "blur" | "glow" | "bloom" | "halation" | "unsharp-mask" | "inner-glow" | "inner-shadow" => r * 4.0,
        "lens-blur" | "tilt-shift" => r * 2.0,
        "drop-shadow" => r * 2.0 + a.num("offsetX", 8.0).abs().max(a.num("offsetY", 8.0).abs()),
        "directional-blur" => r * 2.0,
        "long-shadow" => {
            if sz > 1.0 {
                sz
            } else {
                60.0
            }
        }
        "stroke" | "outline" => sz.max(1.0),
        "matte-choke" => a.num("amount", 1.0).abs(),
        "turbulent-displace" | "heat-haze" | "wave-warp" | "ripple" | "glitch" | "vhs" | "chromatic-aberration" => {
            a.num("amount", 1.0).abs() * 4.0 + 8.0
        }
        "rgb-split" => a.num("offsetX", 8.0).abs().max(a.num("offsetY", 8.0).abs()),
        "pixel-motion-blur" => 64.0,
        "shader" => crate::shader::padding(e as &dyn Element),
        _ => 0.0,
    }
}

/// Effects that move pixels in from outside the frame.
const WARPS: &[&str] = &["turbulent-displace", "displacement-map", "wave-warp", "heat-haze", "ripple"];

/// Why a node cannot be drawn as it was at another time.
enum SubMiss {
    /// The caller passed no sub-frame provider.
    NoProvider,
    /// The node did not exist at that time.
    Absent,
}

impl Renderer {
    /// Handles transitions, motion blur and node effects; returns whether the node was drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn special(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) -> bool {
        let g = ctx.g;
        let n = &g.nodes[i];
        if !self.in_transition.contains(&n.id) {
            if let Some(tr) = g.transitions.iter().find(|t| t.from == Some(i as u32) || t.to == Some(i as u32)) {
                // both sides draw at the incoming node's place (or the only node's)
                if tr.to.or(tr.from) == Some(i as u32) {
                    self.transition(plan, ctx, tr, space, iso_op, cmds, root_hash);
                }
                return true;
            }
        }
        if !self.sampling && !self.bare.contains(&n.id) && self.motion_blur_on(ctx, i) {
            if ctx.sub.is_some() {
                if self.motion_blur(plan, ctx, i, space, iso_op, cmds, root_hash) {
                    return true;
                }
            } else if !n.kind.eq("group") {
                plan.stats
                    .unsupported
                    .push(format!("{}: motion blur needs a sub-frame provider (Renderer::render_with)", n.id));
            }
        }
        if !self.bare.contains(&n.id) && n.kind != "adjustment" {
            let ids = effect_ids(&*n.elem);
            if !ids.is_empty() {
                return self.node_effects(plan, ctx, i, space, iso_op, cmds, root_hash, &ids);
            }
        }
        false
    }

    fn color_value(&self, v: &Value) -> Option<[f64; 4]> {
        match v {
            Value::Color(c) => Some(self.working.from_literal(*c)),
            Value::Str(s) if s.starts_with("token:") => self.tokens.get(&s[6..]).map(|c| self.working.from_literal(*c)),
            _ => None,
        }
    }

    fn gradient_value(&self, p: &Program, g: &FrameGraph, v: &Value) -> Option<fx::Stops> {
        let Value::PaintRef(id) = v else { return None };
        let pc = p.scene.paints.as_ref()?.children.iter().find(|c| c.id() == Some(id))?;
        let tokens = &self.tokens;
        crate::paint::gradient_stops(&self.working, pc, g, &|t: &str| tokens.get(t).copied())
    }

    pub(super) fn base_dir(p: &Program) -> std::path::PathBuf {
        p.base_dirs.first().cloned().unwrap_or_default()
    }

    /// A pooled texture that lives until the frame is submitted.
    pub(super) fn temp(&mut self, plan: &mut Plan, size: [u32; 2]) -> Arc<Tex> {
        let t = self.pool.get(&self.gpu.device, &self.bgl1, [size[0].max(1), size[1].max(1)]);
        plan.fx_temps.push(t.clone());
        t
    }

    pub(super) fn builder<'b>(&'b mut self, _plan: &Plan) -> Builder<'b> {
        let store = if self.working.linear {
            0
        } else {
            let t = color::default_transfer(self.working.space);
            1 + color::transfer_id(if t == m::Transfer::Linear { m::Transfer::Srgb } else { t })
        };
        Builder {
            eng: &mut self.fx,
            pool: &mut self.pool,
            device: &self.gpu.device,
            bgl1: &self.bgl1,
            passes: Vec::new(),
            temps: Vec::new(),
            store,
            max_samples: self.tier.line_samples,
            problems: Vec::new(),
        }
    }

    fn finish_builder(plan: &mut Plan, passes: Vec<fx::Pass>, temps: Vec<Arc<Tex>>, problems: Vec<String>, who: &str) {
        let passes = labelled(fx::fuse_colour(passes), who);
        plan.fx_temps.extend(temps);
        plan.stats.unsupported.extend(problems.into_iter().map(|m| format!("{who}: {m}")));
        if !passes.is_empty() {
            plan.jobs.push(Job {
                target: passes[0].out.clone(),
                clear: false,
                cmds: Vec::new(),
                root: false,
                fx: passes,
                flow: None,
                draw: false,
                parts: None,
            });
        }
    }

    /// Draws blurred glyphs of a text layer: the node-local `part` renders into an
    /// offscreen, blurs by `radius` node pixels (scaled into the target) and composites
    /// with the node's opacity, blend, masks and matte.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn blurred_part(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
        radius: f64,
        part: Scene,
    ) {
        let n = &ctx.g.nodes[i];
        let ta = space.xform.then(&n.world);
        let scene = part.transformed(&Xf(ta.0));
        let tex = self.temp(plan, space.size);
        let mut inner = Vec::new();
        let key = &inner as *const Vec<Cmd> as usize;
        plan.vbatches.push(VBatch {
            key,
            space: *space,
            hash: h(&[root_hash, hf(radius), hf(ctx.g.time), 0xb1]),
            scene,
            first: n.id.to_string(),
        });
        self.flush_vec(plan, &mut inner);
        plan.jobs.push(Job::draws(tex.clone(), true, inner, false));
        let mut b = self.builder(plan);
        // a text blur radius is twice the standard deviation
        let out = b.blur(&tex, 0.5 * radius * Xf(ta.0).max_scale());
        let (passes, temps, problems) =
            (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
        Self::finish_builder(plan, passes, temps, problems, &n.id);
        let (w, hgt) = (space.size[0] as f64, space.size[1] as f64);
        let hash = h(&[root_hash, sr_eval::rng::hash_str(&n.id), hf(radius), hf(ctx.g.time), 0xb2]);
        self.composite(plan, ctx, i, space, op, out, [0.0, 0.0, w, hgt], cmds, hash);
    }

    /// Fills and strokes of a shape that use pattern paints: the parts using each
    /// pattern render as coverage, and a pattern-filled quad over them draws through
    /// that coverage as its alpha matte.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pattern_fills(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        blend: u32,
        seed: u32,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let n = &ctx.g.nodes[i];
        let mut ids: Vec<String> = Vec::new();
        for name in ["fill", "stroke"] {
            let id = match n.props.get(name) {
                Some(Value::PaintRef(r)) => Some(r.to_string()),
                Some(_) => None,
                None => match n.elem.get_attr(name) {
                    Some(AttrValue::Paint(sr_model::values::Paint::Ref(r))) => Some(r.0.clone()),
                    _ => None,
                },
            };
            if let Some(id) = id {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        for pid in ids {
            let Some(m::PaintsChild::Pattern(pt)) =
                ctx.p.scene.paints.as_ref().and_then(|ps| ps.children.iter().find(|c| c.id() == Some(pid.as_str())))
            else {
                continue;
            };
            let img =
                ctx.p.scene.assets.as_ref().and_then(|a| a.children.iter().find(|c| c.id() == Some(pt.asset.as_str())));
            let Some(AssetsChild::Image(img)) = img else {
                plan.stats.errors.push(format!("{}: pattern {pid} needs an image asset", n.id));
                continue;
            };
            let base = Self::base_dir(ctx.p);
            let Some(itex) = (match sr_model::assets::resolve(&img.src, &base) {
                sr_model::assets::Resolved::Local(p) => self.image(
                    p,
                    img.color_space,
                    img.transfer,
                    img.alpha,
                    img.color_profile == sr_model::model::ColorProfile::Embedded,
                ),
                _ => None,
            }) else {
                continue;
            };
            // coverage of the parts painted with this pattern
            let ta = space.xform.then(&n.world);
            let tol = 0.05 / Xf(ta.0).max_scale().max(1e-6);
            let target = Value::PaintRef(pid.as_str().into());
            let mut pf = |v: &Value, _b: [f64; 4]| {
                (v == &target).then_some(sr_vector::Paint::Solid { rgba: [1.0; 4], srgb: false })
            };
            let Ok(mut scene) = crate::vector::shape_scene(n, &mut pf, tol) else { continue };
            let ds = crate::vector::deformers(n, ctx.g, n.size.unwrap_or([0.0, 0.0]));
            crate::vector::deform_scene(&mut scene, &ds, tol);
            let lb = scene.bounds().0;
            if !(lb[2] > lb[0] && lb[3] > lb[1]) {
                continue;
            }
            let cover = self.temp(plan, space.size);
            let mut inner = Vec::new();
            let key = &inner as *const Vec<Cmd> as usize;
            plan.vbatches.push(VBatch {
                key,
                space: *space,
                hash: h(&[root_hash, sr_eval::rng::hash_str(&pid), hf(ctx.g.time), 0x9a7]),
                scene: scene.transformed(&Xf(ta.0)),
                first: n.id.to_string(),
            });
            self.flush_vec(plan, &mut inner);
            plan.jobs.push(Job::draws(cover.clone(), true, inner, false));
            // the pattern over the covered box, through the coverage
            self.flush_vec(plan, cmds);
            let local = [lb[0], lb[1], lb[2], lb[3]];
            let proj = self.proj25(ctx.g, ctx.p, space);
            let (bounds, first_vertex) =
                self.push_quad(plan, space, &n.world, None, local, [0.0, 0.0, 1.0, 1.0], &proj);
            let paint = plan.paints.pattern(pt, ctx.g, [img.width as f64, img.height as f64]);
            let d = Draw {
                opacity: op as f32,
                blend,
                src_kind: src::PATTERN,
                paint,
                seed,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                target_size: [space.size[0] as f32, space.size[1] as f32],
                box_rect: local.map(|v| v as f32),
                flags: flag::EDGE | flag::MATTE,
                matte_mode: 0,
                ..Default::default()
            };
            plan.draws.push(d);
            cmds.push(Cmd {
                draw: (plan.draws.len() - 1) as u32,
                first_vertex,
                count: 6,
                src: itex,
                backdrop: (blend >= 2 && !fixed_function_blend(blend)).then_some(bounds),
                matte: Some(cover),
                hash: h(&[
                    root_hash,
                    sr_eval::rng::hash_str(&n.id),
                    sr_eval::rng::hash_str(&pid),
                    hf(ctx.g.time),
                    0x9a8,
                ]),
                pre: None,
            });
        }
        plan.stats.unsupported.retain(|m| m != &format!("{}: pattern paint on vector content", n.id));
    }

    /// Draws node `i` bare into a new offscreen in `inner` space.
    fn bare_render(&mut self, plan: &mut Plan, ctx: &Ctx, i: usize, inner: &Space) -> Arc<Tex> {
        let n = &ctx.g.nodes[i];
        let tex = self.temp(plan, inner.size);
        self.bare.insert(n.id.clone());
        let mut c = Vec::new();
        self.emit(plan, ctx, i, inner, n.world_opacity, &mut c, true, 0);
        self.flush_vec(plan, &mut c);
        self.bare.remove(&n.id);
        plan.jobs.push(Job::draws(tex.clone(), true, c, false));
        tex
    }

    /// Draws the node with id `id` from the scene at time `t`, bare, into `inner`; or says why it cannot.
    fn bare_at(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        id: &Arc<str>,
        t: f64,
        inner: &Space,
    ) -> Result<Arc<Tex>, SubMiss> {
        let sub = ctx.sub.ok_or(SubMiss::NoProvider)?;
        let sg = sub.at(t);
        let j = *sg.index.get(id).ok_or(SubMiss::Absent)?;
        let sctx = ctx.at(&sg);
        let was = std::mem::replace(&mut self.sampling, true);
        let t = self.bare_render(plan, &sctx, j, inner);
        self.sampling = was;
        Ok(t)
    }

    /// Composites `tex` covering pixel rectangle `rect` of `space` with node `i`'s opacity, blend, masks and matte.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn composite(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        tex: Arc<Tex>,
        rect: [f64; 4],
        cmds: &mut Vec<Cmd>,
        hash: u64,
    ) {
        let n = &ctx.g.nodes[i];
        let clip = if n.clip { n.size } else { None };
        let mask_box = n.size.unwrap_or(space.extent());
        let (mask_off, mask_count) = self.masks_of(plan, n, mask_box, clip);
        let d = Draw {
            opacity: op as f32,
            blend: match (self.bare.contains(&n.id), self.blend_of(n)) {
                (true, _) => 0,
                // an adjustment layer replaces the backdrop by its effect within its coverage
                (false, 0) if n.kind == "adjustment" => BLEND_ADJUST,
                (false, b) => b,
            },
            src_kind: src::TEXTURE,
            mask_off,
            mask_count,
            seed: sr_eval::rng::hash_str(&n.id) as u32,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        };
        self.flush_vec(plan, cmds);
        self.frame_rect = Some(rect);
        // a node placed in 2.5D composites its (flat) result through the camera, about the node's pivot
        let mut hash = hash;
        if let (Some(t), None) = (n.three_d, self.frame_three) {
            let proj = self.proj25(ctx.g, ctx.p, space);
            let cols = proj.to_cols_array().map(|v| v.to_bits() as u64);
            hash = h(&[hash, hf(t[0]), hf(t[1]), hf(t[2]), hf(n.anchor[0]), hf(n.anchor[1]), h(&cols)]);
            self.frame_three = Some((t, n.anchor, proj));
        }
        self.draw_cmd(
            plan,
            ctx,
            i,
            space,
            d,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            &n.world,
            tex,
            cmds,
            hash,
            true,
        );
        self.frame_rect = None;
        self.frame_three = None;
    }

    #[allow(clippy::too_many_arguments)]
    fn cx_for<'c>(
        &self,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        rect: [f64; 4],
        to_uv: &'c dyn Fn([f64; 2]) -> [f64; 2],
        color: &'c fx::ColorFn<'c>,
        gradient: &'c fx::GradientFn<'c>,
        base: &'c std::path::Path,
    ) -> Cx<'c> {
        let n = &ctx.g.nodes[i];
        let ta = space.xform.then(&n.world);
        let px = Xf(ta.0).max_scale().max(1e-6);
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let center = match n.size {
            Some(s) => {
                let c = ta.apply([s[0] * 0.5, s[1] * 0.5]);
                [(c[0] - rect[0]) / w, (c[1] - rect[1]) / h]
            }
            None => [0.5, 0.5],
        };
        Cx {
            px,
            scale: Xf(space.xform.0).max_scale().max(1e-6),
            lin: [ta.0[0], ta.0[1], ta.0[2], ta.0[3]],
            to_uv,
            center,
            content: None,
            time: ctx.g.time,
            frame: ctx.g.frame,
            color,
            gradient,
            source: None,
            lights: Vec::new(),
            working: self.working,
            base,
            fps: fps_of(ctx.p),
            local_time: n.local_time,
            offset: [0.0, 0.0],
            frame_size: ctx.g.size,
            node: n.id.to_string(),
            named: HashMap::new(),
            audio: self.audio.clone(),
            seed: self.seed,
        }
    }

    /// The box of node `i`'s content in `space`'s pixels: the union of the boxes of the node and its
    /// sized descendants. None when nothing has a size.
    fn content_box(ctx: &Ctx, i: usize, space: &Space) -> Option<[f64; 4]> {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut stack = vec![i];
        while let Some(k) = stack.pop() {
            let n = &ctx.g.nodes[k];
            if let Some(s) = n.size {
                let ta = space.xform.then(&n.world);
                for p in [[0.0, 0.0], [s[0], 0.0], [s[0], s[1]], [0.0, s[1]]] {
                    let q = ta.apply(p);
                    b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
                }
            }
            stack.extend(ctx.kids[k].iter().copied());
        }
        // tiles: each box grown by 2 px to whole pixels, within 64 px of the target
        let (w, h) = (space.size[0] as f64, space.size[1] as f64);
        let b = [
            (b[0] - 2.0).floor().max(-64.0),
            (b[1] - 2.0).floor().max(-64.0),
            (b[2] + 2.0).ceil().min(w + 64.0),
            (b[3] + 2.0).ceil().min(h + 64.0),
        ];
        (b[2] > b[0] && b[3] > b[1]).then_some(b)
    }

    /// Point lights referenced by an effect, as uv position, intensity and radius in target
    /// heights (the unit the shader measures distances in). `scale` is target pixels per unit of
    /// the node's space, where the light's position and range are given; `height` the target's.
    fn lights_of(
        &self,
        ctx: &Ctx,
        a: &Attrs,
        to_uv: &dyn Fn([f64; 2]) -> [f64; 2],
        scale: f64,
        height: f64,
    ) -> Vec<[f32; 4]> {
        crate::vector::note_read(a.e, "lights");
        let Some(AttrValue::Tokens(ids)) = a.e.get_attr("lights") else { return Vec::new() };
        let Some(lights) = ctx.p.scene.lights.as_ref() else { return Vec::new() };
        ids.iter()
            .filter_map(|id| {
                sr_model::element::children(lights).into_iter().find(|l| eid(*l).as_deref() == Some(id.as_str()))
            })
            .map(|l| {
                let key = eid(l).unwrap_or_default();
                let la = Attrs { e: l, props: element_props(ctx.g, &key) };
                let uv = to_uv([la.num("x", 0.0), la.num("y", 0.0)]);
                let range = la.opt("range").or(la.opt("distance")).map(|r| r * scale / height).unwrap_or(0.5);
                [uv[0] as f32, uv[1] as f32, la.num("intensity", 1.0) as f32, range as f32]
            })
            .collect()
    }

    /// Resolves an effect's `source` IDREF (a node or an image asset) over the offscreen `rect`.
    fn effect_source(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        a: &Attrs,
        space: &Space,
        rect: [f64; 4],
    ) -> Option<Arc<Tex>> {
        let id = a.str("source")?;
        self.node_texture(plan, ctx, &id, space, rect, true)
    }

    /// A node rendered over the offscreen `rect` (`report`: note an unknown id).
    fn node_texture(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        id: &str,
        space: &Space,
        rect: [f64; 4],
        report: bool,
    ) -> Option<Arc<Tex>> {
        let size = [(rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32];
        if let Some(j) = ctx.g.nodes.iter().position(|n| *n.id == *id) {
            let inner = Space {
                xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform),
                size,
                unit: space.unit,
            };
            let sn = &ctx.g.nodes[j];
            let tex = self.temp(plan, size);
            let mut c = Vec::new();
            self.emit(plan, ctx, j, &inner, sn.world_opacity.max(1e-6), &mut c, true, 0);
            self.flush_vec(plan, &mut c);
            plan.jobs.push(Job::draws(tex.clone(), true, c, false));
            return Some(tex);
        }
        if report {
            plan.stats
                .unsupported
                .push(format!("effect source {id}: must name a node (place an image asset on a hidden layer)"));
        }
        None
    }

    /// Named sampler inputs of a shader effect: `<param>` values naming a node (rendered over
    /// the offscreen) or an image asset (at its own size).
    fn shader_samplers(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        e: &m::Effect,
        space: &Space,
        rect: [f64; 4],
    ) -> HashMap<String, Arc<Tex>> {
        let mut out = HashMap::new();
        for (name, value) in crate::shader::param_map(e as &dyn Element) {
            let value = value.trim().to_string();
            if value.is_empty() || name == "padding" {
                continue;
            }
            if ctx.g.nodes.iter().any(|n| *n.id == *value) {
                if let Some(t) = self.node_texture(plan, ctx, &value, space, rect, false) {
                    out.insert(name, t);
                }
                continue;
            }
            let Some((a, doc)) = self.asset(ctx.p, &value) else { continue };
            if let AssetsChild::Image(i) = a {
                let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
                let (src, cs, tf) = self.representation(&i.representations, &i.src, i.color_space, i.transfer);
                if let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(src, &base) {
                    if let Some(t) =
                        self.image(path, cs, tf, i.alpha, i.color_profile == sr_model::model::ColorProfile::Embedded)
                    {
                        out.insert(name, t);
                    }
                }
            }
        }
        out
    }

    /// Applies node `i`'s effects. Returns true when the node was handled (drawn or invisible).
    #[allow(clippy::too_many_arguments)]
    fn node_effects(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
        ids: &[String],
    ) -> bool {
        let g = ctx.g;
        let n = &g.nodes[i];
        let effs: Vec<&m::Effect> =
            ids.iter().filter_map(|id| find_effect(ctx.p, id)).filter(|e| e.enabled && self.drawn_at_tier(e)).collect();
        if effs.is_empty() {
            return false;
        }
        let op = if iso_op > 0.0 { n.world_opacity / iso_op } else { 0.0 };
        let ta = space.xform.then(&n.world);
        let px = Xf(ta.0).max_scale().max(1e-6);
        let (fw, fh) = (space.size[0] as f64, space.size[1] as f64);
        let attrs: Vec<Attrs> =
            effs.iter().map(|e| Attrs { e: *e as &dyn Element, props: element_props(g, &e.id) }).collect();
        // custom shaders take `padding` in document pixels at the render scale (not the node's)
        let render_scale = Xf(space.xform.0).max_scale().max(1e-6);
        let pad: f64 = effs
            .iter()
            .zip(&attrs)
            .map(|(e, a)| reach(e, a) * if e.r#type.as_str() == "shader" { render_scale } else { px })
            .sum();
        // the times whose content the chain shows: posterize-time shows the start of its step
        // instead of now, echo adds earlier frames
        let t = g.time;
        let post =
            effs.iter().zip(&attrs).find(|(e, _)| e.r#type.as_str() == "posterize-time").map(|(_, a)| posterized(t, a));
        let echo = effs.iter().zip(&attrs).find(|(e, _)| e.r#type.as_str() == "echo").map(|(_, a)| {
            let count = (a.num("samples", 16.0) as usize).min(self.tier.echo_samples).clamp(2, 16);
            (count, a.num("amount", 1.0) / fps_of(ctx.p))
        });
        let mut shown: Vec<f64> = match (post, echo) {
            (Some(tq), None) => vec![tq],
            (post, Some((count, delay))) => (0..count).map(|k| t - k as f64 * delay).chain(post).collect(),
            (None, None) => vec![t],
        };
        if ctx.sub.is_none() {
            // without other frames the chain draws the node as it is now
            shown = vec![t];
        }
        let graphs: Vec<Option<Arc<super::SubGraph>>> =
            shown.iter().map(|&s| (s != t).then(|| ctx.sub.map(|sub| sub.at(s))).flatten()).collect();
        // where the node is at each shown time: its box and its transform into the target
        let boxes: Vec<(Affine, Option<[f64; 2]>)> = graphs
            .iter()
            .filter_map(|sg| match sg {
                Some(sg) => sg.index.get(&n.id).map(|&j| (space.xform.then(&sg.g.nodes[j].world), sg.g.nodes[j].size)),
                None => Some((ta, n.size)),
            })
            .collect();
        let reshaped = sr_model::element::children(&*n.elem).iter().any(|c| RESHAPING.contains(&c.element_name()));
        // a stencil blend cuts the backdrop away outside the node, so its result must cover the whole target
        let bounded = matches!(n.kind, "layer" | "shape")
            && ctx.kids[i].is_empty()
            && !matches!(self.blend_of(n), 29 | 30)
            && !reshaped
            && !boxes.is_empty()
            && boxes.iter().all(|(_, s)| s.is_some())
            && pad.is_finite();
        // warps move pixels in from beyond the frame: the offscreen then covers the frame plus their reach, so a
        // plane that overscans the frame keeps its picture there (up to 256 px)
        let warps = effs.iter().any(|e| WARPS.contains(&e.r#type.as_str()));
        let warp_margin = if warps { pad.ceil().clamp(0.0, 256.0) } else { 0.0 };
        let mut rect = [-warp_margin, -warp_margin, fw + warp_margin, fh + warp_margin];
        if bounded {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            let mut reach_px: f64 = 0.0;
            for (w, s) in &boxes {
                let s = s.unwrap_or([0.0; 2]);
                let scale = Xf(w.0).max_scale().max(1e-6);
                // effects' reach (at the largest scale shown) plus the stroke, in target pixels
                reach_px = reach_px.max(pad / px * scale + stroke_reach(n) * scale);
                for q in [[0.0, 0.0], [s[0], 0.0], [s[0], s[1]], [0.0, s[1]]].map(|p| w.apply(p)) {
                    x0 = x0.min(q[0]);
                    y0 = y0.min(q[1]);
                    x1 = x1.max(q[0]);
                    y1 = y1.max(q[1]);
                }
            }
            // custom shaders see tile: the layer's pixels with a 2 px margin,
            // reaching up to 64 px past the frame (their resolution and uv depend on it); a warp reads the
            // node's overscan past the frame as far as it reaches
            let shader = effs.iter().any(|e| e.r#type.as_str() == "shader");
            // a node in 2.5D is drawn flat and placed afterwards, so what shows can come from beyond the frame
            // as well as from inside it: its offscreen is not clamped to the frame (at most 2048 px past it)
            let (m, lo, hx, hy) = if n.three_d.is_some() {
                (1.0, -2048.0, fw + 2048.0, fh + 2048.0)
            } else if shader {
                (2.0, -64.0, fw + 64.0, fh + 64.0)
            } else {
                (1.0, -warp_margin, fw + warp_margin, fh + warp_margin)
            };
            rect = [
                (x0 - reach_px - m).floor().max(lo),
                (y0 - reach_px - m).floor().max(lo),
                (x1 + reach_px + m).ceil().min(hx),
                (y1 + reach_px + m).ceil().min(hy),
            ];
            // custom shaders see the tile's size, so only built-in chains get the rounded target
            if !shader {
                rect = round_rect(rect, [lo, lo], [hx, hy]);
            }
        }
        if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
            return true;
        }
        let size = [(rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32];
        let inner = Space {
            xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform),
            size,
            unit: space.unit,
        };
        // what the result depends on: the node as the chain shows it (its own state and transform
        // into the target, and its subtree), the elements it uses, its effects' parameters now,
        // and the time for effects that change with it (posterize-time counts by its step)
        let (src_ctx, j) = match (post, echo, graphs.first()) {
            (Some(_), None, Some(Some(sg))) => match sg.index.get(&n.id) {
                Some(&j) => (ctx.at(sg), j),
                None => (ctx.at(sg), usize::MAX),
            },
            _ => (*ctx, i),
        };
        let shown_hash = if j == usize::MAX {
            0
        } else {
            let sn = &src_ctx.g.nodes[j];
            h(&[
                Self::node_hash(&src_ctx, sn, &inner.xform.then(&sn.world)),
                Self::content_hash(&src_ctx, j, &inner.xform),
                Self::deps_hash(&src_ctx, j, &inner.xform),
            ])
        };
        // the nodes the effects read are drawn as they are now, whatever time the chain shows
        let read: Vec<u64> =
            Self::source_nodes(ctx, n).into_iter().map(|j| Self::subtree_hash(ctx, j, &inner.xform)).collect();
        let time_dep =
            effs.iter().any(|e| TIME_VARYING.contains(&e.r#type.as_str()) && e.r#type.as_str() != "posterize-time");
        let echo_times = if echo.is_some() { h(&shown.iter().map(|&s| hf(s)).collect::<Vec<_>>()) } else { 0 };
        let hash = h(&[
            shown_hash,
            Self::effects_state(ctx, n),
            size[0] as u64,
            size[1] as u64,
            if time_dep { hf(t) } else { 0 },
            echo_times,
            sr_eval::rng::hash_str(&ids.join(" ")),
            h(&read),
        ]);
        let key = format!("fx:{}:{}x{}", n.id, size[0], size[1]);
        self.used.insert(key.clone());
        let out = match self.subtree.get(&key) {
            Some((hh, t)) if *hh == hash && self.fx_cache => {
                plan.stats.cache_hits += 1;
                t.clone()
            }
            _ => {
                let target = match self.subtree.get(&key) {
                    Some((_, t)) if t.size == size && Arc::strong_count(t) <= 2 => t.clone(),
                    _ => {
                        self.pool.created += 1;
                        Arc::new(resources::create_as(&self.gpu.device, &self.bgl1, self.format, size, 1, "effects"))
                    }
                };
                plan.stats.effect_pixels += size[0] as u64 * size[1] as u64;
                self.run_chain(plan, ctx, i, space, rect, &inner, &effs, &attrs, &target);
                self.subtree.insert(key, (hash, target.clone()));
                target
            }
        };
        let hash_cmd = h(&[root_hash, hash, hf(op), rect[0] as u64, rect[1] as u64, 0x6678]);
        self.composite(plan, ctx, i, space, op, out, rect, cmds, hash_cmd);
        true
    }

    /// Renders the node's source (honouring time effects) and runs the chain into `target`.
    #[allow(clippy::too_many_arguments)]
    fn run_chain(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        rect: [f64; 4],
        inner: &Space,
        effs: &[&m::Effect],
        attrs: &[Attrs],
        target: &Arc<Tex>,
    ) {
        let g = ctx.g;
        let n = &g.nodes[i];
        let fps = fps_of(ctx.p);
        let t = g.time;
        // posterize-time and echo change what the source is; they apply first, in document order
        let mut src: Option<Arc<Tex>> = None;
        for (e, a) in effs.iter().zip(attrs) {
            match e.r#type.as_str() {
                "posterize-time" => {
                    let tq = posterized(t, a);
                    match self.bare_at(plan, ctx, &n.id, tq, inner) {
                        Ok(tx) => src = Some(tx),
                        Err(SubMiss::NoProvider) => {
                            plan.stats.unsupported.push(format!("{}: posterize-time needs a sub-frame provider", e.id))
                        }
                        // the node was not there at the start of its step: it is drawn as it was, which is nothing
                        Err(SubMiss::Absent) => src = Some(self.solid_texture([0.0; 4])),
                    }
                }
                "echo" => {
                    if ctx.sub.is_none() {
                        plan.stats.unsupported.push(format!("{}: echo needs a sub-frame provider", e.id));
                        continue;
                    }
                    let count = (a.num("samples", 16.0) as usize).min(self.tier.echo_samples).clamp(2, 16);
                    let delay = a.num("amount", 1.0) / fps;
                    let decay = a.num("intensity", 1.0).clamp(0.0, 1.0);
                    let weights: Vec<f64> =
                        (0..count).map(|k| if decay >= 1.0 { 1.0 } else { decay.powi(k as i32) }).collect();
                    let total: f64 = weights.iter().sum();
                    let mut frames = Vec::new();
                    for k in 0..count {
                        let tex = if k == 0 {
                            src.clone().unwrap_or_else(|| self.bare_render(plan, ctx, i, inner))
                        } else {
                            self.bare_at(plan, ctx, &n.id, t - k as f64 * delay, inner)
                                .unwrap_or_else(|_| self.solid_texture([0.0; 4]))
                        };
                        frames.push(tex);
                    }
                    let acc = self.temp(plan, inner.size);
                    let mut b = self.builder(plan);
                    for (k, f) in frames.iter().enumerate() {
                        b.accumulate(&acc, f, weights[k] / total, k == 0);
                    }
                    let (passes, temps, problems) =
                        (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
                    Self::finish_builder(plan, labelled(passes, &format!("{}/{}", n.id, e.id)), temps, problems, &e.id);
                    src = Some(acc);
                }
                _ => {}
            }
        }
        let mut cur = src.unwrap_or_else(|| self.bare_render(plan, ctx, i, inner));
        let (w, hgt) = (inner.size[0] as f64, inner.size[1] as f64);
        let ta = inner.xform.then(&n.world);
        let to_uv = move |p: [f64; 2]| {
            let q = ta.apply(p);
            [q[0] / w, q[1] / hgt]
        };
        let tokens = self.tokens.clone();
        let working = self.working;
        let color = move |v: &Value| -> Option<[f64; 4]> {
            match v {
                Value::Color(c) => Some(working.from_literal(*c)),
                Value::Str(s) if s.starts_with("token:") => tokens.get(&s[6..]).map(|c| working.from_literal(*c)),
                _ => None,
            }
        };
        let grads: HashMap<String, fx::Stops> = attrs
            .iter()
            .flat_map(|a| {
                crate::vector::quiet(|| [a.paint("paint"), a.str("source").map(|s| Value::PaintRef(s.into()))])
            })
            .flatten()
            .filter_map(|v| {
                let key = match &v {
                    Value::PaintRef(id) => id.to_string(),
                    _ => return None,
                };
                self.gradient_value(ctx.p, g, &v).map(|s| (key, s))
            })
            .collect();
        let gradient = move |v: &Value| -> Option<fx::Stops> {
            match v {
                Value::PaintRef(id) => grads.get(&**id).cloned(),
                _ => None,
            }
        };
        let base = Self::base_dir(ctx.p);
        for (e, a) in effs.iter().zip(attrs) {
            let kind = e.r#type.as_str();
            if matches!(kind, "posterize-time" | "echo") {
                continue;
            }
            if kind == "pixel-motion-blur" {
                let prev = match self.bare_at(plan, ctx, &n.id, t - 1.0 / fps, inner) {
                    Ok(prev) => prev,
                    Err(SubMiss::NoProvider) => {
                        plan.stats.unsupported.push(format!("{}: pixel-motion-blur needs a sub-frame provider", e.id));
                        continue;
                    }
                    // nothing was there a frame ago: no motion to blur
                    Err(SubMiss::Absent) => continue,
                };
                let slot: fx::FlowSlot = Arc::new(std::sync::OnceLock::new());
                let now = cur.clone();
                plan.jobs.push(Job {
                    target: now.clone(),
                    clear: false,
                    cmds: Vec::new(),
                    root: false,
                    fx: Vec::new(),
                    flow: Some((prev, now.clone(), slot.clone())),
                    draw: false,
                    parts: None,
                });
                let shutter = a.num("amount", 1.0) * 0.5;
                let samples = a.num("samples", 16.0) as u32;
                let mut b = self.builder(plan);
                let out = b.flow_blur(&now, slot, shutter, samples);
                let (passes, temps, problems) =
                    (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
                Self::finish_builder(plan, labelled(passes, &format!("{}/{}", n.id, e.id)), temps, problems, &e.id);
                cur = out;
                continue;
            }
            let source =
                if matches!(kind, "displacement-map" | "difference-key" | "shader") && a.str("source").is_some() {
                    self.effect_source(plan, ctx, a, space, rect)
                } else {
                    None
                };
            let named = if kind == "shader" { self.shader_samplers(plan, ctx, e, space, rect) } else { HashMap::new() };
            let mut cx = self.cx_for(ctx, i, inner, [0.0, 0.0, w, hgt], &to_uv, &color, &gradient, &base);
            cx.content = Self::content_box(ctx, i, inner);
            cx.source = source;
            cx.named = named;
            cx.offset = [rect[0], space.size[1] as f64 - rect[3]];
            cx.frame_size = [space.size[0] as f64, space.size[1] as f64];
            cx.lights = if kind == "lighting" { self.lights_of(ctx, a, &to_uv, cx.px, hgt) } else { Vec::new() };
            let mut b = self.builder(plan);
            let r = b.effect(*e as &dyn Element, a, &cur, &cx);
            let (passes, temps, problems) =
                (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
            Self::finish_builder(plan, labelled(passes, &format!("{}/{}", n.id, e.id)), temps, problems, &e.id);
            match r {
                Ok(t) => cur = t,
                Err(msg) => plan.stats.unsupported.push(format!("{}: {msg}", e.id)),
            }
        }
        // the chain's last image lands in the cached target: retarget the pass that produced it,
        // else copy (a chain that ran no passes)
        for job in plan.jobs.iter_mut().rev() {
            if let Some(pass) = job.fx.iter_mut().rev().find(|p| Arc::ptr_eq(&p.out, &cur)) {
                if !pass.additive && pass.out.size == target.size {
                    pass.out = target.clone();
                    job.target = target.clone();
                    return;
                }
                break;
            }
        }
        let mut b = self.builder(plan);
        let mut v = [[0.0f32; 4]; 8];
        v[0][0] = 1.0;
        b.passes.push(fx::Pass {
            entry: fx::Entry::Copy,
            params: fx::Params::new(v, [0; 4]),
            src: cur,
            aux: fx::Aux::None,
            aux2: None,
            lut: None,
            out: target.clone(),
            additive: false,
            clear: true,
            custom: None,
            label: String::new(),
        });
        let (passes, temps, problems) =
            (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
        Self::finish_builder(plan, passes, temps, problems, &n.id);
    }

    /// An adjustment layer: effects on the composite of everything below it in this target.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn adjust(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let g = ctx.g;
        let n = &g.nodes[i];
        let ids = effect_ids(&*n.elem);
        let effs: Vec<&m::Effect> =
            ids.iter().filter_map(|id| find_effect(ctx.p, id)).filter(|e| e.enabled && self.drawn_at_tier(e)).collect();
        if effs.is_empty() || op <= 0.0 {
            return;
        }
        self.flush_vec(plan, cmds);
        let snapshot = self.temp(plan, space.size);
        let (w, hgt) = (space.size[0] as f64, space.size[1] as f64);
        let ta = space.xform.then(&n.world);
        let to_uv = move |p: [f64; 2]| {
            let q = ta.apply(p);
            [q[0] / w, q[1] / hgt]
        };
        let tokens = self.tokens.clone();
        let working = self.working;
        let color = move |v: &Value| -> Option<[f64; 4]> {
            match v {
                Value::Color(c) => Some(working.from_literal(*c)),
                Value::Str(s) if s.starts_with("token:") => tokens.get(&s[6..]).map(|c| working.from_literal(*c)),
                _ => None,
            }
        };
        let attrs: Vec<Attrs> =
            effs.iter().map(|e| Attrs { e: *e as &dyn Element, props: element_props(g, &e.id) }).collect();
        let grads: HashMap<String, fx::Stops> = attrs
            .iter()
            .filter_map(|a| crate::vector::quiet(|| a.paint("paint")))
            .filter_map(|v| match &v {
                Value::PaintRef(id) => self.gradient_value(ctx.p, g, &v).map(|s| (id.to_string(), s)),
                _ => None,
            })
            .collect();
        let gradient = move |v: &Value| -> Option<fx::Stops> {
            match v {
                Value::PaintRef(id) => grads.get(&**id).cloned(),
                _ => None,
            }
        };
        let base = Self::base_dir(ctx.p);
        let mut cur = snapshot.clone();
        let mut passes = Vec::new();
        for (e, a) in effs.iter().zip(&attrs) {
            let kind = e.r#type.as_str();
            if matches!(kind, "posterize-time" | "echo" | "pixel-motion-blur") {
                plan.stats.unsupported.push(format!("{}: {kind} on an adjustment layer", e.id));
                continue;
            }
            let mut cx = self.cx_for(ctx, i, space, [0.0, 0.0, w, hgt], &to_uv, &color, &gradient, &base);
            cx.lights = if kind == "lighting" { self.lights_of(ctx, a, &to_uv, cx.px, hgt) } else { Vec::new() };
            let mut b = self.builder(plan);
            let r = b.effect(*e as &dyn Element, a, &cur, &cx);
            passes.append(&mut b.passes);
            let (temps, problems) = (std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
            plan.fx_temps.extend(temps);
            plan.stats.unsupported.extend(problems.into_iter().map(|m| format!("{}: {m}", e.id)));
            match r {
                Ok(t) => cur = t,
                Err(msg) => plan.stats.unsupported.push(format!("{}: {msg}", e.id)),
            }
        }
        if passes.is_empty() {
            return;
        }
        let hash = h(&[root_hash, ctx.elements, sr_eval::rng::hash_str(&ids.join(" ")), hf(op), hf(g.time), 0x6164]);
        self.composite(plan, ctx, i, space, op, cur, [0.0, 0.0, w, hgt], cmds, hash);
        if let Some(c) = cmds.last_mut() {
            c.pre = Some(Box::new(AdjPre { snapshot, passes: labelled(fx::fuse_colour(passes), &n.id), three: None }));
        }
    }

    /// Whether the quality tier draws effect `e` (drafts leave out grain and noise).
    fn drawn_at_tier(&self, e: &m::Effect) -> bool {
        self.tier.grain || !matches!(e.r#type.as_str(), "film-grain" | "noise")
    }

    /// Whether node `i` has motion blur (its own setting, else its parents', else the project's).
    pub(super) fn motion_blur_on(&self, ctx: &Ctx, i: usize) -> bool {
        let g = ctx.g;
        let mut k = Some(i);
        while let Some(j) = k {
            match g.nodes[j].elem.get_attr("motionBlur") {
                Some(AttrValue::Str(s)) if s == "on" => return true,
                Some(AttrValue::Str(s)) if s == "off" => return false,
                _ => {}
            }
            k = g.nodes[j].parent.map(|p| p as usize);
        }
        ctx.p.scene.project.motion_blur
    }

    /// The `shutterAngle` of node `i` or of its nearest ancestor that sets one.
    fn node_shutter_angle(g: &sr_eval::FrameGraph, i: usize) -> Option<f64> {
        let mut k = Some(i);
        while let Some(j) = k {
            if let Some(AttrValue::Num(v)) = g.nodes[j].elem.get_attr("shutterAngle") {
                return Some(v);
            }
            k = g.nodes[j].parent.map(|p| p as usize);
        }
        None
    }

    /// Accumulates the node over the shutter. Returns false when it does not move enough to need it.
    #[allow(clippy::too_many_arguments)]
    fn motion_blur(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) -> bool {
        let Some(sub) = ctx.sub else { return false };
        let g = ctx.g;
        let n = &g.nodes[i];
        let three = sr_eval::draws_in_3d(n.kind);
        if three && Self::three_members(g, i).first() != Some(&i) {
            // drawn, and blurred, with the pass of the first object that shares its parent
            return true;
        }
        let pr = &ctx.p.scene.project;
        let fps = fps_of(ctx.p);
        // the node's own shutter angle, else the nearest ancestor's, else the project's
        let angle = Self::node_shutter_angle(g, i).unwrap_or(pr.shutter_angle);
        if angle <= 0.0 {
            return false;
        }
        let count = (pr.motion_blur_samples as usize).min(self.tier.motion_blur_samples).clamp(1, 256);
        let times: Vec<f64> = (0..count)
            .map(|k| g.time + (pr.shutter_phase / 360.0 + angle / 360.0 * (k as f64 + 0.5) / count as f64) / fps)
            .collect();
        let (first, last) = (sub.at(times[0]), sub.at(times[count - 1]));
        let (Some(&a), Some(&b)) = (first.index.get(&n.id), last.index.get(&n.id)) else { return false };
        let s = n.size.unwrap_or([100.0, 100.0]);
        let corners = [[0.0, 0.0], [s[0], 0.0], [s[0], s[1]], [0.0, s[1]]];
        let (wa, wb) = (space.xform.then(&first.g.nodes[a].world), space.xform.then(&last.g.nodes[b].world));
        let moved = corners.iter().map(|c| {
            let (p, q) = (wa.apply(*c), wb.apply(*c));
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
        });
        let moved = moved.fold(0.0f64, f64::max);
        // a 3D pass moves whenever its camera or any member does; its 2D box says nothing about that
        let own_motion = three || moved >= 0.5;
        if pr.adaptive_motion_blur && !own_motion {
            // a group whose own transform is still lets moving children blur themselves
            return false;
        }
        if n.kind == "group" && !own_motion {
            return false;
        }
        if n.kind == "adjustment" {
            // its effects act on what is below it, which samples drawn apart do not have
            if own_motion {
                plan.stats.unsupported.push(format!(
                    "{}: motion blur is not applied to adjustment layers (it adjusts where it is at the frame time)",
                    n.id
                ));
            }
            return false;
        }
        // stencil modes cut the backdrop away outside the node: the accumulation covers the target
        let stencil = matches!(self.blend_of(n), 29 | 30);
        // samples cover the box swept across the shutter plus the effects' reach (sized leaf layers, and
        // shapes whose ink stays near their box), else the whole target
        let (fw, fh) = (space.size[0] as f64, space.size[1] as f64);
        let mut rect = [0.0, 0.0, fw, fh];
        if matches!(n.kind, "layer" | "shape") && ctx.kids[i].is_empty() && !stencil {
            // Size, stroke width, effect reach and scale can all change during the shutter.
            // Bound each evaluated sample before taking their union; if any sample cannot
            // be bounded safely, retain the full target for the entire accumulation.
            let bounds = times.iter().try_fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b: [f64; 4], &t| {
                let sg = sub.at(t);
                let Some(&j) = sg.index.get(&n.id) else { return Some(b) };
                let sample = &sg.g.nodes[j];
                if sample.kind == "shape" {
                    crate::vector::box_overhang(sample)?;
                }
                let q = drawn_bounds(&ctx.at(&sg), j, space)?;
                Some([b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[2]), b[3].max(q[3])])
            });
            if let Some(b) = bounds {
                rect = round_rect(
                    [b[0].floor().max(0.0), b[1].floor().max(0.0), b[2].ceil().min(fw), b[3].ceil().min(fh)],
                    [0.0, 0.0],
                    [fw, fh],
                );
                if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
                    return true;
                }
            }
        }
        let size = [(rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32];
        // what a blurred node holds until the frame is submitted: the accumulator and one reference (a rigid move),
        // or the accumulator, one sample and each sample's vector raster. Past the budget a frame of many blurred
        // full-frame nodes would exhaust video memory, so the rest draw sharp
        let texture = size[0] as u64 * size[1] as u64 * 8;
        // Plain shapes have no external image state or effect history. Key every
        // shutter sample, including inherited opacity and referenced paints. A
        // contrast counterfactual elsewhere in the frame then leaves this work reusable.
        let cache_hash = (self.fx_cache && Self::cacheable_blur_shape(ctx, i))
            .then(|| {
                let mut words = vec![
                    Self::node_hash(ctx, n, &space.xform.then(&n.world)),
                    hf(n.world_opacity),
                    hf(iso_op),
                    hf(space.unit),
                    h(&rect.map(hf)),
                    space.size[0] as u64,
                    space.size[1] as u64,
                ];
                for &t in &times {
                    let sg = sub.at(t);
                    let &j = sg.index.get(&n.id)?;
                    let sc = ctx.at(&sg);
                    if !Self::cacheable_blur_shape(&sc, j) {
                        return None;
                    }
                    let sn = &sg.g.nodes[j];
                    words.extend([
                        Self::node_hash(&sc, sn, &space.xform.then(&sn.world)),
                        hf(sn.world_opacity),
                        Self::used_hash(&sc, j, &space.xform),
                    ]);
                }
                Some(h(&words))
            })
            .flatten();
        // The root prefix also needs the shutter's content identity. The main
        // frame time alone is unchanged during contrast counterfactual renders.
        let root_hash = cache_hash.map(|hash| h(&[root_hash, hash])).unwrap_or(root_hash);
        if let Some(hash) = cache_hash {
            if let Some(cached) = self.motion_cache.get_mut(&n.id).filter(|c| c.hash == hash) {
                if !Self::blur_budget(plan, cached.charge) {
                    return false;
                }
                cached.used = self.cache_frame;
                plan.blur_bytes += cached.charge;
                plan.stats.cache_hits += 1;
                let acc = cached.texture.clone();
                self.draw_accumulated(plan, ctx, i, space, rect, acc, cmds, root_hash);
                return true;
            }
        }
        if !Self::blur_budget(plan, 2 * texture) {
            return false;
        }
        // Unsized, childless controllers have no own ink. Prove that this stays
        // true at every shutter sample before omitting their transparent passes.
        // Preserve the original transient budget charge so later nodes keep the
        // same blur admission and diagnostics.
        let empty_control = |c: &Ctx, k: usize| {
            let node = &c.g.nodes[k];
            matches!(node.kind, "group" | "camera")
                && node.size.is_none()
                && c.kids[k].is_empty()
                && node.matte.is_none()
                && !node.is_matte
                && !node.clip
                && self.blend_of(node) == 0
                && effect_ids(&*node.elem).is_empty()
                && sr_model::element::children(&*node.elem).iter().all(|child| {
                    matches!(child.element_name(), "animate" | "expression" | "motionPath" | "transformConstraint")
                })
        };
        if empty_control(ctx, i)
            && times.iter().all(|&t| {
                let sample = sub.at(t);
                sample.index.get(&n.id).is_some_and(|&k| empty_control(&ctx.at(&sample), k))
            })
        {
            if !Self::blur_budget(plan, (count as u64 + 2) * texture) {
                return false;
            }
            plan.blur_bytes += (count as u64 + 2) * texture;
            return true;
        }
        let inner = Space {
            xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform),
            size,
            unit: space.unit,
        };
        let diagnostics = (plan.stats.errors.len(), plan.stats.unsupported.len());
        let acc = self.temp(plan, size);
        if let Some((reference, origin, w_ref)) = self.rigid_reference(plan, ctx, i, space, iso_op, &times) {
            // one drawing, moved to where each sample has the node: render once, accumulate moved copies
            let mut b = self.builder(plan);
            for (k, &t) in times.iter().enumerate() {
                let sg = sub.at(t);
                let j = sg.index[&n.id];
                let wk = space.xform.then(&sg.g.nodes[j].world);
                let Some(inv) = wk.inverse() else { continue };
                let m = Affine::translate(-origin[0], -origin[1])
                    .then(&w_ref)
                    .then(&inv)
                    .then(&Affine::translate(rect[0], rect[1]));
                b.accumulate_moved(&acc, &reference, 1.0 / count as f64, k == 0, m.0);
            }
            let (passes, temps, problems) =
                (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
            Self::finish_builder(plan, passes, temps, problems, &n.id);
            plan.blur_bytes += 2 * texture;
            self.cache_motion_blur(plan, n, cache_hash, &acc, 2 * texture, diagnostics);
            self.draw_accumulated(plan, ctx, i, space, rect, acc, cmds, root_hash);
            return true;
        }
        if !Self::blur_budget(plan, (count as u64 + 2) * texture) {
            return false;
        }
        plan.blur_bytes += (count as u64 + 2) * texture;
        // one sample texture: each sample is drawn, then added to the accumulator, before the next is drawn
        let tex = self.temp(plan, size);
        let mut first_pass = true;
        for &t in &times {
            let sg = sub.at(t);
            let Some(&j) = sg.index.get(&n.id) else {
                continue;
            };
            let sctx = ctx.at(&sg);
            let mut c = Vec::new();
            self.sampling = true;
            self.unblended = Some(n.id.clone());
            self.emit(plan, &sctx, j, &inner, iso_op, &mut c, true, 0);
            self.flush_vec(plan, &mut c);
            self.unblended = None;
            self.sampling = false;
            plan.jobs.push(Job::draws(tex.clone(), true, c, false));
            let mut b = self.builder(plan);
            b.accumulate(&acc, &tex, 1.0 / count as f64, first_pass);
            let (passes, temps, problems) =
                (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
            Self::finish_builder(plan, passes, temps, problems, &n.id);
            first_pass = false;
        }
        self.cache_motion_blur(plan, n, cache_hash, &acc, (count as u64 + 2) * texture, diagnostics);
        self.draw_accumulated(plan, ctx, i, space, rect, acc, cmds, root_hash);
        true
    }

    /// Keep one accumulation per node, subject to a total byte limit. Do not hide
    /// diagnostics that would otherwise be emitted when the node is rendered again.
    fn cache_motion_blur(
        &mut self,
        plan: &Plan,
        n: &FrameNode,
        hash: Option<u64>,
        texture: &Arc<Tex>,
        charge: u64,
        diagnostics: (usize, usize),
    ) {
        let Some(hash) = hash else { return };
        if diagnostics != (plan.stats.errors.len(), plan.stats.unsupported.len()) {
            return;
        }
        if let Some(old) = self.motion_cache.remove(&n.id) {
            self.pool.put(old.texture);
        }
        let bytes = |t: &Tex| t.size[0] as u64 * t.size[1] as u64 * 8;
        let requested = bytes(texture);
        if requested > self.motion_cache_budget {
            return;
        }
        let mut held: u64 = self.motion_cache.values().map(|c| bytes(&c.texture)).sum();
        while held + requested > self.motion_cache_budget {
            let Some(id) = self
                .motion_cache
                .iter()
                .min_by(|(aid, a), (bid, b)| (a.used, aid.as_ref()).cmp(&(b.used, bid.as_ref())))
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            let old = self.motion_cache.remove(&id).unwrap();
            held -= bytes(&old.texture);
            self.pool.put(old.texture);
        }
        self.motion_cache
            .insert(n.id.clone(), MotionBlurCache { hash, texture: texture.clone(), charge, used: self.cache_frame });
    }

    fn cacheable_blur_shape(ctx: &Ctx, i: usize) -> bool {
        let n = &ctx.g.nodes[i];
        n.kind == "shape"
            && ctx.kids[i].is_empty()
            && n.three_d.is_none()
            && n.matte.is_none()
            && !n.is_matte
            && effect_ids(&*n.elem).is_empty()
            && sr_model::element::children(&*n.elem)
                .iter()
                .all(|c| matches!(c.element_name(), "animate" | "expression" | "motionPath" | "rigidBody"))
    }

    /// Whether `bytes` more of motion blur fit in the frame's budget; reports it once when they do not.
    fn blur_budget(plan: &mut Plan, bytes: u64) -> bool {
        if plan.blur_bytes + bytes <= MOTION_BLUR_BUDGET {
            return true;
        }
        let m = format!(
            "motion blur: over the frame's {} MiB budget, some moving nodes were drawn without it",
            MOTION_BLUR_BUDGET >> 20
        );
        if !plan.stats.unsupported.contains(&m) {
            plan.stats.unsupported.push(m);
        }
        false
    }

    /// Draws a motion-blur accumulation covering `rect` of `space` in place of node `i`.
    #[allow(clippy::too_many_arguments)]
    fn draw_accumulated(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        rect: [f64; 4],
        acc: Arc<Tex>,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let n = &ctx.g.nodes[i];
        let hash = h(&[root_hash, hf(ctx.g.time), sr_eval::rng::hash_str(&n.id), 0x6d62]);
        // opacity, masks and matte were applied inside each sample; the samples blend as one
        let d = Draw {
            opacity: 1.0,
            blend: self.blend_of(n),
            src_kind: src::TEXTURE,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        };
        self.flush_vec(plan, cmds);
        self.frame_rect = Some(rect);
        let bare_before = self.bare.insert(n.id.clone());
        self.draw_cmd(
            plan,
            ctx,
            i,
            space,
            d,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            &n.world,
            acc,
            cmds,
            hash,
            true,
        );
        if bare_before {
            self.bare.remove(&n.id);
        }
        self.frame_rect = None;
    }

    /// When node `i` looks the same at every sample time apart from where it is, draws it once,
    /// at the middle of the shutter, into a target covering all it draws (not clipped to the
    /// frame: other samples can bring parts that are outside it now into view). Returns the
    /// drawing, the target-space position of its top-left texel and the node's transform then.
    #[allow(clippy::too_many_arguments)]
    fn rigid_reference(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        times: &[f64],
    ) -> Option<(Arc<Tex>, [f64; 2], Affine)> {
        let sub = ctx.sub?;
        let n = &ctx.g.nodes[i];
        if n.matte.is_some() || n.three_d.is_some() || sr_eval::draws_in_3d(n.kind) {
            return None;
        }
        let mut look = None;
        for &t in times {
            let sg = sub.at(t);
            let j = *sg.index.get(&n.id)?;
            let k = Self::appearance_hash(&ctx.at(&sg), j)?;
            if look.is_some_and(|l| l != k) {
                return None;
            }
            look = Some(k);
        }
        let sg = sub.at(times[times.len() / 2]);
        let j = *sg.index.get(&n.id)?;
        let sctx = ctx.at(&sg);
        // a drawing that turns or grows takes its effects with it: only those that have no
        // direction or pattern of their own on screen stay right
        let lin = |w: &Affine| [w.0[0], w.0[1], w.0[2], w.0[3]];
        let turned = times.iter().any(|&t| {
            let s = sub.at(t);
            s.index.get(&n.id).is_some_and(|&k| {
                lin(&s.g.nodes[k].world).iter().zip(lin(&sg.g.nodes[j].world)).any(|(a, b)| (a - b).abs() > 1e-9)
            })
        });
        if turned {
            let mut stack = vec![j];
            while let Some(k) = stack.pop() {
                let fixed = effect_ids(&*sg.g.nodes[k].elem)
                    .iter()
                    .filter_map(|id| find_effect(ctx.p, id))
                    .any(|e| e.enabled && !TURNING.contains(&e.r#type.as_str()));
                if fixed {
                    return None;
                }
                stack.extend(sctx.kids[k].iter().copied());
            }
        }
        let b = drawn_bounds(&sctx, j, space)?;
        // within a frame's width or height beyond the target: anything further never shows
        let (fw, fh) = (space.size[0] as f64, space.size[1] as f64);
        let b = round_rect(
            [b[0].max(-fw).floor(), b[1].max(-fh).floor(), b[2].min(2.0 * fw).ceil(), b[3].min(2.0 * fh).ceil()],
            [-fw, -fh],
            [2.0 * fw, 2.0 * fh],
        );
        if b[2] - b[0] < 1.0 || b[3] - b[1] < 1.0 {
            return None;
        }
        let size = [(b[2] - b[0]) as u32, (b[3] - b[1]) as u32];
        let at = Space { xform: Affine::translate(-b[0], -b[1]).then(&space.xform), size, unit: space.unit };
        let tex = self.temp(plan, size);
        let mut c = Vec::new();
        self.sampling = true;
        self.unblended = Some(n.id.clone());
        self.emit(plan, &sctx, j, &at, iso_op, &mut c, true, 0);
        self.flush_vec(plan, &mut c);
        self.unblended = None;
        self.sampling = false;
        plan.jobs.push(Job::draws(tex.clone(), true, c, false));
        Some((tex, [b[0], b[1]], space.xform.then(&sg.g.nodes[j].world)))
    }

    /// Draws both sides of a transition and combines them.
    #[allow(clippy::too_many_arguments)]
    fn transition(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        tr: &sr_eval::FrameTransition,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let g = ctx.g;
        let mut sides = Vec::new();
        for side in [tr.from, tr.to] {
            match side {
                Some(j) => {
                    let j = j as usize;
                    let id = g.nodes[j].id.clone();
                    let tex = self.temp(plan, space.size);
                    self.in_transition.insert(id.clone());
                    let mut c = Vec::new();
                    self.emit(plan, ctx, j, space, iso_op, &mut c, true, 0);
                    self.flush_vec(plan, &mut c);
                    self.in_transition.remove(&id);
                    plan.jobs.push(Job::draws(tex.clone(), true, c, false));
                    sides.push(tex);
                }
                None => sides.push(self.solid_texture([0.0; 4])),
            }
        }
        let lead = tr.to.or(tr.from).map(|x| x as usize).unwrap_or(0);
        let Some(elem) = tr.elem.clone() else {
            // sequence auto-transitions carry no element: the type with default attributes
            let empty = sr_eval::Props::default();
            self.transition_pass(plan, ctx, tr, None, &empty, sides, space, lead, cmds, root_hash);
            return;
        };
        self.transition_pass(plan, ctx, tr, Some(&*elem), &tr.props, sides, space, lead, cmds, root_hash);
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_pass(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        tr: &sr_eval::FrameTransition,
        e: Option<&dyn Element>,
        props: &sr_eval::Props,
        sides: Vec<Arc<Tex>>,
        space: &Space,
        lead: usize,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let g = ctx.g;
        let none = m::Effects { loc: Default::default(), effects: Vec::new() };
        let e: &dyn Element = e.unwrap_or(&none);
        let a = Attrs { e, props: Some(props) };
        let color = a.paint("color").and_then(|v| self.color_value(&v)).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let luma = match a.str("matte") {
            Some(id) => match g.nodes.iter().position(|n| *n.id == *id) {
                Some(j) => Some(self.matte(plan, ctx, j, space)),
                None => {
                    plan.stats.unsupported.push(format!("transition matte {id}: must name a node"));
                    None
                }
            },
            None => None,
        };
        let base = Self::base_dir(ctx.p);
        // sequence junctions carry the sequence's @transition type
        let mut kind = tr.kind.to_string();
        let shader = match a.str("shader") {
            Some(src) if kind == "shader" => match crate::glsl::load_source(&src, &base) {
                Ok((code, _)) => {
                    Some((code, std::path::PathBuf::from(if src.starts_with("data:") { "data:" } else { &src })))
                }
                Err(err) => {
                    plan.stats.unsupported.push(format!("transition shader {src}: {err}; crossfade"));
                    None
                }
            },
            _ => None,
        };
        if kind == "shader" && shader.is_none() {
            if a.str("shader").is_none() {
                plan.stats.unsupported.push("transition type='shader' without @shader; crossfade".to_string());
            }
            kind = "crossfade".to_string();
        }
        let to_uv = |p: [f64; 2]| [p[0] / space.size[0] as f64, p[1] / space.size[1] as f64];
        let (working, tokens) = (self.working, self.tokens.clone());
        let colorf = move |v: &Value| -> Option<[f64; 4]> {
            match v {
                Value::Color(c) => Some(working.from_literal(*c)),
                Value::Str(s) if s.starts_with("token:") => tokens.get(&s[6..]).map(|c| working.from_literal(*c)),
                _ => None,
            }
        };
        let gradient = |_: &Value| None;
        let lead_ix = lead;
        let mut cx = self.cx_for(
            ctx,
            lead_ix,
            space,
            [0.0, 0.0, space.size[0] as f64, space.size[1] as f64],
            &to_uv,
            &colorf,
            &gradient,
            &base,
        );
        cx.frame_size = [space.size[0] as f64, space.size[1] as f64];
        let mut b = self.builder(plan);
        let mut r = b.transition(
            &kind,
            &sides[0],
            &sides[1],
            luma.clone(),
            tr.progress,
            e,
            &a,
            color,
            shader.as_ref().map(|(c, p)| (c.clone(), p.as_path())),
            tr.velocity,
            Some(&cx),
        );
        if kind == "shader" && r.is_err() {
            // a failing shader (compile error) renders as a crossfade
            if let Err(msg) = &r {
                b.problems.push(msg.clone());
            }
            r = b.transition("crossfade", &sides[0], &sides[1], luma, tr.progress, e, &a, color, None, 0.0, None);
        }
        let (passes, temps, problems) =
            (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
        Self::finish_builder(plan, passes, temps, problems, &g.nodes[lead].id);
        let out = match r {
            Ok(t) => t,
            Err(msg) => {
                plan.stats.unsupported.push(format!("{}: {msg}", g.nodes[lead].id));
                sides[1].clone()
            }
        };
        let hash = h(&[root_hash, hf(tr.progress), hf(g.time), 0x7472]);
        let d = Draw { opacity: 1.0, src_kind: src::TEXTURE, uv_rect: [0.0, 0.0, 1.0, 1.0], ..Default::default() };
        self.flush_vec(plan, cmds);
        self.frame_rect = Some([0.0, 0.0, space.size[0] as f64, space.size[1] as f64]);
        // the sides already carry their own opacity, masks and matte
        let id = g.nodes[lead].id.clone();
        let inserted = self.bare.insert(id.clone());
        self.draw_cmd(
            plan,
            ctx,
            lead,
            space,
            d,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            &Affine::IDENTITY,
            out,
            cmds,
            hash,
            true,
        );
        if inserted {
            self.bare.remove(&id);
        }
        self.frame_rect = None;
    }

    /// A texture of `size` the caller keeps across renders (the placed sides of a join).
    pub fn texture(&self, size: [u32; 2]) -> Arc<Tex> {
        Arc::new(resources::create_as(&self.gpu.device, &self.bgl1, self.format, size, 1, "kept"))
    }

    /// Combines two finished, frame-sized pictures with a transition element into `into`: the join
    /// between two segments of an output (SREP 13). `time` is output time. Returns what was not drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn join(
        &mut self,
        p: &Program,
        tr: &m::Transition,
        from: &Arc<Tex>,
        to: &Arc<Tex>,
        progress: f64,
        velocity: f64,
        time: f64,
        into: &Tex,
    ) -> Vec<String> {
        let mut plan = Plan::default();
        let a = Attrs { e: tr, props: None };
        let color = a.paint("color").and_then(|v| self.color_value(&v)).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let who = tr.id.clone().unwrap_or_else(|| "segment transition".into());
        let mut problems = Vec::new();
        if tr.matte.is_some() {
            problems.push(format!("{who}: a matte names a node, and a join between segments has none; no matte"));
        }
        let base = Self::base_dir(p);
        let mut kind = tr.r#type.as_str().to_string();
        let shader = match &tr.shader {
            Some(src) if kind == "shader" => match crate::glsl::load_source(src, &base) {
                Ok((code, _)) => {
                    Some((code, std::path::PathBuf::from(if src.starts_with("data:") { "data:" } else { src })))
                }
                Err(err) => {
                    problems.push(format!("{who}: shader {src}: {err}; crossfade"));
                    None
                }
            },
            _ => None,
        };
        if kind == "shader" && shader.is_none() {
            kind = "crossfade".to_string();
        }
        let size = into.size;
        let (fw, fh) = (size[0] as f64, size[1] as f64);
        let to_uv = move |q: [f64; 2]| [q[0] / fw, q[1] / fh];
        let (working, tokens) = (self.working, self.tokens.clone());
        let colorf = move |v: &Value| -> Option<[f64; 4]> {
            match v {
                Value::Color(c) => Some(working.from_literal(*c)),
                Value::Str(s) if s.starts_with("token:") => tokens.get(&s[6..]).map(|c| working.from_literal(*c)),
                _ => None,
            }
        };
        let gradient = |_: &Value| None;
        let fps = fps_of(p);
        let cx = Cx {
            px: 1.0,
            scale: 1.0,
            lin: [1.0, 0.0, 0.0, 1.0],
            to_uv: &to_uv,
            center: [0.5, 0.5],
            content: None,
            time,
            frame: (time * fps).round() as i64,
            color: &colorf,
            gradient: &gradient,
            source: None,
            lights: Vec::new(),
            working: self.working,
            base: &base,
            fps,
            local_time: time,
            offset: [0.0, 0.0],
            frame_size: [fw, fh],
            node: who.clone(),
            named: HashMap::new(),
            audio: self.audio.clone(),
            seed: self.seed,
        };
        let mut b = self.builder(&plan);
        let shader_ref = shader.as_ref().map(|(c, p)| (c.clone(), p.as_path()));
        let mut r = b.transition(&kind, from, to, None, progress, tr, &a, color, shader_ref, velocity, Some(&cx));
        if kind == "shader" && r.is_err() {
            if let Err(msg) = &r {
                b.problems.push(msg.clone());
            }
            r = b.transition("crossfade", from, to, None, progress, tr, &a, color, None, 0.0, None);
        }
        let (passes, temps, bproblems) =
            (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
        Self::finish_builder(&mut plan, passes, temps, bproblems, &who);
        let out = match r {
            Ok(t) => t,
            Err(msg) => {
                plan.stats.unsupported.push(format!("{who}: {msg}"));
                to.clone()
            }
        };
        // the result is a pooled temporary: copy it out before the pool hands it to the next render
        let keep = out.clone();
        let stats = self.execute(plan, 0, None, &[]);
        let mut enc = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("join") });
        Self::copy(&mut enc, &keep, into, [0, 0, size[0], size[1]]);
        self.last_submit = Some(self.gpu.queue.submit([enc.finish()]));
        problems.extend(stats.unsupported);
        problems.extend(stats.errors);
        problems
    }

    /// Scene-referred finishing: looks (LUT or ASC CDL), exposure and tone mapping on the frame.
    pub(super) fn finishing(&mut self, plan: &mut Plan, p: &Program, frame: &Arc<Tex>) {
        let Some(cm) = p.scene.color_management.as_ref() else { return };
        let base = Self::base_dir(p);
        let working = self.working;
        // an OCIO config replaces the tone mapper with its display and view; a <look> with neither
        // a LUT nor CDL values names a look of the config by its id
        let ocio_look = |l: &m::Look| {
            cm.ocio_config.is_some()
                && l.src.is_none()
                && l.slope.is_none()
                && l.offset.is_none()
                && l.power.is_none()
                && l.saturation == 1.0
        };
        let mut notes = Vec::new();
        let ocio = cm.ocio_config.as_ref().map(|src| {
            let config = if src.starts_with("ocio://") {
                src.clone()
            } else {
                match sr_model::assets::resolve(src, &base) {
                    sr_model::assets::Resolved::Local(path) => path.display().to_string(),
                    _ => return Err("only local configs and ocio:// built-in configs are supported".to_string()),
                }
            };
            let looks: Vec<String> = cm
                .looks
                .iter()
                .flatten()
                .filter(|id| cm.look_list.iter().any(|l| &l.id == *id && ocio_look(l)))
                .cloned()
                .collect();
            let key =
                format!("ocio:{config}|{}|{}|{}|{}", working.space.as_str(), cm.display, cm.view, looks.join(","));
            self.fx.lut_with(&key, || {
                crate::ocio::bake(&config, working.space, &cm.display, &cm.view, &looks).map(|b| {
                    notes.extend(b.notes);
                    (crate::ocio::SIZE, b.table)
                })
            })
        });
        // the display's code values decode as the (first) output encodes them
        let (out_space, out_transfer) = p
            .scene
            .outputs
            .first()
            .map(|o| (o.color_space, o.transfer))
            .unwrap_or((m::ColorSpace::Srgb, m::Transfer::Auto));
        let mut b = self.builder(plan);
        let mut cur = frame.clone();
        let mut problems = Vec::new();
        for id in cm.looks.iter().flatten() {
            let Some(look) = cm.look_list.iter().find(|l| &l.id == id && !ocio_look(l)) else { continue };
            let mixv = look.mix.get();
            if let Some(src) = &look.src {
                let path = match sr_model::assets::resolve(src, &base) {
                    sr_model::assets::Resolved::Local(p) => p,
                    _ => {
                        problems.push(format!("look {id}: only local LUT files are supported"));
                        continue;
                    }
                };
                match b.eng.lut(&path) {
                    Ok(lut) => cur = b.lut_pass(&cur, lut, look.space, mixv, working),
                    Err(e) => problems.push(format!("look {id}: {e}")),
                }
            } else {
                let tri = |s: &Option<String>, d: f64| {
                    let v: Vec<f64> = s
                        .as_deref()
                        .unwrap_or("")
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    match v.len() {
                        0 => [d; 3],
                        1 | 2 => [v[0]; 3],
                        _ => [v[0], v[1], v[2]],
                    }
                };
                let (s, o, pw) = (tri(&look.slope, 1.0), tri(&look.offset, 0.0), tri(&look.power, 1.0));
                let mut v = [[0.0f32; 4]; 8];
                v[0] = [s[0] as f32, s[1] as f32, s[2] as f32, 0.0];
                v[1] = [o[0] as f32, o[1] as f32, o[2] as f32, 0.0];
                v[2] = [pw[0] as f32, pw[1] as f32, pw[2] as f32, 0.0];
                v[3] = [look.saturation as f32, 0.0, 0.0, 0.0];
                let graded = b.color(&cur, 3, v);
                cur = if mixv < 1.0 {
                    let mut mv = [[0.0f32; 4]; 8];
                    mv[0][0] = mixv as f32;
                    let size = cur.size;
                    b.run(fx::Entry::Combine, [0; 4], mv, &cur, fx::Aux::Tex(graded), None, None, size)
                } else {
                    graded
                };
            }
        }
        if cm.exposure != 0.0 {
            let mut v = [[0.0f32; 4]; 8];
            v[0][0] = cm.exposure as f32;
            cur = b.color(&cur, 7, v);
        }
        let mut ocio_applied = false;
        match ocio {
            Some(Ok(lut)) => {
                problems.extend(notes);
                cur = b.lut_pass_between(&cur, lut, m::Transfer::Acescct, out_space, out_transfer, working);
                ocio_applied = true;
            }
            Some(Err(e)) => problems
                .push(format!("colorManagement/@ocioConfig: {e}; the built-in looks and tone mappers apply instead")),
            None => {}
        }
        let tm = match cm.tone_mapping.as_str() {
            _ if ocio_applied => None,
            "aces" | "aces2" => Some(0.0),
            "agx" => Some(1.0),
            "filmic" => Some(2.0),
            "reinhard" => Some(3.0),
            _ => None,
        };
        if let Some(op) = tm {
            let mut v = [[0.0f32; 4]; 8];
            v[0] = [0.0, op, 0.0, 0.0];
            cur = b.color(&cur, 9, v);
        }
        let (passes, temps) = (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps));
        plan.fx_temps.extend(temps);
        plan.stats.unsupported.extend(problems);
        if !passes.is_empty() {
            plan.post = labelled(fx::fuse_colour(passes), "finishing");
            plan.post_out = Some(cur);
        }
    }
}

#[cfg(test)]
mod motion_cache_tests {
    use super::*;

    #[test]
    fn retained_motion_textures_respect_budget_and_recent_use() {
        let gpu = match crate::gpu::test_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                return;
            }
        };
        let document = sr_model::load_str(
            r##"<scene version="1.1"><project width="32" height="32" fps="10" duration="1"/>
                <composition><shape id="a" shape="rect" width="8" height="8" fill="#FFFFFF"/>
                <shape id="b" shape="rect" width="8" height="8" fill="#FFFFFF"/>
                <shape id="c" shape="rect" width="8" height="8" fill="#FFFFFF"/>
                <shape id="d" shape="rect" width="8" height="8" fill="#FFFFFF"/></composition></scene>"##,
            &sr_model::LoadOptions::default(),
        )
        .unwrap();
        let ev = sr_eval::Evaluator::new(&document, &Default::default()).unwrap();
        let frame = ev.evaluate(0.0);
        let node = |id: &str| frame.nodes.iter().find(|n| &*n.id == id).unwrap();
        let mut renderer = Renderer::new(gpu, ev.program());
        renderer.motion_cache_budget = 2 * 32 * 32 * 8;
        let mut plan = Plan::default();
        for (at, id) in ["a", "b", "c"].into_iter().enumerate() {
            renderer.cache_frame = at as u64;
            let texture = renderer.pool.get(&renderer.gpu.device, &renderer.bgl1, [32, 32]);
            renderer.cache_motion_blur(&plan, node(id), Some(at as u64), &texture, 2 * 32 * 32 * 8, (0, 0));
        }
        assert_eq!(renderer.motion_cache.len(), 2);
        assert!(!renderer.motion_cache.contains_key("a"), "oldest entry is evicted");
        renderer.motion_cache.get_mut("b").unwrap().used = 3;
        renderer.cache_frame = 4;
        let texture = renderer.pool.get(&renderer.gpu.device, &renderer.bgl1, [32, 32]);
        renderer.cache_motion_blur(&plan, node("d"), Some(4), &texture, 2 * 32 * 32 * 8, (0, 0));
        assert!(renderer.motion_cache.contains_key("b"), "recent hit remains resident");
        assert!(!renderer.motion_cache.contains_key("c"));
        assert!(renderer.motion_cache.contains_key("d"));
        let held: u64 =
            renderer.motion_cache.values().map(|c| c.texture.size[0] as u64 * c.texture.size[1] as u64 * 8).sum();
        assert_eq!(held, renderer.motion_cache_budget);
        let oversized = renderer.pool.get(&renderer.gpu.device, &renderer.bgl1, [64, 64]);
        renderer.cache_motion_blur(&plan, node("a"), Some(5), &oversized, 2 * 64 * 64 * 8, (0, 0));
        assert!(!renderer.motion_cache.contains_key("a"), "oversized texture is never retained");
        assert_eq!(renderer.motion_cache.len(), 2);
        plan.stats.unsupported.push("diagnostic from rendering this shape".into());
        renderer.cache_motion_blur(&plan, node("a"), Some(6), &texture, 2 * 32 * 32 * 8, (0, 0));
        assert!(!renderer.motion_cache.contains_key("a"), "cache must not suppress a render diagnostic");
    }
}
