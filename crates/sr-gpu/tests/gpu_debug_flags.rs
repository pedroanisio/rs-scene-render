//! The GPU debug and validation layers are off unless asked for.

#[test]
fn debug_layers_are_off_by_default_and_on_when_asked() {
    // one test in this binary: it changes the environment
    std::env::remove_var("SR_GPU_DEBUG");
    assert_eq!(sr_gpu::gpu::instance_flags(), wgpu::InstanceFlags::empty());
    std::env::set_var("SR_GPU_DEBUG", "0");
    assert_eq!(sr_gpu::gpu::instance_flags(), wgpu::InstanceFlags::empty());
    std::env::set_var("SR_GPU_DEBUG", "1");
    assert_eq!(sr_gpu::gpu::instance_flags(), wgpu::InstanceFlags::from_build_config());
}
