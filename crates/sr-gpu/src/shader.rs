//! Custom GLSL effects (`effect type="shader"`) and transitions (`transition type="shader"`)
//! with the semantics of the Python engine: Shadertoy, ISF (inputs, imported images, passes,
//! persistent feedback) and plain programs; gl-transitions; `<param>` children, attributes
//! and `// =` defaults as uniforms; `@space` colour handling; `padding`; `@source` and named
//! sampler inputs; compile errors report user line numbers and fall back (effects pass their
//! input through, transitions crossfade).
//!
//! Colour: the input is un-premultiplied and converted from the stored working space to
//! `@space` (default `srgb`: straight, display-encoded sRGB, what Shadertoy/ISF code expects);
//! the output is read as straight colour in the same space and converted back. `raw` hands
//! premultiplied working values over untouched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sr_model::element::{AttrValue, Element};
use sr_model::model as m;

use crate::color::{self, Working};
use crate::fx::{Builder, Cx, FxEngine, Pass};
use crate::glsl::{self, Program};
use crate::resources::{Tex, FORMAT};
use crate::vector::Attrs;

/// A compiled custom program.
pub struct CustomPipe {
    pub program: Program,
    pub bgl: wgpu::BindGroupLayout,
    pub pipe: wgpu::RenderPipeline,
}

/// Uniform bytes and textures of one custom pass.
pub struct CustomBind {
    pub pipe: Arc<CustomPipe>,
    pub block: Vec<u8>,
    pub textures: Vec<Arc<Tex>>,
}

impl FxEngine {
    /// Compiles `program` (cached by its GLSL); errors carry the user's line numbers.
    pub fn program(&mut self, program: &Program) -> Result<Arc<CustomPipe>, String> {
        let h = sr_eval::rng::hash_str(&program.glsl);
        if let Some(r) = self.programs.get(&h) {
            return r.clone();
        }
        let r = crate::fx::check_glsl(&program.glsl)
            .map_err(|e| glsl::user_lines(&e, program.prelude_lines))
            .map(|_| Arc::new(self.build_pipe(program)));
        self.programs.insert(h, r.clone());
        r
    }

    fn build_pipe(&self, program: &Program) -> CustomPipe {
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("custom shader"),
            source: wgpu::ShaderSource::Glsl {
                shader: program.glsl.clone().into(),
                stage: wgpu::naga::ShaderStage::Fragment,
                defines: &[],
            },
        });
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        for k in 0..program.samplers.len() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + k as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let bgl = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("custom shader"),
            entries: &entries,
        });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("custom shader"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("custom shader"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        CustomPipe { program: program.clone(), bgl, pipe }
    }

    /// Records one custom pass.
    pub(crate) fn record_custom(&self, enc: &mut wgpu::CommandEncoder, bind: &CustomBind, out: &Tex) {
        use wgpu::util::DeviceExt;
        let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("custom uniforms"),
            contents: &bind.block,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.samp) },
        ];
        for (k, t) in bind.textures.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + k as u32,
                resource: wgpu::BindingResource::TextureView(&t.view),
            });
        }
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("custom shader"),
            layout: &bind.pipe.bgl,
            entries: &entries,
        });
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("custom shader"),
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
        rp.set_pipeline(&bind.pipe.pipe);
        rp.set_bind_group(0, &bg, &[]);
        rp.draw(0..3, 0..1);
    }

    /// Uploads premultiplied, stored-working RGBA pixels (row 0 on top).
    pub(crate) fn upload(&self, pool_tex: Arc<Tex>, px: &[[f32; 4]]) -> Arc<Tex> {
        let [w, h] = pool_tex.size;
        let bytes: Vec<u8> =
            px.iter().flat_map(|c| c.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes())).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &pool_tex.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        pool_tex
    }
}

// ------------------------------------------------------------------ colour spaces

/// The shader-facing colour space of `@space`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Space {
    Raw,
    /// Linear Rec.709/D65 → space matrix and transfer id (0 linear, 1 sRGB, 2 BT.709 OETF,
    /// 3 gamma 2.6, 4 ACEScct).
    Encoded {
        m: [[f64; 3]; 3],
        tf: u32,
        srgb: bool,
    },
}

const BRADFORD: [[f64; 3]; 3] = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

fn xyz(xy: (f64, f64)) -> [f64; 3] {
    [xy.0 / xy.1, 1.0, (1.0 - xy.0 - xy.1) / xy.1]
}

fn mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

pub(crate) fn inv(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |a: usize, b: usize, c: usize, dd: usize| m[a][b] * m[c][dd] - m[a][dd] * m[c][b];
    [
        [c(1, 1, 2, 2) / d, -c(0, 1, 2, 2) / d, c(0, 1, 1, 2) / d],
        [-c(1, 0, 2, 2) / d, c(0, 0, 2, 2) / d, -c(0, 0, 1, 2) / d],
        [c(1, 0, 2, 1) / d, -c(0, 0, 2, 1) / d, c(0, 0, 1, 1) / d],
    ]
}

fn npm(prim: [(f64, f64); 3], white: (f64, f64)) -> [[f64; 3]; 3] {
    let cols = prim.map(xyz);
    let p = [
        [cols[0][0], cols[1][0], cols[2][0]],
        [cols[0][1], cols[1][1], cols[2][1]],
        [cols[0][2], cols[1][2], cols[2][2]],
    ];
    let w = xyz(white);
    let pi = inv(&p);
    let s = [0, 1, 2].map(|i| (0..3).map(|k| pi[i][k] * w[k]).sum::<f64>());
    let mut o = p;
    for row in o.iter_mut() {
        for j in 0..3 {
            row[j] *= s[j];
        }
    }
    o
}

const D65: (f64, f64) = (0.3127, 0.3290);
const DCI: (f64, f64) = (0.314, 0.351);
const ACES: (f64, f64) = (0.32168, 0.33767);
const P709: [(f64, f64); 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const PP3: [(f64, f64); 3] = [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)];
const P2020: [(f64, f64); 3] = [(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)];
const AP1: [(f64, f64); 3] = [(0.713, 0.293), (0.165, 0.830), (0.128, 0.044)];
const AP0: [(f64, f64); 3] = [(0.7347, 0.2653), (0.0, 1.0), (0.0001, -0.0770)];

impl Space {
    /// `@space` (default srgb); `None` for an unknown name.
    pub fn parse(s: Option<&str>) -> Option<Space> {
        type Def = (Option<[(f64, f64); 3]>, (f64, f64), u32);
        let (prim, white, tf): Def = match s.unwrap_or("srgb") {
            "raw" => return Some(Space::Raw),
            "srgb" => (Some(P709), D65, 1),
            "linear-srgb" => (Some(P709), D65, 0),
            "rec709" => (Some(P709), D65, 2),
            "display-p3" => (Some(PP3), D65, 1),
            "dci-p3" => (Some(PP3), DCI, 3),
            "rec2020" => (Some(P2020), D65, 2),
            "acescg" => (Some(AP1), ACES, 0),
            "aces2065-1" => (Some(AP0), ACES, 0),
            "acescct" => (Some(AP1), ACES, 4),
            "xyz-d65" => (None, D65, 0),
            _ => return None,
        };
        let mut mm = npm(P709, D65);
        if white != D65 {
            let s = [0, 1, 2].map(|i| (0..3).map(|k| BRADFORD[i][k] * xyz(D65)[k]).sum::<f64>());
            let d = [0, 1, 2].map(|i| (0..3).map(|k| BRADFORD[i][k] * xyz(white)[k]).sum::<f64>());
            let diag = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
            mm = mul(&inv(&BRADFORD), &mul(&diag, &mul(&BRADFORD, &mm)));
        }
        let m = match prim {
            Some(p) => mul(&inv(&npm(p, white)), &mm),
            None => mm,
        };
        Some(Space::Encoded { m, tf, srgb: s.unwrap_or("srgb") == "srgb" })
    }
}

fn encode(tf: u32, x: f64) -> f64 {
    let x = x.max(0.0);
    match tf {
        1 => {
            if x <= 0.0031308 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            }
        }
        2 => {
            if x < 0.018 {
                4.5 * x
            } else {
                1.099 * x.powf(0.45) - 0.099
            }
        }
        3 => x.powf(1.0 / 2.6),
        4 => {
            if x <= 0.0078125 {
                10.5402377416545 * x + 0.0729055341958355
            } else {
                (x.max(1e-10).log2() + 9.72) / 17.52
            }
        }
        _ => x,
    }
}

/// Working space → linear Rec.709 matrix, and the stored transfer id (0 when stored linear).
fn working_io(w: &Working) -> ([[f64; 3]; 3], u32) {
    let m = color::convert(w.space, m::ColorSpace::LinearSrgb);
    let tf = if w.linear {
        0
    } else {
        match color::default_transfer(w.space) {
            m::Transfer::Bt1886 => 3,
            m::Transfer::Gamma22 => 4,
            m::Transfer::Gamma26 => 5,
            _ => 1,
        }
    };
    (m, tf)
}

/// A straight stored-working colour → the shader's space (for colour uniforms).
pub fn color_to_space(c: [f64; 4], w: &Working, space: &Space) -> Vec<f64> {
    match space {
        Space::Raw => vec![c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]],
        Space::Encoded { m, tf, .. } => {
            let lin = w.to_linear([c[0], c[1], c[2]]);
            let (wm, _) = working_io(w);
            let r = color::apply(&wm, lin);
            let s = [0, 1, 2].map(|i| (0..3).map(|k| m[i][k] * r[k]).sum::<f64>());
            vec![encode(*tf, s[0]), encode(*tf, s[1]), encode(*tf, s[2]), c[3]]
        }
    }
}

// Internal passes, written in the plain convention (uv is texture space here: row 0 on top).
const CONVERT: &str = r#"
uniform sampler2D src, other;
uniform mat3 M;
uniform int stored, tf, raw, to_space, flip, bleed, srgb;
float sdec(int t, float v) {
    if (t == 0) return v;
    if (t == 3) return pow(max(v, 0.0), 2.4);
    if (t == 4) return pow(max(v, 0.0), 2.2);
    if (t == 5) return pow(max(v, 0.0), 2.6);
    return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4);
}
float senc(int t, float x) {
    x = max(x, 0.0);
    if (t == 0) return x;
    if (t == 3) return pow(x, 1.0 / 2.4);
    if (t == 4) return pow(x, 1.0 / 2.2);
    if (t == 5) return pow(x, 1.0 / 2.6);
    return x <= 0.0031308 ? 12.92 * x : 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}
float enc(int t, float x) {
    x = max(x, 0.0);
    if (t == 1) return x <= 0.0031308 ? 12.92 * x : 1.055 * pow(x, 1.0 / 2.4) - 0.055;
    if (t == 2) return x < 0.018 ? 4.5 * x : 1.099 * pow(x, 0.45) - 0.099;
    if (t == 3) return pow(x, 1.0 / 2.6);
    if (t == 4) return x <= 0.0078125 ? 10.5402377416545 * x + 0.0729055341958355 : (log2(max(x, 1e-10)) + 9.72) / 17.52;
    return x;
}
float dec(int t, float v) {
    if (t == 1) { v = clamp(v, 0.0, 1.0); return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4); }
    if (t == 2) { v = clamp(v, 0.0, 1.0); return v < 0.081 ? v / 4.5 : pow((v + 0.099) / 1.099, 1.0 / 0.45); }
    if (t == 3) return pow(clamp(v, 0.0, 1.0), 2.6);
    if (t == 4) return max(v <= 0.155251141552511 ? (v - 0.0729055341958355) / 10.5402377416545 : exp2(min(v, 1.468) * 17.52 - 9.72), 0.0);
    return max(v, 0.0);
}
vec4 into_space(vec4 c) {
    if (raw != 0) return c;
    // the rasterizer leaves alpha of ~1e-5 just outside edges: colour there is not meaningful
    vec3 s = c.a > 1e-4 ? c.rgb / c.a : vec3(0.0);
    vec3 lin = M * vec3(sdec(stored, s.r), sdec(stored, s.g), sdec(stored, s.b));
    return vec4(enc(tf, lin.r), enc(tf, lin.g), enc(tf, lin.b), c.a);
}
vec4 from_space(vec4 c) {
    float a = clamp(c.a, 0.0, 1.0);
    if (raw != 0) return vec4(max(c.rgb, vec3(0.0)), a);
    vec3 lin = M * vec3(dec(tf, c.r), dec(tf, c.g), dec(tf, c.b));
    if (srgb == 0) lin = max(lin, vec3(0.0));
    if (stored != 0) lin = clamp(lin, 0.0, 1.0);
    vec3 st = vec3(senc(stored, lin.r), senc(stored, lin.g), senc(stored, lin.b));
    return vec4(st * a, a);
}
void main() {
    vec2 p = flip != 0 ? vec2(uv.x, 1.0 - uv.y) : uv;
    vec4 c = texture(src, p);
    if (to_space == 0) { fragColor = from_space(c); return; }
    vec4 o = into_space(c);
    if (bleed != 0 && c.a <= 1e-4) { vec4 d = into_space(texture(other, p)); o = vec4(d.rgb, o.a); }
    fragColor = o;
}
"#;

const RESIZE: &str = "uniform sampler2D src;\nvoid main() { fragColor = texture(src, uv); }\n";

/// Built-in and parameter inputs of a shader run.
struct Run<'r> {
    e: &'r dyn Element,
    a: &'r Attrs<'r>,
    params: HashMap<String, String>,
    builtins: HashMap<&'static str, Vec<f64>>,
    space: Space,
    working: Working,
    color: &'r crate::fx::ColorFn<'r>,
    /// Attribute-specific mappings (centre, seed, direction, pixel scale).
    special: &'r dyn Fn(&str, bool) -> Option<Vec<f64>>,
}

/// `<param name value>` children as raw strings.
pub fn param_map(e: &dyn Element) -> HashMap<String, String> {
    sr_model::element::children(e)
        .into_iter()
        .filter(|c| crate::vector::is(*c, "param"))
        .filter_map(|c| {
            let a = Attrs { e: c, props: None };
            Some((a.str("name")?, a.str("value").unwrap_or_default()))
        })
        .collect()
}

fn decl(e: &dyn Element, name: &str) -> Option<&'static sr_model::xsd::AttrDecl> {
    sr_model::xsd::COMPLEX_TYPES[e.xsd_type()].attr(name)
}

fn enums_of(ty: usize) -> &'static [&'static str] {
    match sr_model::xsd::SIMPLE_TYPES[ty].kind {
        sr_model::xsd::SimpleKind::Restriction(r) => {
            if r.enums.is_empty() {
                enums_of(r.base)
            } else {
                r.enums
            }
        }
        _ => &[],
    }
}

/// Written on the element (differs from the schema default) or animated.
fn explicit(e: &dyn Element, a: &Attrs, name: &str) -> bool {
    if a.props.and_then(|p| p.get(name)).is_some() {
        return true;
    }
    use sr_model::parse::ParseValue;
    let Some(v) = e.get_attr(name) else { return false };
    let Some(d) = decl(e, name).and_then(|d| d.default) else { return true };
    // the typed model applies schema defaults: a value equal to the default counts as unwritten
    match &v {
        AttrValue::Num(x) => d.trim().parse::<f64>().map(|y| (x - y).abs() > 1e-12).unwrap_or(true),
        AttrValue::Length(l) => d
            .trim()
            .trim_end_matches(|c: char| c.is_alphabetic() || c == '%')
            .parse::<f64>()
            .map(|y| (l.value - y).abs() > 1e-12)
            .unwrap_or(true),
        AttrValue::Bool(b) => *b != (d.trim() == "true"),
        AttrValue::Color(c) => sr_model::values::Color::parse_value(d.trim()).map(|dc| dc != *c).unwrap_or(true),
        AttrValue::Paint(pt) => sr_model::values::Paint::parse_value(d.trim()).map(|dp| dp != *pt).unwrap_or(true),
        other => other.to_string() != d,
    }
}

impl Run<'_> {
    /// A raw param/default value: numbers, booleans or a colour.
    fn parse(&self, raw: &str) -> Option<Vec<f64>> {
        if let Some(v) = glsl::parse_numbers(raw) {
            return Some(v);
        }
        use sr_model::parse::ParseValue;
        let c = sr_model::values::Color::parse_value(raw.trim()).ok()?;
        let val = crate::value_of_paint_tok(&sr_model::values::Paint::Color(c));
        let s = (self.color)(&val)?;
        Some(color_to_space(s, &self.working, &self.space))
    }

    /// The live (possibly animated) value of property `name`, else `raw`.
    fn live(&self, name: &str, raw: &str) -> Option<Vec<f64>> {
        if let Some(v) = self.a.props.and_then(|p| p.get(name)) {
            if let Some(n) = v.as_num() {
                return Some(vec![n]);
            }
            if let Some(c) = (self.color)(v) {
                return Some(color_to_space(c, &self.working, &self.space));
            }
        }
        self.parse(raw)
    }

    fn attribute(&self, name: &str, only_explicit: bool) -> Option<Vec<f64>> {
        if let Some(v) = (self.special)(name, only_explicit) {
            return Some(v);
        }
        let d = decl(self.e, name)?;
        if only_explicit && !explicit(self.e, self.a, name) {
            return None;
        }
        if let Some(v) = self.a.props.and_then(|p| p.get(name)) {
            if let Some(n) = v.as_num() {
                return Some(vec![n]);
            }
            if let Some(c) = (self.color)(v) {
                return Some(color_to_space(c, &self.working, &self.space));
            }
        }
        match self.e.get_attr(name)? {
            AttrValue::Num(v) => Some(vec![v]),
            AttrValue::Bool(b) => Some(vec![b as u8 as f64]),
            AttrValue::Length(l) => Some(vec![l.value]),
            AttrValue::Numbers(v) => Some(v),
            AttrValue::Point(p) => Some(vec![p.x, p.y]),
            AttrValue::Color(_) | AttrValue::Paint(_) => {
                let c = (self.color)(&self.a.paint(name)?)?;
                Some(color_to_space(c, &self.working, &self.space))
            }
            AttrValue::Str(s) => {
                let en = enums_of(d.ty);
                if let Some(i) = en.iter().position(|x| *x == s) {
                    return Some(vec![i as f64]);
                }
                glsl::parse_numbers(&s)
            }
            AttrValue::Tokens(_) => None,
        }
    }

    /// The precedence chain: built-ins, `<param>`, explicit attribute, `// =`/ISF default,
    /// schema default of an attribute.
    fn value(&self, name: &str, defaults: &HashMap<String, String>) -> Option<Vec<f64>> {
        if let Some(v) = self.builtins.get(name) {
            return Some(v.clone());
        }
        let native = decl(self.e, name).is_some();
        if let Some(raw) = self.params.get(name).filter(|_| name != "padding") {
            let v = if native { self.parse(raw) } else { self.live(name, raw) };
            if v.is_some() {
                return v;
            }
        }
        if let Some(v) = self.attribute(name, true) {
            return Some(v);
        }
        if let Some(raw) = defaults.get(name) {
            let v = if native { self.parse(raw) } else { self.live(name, raw) };
            if v.is_some() {
                return v;
            }
        } else if !native {
            if let Some(v) = self.live(name, "") {
                return Some(v);
            }
        }
        self.attribute(name, false)
    }

    fn block(&self, program: &Program, extra: &HashMap<&'static str, Vec<f64>>) -> Vec<u8> {
        let mut b = program.block();
        for u in &program.uniforms {
            let v = extra.get(u.name.as_str()).cloned().or_else(|| self.value(&u.name, &program.defaults));
            if let Some(v) = v {
                program.write(&mut b, u, &v);
            }
        }
        b
    }
}

/// Persistent ISF buffers of one effect instance, as of a frame.
#[derive(Default, Clone)]
pub struct Feedback {
    pub frame: i64,
    pub targets: HashMap<String, Arc<Tex>>,
}

impl Builder<'_> {
    fn custom(&mut self, pipe: &Arc<CustomPipe>, block: Vec<u8>, textures: Vec<Arc<Tex>>, size: [u32; 2]) -> Arc<Tex> {
        let out = self.tex(size);
        self.passes.push(Pass::custom(CustomBind { pipe: pipe.clone(), block, textures }, out.clone()));
        out
    }

    fn internal(&mut self, name: &str, code: &str) -> Arc<CustomPipe> {
        let p = glsl::build_effect(code).unwrap_or_else(|e| panic!("internal shader {name}: {e}"));
        self.eng.program(&p).unwrap_or_else(|e| panic!("internal shader {name}: {e}"))
    }

    /// A transparent 1×1 texture.
    fn empty(&mut self) -> Arc<Tex> {
        let t = self.tex([1, 1]);
        self.eng.upload(t, &[[0.0; 4]])
    }

    /// Stored working premultiplied → the shader's space, GL row order (optionally bleeding the
    /// colour of `other` into transparent texels).
    fn space_in(&mut self, input: &Arc<Tex>, other: Option<&Arc<Tex>>, space: &Space, working: &Working) -> Arc<Tex> {
        self.convert(input, other, space, working, true)
    }

    fn convert(
        &mut self,
        input: &Arc<Tex>,
        other: Option<&Arc<Tex>>,
        space: &Space,
        working: &Working,
        to: bool,
    ) -> Arc<Tex> {
        let pipe = self.internal("convert", CONVERT);
        let p = &pipe.program;
        let mut b = p.block();
        let (wm, stored) = working_io(working);
        let (mm, tf, raw, srgb) = match space {
            Space::Raw => ([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], 0, 1, 0),
            Space::Encoded { m, tf, srgb } => {
                let fwd = mul(m, &wm);
                (if to { fwd } else { inv(&fwd) }, *tf, 0, *srgb as u32)
            }
        };
        let cols: Vec<f64> = (0..3).flat_map(|j| (0..3).map(move |i| mm[i][j])).collect();
        let set = |b: &mut Vec<u8>, n: &str, v: &[f64]| {
            if let Some(u) = p.uniform(n) {
                p.write(b, u, v);
            }
        };
        set(&mut b, "M", &cols);
        set(&mut b, "stored", &[stored as f64]);
        set(&mut b, "tf", &[tf as f64]);
        set(&mut b, "raw", &[raw as f64]);
        set(&mut b, "to_space", &[to as u8 as f64]);
        set(&mut b, "flip", &[1.0]);
        set(&mut b, "bleed", &[other.is_some() as u8 as f64]);
        set(&mut b, "srgb", &[srgb as f64]);
        let other = match other {
            Some(o) => o.clone(),
            None => self.empty(),
        };
        let size = input.size;
        let tex = |n: &str| if n == "src" { input.clone() } else { other.clone() };
        let textures = p.samplers.iter().map(|n| tex(n)).collect();
        self.custom(&pipe, b, textures, size)
    }

    /// Runs a custom effect. `named` holds sampler inputs resolved by the renderer (nodes and
    /// image assets, stored working premultiplied, row 0 on top).
    pub fn shader_effect(&mut self, e: &dyn Element, a: &Attrs, input: &Arc<Tex>, cx: &Cx) -> Result<Arc<Tex>, String> {
        let src = a.str("src").ok_or("effect type='shader' without @src; passed through")?;
        let (code, dir) = glsl::load_source(&src, cx.base)?;
        let program = glsl::build_effect(&code)?;
        let pipe = self.eng.program(&program).map_err(|e| format!("GLSL compile/link failed; passed through.\n{e}"))?;
        let space = match Space::parse(a.str("space").as_deref()) {
            Some(s) => s,
            None => {
                self.problems.push(format!("unknown shader colour space {:?}; using srgb", a.str("space")));
                Space::parse(None).unwrap()
            }
        };
        let [w, h] = input.size;
        let working = cx.working;
        let inp = self.space_in(input, None, &space, &working);
        let source = cx.source.clone().map(|s| self.space_in(&s, None, &space, &working));
        let mut textures: HashMap<String, Arc<Tex>> = HashMap::new();
        for (n, t) in &cx.named {
            let t = self.space_in(t, None, &space, &working);
            textures.insert(n.clone(), t);
        }
        let header = program.isf.clone();
        let images: Vec<String> = header
            .as_ref()
            .and_then(|h| h.get("INPUTS"))
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter(|i| i.get("TYPE").and_then(|t| t.as_str()) == Some("image"))
            .filter_map(|i| i.get("NAME").and_then(|n| n.as_str()).map(String::from))
            .collect();
        for (name, t) in images.iter().zip([Some(inp.clone()), source.clone()]) {
            if let Some(t) = t {
                textures.entry(name.clone()).or_insert(t);
            }
        }
        for n in ["iChannel0", "inputTexture"] {
            textures.insert(n.into(), inp.clone());
        }
        if let Some(s) = &source {
            for n in ["iChannel1", "sourceTexture"] {
                textures.insert(n.into(), s.clone());
            }
        }
        let params = param_map(e);
        // named samplers given as image files (nodes and assets arrive in cx.named)
        for n in &program.samplers {
            if textures.contains_key(n) {
                continue;
            }
            if let Some(v) = params.get(n) {
                match self.image(Path::new(v), cx.base, &working) {
                    Ok(t) => {
                        let t = self.space_in(&t, None, &space, &working);
                        textures.insert(n.clone(), t);
                    }
                    Err(err) => self.problems.push(format!("sampler {n}: {err}")),
                }
            }
        }
        if let Some(h) = &header {
            for inp in h.get("INPUTS").and_then(|v| v.as_array()).into_iter().flatten() {
                let kind = inp.get("TYPE").and_then(|v| v.as_str()).unwrap_or("");
                let Some(name) = inp.get("NAME").and_then(|v| v.as_str()) else { continue };
                if !matches!(kind, "audio" | "audioFFT") {
                    continue;
                }
                let source = params.get(name).cloned().or_else(|| a.str(name)).unwrap_or_else(|| "master".into());
                let max = inp.get("MAX").and_then(|v| v.as_f64()).map(|m| m.max(1.0) as usize);
                let (w, hh, px) = match cx.audio.as_ref() {
                    Some(au) => match au.channels.get(&source) {
                        Some(sig) => audio_texture(sig, au.rate, cx.time, kind == "audioFFT", max),
                        None => {
                            self.problems.push(format!("audio input {name}: no track or bus {source:?}; silent"));
                            audio_texture(&[], au.rate, cx.time, kind == "audioFFT", max)
                        }
                    },
                    None => {
                        self.problems.push(format!("audio input {name}: no audio mix in this render; silent"));
                        audio_texture(&[], 48000.0, cx.time, kind == "audioFFT", max)
                    }
                };
                let t = Arc::new(crate::resources::create(self.device, self.bgl1, [w, hh], 1, "shader audio"));
                let t = self.eng.upload(t, &px);
                self.temps.push(t.clone());
                textures.insert(name.to_string(), t);
            }
            for (name, spec) in h.get("IMPORTED").and_then(|v| v.as_object()).into_iter().flatten() {
                let Some(path) = spec.get("PATH").and_then(|p| p.as_str()) else { continue };
                match self.image(Path::new(path), &dir, &working) {
                    Ok(t) => {
                        let t = self.space_in(&t, None, &space, &working);
                        textures.insert(name.clone(), t);
                    }
                    Err(err) => self.problems.push(format!("ISF IMPORTED {path}: {err}; transparent")),
                }
            }
        }
        let fps = cx.fps.max(1e-6);
        let now = cx.time;
        let chan = |i: usize, t: &HashMap<String, Arc<Tex>>| t.get(&format!("iChannel{i}")).map(|x| x.size);
        let mut builtins: HashMap<&'static str, Vec<f64>> = HashMap::new();
        let mut put = |k: &'static str, v: Vec<f64>| {
            builtins.insert(k, v);
        };
        put("iResolution", vec![w as f64, h as f64, 1.0]);
        put("iTime", vec![now]);
        put("iTimeDelta", vec![1.0 / fps]);
        put("iFrame", vec![cx.frame as f64]);
        put("iFrameRate", vec![fps]);
        put("iDate", vec![0.0, 0.0, 0.0, now]);
        put("iOffset", vec![cx.offset[0], cx.offset[1]]);
        put("iFrameResolution", vec![cx.frame_size[0], cx.frame_size[1]]);
        put(
            "iChannelResolution",
            (0..4)
                .flat_map(|i| chan(i, &textures).map(|s| [s[0] as f64, s[1] as f64, 1.0]).unwrap_or([1.0; 3]))
                .collect(),
        );
        put("iChannelTime", (0..4).map(|i| if chan(i, &textures).is_some() { now } else { 0.0 }).collect());
        put("resolution", vec![w as f64, h as f64]);
        put("frameResolution", vec![cx.frame_size[0], cx.frame_size[1]]);
        put("tileOffset", vec![cx.offset[0], cx.offset[1]]);
        put("time", vec![now]);
        put("localTime", vec![cx.local_time]);
        put("timeDelta", vec![1.0 / fps]);
        put("fps", vec![fps]);
        put("frame", vec![cx.frame as f64]);
        put("TIME", vec![now]);
        put("TIMEDELTA", vec![if now == 0.0 { 0.0 } else { 1.0 / fps }]);
        put("FRAMEINDEX", vec![cx.frame as f64]);
        put("DATE", vec![0.0, 0.0, 0.0, now]);
        let mut defaults = program.defaults.clone();
        defaults.entry("iMouse".into()).or_insert_with(|| "0".into());
        defaults.entry("iSampleRate".into()).or_insert_with(|| "48000".into());
        let program = Program { defaults, ..program };
        let seed = match a.opt("seed") {
            Some(s) if e.get_attr("seed").is_some() => s,
            _ => (sr_eval::rng::hash_str(&format!("{}:shader", e.element_id().unwrap_or(""))) % 65536) as f64,
        };
        let scale = cx.scale;
        let (cxu, cyu) = (cx.center[0], cx.center[1]);
        let center_of = |a: &Attrs| -> [f64; 2] {
            let uvc = match (a.opt("centerX"), a.opt("centerY")) {
                (None, None) => [cxu, cyu],
                (x, y) => {
                    let p = (cx.to_uv)([x.unwrap_or(0.0), y.unwrap_or(0.0)]);
                    [if x.is_some() { p[0] } else { cxu }, if y.is_some() { p[1] } else { cyu }]
                }
            };
            [uvc[0] * w as f64, (1.0 - uvc[1]) * h as f64]
        };
        let special = |name: &str, only_explicit: bool| -> Option<Vec<f64>> {
            match name {
                "centerX" | "centerY" | "center" => {
                    if only_explicit && !(explicit(e, a, "centerX") || explicit(e, a, "centerY")) {
                        return None;
                    }
                    let c = center_of(a);
                    Some(match name {
                        "centerX" => vec![c[0]],
                        "centerY" => vec![c[1]],
                        _ => vec![c[0], c[1]],
                    })
                }
                "seed" => Some(vec![seed]),
                "radius" | "offsetX" | "offsetY" => {
                    if only_explicit && !explicit(e, a, name) {
                        return None;
                    }
                    decl(e, name)?;
                    Some(vec![a.num(name, 0.0) * scale])
                }
                _ => None,
            }
        };
        let run = Run { e, a, params, builtins, space, working, color: cx.color, special: &special };
        let passes: Vec<serde_json::Value> = header
            .as_ref()
            .and_then(|h| h.get("PASSES"))
            .and_then(|v| v.as_array())
            .cloned()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| vec![serde_json::json!({})]);
        let persistent: Vec<String> = passes
            .iter()
            .filter(|p| p.get("PERSISTENT").and_then(|v| v.as_bool()).unwrap_or(false))
            .filter_map(|p| p.get("TARGET").and_then(|t| t.as_str()).map(String::from))
            .collect();
        let key = format!("{}|{}|{}", e.element_id().unwrap_or(""), cx.node, sr_eval::rng::hash_str(&code));
        if !persistent.is_empty() {
            // every read within a frame sees the history as it stood when the frame began
            if let Some(fb) = self.eng.feedback_at(&key, cx.frame) {
                for (n, t) in fb.targets {
                    textures.entry(n).or_insert(t);
                }
            }
        }
        let mut names: HashMap<String, f64> =
            [("WIDTH".to_string(), w as f64), ("HEIGHT".to_string(), h as f64)].into();
        for inp in header.as_ref().and_then(|h| h.get("INPUTS")).and_then(|v| v.as_array()).into_iter().flatten() {
            let (Some(n), Some(t)) =
                (inp.get("NAME").and_then(|v| v.as_str()), inp.get("TYPE").and_then(|v| v.as_str()))
            else {
                continue;
            };
            if matches!(t, "float" | "long" | "bool" | "event") {
                if let Some(v) = run.value(n, &program.defaults) {
                    names.insert(n.to_string(), v[0]);
                }
            }
        }
        let empty = self.empty();
        let mut out = inp.clone();
        let mut sizes = [w, h];
        for (i, ps) in passes.iter().enumerate() {
            let pw = glsl::isf_size(ps.get("WIDTH"), &names, w);
            let ph = glsl::isf_size(ps.get("HEIGHT"), &names, h);
            let extra: HashMap<&'static str, Vec<f64>> =
                [("RENDERSIZE", vec![pw as f64, ph as f64]), ("PASSINDEX", vec![i as f64])].into();
            let block = run.block(&program, &extra);
            let texs =
                program.samplers.iter().map(|n| textures.get(n).cloned().unwrap_or_else(|| empty.clone())).collect();
            out = self.custom(&pipe, block, texs, [pw, ph]);
            sizes = [pw, ph];
            if let Some(t) = ps.get("TARGET").and_then(|t| t.as_str()) {
                textures.insert(t.to_string(), out.clone());
            }
        }
        if !persistent.is_empty() {
            let targets = persistent.iter().filter_map(|n| textures.get(n).map(|t| (n.clone(), t.clone()))).collect();
            self.eng.feedback.insert(key, Feedback { frame: cx.frame, targets });
        }
        if sizes != [w, h] {
            let rs = self.internal("resize", RESIZE);
            let b = rs.program.block();
            out = self.custom(&rs, b, vec![out], [w, h]);
        }
        Ok(self.convert(&out, None, &space, &working, false))
    }

    /// An image file decoded into the stored working space and uploaded (cached by path).
    fn image(&mut self, path: &Path, base: &Path, working: &Working) -> Result<Arc<Tex>, String> {
        let full: PathBuf = if path.is_absolute() { path.to_path_buf() } else { base.join(path) };
        if let Some(t) = self.eng.images.get(&full) {
            return Ok(t.clone());
        }
        let d = crate::resources::decode_image(
            &full,
            m::ColorSpace::Srgb,
            m::Transfer::Auto,
            m::AlphaMode::Auto,
            working,
            16384,
        )?;
        let (w, h, px) = d.levels.into_iter().next().ok_or("empty image")?;
        let t = Arc::new(crate::resources::create(self.device, self.bgl1, [w, h], 1, "shader image"));
        let t = self.eng.upload(t, &px);
        self.eng.images.insert(full, t.clone());
        Ok(t)
    }

    /// A gl-transitions shader between two frame-sized images. `velocity` is d(progress)/dt.
    #[allow(clippy::too_many_arguments)]
    pub fn shader_transition(
        &mut self,
        code: &str,
        from: &Arc<Tex>,
        to: &Arc<Tex>,
        matte: Option<Arc<Tex>>,
        p: f64,
        velocity: f64,
        e: &dyn Element,
        a: &Attrs,
        cx: &Cx,
    ) -> Result<Arc<Tex>, String> {
        let program = glsl::build_transition(code)?;
        let pipe = self.eng.program(&program).map_err(|e| format!("GLSL compile/link failed; crossfading.\n{e}"))?;
        let space = Space::parse(None).unwrap();
        let working = cx.working;
        let [w, h] = from.size;
        let fs = self.space_in(from, Some(to), &space, &working);
        let ts = self.space_in(to, Some(from), &space, &working);
        let mut textures: HashMap<String, Arc<Tex>> = [("from".to_string(), fs), ("to".to_string(), ts)].into();
        if program.samplers.iter().any(|s| s == "matte") {
            match matte {
                Some(m) => {
                    let m = self.space_in(&m, None, &space, &working);
                    textures.insert("matte".into(), m);
                }
                None => self.problems.push("shader samples `matte` but @matte is missing; transparent".into()),
            }
        }
        let dir = || -> (f64, f64) {
            match a.str("direction").as_deref() {
                Some("angle") => {
                    let r = a.num("angle", 0.0).to_radians();
                    (r.cos(), r.sin())
                }
                Some("right") => (1.0, 0.0),
                Some("up") => (0.0, -1.0),
                Some("down") => (0.0, 1.0),
                _ => (-1.0, 0.0),
            }
        };
        let vec_dir = program.uniform("direction").map(|u| u.dimension() >= 2).unwrap_or(false);
        let special = |name: &str, only_explicit: bool| -> Option<Vec<f64>> {
            if name != "direction" {
                return None;
            }
            if only_explicit && !(explicit(e, a, "direction") || explicit(e, a, "angle")) {
                return None;
            }
            let (dx, dy) = dir();
            Some(if vec_dir { vec![dx, -dy] } else { vec![dy.atan2(dx).to_degrees().rem_euclid(360.0)] })
        };
        let mut run = Run {
            e,
            a,
            params: param_map(e),
            builtins: HashMap::new(),
            space,
            working,
            color: cx.color,
            special: &special,
        };
        run.builtins.insert("ratio", vec![w as f64 / h.max(1) as f64]);
        run.builtins.insert("resolution", vec![w as f64, h as f64]);
        run.builtins.insert("time", vec![cx.time]);
        run.builtins.insert("frame", vec![cx.frame as f64]);
        // shutter samples of progress (Python's _progress_samples)
        let blur = a.num("motionBlur", 1.0) != 0.0;
        let dp = velocity.abs() * 0.5 / cx.fps.max(1e-6);
        let ps: Vec<f64> = if !blur || dp < 1.0 / 512.0 {
            vec![p]
        } else {
            let n = ((dp * w.max(h) as f64 / 2.0).ceil() as usize).clamp(2, 16);
            (0..n).map(|k| (p + dp * (k as f64 / (n - 1) as f64 - 0.5)).clamp(0.0, 1.0)).collect()
        };
        let empty = self.empty();
        let mut outs = Vec::new();
        for q in &ps {
            let extra: HashMap<&'static str, Vec<f64>> = [("progress", vec![*q])].into();
            let block = run.block(&program, &extra);
            let texs =
                program.samplers.iter().map(|n| textures.get(n).cloned().unwrap_or_else(|| empty.clone())).collect();
            let o = self.custom(&pipe, block, texs, [w, h]);
            outs.push(self.convert(&o, None, &space, &working, false));
        }
        if outs.len() == 1 {
            return Ok(outs.pop().unwrap());
        }
        let acc = self.tex([w, h]);
        let k = 1.0 / outs.len() as f64;
        for (i, o) in outs.iter().enumerate() {
            self.accumulate(&acc, o, k, i == 0);
        }
        Ok(acc)
    }
}

/// `<param name="padding" value="t[,r[,b,l]]">` in document pixels (CSS order), the largest side.
pub fn padding(e: &dyn Element) -> f64 {
    let Some(v) = param_map(e).get("padding").and_then(|s| glsl::parse_numbers(s)) else { return 0.0 };
    v.iter().cloned().fold(0.0, f64::max).max(0.0)
}

// ------------------------------------------------------------------ ISF audio inputs

/// Mixed audio a shader may read: the master (`master`) and every track and bus by id, planar.
pub struct AudioSignals {
    pub rate: f64,
    pub channels: HashMap<String, Arc<Vec<Vec<f32>>>>,
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for i in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (i + k, i + k + len / 2);
                let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

/// An ISF `audio` (waveform around .5) or `audioFFT` (Hann-windowed linear magnitudes, a
/// full-scale bin-centred sine at 1) texture: 2048 samples centred on `t`, one row per channel,
/// reduced to `max` columns (spectra keep each group's peak, waveforms its mean, or RMS for one
/// column). Rows are channels from the bottom (GL order).
pub fn audio_texture(
    sig: &[Vec<f32>],
    rate: f64,
    t: f64,
    fft_kind: bool,
    max: Option<usize>,
) -> (u32, u32, Vec<[f32; 4]>) {
    const N: usize = 2048;
    let begin = (t * rate).round() as i64 - (N / 2) as i64;
    let chans = sig.len().max(1);
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for c in 0..chans {
        let mut seg = vec![0.0f64; N];
        if let Some(ch) = sig.get(c) {
            for (k, v) in seg.iter_mut().enumerate() {
                let i = begin + k as i64;
                if i >= 0 && (i as usize) < ch.len() {
                    *v = ch[i as usize] as f64;
                }
            }
        }
        let values = if fft_kind {
            let win: Vec<f64> =
                (0..N).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (N - 1) as f64).cos()).collect();
            let sum: f64 = win.iter().sum();
            let mut re: Vec<f64> = seg.iter().zip(&win).map(|(a, w)| a * w).collect();
            let mut im = vec![0.0; N];
            fft(&mut re, &mut im);
            let mut mag: Vec<f64> = (0..=N / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * 2.0 / sum).collect();
            mag[0] *= 0.5;
            let last = mag.len() - 1;
            mag[last] *= 0.5;
            mag
        } else {
            seg
        };
        let len = values.len();
        let width = max.unwrap_or(len).clamp(1, len);
        let reduced = if width < len {
            let edges: Vec<usize> = (0..=width).map(|i| (i as f64 * len as f64 / width as f64) as usize).collect();
            (0..width)
                .map(|i| {
                    let part = &values[edges[i]..edges[i + 1].max(edges[i] + 1).min(len)];
                    if fft_kind {
                        part.iter().cloned().fold(f64::MIN, f64::max)
                    } else if width == 1 {
                        (part.iter().map(|v| v * v).sum::<f64>() / part.len() as f64).sqrt()
                    } else {
                        part.iter().sum::<f64>() / part.len() as f64
                    }
                })
                .collect()
        } else {
            values
        };
        rows.push(reduced.into_iter().map(|v| if fft_kind { v } else { 0.5 + 0.5 * v }).collect());
    }
    let w = rows[0].len() as u32;
    let px = rows
        .iter()
        .flat_map(|r| {
            r.iter().map(|v| {
                let x = v.clamp(0.0, 1.0) as f32;
                [x, x, x, 1.0]
            })
        })
        .collect();
    (w, chans as u32, px)
}

impl FxEngine {
    /// The persistent buffers of `key` at the start of `frame`.
    fn feedback_at(&mut self, key: &str, frame: i64) -> Option<Feedback> {
        if self.feedback_frame.0 != frame {
            self.feedback_frame = (frame, self.feedback.clone());
        }
        self.feedback_frame.1.get(key).cloned()
    }

    /// Records the history after `frame` (about once per second, at most 33 plus frame 0).
    pub(crate) fn checkpoint(&mut self, frame: i64, interval: i64) {
        if frame % interval.max(1) != 0 {
            return;
        }
        self.checkpoints.insert(frame, self.feedback.clone());
        while self.checkpoints.len() > 33 {
            let drop = *self.checkpoints.keys().find(|k| **k != 0).unwrap();
            self.checkpoints.remove(&drop);
        }
    }

    /// Restores the latest history recorded at or before `frame`; returns the first frame to replay.
    pub(crate) fn restore(&mut self, frame: i64) -> i64 {
        self.feedback_frame = (i64::MIN, HashMap::new());
        match self.checkpoints.range(..=frame).next_back() {
            Some((k, v)) => {
                self.feedback = v.clone();
                k + 1
            }
            None => {
                self.feedback.clear();
                0
            }
        }
    }
}

/// Whether any shader effect of the scene has PERSISTENT ISF passes (its renders then replay
/// earlier frames on a seek, so feedback is the same whichever frame is rendered first).
pub fn has_persistent(p: &sr_eval::Program, base: &Path) -> bool {
    let Some(fx) = p.scene.effects.as_ref() else { return false };
    fx.effects.iter().filter(|e| e.r#type.as_str() == "shader").any(|e| {
        let a = Attrs { e: e as &dyn Element, props: None };
        let Some(src) = a.str("src") else { return false };
        let Ok((code, _)) = glsl::load_source(&src, base) else { return false };
        glsl::build_effect(&code)
            .ok()
            .and_then(|p| p.isf)
            .and_then(|h| h.get("PASSES").and_then(|v| v.as_array()).cloned())
            .is_some_and(|ps| ps.iter().any(|p| p.get("PERSISTENT").and_then(|v| v.as_bool()).unwrap_or(false)))
    })
}
