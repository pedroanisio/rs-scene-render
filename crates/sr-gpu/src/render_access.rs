//! Accessibility measurements in the compositor: the contrast of
//! burned-in text against the backdrop actually drawn behind it.
//!
//! When `contrast_probe` is on, each text layer and the caption burn-in in
//! the frame's own target get their own draw command, preceded by a copy of
//! the target (the backdrop). After the frame, the pixels the text changed are
//! its glyphs: their colour in the frame is the text, and their colour in the
//! snapshot is the background behind it. Equal colours require a coverage probe:
//! contrasting inks preserve sampled alpha and establish whether text is visible.

use super::*;

pub(super) const CONTRAST_INK: &str = "__sr_contrast_ink";

/// The integer frame-space rectangle (x, y, w, h) covering a node's box, grown by `pad`, clamped to the frame.
fn node_rect(n: &sr_eval::FrameNode, w: u32, h: u32, pad: f64) -> Option<[u32; 4]> {
    let [nw, nh] = n.size?;
    let pts = [[0.0, 0.0], [nw, 0.0], [nw, nh], [0.0, nh]].map(|p| n.world.apply(p));
    let x0 = (pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min) - pad).floor().clamp(0.0, w as f64);
    let y0 = (pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min) - pad).floor().clamp(0.0, h as f64);
    let x1 = (pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max) + pad).ceil().clamp(0.0, w as f64);
    let y1 = (pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max) + pad).ceil().clamp(0.0, h as f64);
    (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then_some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

impl Renderer {
    /// A counterfactual frame for accessibility measurement. Only the selected text's
    /// RGB changes; its sampled alpha, transforms, masks and later occluders remain.
    /// The private property participates in normal cache hashes.
    pub fn contrast_ink_graph(g: &sr_eval::FrameGraph, id: &str, ink: f64) -> sr_eval::FrameGraph {
        let mut g = g.clone();
        if id == "captions" {
            g.elements.retain(|e| &*e.key != CONTRAST_INK);
            g.elements.push(sr_eval::eval::ElementState {
                key: CONTRAST_INK.into(),
                element: "contrastProbe",
                props: sr_eval::eval::Props(vec![(CONTRAST_INK.into(), Value::Num(ink))]),
            });
        } else if let Some(n) = g.nodes.iter_mut().find(|n| &*n.id == id) {
            n.props.0.retain(|(k, _)| &**k != CONTRAST_INK);
            n.props.0.push((CONTRAST_INK.into(), Value::Num(ink)));
        }
        g
    }

    /// Measure the original colours only where two counterfactual inks establish
    /// visible text coverage. Returns None for fully occluded, transparent or offscreen text.
    pub fn contrast_of_coverage(
        &self,
        before: &[[f32; 4]],
        after: &[[f32; 4]],
        dark: &[[f32; 4]],
        light: &[[f32; 4]],
    ) -> Option<f64> {
        let coverage: Vec<f32> = dark
            .iter()
            .zip(light)
            .map(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs())
            .collect();
        let top = coverage.iter().copied().fold(0.0f32, f32::max);
        if top <= 1e-6 {
            return None;
        }
        let (mut foreground, mut background, mut count) = (0.0, 0.0, 0.0);
        for ((a, b), coverage) in before.iter().zip(after).zip(coverage) {
            if coverage >= top * 0.5 {
                background += self.luminance(*a);
                foreground += self.luminance(*b);
                count += 1.0;
            }
        }
        if count == 0.0 {
            return None;
        }
        let (a, b) = (foreground / count, background / count);
        Some((a.max(b) + 0.05) / (a.min(b) + 0.05))
    }
    fn is_text_layer(&self, ctx: &Ctx, n: &FrameNode) -> bool {
        n.kind == "layer"
            && n.asset
                .as_deref()
                .and_then(|k| self.asset(ctx.p, k))
                .map(|(a, _)| matches!(a, AssetsChild::Text(_)))
                .unwrap_or(false)
    }

    fn is_root_space(&self, ctx: &Ctx, space: &Space) -> bool {
        space.xform == Affine::IDENTITY
            && space.size == [ctx.g.size[0].round() as u32, ctx.g.size[1].round() as u32]
            && self.view_override.is_none()
    }

    /// Starts a probe around a text layer drawn in the frame's target.
    pub(super) fn text_probe_start(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) -> Option<usize> {
        if !self.contrast_probe || !self.is_text_layer(ctx, &ctx.g.nodes[i]) || !ctx.g.nodes[i].draw {
            return None;
        }
        // `root_hash` is 0 exactly when emitting into an offscreen (isolated groups, masks,
        // mattes, effects and transitions all emit with 0; frame roots pass their subtree hash).
        // An unsized isolated group shares the frame's space, so the space alone cannot tell.
        if root_hash == 0 || !self.is_root_space(ctx, space) {
            // inside an isolated group: the backdrop here is not what the viewer sees
            let id = ctx.g.nodes[i].id.to_string();
            if !plan.stats.contrast_unprobed.contains(&id) {
                plan.stats.contrast_unprobed.push(id);
            }
            return None;
        }
        self.flush_vec(plan, cmds);
        Some(cmds.len())
    }

    /// Finishes a text-layer probe: its commands get a backdrop snapshot.
    pub(super) fn text_probe_end(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        cmds: &mut Vec<Cmd>,
        at: usize,
    ) {
        self.flush_vec(plan, cmds);
        let n = &ctx.g.nodes[i];
        let [w, h] = n.size.unwrap_or([0.0, 0.0]);
        let xf = space.xform.then(&n.world);
        let pts = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| xf.apply(p));
        let b = [
            pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min),
            pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
            pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max),
            pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max),
        ];
        let id = n.id.to_string();
        self.attach_probe(plan, space, cmds, at, &id, b);
    }

    /// Gives command `at` a backdrop snapshot and records the probe over `b` (x0, y0, x1, y1).
    pub(super) fn attach_probe(
        &mut self,
        plan: &mut Plan,
        space: &Space,
        cmds: &mut [Cmd],
        at: usize,
        id: &str,
        b: [f64; 4],
    ) {
        let Some(c) = cmds.get_mut(at) else { return };
        if c.pre.is_some() {
            return;
        }
        let (w, h) = (space.size[0] as f64, space.size[1] as f64);
        let r = [
            b[0].floor().clamp(0.0, w),
            b[1].floor().clamp(0.0, h),
            b[2].ceil().clamp(0.0, w),
            b[3].ceil().clamp(0.0, h),
        ];
        if r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0 {
            return;
        }
        let snapshot = self.pool.get(&self.gpu.device, &self.bgl1, space.size);
        c.pre = Some(Box::new(AdjPre { snapshot: snapshot.clone(), passes: Vec::new(), three: None }));
        plan.probes.push((
            id.to_string(),
            [r[0] as u32, r[1] as u32, (r[2] - r[0]) as u32, (r[3] - r[1]) as u32],
            snapshot,
        ));
    }

    /// Reads a rectangle (x, y, w, h) of a texture.
    fn read_rect(&self, t: &Tex, r: [u32; 4]) -> Vec<[f32; 4]> {
        let [x, y, w, h] = r;
        let row = (w * 8).div_ceil(256) * 256;
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.gpu.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.gpu.wait();
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * h) as usize);
        for yy in 0..h {
            for px in data[(yy * row) as usize..(yy * row + w * 8) as usize].chunks_exact(8) {
                let c = |k: usize| half::f16::from_le_bytes([px[k], px[k + 1]]).to_f32();
                out.push([c(0), c(2), c(4), c(6)]);
            }
        }
        out
    }

    /// WCAG relative luminance of a stored working-space pixel (composited over black). Encoding to
    /// display sRGB and decoding it again, as WCAG's formula reads, cancels out on the clamped range,
    /// so the luminance is taken from the clamped linear sRGB directly (no transfer-function pows).
    fn luminance(&self, p: [f32; 4]) -> f64 {
        let l = self.working.to_display_linear_srgb([p[0] as f64, p[1] as f64, p[2] as f64]);
        0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
    }

    /// Contrast ratio of the text a probe covered against the backdrop behind its glyphs; the project
    /// background `bg`, drawn after everything, goes beneath the snapshot.
    pub(super) fn measure_contrast(&self, snapshot: &Tex, bg: Option<&Tex>, frame: &Tex, r: [u32; 4]) -> Option<f64> {
        let mut before = self.read_rect(snapshot, r);
        if let Some(bg) = bg {
            for (p, b) in before.iter_mut().zip(self.read_rect(bg, r)) {
                let k = 1.0 - p[3];
                *p = [p[0] + b[0] * k, p[1] + b[1] * k, p[2] + b[2] * k, p[3] + b[3] * k];
            }
        }
        let after = self.read_rect(frame, r);
        self.contrast_of(&before, &after)
    }

    /// Contrast of the pixels that changed between `before` (backdrop) and `after` (with text).
    /// Both arrays must be corresponding pixels in this renderer's premultiplied working space.
    /// Delivery uses this after placing an overlay over the output's picture.
    pub fn contrast_of(&self, before: &[[f32; 4]], after: &[[f32; 4]]) -> Option<f64> {
        // Only pixels the text changed can be glyphs. Find them on the stored values first and take the
        // luminance (two transfer-function pows per channel) of those alone: over a full frame the
        // luminance of every pixel cost ~24 million pows per measured text and dominated encodes.
        // Film grain, vignettes and later layers change every pixel a little, so keep the pixels whose raw
        // change is at least a fifth of the largest: a superset of the glyph interiors selected below.
        let n = before.len().min(after.len());
        let raw: Vec<f32> = (0..n)
            .map(|k| {
                let (a, b) = (before[k], after[k]);
                (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
            })
            .collect();
        let top = raw.iter().copied().fold(0.0f32, f32::max);
        if top <= 1e-6 {
            return None;
        }
        let changed: Vec<usize> = (0..n).filter(|&k| raw[k] >= top * 0.2).collect();
        let lb: Vec<f64> = changed.iter().map(|&k| self.luminance(before[k])).collect();
        let la: Vec<f64> = changed.iter().map(|&k| self.luminance(after[k])).collect();
        let delta: Vec<f64> = lb.iter().zip(&la).map(|(a, b)| (a - b).abs()).collect();
        let max = delta.iter().copied().fold(0.0, f64::max);
        if max < 1e-4 {
            return None;
        }
        // glyph interiors: pixels the text changed by at least half the largest change
        let (mut t, mut bg, mut n) = (0.0, 0.0, 0.0);
        for k in 0..delta.len() {
            if delta[k] >= max * 0.5 {
                t += la[k];
                bg += lb[k];
                n += 1.0;
            }
        }
        let (t, bg) = (t / n, bg / n);
        Some((t.max(bg) + 0.05) / (t.min(bg) + 0.05))
    }

    /// Contrast of one text layer as the viewer sees it, for text the inline probe cannot reach
    /// (inside isolated groups, masks, mattes or effects): renders the frame with and without the
    /// node and measures the pixels it changes — their colour with the node is the text, without
    /// it the backdrop. When colours are identical, contrasting probe inks establish
    /// coverage. `None` when no text coverage reaches the delivered frame.
    pub fn contrast_with_without(
        &mut self,
        g: &sr_eval::FrameGraph,
        p: &sr_eval::Program,
        node_id: &str,
        sub: &mut dyn FnMut(f64) -> sr_eval::FrameGraph,
    ) -> Option<f64> {
        let k = g.nodes.iter().position(|n| &*n.id == node_id);
        let captions = node_id == "captions";
        if !captions && k.is_none() {
            return None;
        }
        let probe = std::mem::replace(&mut self.contrast_probe, false);
        // read each frame back before the next render: frame targets may be pooled
        let with = self.render_with(g, p, Some(&mut *sub)).texture;
        let [w, h] = with.size;
        // read back only around the node (its box in frame space, plus room for glows), not the whole frame
        let r = k.and_then(|k| node_rect(&g.nodes[k], w, h, 48.0)).unwrap_or([0, 0, w, h]);
        let after = self.read_rect(&with, r);
        let mut hidden = g.clone();
        if let Some(k) = k {
            hidden.nodes[k].draw = false;
        }
        let previous_captions_off = self.captions_off;
        self.captions_off |= captions;
        let without = {
            let mut hidden_sub = |t| {
                let mut graph = sub(t);
                for n in &mut graph.nodes {
                    if &*n.id == node_id {
                        n.draw = false;
                    }
                }
                graph
            };
            self.render_with(&hidden, p, Some(&mut hidden_sub)).texture
        };
        let before = self.read_rect(&without, r);
        self.captions_off = previous_captions_off;
        let ratio = self.contrast_of(&before, &after).or_else(|| {
            let mut inks = Vec::new();
            for ink in [0.0, 1.0] {
                let graph = Self::contrast_ink_graph(g, node_id, ink);
                let mut ink_sub = |t| Self::contrast_ink_graph(&sub(t), node_id, ink);
                let frame = self.render_with(&graph, p, Some(&mut ink_sub));
                inks.push(self.read_rect(&frame.texture, r));
            }
            self.contrast_of_coverage(&before, &after, &inks[0], &inks[1])
        });
        self.contrast_probe = probe;
        ratio
    }

    /// Display-referred linear sRGB of each cell of a 48 × 27 grid over a frame (flash analysis).
    pub fn flash_grid(&mut self, frame: &Tex) -> Vec<[f64; 3]> {
        let d = self.gpu.device.clone();
        let (pipe, bgl) = self.grid.get_or_insert_with(|| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("flash-grid"),
                source: wgpu::ShaderSource::Wgsl(include_str!("analysis.wgsl").into()),
            });
            let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("flash-grid"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flash-grid"),
                bind_group_layouts: &[Some(&bgl)],
                immediate_size: 0,
            });
            let pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("flash-grid"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(resources::FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
            (pipe, bgl)
        });
        let out = resources::create(&d, &self.bgl1, [48, 27], 1, "flash-grid");
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flash-grid"),
            layout: bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame.view) }],
        });
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flash-grid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..3, 0..1);
        }
        self.gpu.queue.submit([enc.finish()]);
        self.read(&out)
            .into_iter()
            .map(|p| {
                let d = self.working.to_display_srgb([p[0] as f64, p[1] as f64, p[2] as f64]);
                d.map(|c| {
                    let c = c.clamp(0.0, 1.0);
                    if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                })
            })
            .collect()
    }
}
