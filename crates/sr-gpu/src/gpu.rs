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

fn enumerate(backends: wgpu::Backends) -> Vec<wgpu::Adapter> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
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
}

impl Gpu {
    /// Opens the best adapter without a window, following `SR_GPU_BACKEND` and `SR_GPU_ADAPTER`.
    pub fn new() -> Result<Gpu, GpuError> {
        Gpu::with_options(&GpuOptions::from_env())
    }

    /// Opens the adapter `opts` selects.
    pub fn with_options(opts: &GpuOptions) -> Result<Gpu, GpuError> {
        let mut found = enumerate(opts.backends);
        let infos = found.iter().map(|a| a.get_info()).collect::<Vec<_>>();
        let i = choose_adapter(&infos, opts.adapter.as_deref()).map_err(|e| match e {
            GpuError::NoAdapter(_) => GpuError::NoAdapter(format!("none on {:?}", opts.backends)),
            e => e,
        })?;
        let adapter = found.swap_remove(i);
        let info = adapter.get_info();
        let limits = wgpu::Limits { max_storage_buffers_per_shader_stage: 8, ..wgpu::Limits::downlevel_defaults() }
            .using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("scene-render"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        Ok(Gpu { device: Arc::new(device), queue: Arc::new(queue), info })
    }

    /// A new device on the same adapter, for work that needs a device of its own.
    pub fn open_like(&self) -> Result<Gpu, GpuError> {
        let backends = wgpu::Backends::from(self.info.backend);
        Gpu::with_options(&GpuOptions { backends, adapter: Some(self.info.name.clone()) })
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
