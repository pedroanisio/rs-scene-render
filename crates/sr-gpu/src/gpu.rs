//! Headless GPU device, and the choice of adapter behind it.
//!
//! Every adapter the enabled backends expose is ranked: discrete GPUs first, then integrated,
//! unknown and virtual ones, and software rasterisers (llvmpipe, SwiftShader, WARP) last, so a
//! machine with a GPU never renders on the CPU unless asked to. Between equal devices a native
//! backend (Vulkan, Metal, DirectX 12) beats OpenGL, whose submission costs more.
//! `SR_GPU_ADAPTER` names an adapter explicitly and `SR_GPU_BACKEND` restricts the backends.

use std::sync::Arc;

/// Why no device could be created.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter matched.
    #[error("no GPU adapter is available ({0}); install a Vulkan, Metal or DirectX 12 driver, or Mesa lavapipe for software rendering")]
    NoAdapter(String),
    /// No adapter has the requested name.
    #[error("no GPU adapter matches \"{wanted}\"; available: {}", if .available.is_empty() { "none".to_string() } else { .available.join(", ") })]
    NoMatch {
        /// The name asked for (`SR_GPU_ADAPTER`).
        wanted: String,
        /// Names of the adapters found.
        available: Vec<String>,
    },
    /// The adapter refused the device.
    #[error("the GPU adapter refused to create a device: {0}")]
    Device(String),
    /// The document has 3D objects, and no adapter that can run the 3D pass was chosen or found.
    #[error("the document has 3D objects, which need a Vulkan, Metal or DirectX 12 adapter, but {why}; \
             SR_GPU_BACKEND=vulkan selects Vulkan (on WSL2 that is Mesa lavapipe, a software renderer: complete but slow); \
             `scene-render gpus` lists the adapters found")]
    NoThreeD {
        /// What was wrong with the choice.
        why: String,
    },
}

/// Which adapters to consider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuOptions {
    /// Backends to enumerate.
    pub backends: wgpu::Backends,
    /// Case-insensitive part of the adapter name to use instead of the best-ranked one.
    pub adapter: Option<String>,
}

impl Default for GpuOptions {
    fn default() -> Self {
        GpuOptions { backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL, adapter: None }
    }
}

impl GpuOptions {
    /// Options from `SR_GPU_BACKEND` (`vulkan`, `metal`, `dx12`, `gl`) and `SR_GPU_ADAPTER`.
    pub fn from_env() -> Self {
        let backends = match std::env::var("SR_GPU_BACKEND").ok().as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("metal") => wgpu::Backends::METAL,
            Some("dx12") => wgpu::Backends::DX12,
            Some("gl") => wgpu::Backends::GL,
            _ => GpuOptions::default().backends,
        };
        let adapter = std::env::var("SR_GPU_ADAPTER").ok().filter(|s| !s.trim().is_empty());
        GpuOptions { backends, adapter }
    }
}

/// Names software rasterisers report; some report a device type other than `Cpu`
/// (llvmpipe through OpenGL, for one).
const SOFTWARE_NAMES: &[&str] = &["llvmpipe", "lavapipe", "softpipe", "swiftshader", "microsoft basic render driver"];

/// True when the adapter rasterises on the CPU.
pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    let name = info.name.to_lowercase();
    info.device_type == wgpu::DeviceType::Cpu || SOFTWARE_NAMES.iter().any(|s| name.contains(s))
}

/// Ranking key: lower is better.
fn rank(info: &wgpu::AdapterInfo) -> (u8, u8) {
    let class = if is_software(info) {
        4
    } else {
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::Other => 2,
            wgpu::DeviceType::VirtualGpu => 3,
            wgpu::DeviceType::Cpu => 4,
        }
    };
    (class, (info.backend == wgpu::Backend::Gl) as u8)
}

/// Index of the adapter to use among `found`: the first whose name contains `name`
/// (ignoring case) when one is given, otherwise the best-ranked, the earliest on a tie.
pub fn choose_adapter(found: &[wgpu::AdapterInfo], name: Option<&str>) -> Result<usize, GpuError> {
    if found.is_empty() {
        return Err(GpuError::NoAdapter("no adapter was found".into()));
    }
    match name {
        Some(want) => {
            let w = want.to_lowercase();
            found.iter().position(|a| a.name.to_lowercase().contains(&w)).ok_or_else(|| GpuError::NoMatch {
                wanted: want.to_string(),
                available: found.iter().map(|a| a.name.clone()).collect(),
            })
        }
        None => Ok((0..found.len()).min_by_key(|&i| rank(&found[i])).unwrap_or(0)),
    }
}

/// True when the 3D pass can run on the adapter.
pub fn can_run_3d(info: &wgpu::AdapterInfo) -> bool {
    // the 3D pass resolves multisampled depth, which OpenGL shaders cannot read
    info.backend != wgpu::Backend::Gl
}

/// Like [`choose_adapter`], for a document that does (`needs_3d`) or does not need the 3D pass.
///
/// A document with 3D objects takes the best-ranked adapter that can run them (so, on WSL2 where the GPU is
/// reachable only through OpenGL, the Vulkan one: lavapipe, slow but complete) instead of one that would drop the objects
/// without a word. When none can, or the adapter named cannot, that is [`GpuError::NoThreeD`].
pub fn choose_adapter_for(found: &[wgpu::AdapterInfo], name: Option<&str>, needs_3d: bool) -> Result<usize, GpuError> {
    if !needs_3d {
        return choose_adapter(found, name);
    }
    match name {
        Some(_) => {
            let i = choose_adapter(found, name)?;
            if can_run_3d(&found[i]) {
                Ok(i)
            } else {
                Err(GpuError::NoThreeD {
                    why: format!("the adapter asked for, {}, cannot run them", describe(&found[i])),
                })
            }
        }
        None => {
            if found.is_empty() {
                return Err(GpuError::NoAdapter("no adapter was found".into()));
            }
            (0..found.len()).filter(|&i| can_run_3d(&found[i])).min_by_key(|&i| rank(&found[i])).ok_or_else(|| {
                GpuError::NoThreeD {
                    why: format!(
                        "none of the adapters found can run them ({})",
                        found.iter().map(describe).collect::<Vec<_>>().join(", ")
                    ),
                }
            })
        }
    }
}

/// `Ok` when the adapter can run the 3D pass, else [`GpuError::NoThreeD`]: for a device chosen by the caller.
pub fn require_3d(info: &wgpu::AdapterInfo) -> Result<(), GpuError> {
    if can_run_3d(info) {
        Ok(())
    } else {
        Err(GpuError::NoThreeD { why: format!("the adapter in use, {}, cannot run them", describe(info)) })
    }
}

/// A warning for renders on a software adapter, `None` on a GPU.
pub fn software_warning(info: &wgpu::AdapterInfo) -> Option<String> {
    is_software(info).then(|| {
        format!(
            "rendering on a software adapter ({}, {:?}): expect renders many times slower than on a GPU; \
             `scene-render gpus` lists the adapters found and SR_GPU_ADAPTER picks one",
            info.name, info.backend
        )
    })
}

/// Why 3D objects are left out on this adapter and how to get them drawn; `None` where they draw.
pub fn three_d_warning(info: &wgpu::AdapterInfo) -> Option<String> {
    // the 3D pass resolves multisampled depth, which OpenGL shaders cannot read
    (info.backend == wgpu::Backend::Gl).then(|| {
        format!(
            "3D objects are not drawn: the adapter in use ({}) is an OpenGL one, which cannot run the 3D pass.              Set SR_GPU_BACKEND=vulkan to render them (on WSL2 that is Mesa lavapipe, a software renderer:              complete but slow), or name a Vulkan, Metal or DirectX 12 adapter with SR_GPU_ADAPTER;              `scene-render gpus` lists the adapters found",
            info.name
        )
    })
}

/// Index among `found` of the adapter `want` describes: the same name, backend, vendor, device,
/// type and bus address, not merely a name that contains it ("RTX 3080" is not "RTX 3080 Ti").
pub fn same_adapter(found: &[wgpu::AdapterInfo], want: &wgpu::AdapterInfo) -> Option<usize> {
    found.iter().position(|a| {
        a.name == want.name
            && a.backend == want.backend
            && a.vendor == want.vendor
            && a.device == want.device
            && a.device_type == want.device_type
            && a.device_pci_bus_id == want.device_pci_bus_id
    })
}

/// The limits a device is created with: the downlevel set every backend meets, at the
/// adapter's texture sizes, with the adapter's own buffer, binding and layer limits where
/// those are larger, so large frames, splat sets and shadow arrays fit where the hardware has room.
pub fn device_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    let base = wgpu::Limits { max_storage_buffers_per_shader_stage: 8, ..wgpu::Limits::downlevel_defaults() }
        .using_resolution(adapter.clone());
    wgpu::Limits {
        max_buffer_size: base.max_buffer_size.max(adapter.max_buffer_size),
        max_storage_buffer_binding_size: base
            .max_storage_buffer_binding_size
            .max(adapter.max_storage_buffer_binding_size),
        max_uniform_buffer_binding_size: base
            .max_uniform_buffer_binding_size
            .max(adapter.max_uniform_buffer_binding_size),
        max_texture_array_layers: base.max_texture_array_layers.max(adapter.max_texture_array_layers),
        ..base
    }
}

/// A one-line description: name, backend and device type.
pub fn describe(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
}

/// Every adapter the backends in `opts` expose, in enumeration order, and the index
/// `Gpu::with_options` would choose (or why none would be).
pub fn adapters(opts: &GpuOptions) -> (Vec<wgpu::AdapterInfo>, Result<usize, GpuError>) {
    let found = enumerate(opts.backends).into_iter().map(|a| a.get_info()).collect::<Vec<_>>();
    let chosen = choose_adapter(&found, opts.adapter.as_deref());
    (found, chosen)
}

/// Instance flags: the debug and validation layers the build config would turn on (debug builds) only when asked for with
/// `SR_GPU_DEBUG=1` or `--debug-gpu`. They name every object through the Vulkan loader, which crashed under concurrent
/// pipeline creation on llvmpipe, and a delivery does not need them.
pub fn instance_flags() -> wgpu::InstanceFlags {
    if debug_layers() {
        wgpu::InstanceFlags::from_build_config()
    } else {
        wgpu::InstanceFlags::empty()
    }
}

/// Whether the debug and validation layers are on (`SR_GPU_DEBUG` set to anything but empty or `0`; `--debug-gpu` sets it).
/// With them on, object naming reaches the Vulkan loader from every buffer and texture creation, which is not serialised:
/// a delivery therefore renders with one worker.
pub fn debug_layers() -> bool {
    std::env::var("SR_GPU_DEBUG").is_ok_and(|v| !v.is_empty() && v != "0")
}

static CREATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The process-wide lock held while a device or a pipeline is created. Several renderers are created at once by a parallel
/// delivery, and creating pipelines concurrently on a software adapter crashed the Vulkan loader; rendering takes no lock.
pub fn creation_lock() -> std::sync::MutexGuard<'static, ()> {
    CREATION.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn enumerate(backends: wgpu::Backends) -> Vec<wgpu::Adapter> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
    desc.flags = instance_flags();
    let instance = wgpu::Instance::new(desc);
    pollster::block_on(instance.enumerate_adapters(backends))
}

/// What decides how WSL2 can reach its GPU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WslProbe {
    /// `/dev/dxg`, the paravirtualised GPU of WSL2, exists.
    pub dxg: bool,
    /// Mesa's D3D12 Gallium driver (`d3d12_dri.so`) is installed.
    pub d3d12_driver: bool,
    /// The NVIDIA user-mode driver is mapped into WSL (`/usr/lib/wsl/lib`).
    pub nvidia: bool,
    /// `GALLIUM_DRIVER` as set by the user.
    pub gallium_driver: Option<String>,
    /// `MESA_D3D12_DEFAULT_ADAPTER_NAME` as set by the user.
    pub d3d12_adapter: Option<String>,
    /// `SR_GPU_WSL=off`.
    pub opt_out: bool,
}

impl WslProbe {
    /// Looks at this machine.
    pub fn detect() -> Self {
        let exists = |p: &str| std::path::Path::new(p).exists();
        let d3d12_driver =
            ["/usr/lib/x86_64-linux-gnu/dri", "/usr/lib/aarch64-linux-gnu/dri", "/usr/lib64/dri", "/usr/lib/dri"]
                .iter()
                .any(|d| exists(&format!("{d}/d3d12_dri.so")));
        let nvidia = std::fs::read_dir("/usr/lib/wsl/lib")
            .map(|r| r.flatten().any(|e| e.file_name().to_string_lossy().starts_with("libnvidia")))
            .unwrap_or(false);
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        WslProbe {
            dxg: exists("/dev/dxg"),
            d3d12_driver,
            nvidia,
            gallium_driver: var("GALLIUM_DRIVER"),
            d3d12_adapter: var("MESA_D3D12_DEFAULT_ADAPTER_NAME"),
            opt_out: var("SR_GPU_WSL")
                .is_some_and(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "false" | "no")),
        }
    }
}

/// Environment variables that route OpenGL through Mesa's D3D12 driver on WSL2, where no
/// hardware Vulkan driver exists and the Vulkan loader only finds lavapipe; on an NVIDIA
/// machine they also pick the NVIDIA GPU over an integrated one. Nothing the user set is changed.
pub fn wsl_environment(p: &WslProbe) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if !p.dxg || !p.d3d12_driver || p.opt_out || p.gallium_driver.is_some() {
        return v;
    }
    v.push(("GALLIUM_DRIVER".to_string(), "d3d12".to_string()));
    if p.nvidia && p.d3d12_adapter.is_none() {
        v.push(("MESA_D3D12_DEFAULT_ADAPTER_NAME".to_string(), "NVIDIA".to_string()));
    }
    v
}

/// Applies [`wsl_environment`] to this process. Mesa reads these variables when a GL
/// display is first opened, so call it at program start, before any other thread runs.
pub fn prepare_environment() -> Vec<(String, String)> {
    let set = wsl_environment(&WslProbe::detect());
    for (k, v) in &set {
        std::env::set_var(k, v);
    }
    set
}

/// A device and queue shared by renderers.
#[derive(Debug, Clone)]
pub struct Gpu {
    /// Device.
    pub device: Arc<wgpu::Device>,
    /// Queue.
    pub queue: Arc<wgpu::Queue>,
    /// Adapter description.
    pub info: wgpu::AdapterInfo,
    /// The device has timestamp queries (GPU time of passes can be measured).
    pub timestamps: bool,
    /// The widest float format the device renders to that holds 3D view depth and reflectance
    /// (32-bit float where it can; OpenGL may not render to RGBA32F).
    pub gbuffer_depth: wgpu::TextureFormat,
    /// The single-channel float format the device renders to for depth and blur-size maps
    /// (R32Float where it can, else R16Float).
    pub scalar_target: wgpu::TextureFormat,
}

/// What the renderer's shaders need of a device, tried on a minimal pipeline: a fragment shader that reads a runtime-sized
/// storage buffer. An OpenGL adapter older than 4.3 (the one a WSL2 host without a D3D12 driver falls back to) opens a
/// device and then fails the first pipeline the renderer creates, which wgpu treats as fatal; made inside an error scope it
/// is an error here, so that the adapter is refused with its reason, as a caller (a test of the OpenGL backend, the
/// adapter choice) can act on.
fn probe(device: &wgpu::Device) -> Result<(), GpuError> {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene-render-probe"),
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var<storage, read> data: array<u32>;\n\
             @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n\
             @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(f32(data[0]) + f32(arrayLength(&data))); }"
                .into(),
        ),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene-render-probe"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scene-render-probe"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene-render-probe"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    // the scopes are a stack: both are popped, the one pushed last first, whatever the first one caught
    let from_validation = pollster::block_on(validation.pop());
    let from_internal = pollster::block_on(internal.pop());
    match from_validation.or(from_internal) {
        None => Ok(()),
        Some(e) => Err(GpuError::Device(format!("the device cannot run the renderer's shaders: {e}"))),
    }
}

impl Gpu {
    /// Opens the best adapter without a window, following `SR_GPU_BACKEND` and `SR_GPU_ADAPTER`.
    pub fn new() -> Result<Gpu, GpuError> {
        Gpu::with_options(&GpuOptions::from_env())
    }

    /// Like [`Gpu::new`], for a document that does or does not need the 3D pass
    /// (see [`choose_adapter_for`]).
    pub fn new_for(needs_3d: bool) -> Result<Gpu, GpuError> {
        Gpu::with_options_for(&GpuOptions::from_env(), needs_3d)
    }

    /// Opens the adapter `opts` selects.
    pub fn with_options(opts: &GpuOptions) -> Result<Gpu, GpuError> {
        Gpu::with_options_for(opts, false)
    }

    /// Opens the adapter `opts` selects for a document that does or does not need the 3D pass.
    pub fn with_options_for(opts: &GpuOptions, needs_3d: bool) -> Result<Gpu, GpuError> {
        let mut found = enumerate(opts.backends);
        let infos = found.iter().map(|a| a.get_info()).collect::<Vec<_>>();
        let i = choose_adapter_for(&infos, opts.adapter.as_deref(), needs_3d).map_err(|e| match e {
            GpuError::NoAdapter(_) => GpuError::NoAdapter(format!("none on {:?}", opts.backends)),
            e => e,
        })?;
        Gpu::open(found.swap_remove(i))
    }

    fn open(adapter: wgpu::Adapter) -> Result<Gpu, GpuError> {
        let info = adapter.get_info();
        let limits = device_limits(&adapter.limits());
        // timestamp queries, where the adapter has them, so renders can report GPU time
        let features =
            adapter.features() & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
        let _creation = creation_lock();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("scene-render"),
            required_features: features,
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        probe(&device)?;
        let timestamps = features.contains(wgpu::Features::TIMESTAMP_QUERY);
        let renderable = |f: wgpu::TextureFormat| {
            adapter.get_texture_format_features(f).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        };
        let gbuffer_depth = [wgpu::TextureFormat::Rgba32Float, wgpu::TextureFormat::Rg32Float]
            .into_iter()
            .find(|f| renderable(*f))
            .unwrap_or(wgpu::TextureFormat::Rgba16Float);
        let scalar_target = if renderable(wgpu::TextureFormat::R32Float) {
            wgpu::TextureFormat::R32Float
        } else {
            wgpu::TextureFormat::R16Float
        };
        Ok(Gpu { device: Arc::new(device), queue: Arc::new(queue), info, timestamps, gbuffer_depth, scalar_target })
    }

    /// A new device on the same adapter, for work that needs a device of its own.
    pub fn open_like(&self) -> Result<Gpu, GpuError> {
        let mut found = enumerate(wgpu::Backends::from(self.info.backend));
        let infos = found.iter().map(|a| a.get_info()).collect::<Vec<_>>();
        let i = same_adapter(&infos, &self.info).ok_or_else(|| GpuError::NoMatch {
            wanted: describe(&self.info),
            available: infos.iter().map(describe).collect(),
        })?;
        Gpu::open(found.swap_remove(i))
    }

    /// True when this device rasterises on the CPU.
    pub fn is_software(&self) -> bool {
        is_software(&self.info)
    }

    /// Blocks until submitted work completes.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Waits for one submission (and everything before it), leaving later submissions in flight.
    pub fn wait_for(&self, idx: wgpu::SubmissionIndex) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None });
    }
}
