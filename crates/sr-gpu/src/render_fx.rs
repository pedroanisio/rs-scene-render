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
const TIME_VARYING: &[&str] = &[
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
    "echo",
    "posterize-time",
    "light-sweep",
    "fractal-noise",
];

fn effect_ids(e: &dyn Element) -> Vec<String> {
    match e.get_attr("effects") {
        Some(AttrValue::Tokens(t)) => t,
        _ => Vec::new(),
    }
}

fn find_effect<'p>(p: &'p Program, id: &str) -> Option<&'p m::Effect> {
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

/// Document-pixel reach of an effect beyond the node's box (infinite: whole frame).
fn reach(e: &m::Effect, a: &Attrs) -> f64 {
    let kind = e.r#type.as_str();
    if UNBOUNDED.contains(&kind) {
        return f64::INFINITY;
    }
    let r = a.num("radius", 4.0);
    let sz = a.num("size", 1.0);
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

    fn builder<'b>(&'b mut self, _plan: &Plan) -> Builder<'b> {
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
            problems: Vec::new(),
        }
    }

    fn finish_builder(plan: &mut Plan, passes: Vec<fx::Pass>, temps: Vec<Arc<Tex>>, problems: Vec<String>, who: &str) {
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
                sr_model::assets::Resolved::Local(p) => self.image(p, img.color_space, img.transfer, img.alpha),
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
            let proj = self.proj25(ctx.g, space);
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
                backdrop: (blend >= 2).then_some(bounds),
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

    /// Draws the node with id `id` from the scene at time `t`, bare, into `inner`.
    fn bare_at(&mut self, plan: &mut Plan, ctx: &Ctx, id: &Arc<str>, t: f64, inner: &Space) -> Option<Arc<Tex>> {
        let sub = ctx.sub?;
        let sg = sub.at(t);
        let j = *sg.index.get(id)?;
        let sctx = Ctx {
            g: &sg.g,
            p: ctx.p,
            kids: &sg.kids,
            timed: ctx.timed,
            generated: ctx.generated,
            elements: sg.elements,
            sub: ctx.sub,
        };
        let was = std::mem::replace(&mut self.sampling, true);
        let t = self.bare_render(plan, &sctx, j, inner);
        self.sampling = was;
        Some(t)
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
        let e: &dyn Element = &*n.elem;
        let clip = if n.clip { n.size } else { None };
        let mask_box = n.size.unwrap_or([space.size[0] as f64, space.size[1] as f64]);
        let (mask_off, mask_count) = self.masks_of(plan, n, mask_box, clip);
        let d = Draw {
            opacity: op as f32,
            blend: if self.bare.contains(&n.id) { 0 } else { blend_index(e) },
            src_kind: src::TEXTURE,
            mask_off,
            mask_count,
            seed: sr_eval::rng::hash_str(&n.id) as u32,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        };
        self.flush_vec(plan, cmds);
        self.frame_rect = Some(rect);
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
        }
    }

    /// Point lights referenced by an effect, as uv position, intensity and radius.
    fn lights_of(&self, ctx: &Ctx, a: &Attrs, to_uv: &dyn Fn([f64; 2]) -> [f64; 2], w: f64) -> Vec<[f32; 4]> {
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
                let range = la.opt("range").or(la.opt("distance")).unwrap_or(w * 0.5);
                [uv[0] as f32, uv[1] as f32, la.num("intensity", 1.0) as f32, (range / w) as f32]
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
            let inner = Space { xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform), size };
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
                    if let Some(t) = self.image(path, cs, tf, i.alpha) {
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
        let effs: Vec<&m::Effect> = ids.iter().filter_map(|id| find_effect(ctx.p, id)).filter(|e| e.enabled).collect();
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
        let bounded = n.kind == "layer" && ctx.kids[i].is_empty() && n.size.is_some() && pad.is_finite();
        let mut rect = [0.0, 0.0, fw, fh];
        if bounded {
            let s = n.size.unwrap_or([0.0; 2]);
            let pts = [[0.0, 0.0], [s[0], 0.0], [s[0], s[1]], [0.0, s[1]]].map(|p| ta.apply(p));
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for q in pts {
                x0 = x0.min(q[0]);
                y0 = y0.min(q[1]);
                x1 = x1.max(q[0]);
                y1 = y1.max(q[1]);
            }
            // custom shaders see the Python engine's tile: the layer's pixels with a 2 px margin,
            // reaching up to 64 px past the frame (their resolution and uv depend on it)
            let shader = effs.iter().any(|e| e.r#type.as_str() == "shader");
            let (m, lo, hx, hy) = if shader { (2.0, -64.0, fw + 64.0, fh + 64.0) } else { (1.0, 0.0, fw, fh) };
            rect = [
                (x0 - pad - m).floor().max(lo),
                (y0 - pad - m).floor().max(lo),
                (x1 + pad + m).ceil().min(hx),
                (y1 + pad + m).ceil().min(hy),
            ];
        }
        if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
            return true;
        }
        let size = [(rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32];
        let inner = Space { xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform), size };
        let time_dep = effs.iter().any(|e| TIME_VARYING.contains(&e.r#type.as_str()));
        let hash = h(&[
            Self::content_hash(ctx, i, &inner.xform),
            ctx.elements,
            size[0] as u64,
            size[1] as u64,
            if time_dep { hf(g.time) } else { 0 },
            sr_eval::rng::hash_str(&ids.join(" ")),
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
                    _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, size, 1, "effects")),
                };
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
                    let rate = a.num("frequency", 1.0).max(1e-3);
                    let tq = (t * rate + 1e-9).floor() / rate;
                    match self.bare_at(plan, ctx, &n.id, tq, inner) {
                        Some(tx) => src = Some(tx),
                        None => {
                            plan.stats.unsupported.push(format!("{}: posterize-time needs a sub-frame provider", e.id))
                        }
                    }
                }
                "echo" => {
                    if ctx.sub.is_none() {
                        plan.stats.unsupported.push(format!("{}: echo needs a sub-frame provider", e.id));
                        continue;
                    }
                    let count = (a.num("samples", 16.0) as usize).clamp(2, 16);
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
                                .unwrap_or_else(|| self.solid_texture([0.0; 4]))
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
                    Self::finish_builder(plan, passes, temps, problems, &e.id);
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
            .flat_map(|a| [a.paint("paint"), a.str("source").map(|s| Value::PaintRef(s.into()))])
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
                let Some(prev) = self.bare_at(plan, ctx, &n.id, t - 1.0 / fps, inner) else {
                    plan.stats.unsupported.push(format!("{}: pixel-motion-blur needs a sub-frame provider", e.id));
                    continue;
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
                Self::finish_builder(plan, passes, temps, problems, &e.id);
                cur = out;
                continue;
            }
            let source =
                if a.str("source").is_some() && matches!(kind, "displacement-map" | "difference-key" | "shader") {
                    self.effect_source(plan, ctx, a, space, rect)
                } else {
                    None
                };
            let named = if kind == "shader" { self.shader_samplers(plan, ctx, e, space, rect) } else { HashMap::new() };
            let mut cx = self.cx_for(ctx, i, inner, [0.0, 0.0, w, hgt], &to_uv, &color, &gradient, &base);
            cx.source = source;
            cx.named = named;
            cx.offset = [rect[0], space.size[1] as f64 - rect[3]];
            cx.frame_size = [space.size[0] as f64, space.size[1] as f64];
            cx.lights = if kind == "lighting" { self.lights_of(ctx, a, &to_uv, w) } else { Vec::new() };
            let mut b = self.builder(plan);
            let r = b.effect(*e as &dyn Element, a, &cur, &cx);
            let (passes, temps, problems) =
                (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
            Self::finish_builder(plan, passes, temps, problems, &e.id);
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
            params: fx::Params { v, i: [0; 4] },
            src: cur,
            aux: fx::Aux::None,
            aux2: None,
            lut: None,
            out: target.clone(),
            additive: false,
            clear: true,
            custom: None,
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
        let effs: Vec<&m::Effect> = ids.iter().filter_map(|id| find_effect(ctx.p, id)).filter(|e| e.enabled).collect();
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
            .filter_map(|a| a.paint("paint"))
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
            cx.lights = if kind == "lighting" { self.lights_of(ctx, a, &to_uv, w) } else { Vec::new() };
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
            c.pre = Some(Box::new(AdjPre { snapshot, passes, three: None }));
        }
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
        let three = n.kind == "object3D";
        if three && Self::three_members(g, i).first() != Some(&i) {
            // drawn, and blurred, with the pass of the first object that shares its parent
            return true;
        }
        let pr = &ctx.p.scene.project;
        let fps = fps_of(ctx.p);
        let angle = pr.shutter_angle;
        if angle <= 0.0 {
            return false;
        }
        let count = (pr.motion_blur_samples as usize).clamp(1, 256);
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
        // samples cover the box swept across the shutter plus the effects' reach (sized leaf layers),
        // else the whole target
        let (fw, fh) = (space.size[0] as f64, space.size[1] as f64);
        let mut rect = [0.0, 0.0, fw, fh];
        if n.kind == "layer" && ctx.kids[i].is_empty() && n.size.is_some() {
            let effs: Vec<&m::Effect> =
                effect_ids(&*n.elem).iter().filter_map(|id| find_effect(ctx.p, id)).filter(|e| e.enabled).collect();
            let pad: f64 = effs
                .iter()
                .map(|e| reach(e, &Attrs { e: *e as &dyn Element, props: element_props(g, &e.id) }))
                .sum::<f64>()
                * Xf(space.xform.then(&n.world).0).max_scale();
            if pad.is_finite() && n.kind != "object3D" {
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &t in &times {
                    let sg = sub.at(t);
                    let Some(&j) = sg.index.get(&n.id) else { continue };
                    let wt = space.xform.then(&sg.g.nodes[j].world);
                    for c in corners {
                        let q = wt.apply(c);
                        x0 = x0.min(q[0]);
                        y0 = y0.min(q[1]);
                        x1 = x1.max(q[0]);
                        y1 = y1.max(q[1]);
                    }
                }
                rect = [
                    (x0 - pad - 1.0).floor().max(0.0),
                    (y0 - pad - 1.0).floor().max(0.0),
                    (x1 + pad + 1.0).ceil().min(fw),
                    (y1 + pad + 1.0).ceil().min(fh),
                ];
                if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
                    return true;
                }
            }
        }
        let size = [(rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32];
        let inner = Space { xform: Affine([1.0, 0.0, 0.0, 1.0, -rect[0], -rect[1]]).then(&space.xform), size };
        let acc = self.temp(plan, size);
        let mut samples = Vec::with_capacity(count);
        for &t in &times {
            let sg = sub.at(t);
            let Some(&j) = sg.index.get(&n.id) else {
                samples.push(None);
                continue;
            };
            let sctx = Ctx {
                g: &sg.g,
                p: ctx.p,
                kids: &sg.kids,
                timed: ctx.timed,
                generated: ctx.generated,
                elements: sg.elements,
                sub: ctx.sub,
            };
            let tex = self.temp(plan, size);
            let mut c = Vec::new();
            self.sampling = true;
            self.emit(plan, &sctx, j, &inner, iso_op, &mut c, true, 0);
            self.flush_vec(plan, &mut c);
            self.sampling = false;
            plan.jobs.push(Job::draws(tex.clone(), true, c, false));
            samples.push(Some(tex));
        }
        let mut b = self.builder(plan);
        let mut first_pass = true;
        for tex in samples.iter().flatten() {
            b.accumulate(&acc, tex, 1.0 / count as f64, first_pass);
            first_pass = false;
        }
        let (passes, temps, problems) =
            (std::mem::take(&mut b.passes), std::mem::take(&mut b.temps), std::mem::take(&mut b.problems));
        Self::finish_builder(plan, passes, temps, problems, &n.id);
        let hash = h(&[root_hash, hf(g.time), sr_eval::rng::hash_str(&n.id), 0x6d62]);
        // opacity, blend, masks and matte were applied inside each sample
        let d = Draw { opacity: 1.0, src_kind: src::TEXTURE, uv_rect: [0.0, 0.0, 1.0, 1.0], ..Default::default() };
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
        true
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
            // sequence auto-transitions carry no element: a plain crossfade
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
        let mut kind = if tr.elem.is_some() { tr.kind.to_string() } else { "crossfade".to_string() };
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
            // a failing shader (compile error) renders as a crossfade, like the Python engine
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

    /// Scene-referred finishing: looks (LUT or ASC CDL), exposure and tone mapping on the frame.
    pub(super) fn finishing(&mut self, plan: &mut Plan, p: &Program, frame: &Arc<Tex>) {
        let Some(cm) = p.scene.color_management.as_ref() else { return };
        if cm.ocio_config.is_some() {
            plan.stats.unsupported.push("colorManagement/@ocioConfig: no OpenColorIO library is linked; the built-in looks and tone mappers apply instead".into());
        }
        let base = Self::base_dir(p);
        let working = self.working;
        let mut b = self.builder(plan);
        let mut cur = frame.clone();
        let mut problems = Vec::new();
        for id in cm.looks.iter().flatten() {
            let Some(look) = cm.look_list.iter().find(|l| &l.id == id) else { continue };
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
        let tm = match cm.tone_mapping.as_str() {
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
            plan.post = passes;
            plan.post_out = Some(cur);
        }
    }
}
