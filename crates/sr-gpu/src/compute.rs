//! SREP 67, Semantics 1 to 3: the `compute` node. A WGSL compute shader, with the engine's prelude, calls `sr_point(i)`
//! once for each invocation; `sr_accumulate` adds integer amounts into a histogram of 64-bit cells; a density tonemap
//! turns the histogram into the node's picture.
//!
//! **64-bit cells from 32-bit atomics.** WGSL atomics are 32-bit. Each cell is a low and a high word: `old =
//! atomicAdd(&lo, a)`, and `atomicAdd(&hi, 1)` when `old > 0xFFFFFFFF − a` (the low word wrapped). The number of wraps is
//! ⌊Σa / 2³²⌋ whatever the order, so the pair is the exact sum (the SREP's engine note; verified by the tests here on the
//! adapters they run on).

use std::sync::Arc;

/// The prelude the engine puts before the author's source: the declarations of SREP 67, Semantics 1, and the entry point
/// that calls `sr_point`. `sr_internal` holds (channels, fractionBits, invocations per dispatch row, 0).
pub const PRELUDE: &str = r#"
struct SrCompute { size: vec2<u32>, invocations: u32, frame: i32, time: f32, seed_lo: u32, seed_hi: u32, nparams: u32 }
@group(0) @binding(0) var<uniform> sr: SrCompute;
@group(0) @binding(1) var<storage, read_write> sr_hist: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read> sr_params: array<f32>;
@group(0) @binding(3) var<uniform> sr_internal: vec4<u32>;

fn sr_accumulate(x: i32, y: i32, channel: u32, amount: u32) {
    if (x < 0 || y < 0 || u32(x) >= sr.size.x || u32(y) >= sr.size.y || channel >= sr_internal.x || amount == 0u) {
        return;
    }
    let cell = ((channel * sr.size.y + u32(y)) * sr.size.x + u32(x)) * 2u;
    let old = atomicAdd(&sr_hist[cell], amount);
    if (old > 0xFFFFFFFFu - amount) {
        atomicAdd(&sr_hist[cell + 1u], 1u);
    }
}

fn sr_accumulate_f(x: i32, y: i32, channel: u32, amount: f32) {
    let scale = f32(1u << sr_internal.y);
    let top = 4294967295.0 / scale;
    // 4294967040 is the largest f32 below 2^32
    let q = min(floor(clamp(amount, 0.0, top) * scale + 0.5), 4294967040.0);
    sr_accumulate(x, y, channel, u32(q));
}

fn sr_param(index: u32) -> f32 {
    if (index >= sr.nparams) {
        return 0.0;
    }
    return sr_params[index];
}

fn sr_hash(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
"#;

const ENTRY: &str = r#"
@compute @workgroup_size(64)
fn sr_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * sr_internal.z + gid.x;
    if (i >= sr.invocations) {
        return;
    }
    sr_point(i);
}
"#;

/// The whole module of an author's source.
pub fn module_source(user: &str) -> String {
    format!("{PRELUDE}\n{user}\n{ENTRY}")
}

/// Checks an author's source against the prelude without a device (CMP11): it compiles, defines `sr_point`, and declares
/// no group-0 bindings of its own.
pub fn check_source(user: &str) -> Result<(), String> {
    if group_zero(user) {
        return Err("CMP11: the shader declares its own @group(0) bindings; the engine binds group 0".into());
    }
    let module = naga::front::wgsl::parse_str(&module_source(user)).map_err(|e| format!("CMP11: {}", e.message()))?;
    let point = module.functions.iter().find(|(_, f)| f.name.as_deref() == Some("sr_point"));
    match point {
        Some((_, f)) if f.arguments.len() == 1 && f.result.is_none() => {}
        _ => return Err("CMP11: the shader defines no fn sr_point(i: u32)".into()),
    }
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("CMP11: {}", e.as_inner()))?;
    Ok(())
}

/// Whether `src` names group 0 in an attribute (comments ignored).
fn group_zero(src: &str) -> bool {
    let code: String = src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let mut rest = code.as_str();
    while let Some(i) = rest.find("@group") {
        rest = &rest[i + 6..];
        let t = rest.trim_start();
        if let Some(t) = t.strip_prefix('(') {
            if t.trim_start().starts_with('0') && !t.trim_start()[1..].starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// What a compute node asks for at one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub width: u32,
    pub height: u32,
    pub invocations: u32,
    pub channels: u32,
    pub fraction_bits: u32,
    pub frame: i32,
    pub time: f32,
    pub seed: u64,
    pub params: Vec<f32>,
}

/// The seed words of SREP 67, Semantics 1: `mix64(project seed ⊕ mix64(compute seed))`, low and high 32 bits.
pub fn seed_words(project_seed: u64, compute_seed: u64) -> (u32, u32) {
    let s = sr_eval::rng::mix64(project_seed ^ sr_eval::rng::mix64(compute_seed));
    (s as u32, (s >> 32) as u32)
}

/// A compiled compute program.
pub struct Program {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

/// Compiles an author's source (checked first, so a bad source is a message, not a device error).
pub fn compile(device: &wgpu::Device, user: &str) -> Result<Program, String> {
    check_source(user)?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("compute node"),
        source: wgpu::ShaderSource::Wgsl(module_source(user).into()),
    });
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let storage = |read_only| wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("compute node"),
        entries: &[entry(0, uniform), entry(1, storage(false)), entry(2, storage(true)), entry(3, uniform)],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("compute node"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("compute node"),
        layout: Some(&pl),
        module: &module,
        entry_point: Some("sr_main"),
        compilation_options: Default::default(),
        cache: None,
    });
    Ok(Program { pipeline, layout })
}

/// Runs `job` and returns its histogram: `channels` planes of `width` × `height` exact 64-bit sums (CMP12 when the
/// device cannot hold it).
pub fn run(device: &wgpu::Device, queue: &wgpu::Queue, program: &Program, job: &Job) -> Result<Vec<u64>, String> {
    use wgpu::util::DeviceExt;
    let cells = job.width as u64 * job.height as u64 * job.channels as u64;
    let bytes = cells * 8;
    let limit = device.limits().max_storage_buffer_binding_size.min(device.limits().max_buffer_size);
    if bytes > limit {
        return Err(format!(
            "CMP12: the histogram needs {bytes} bytes ({} x {} x {} cells of 8 bytes); the device binds at most {limit}",
            job.width, job.height, job.channels
        ));
    }
    let (seed_lo, seed_hi) = (job.seed as u32, (job.seed >> 32) as u32);
    let mut uniform = Vec::with_capacity(32);
    for w in [job.width, job.height, job.invocations, job.frame as u32] {
        uniform.extend_from_slice(&w.to_le_bytes());
    }
    uniform.extend_from_slice(&job.time.to_le_bytes());
    for w in [seed_lo, seed_hi, job.params.len() as u32] {
        uniform.extend_from_slice(&w.to_le_bytes());
    }
    let groups = (job.invocations as u64).div_ceil(64).max(1);
    let gx = groups.min(65535) as u32;
    let gy = groups.div_ceil(gx as u64) as u32;
    let internal: Vec<u8> =
        [job.channels, job.fraction_bits, gx * 64, 0].iter().flat_map(|w| w.to_le_bytes()).collect();
    let params: Vec<u8> = if job.params.is_empty() {
        0f32.to_le_bytes().to_vec()
    } else {
        job.params.iter().flat_map(|v| v.to_le_bytes()).collect()
    };
    let buf = |label, contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
    };
    let ub = buf("compute uniforms", &uniform, wgpu::BufferUsages::UNIFORM);
    let ib = buf("compute internal", &internal, wgpu::BufferUsages::UNIFORM);
    let pb = buf("compute params", &params, wgpu::BufferUsages::STORAGE);
    let hist = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("compute histogram"),
        size: bytes.max(8),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("compute readback"),
        size: bytes.max(8),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("compute node"),
        layout: &program.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: hist.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: pb.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: ib.as_entire_binding() },
        ],
    });
    let mut enc = device.create_command_encoder(&Default::default());
    // every cell is zero at the start of each frame
    enc.clear_buffer(&hist, 0, None);
    {
        let mut pass = enc
            .begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("compute node"), timestamp_writes: None });
        pass.set_pipeline(&program.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(gx, gy, 1);
    }
    enc.copy_buffer_to_buffer(&hist, 0, &read, 0, bytes.max(8));
    queue.submit([enc.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = read.slice(..).get_mapped_range().map_err(|e| format!("compute histogram: {e:?}"))?;
    Ok(data
        .as_chunks::<8>()
        .0
        .iter()
        .take(cells as usize)
        .map(|c| {
            let lo = u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64;
            let hi = u32::from_le_bytes([c[4], c[5], c[6], c[7]]) as u64;
            (hi << 32) | lo
        })
        .collect())
}

/// The density tonemap of SREP 67, Semantics 3.
#[derive(Debug, Clone, PartialEq)]
pub struct Tonemap {
    pub log: bool,
    pub gain: f64,
    pub gamma: f64,
    /// `None` for "max".
    pub reference: Option<f64>,
    /// Display-encoded RGBA stops, equally spaced on [0, 1].
    pub ramp: Vec<[f64; 4]>,
    pub alpha_density: bool,
}

impl Default for Tonemap {
    fn default() -> Self {
        Tonemap {
            log: true,
            gain: 1.0,
            gamma: 1.0,
            reference: None,
            ramp: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            alpha_density: false,
        }
    }
}

/// A ramp attribute: `#RRGGBB[AA]` colours separated by white space, as display-encoded RGBA in [0, 1].
pub fn parse_ramp(s: &str) -> Option<Vec<[f64; 4]>> {
    let stops: Option<Vec<[f64; 4]>> = s
        .split_whitespace()
        .map(|t| {
            let hex = t.strip_prefix('#')?;
            if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
            Some([c(0)?, c(2)?, c(4)?, if hex.len() == 8 { c(6)? } else { 1.0 }])
        })
        .collect();
    stops.filter(|s| !s.is_empty())
}

fn ramp_at(ramp: &[[f64; 4]], v: f64) -> [f64; 4] {
    if ramp.len() == 1 {
        return ramp[0];
    }
    let x = v.clamp(0.0, 1.0) * (ramp.len() - 1) as f64;
    let k = (x.floor() as usize).min(ramp.len() - 2);
    let f = x - k as f64;
    std::array::from_fn(|c| ramp[k][c] + (ramp[k + 1][c] - ramp[k][c]) * f)
}

/// The picture of a histogram: display-encoded straight RGBA per pixel, row 0 on top.
pub fn tonemap(hist: &[u64], width: u32, height: u32, channels: u32, fraction_bits: u32, t: &Tonemap) -> Vec<[f64; 4]> {
    let n_px = (width * height) as usize;
    let unit = (1u64 << fraction_bits) as f64;
    let n = |k: usize| hist[k] as f64 / unit;
    let reference = t.reference.unwrap_or_else(|| (0..n_px).map(n).fold(0.0, f64::max));
    (0..n_px)
        .map(|k| {
            let d = n(k);
            let v = if reference <= 0.0 {
                0.0
            } else if t.log {
                ((t.gain * d).ln_1p() / (t.gain * reference).ln_1p()).clamp(0.0, 1.0)
            } else {
                (d / reference).clamp(0.0, 1.0)
            };
            let v = v.powf(1.0 / t.gamma);
            let alpha = if t.alpha_density { v } else { 1.0 };
            if channels == 4 {
                let mean = |c: usize| if hist[k] == 0 { 0.0 } else { hist[c * n_px + k] as f64 / hist[k] as f64 };
                [mean(1) * v, mean(2) * v, mean(3) * v, alpha]
            } else {
                let c = ramp_at(&t.ramp, v);
                [c[0], c[1], c[2], c[3] * alpha]
            }
        })
        .collect()
}

/// The tonemap of a compute node: its `tonemap` child, or the defaults (SREP 67, "Defaults").
pub fn tonemap_of(c: &sr_model::model::Compute) -> Result<Tonemap, String> {
    use sr_model::model::{Alpha, ComputeChild, DensityTonemapScale};
    let Some(t) = c.children.iter().find_map(|k| match k {
        ComputeChild::Tonemap(t) => Some(t),
        _ => None,
    }) else {
        return Ok(Tonemap::default());
    };
    let reference = match t.reference.trim() {
        "max" => None,
        v => Some(v.parse::<f64>().ok().filter(|v| *v > 0.0).ok_or_else(|| format!("tonemap reference {v:?}"))?),
    };
    Ok(Tonemap {
        log: t.scale == DensityTonemapScale::Log,
        gain: t.gain.get(),
        gamma: t.gamma.get(),
        reference,
        ramp: parse_ramp(&t.ramp)
            .ok_or_else(|| format!("tonemap ramp {:?} is not a list of #RRGGBB[AA] colours", t.ramp))?,
        alpha_density: t.alpha == Alpha::Density,
    })
}

/// The `param` children of a compute node, in document order, as f32 (a value that is not a number reads 0).
pub fn params_of(c: &sr_model::model::Compute) -> Vec<f32> {
    c.children
        .iter()
        .filter_map(|k| match k {
            sr_model::model::ComputeChild::Param(p) => Some(p.value.trim().parse::<f32>().unwrap_or(0.0)),
            _ => None,
        })
        .collect()
}

/// Compiled programs by source, for a renderer.
#[derive(Default)]
pub struct Cache {
    programs: std::collections::HashMap<u64, Arc<Result<Program, String>>>,
}

impl Cache {
    /// The program of `user`, compiled once.
    pub fn get(&mut self, device: &wgpu::Device, user: &str) -> Arc<Result<Program, String>> {
        let key = sr_eval::rng::hash_str(user);
        self.programs.entry(key).or_insert_with(|| Arc::new(compile(device, user))).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kit srep-0067-compute-log-density, -reference, -gain-gamma, -linear, -empty-pixel: 1000, 100 and 10 points.
    #[test]
    fn the_tonemap_matches_the_kit() {
        let (w, h) = (4u32, 1u32);
        let hist = vec![1000u64, 100, 10, 0];
        let code = |px: &[[f64; 4]], k: usize| (px[k][0] * 255.0 * 10.0).round() / 10.0;
        let check = |t: Tonemap, want: [f64; 3]| {
            let px = tonemap(&hist, w, h, 1, 0, &t);
            for (k, v) in want.iter().enumerate() {
                assert!((code(&px, k) - v).abs() <= 0.1, "{t:?} pixel {k}: {} against {v}", code(&px, k));
            }
            assert_eq!(code(&px, 3), 0.0, "an empty pixel is ramp(0)");
        };
        check(Tonemap::default(), [255.0, 170.3, 88.5]);
        check(Tonemap { reference: Some(2000.0), ..Default::default() }, [231.8, 154.8, 80.4]);
        check(Tonemap { gain: 0.01, gamma: 2.2, ..Default::default() }, [255.0, 145.1, 58.9]);
        check(Tonemap { log: false, ..Default::default() }, [255.0, 25.5, 2.6]);
    }

    #[test]
    fn colour_planes_give_the_mean_colour_times_the_density() {
        // two pixels: 100 points of red and 50 of blue (fixed point, 8 fraction bits)
        let unit = 256u64;
        let (w, h) = (2u32, 1u32);
        let mut hist = vec![0u64; 8];
        hist[0] = 100 * unit;
        hist[1] = 50 * unit;
        hist[2] = 100 * unit; // red sum of pixel 0
        hist[2 + 2 + 2 + 1] = 50 * unit; // blue sum of pixel 1
        let px = tonemap(&hist, w, h, 4, 8, &Tonemap::default());
        assert_eq!(px[0], [1.0, 0.0, 0.0, 1.0]);
        let v = (51.0f64).ln() / (101.0f64).ln();
        assert!((px[1][2] - v).abs() < 1e-12 && px[1][0] == 0.0, "{:?}", px[1]);
    }

    #[test]
    fn sources_are_checked_without_a_device() {
        assert_eq!(check_source("fn sr_point(i: u32) { sr_accumulate(0, 0, 0u, 1u); }"), Ok(()));
        let bad = [
            "fn sr_point(i: u32) { this is not wgsl }",
            "fn other(i: u32) {}",
            "@group(0) @binding(9) var<uniform> mine: u32;\nfn sr_point(i: u32) {}",
            "fn sr_point(i: u32) -> u32 { return i; }",
        ];
        for b in bad {
            assert!(check_source(b).is_err_and(|e| e.starts_with("CMP11")), "{b}");
        }
        // a binding of another group is the author's, and a comment that mentions group 0 is not a binding
        assert_eq!(check_source("// @group(0) is the engine's\nfn sr_point(i: u32) {}"), Ok(()));
        assert!(!group_zero("@group(1) @binding(0) var<uniform> a: u32;"));
        assert!(!group_zero("@group(01)"));
    }

    #[test]
    fn the_seed_words_follow_mix64() {
        let (lo, hi) = seed_words(1, 0);
        let s = sr_eval::rng::mix64(1 ^ sr_eval::rng::mix64(0));
        assert_eq!((lo, hi), (s as u32, (s >> 32) as u32));
    }

    #[test]
    fn ramps_parse_and_interpolate_in_display_values() {
        let r = parse_ramp("#000000FF #808080 #FFFFFFFF").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(ramp_at(&r, 0.5), r[1]);
        assert!((ramp_at(&r, 0.25)[0] - 128.0 / 255.0 / 2.0).abs() < 1e-12);
        assert_eq!(parse_ramp("#GGG"), None);
        assert_eq!(parse_ramp(""), None);
    }
}
