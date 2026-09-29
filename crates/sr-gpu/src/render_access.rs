//! Accessibility measurements in the compositor (Batch 9): the contrast of
//! burned-in text against the backdrop actually drawn behind it.
//!
//! When `contrast_probe` is on, each text layer and the caption burn-in in
//! the frame's own target get their own draw command, preceded by a copy of
//! the target (the backdrop). After the frame, the pixels the text changed are
//! its glyphs: their colour in the frame is the text, and their colour in the
//! snapshot is the background behind it.

use super::*;

impl Renderer {
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
    ) -> Option<usize> {
        if !self.contrast_probe
            || !self.is_root_space(ctx, space)
            || !self.is_text_layer(ctx, &ctx.g.nodes[i])
            || !ctx.g.nodes[i].draw
        {
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

    /// WCAG relative luminance of a stored working-space pixel (composited over black).
    fn luminance(&self, p: [f32; 4]) -> f64 {
        let d = self.working.to_display_srgb([p[0] as f64, p[1] as f64, p[2] as f64]);
        let lin = |c: f64| {
            let c = c.clamp(0.0, 1.0);
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(d[0]) + 0.7152 * lin(d[1]) + 0.0722 * lin(d[2])
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
        let lb: Vec<f64> = before.iter().map(|p| self.luminance(*p)).collect();
        let la: Vec<f64> = after.iter().map(|p| self.luminance(*p)).collect();
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
