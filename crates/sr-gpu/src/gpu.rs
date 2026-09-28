//! Headless GPU device.

use std::sync::Arc;

/// Why no device could be created.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter matched.
    #[error("no GPU adapter is available ({0}); install a Vulkan, Metal or DirectX 12 driver, or Mesa lavapipe for software rendering")]
    NoAdapter(String),
    /// The adapter refused the device.
    #[error("the GPU adapter refused to create a device: {0}")]
    Device(String),
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
    /// Opens the best available adapter without a window. `SR_GPU_BACKEND`
    /// (`vulkan`, `metal`, `dx12`, `gl`) restricts the backends tried.
    pub fn new() -> Result<Gpu, GpuError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = match std::env::var("SR_GPU_BACKEND").ok().as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("metal") => wgpu::Backends::METAL,
            Some("dx12") => wgpu::Backends::DX12,
            Some("gl") => wgpu::Backends::GL,
            _ => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        };
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
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

    /// Blocks until submitted work completes.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}
