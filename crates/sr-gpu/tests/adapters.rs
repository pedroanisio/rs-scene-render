//! Adapter choice without a GPU: hardware before software, an explicit name wins, software
//! is recognised by name when the driver reports another type, and WSL2 is set up to reach
//! its GPU through Mesa's D3D12 driver.

use sr_gpu::gpu::{choose_adapter, is_software, software_warning, wsl_environment, WslProbe};
use sr_gpu::GpuError;
use wgpu::{AdapterInfo, Backend, DeviceType};

fn info(name: &str, t: DeviceType, b: Backend) -> AdapterInfo {
    AdapterInfo { name: name.into(), ..AdapterInfo::new(t, b) }
}

fn lavapipe() -> AdapterInfo {
    info("llvmpipe (LLVM 19.1.7, 256 bits)", DeviceType::Cpu, Backend::Vulkan)
}

#[test]
fn hardware_is_chosen_before_software() {
    // what WSL2 offers once the D3D12 driver is enabled: lavapipe first, the GPU behind GL second
    let found = [lavapipe(), info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl)];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1);
}

#[test]
fn device_classes_rank_discrete_integrated_other_virtual_then_software() {
    let found = [
        lavapipe(),
        info("virtio", DeviceType::VirtualGpu, Backend::Vulkan),
        info("unknown", DeviceType::Other, Backend::Vulkan),
        info("Radeon 780M", DeviceType::IntegratedGpu, Backend::Vulkan),
        info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan),
    ];
    assert_eq!(choose_adapter(&found, None).unwrap(), 4);
    assert_eq!(choose_adapter(&found[..4], None).unwrap(), 3);
    assert_eq!(choose_adapter(&found[..3], None).unwrap(), 2);
    assert_eq!(choose_adapter(&found[..2], None).unwrap(), 1);
    assert_eq!(choose_adapter(&found[..1], None).unwrap(), 0);
}

#[test]
fn software_is_recognised_by_name_when_the_driver_reports_another_type() {
    for name in [
        "llvmpipe (LLVM 19.1.7, 256 bits)",
        "lavapipe",
        "softpipe",
        "SwiftShader Device (Subzero)",
        "Microsoft Basic Render Driver",
    ] {
        assert!(is_software(&info(name, DeviceType::Other, Backend::Gl)), "{name}");
    }
    assert!(is_software(&info("anything", DeviceType::Cpu, Backend::Vulkan)));
    assert!(!is_software(&info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl)));
    // GL on llvmpipe is software even when it reports itself as an unknown device
    let found = [info("llvmpipe (LLVM 19.1.7, 256 bits)", DeviceType::Other, Backend::Gl), lavapipe()];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1, "a native backend wins between two software devices");
}

#[test]
fn a_native_backend_wins_a_tie_with_gl() {
    let found =
        [info("RTX (GL)", DeviceType::DiscreteGpu, Backend::Gl), info("RTX", DeviceType::DiscreteGpu, Backend::Vulkan)];
    assert_eq!(choose_adapter(&found, None).unwrap(), 1);
}

#[test]
fn a_named_adapter_is_chosen_even_when_it_is_software() {
    let found = [info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan), lavapipe()];
    assert_eq!(choose_adapter(&found, Some("LLVMpipe")).unwrap(), 1);
    assert_eq!(choose_adapter(&found, Some("rtx")).unwrap(), 0);
}

#[test]
fn an_unknown_name_lists_what_exists() {
    let found = [info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan), lavapipe()];
    match choose_adapter(&found, Some("radeon")) {
        Err(e @ GpuError::NoMatch { .. }) => {
            let msg = e.to_string();
            assert!(msg.contains("radeon") && msg.contains("RTX 4050") && msg.contains("llvmpipe"), "{msg}");
        }
        other => panic!("expected NoMatch, got {other:?}"),
    }
}

#[test]
fn nothing_found_is_an_error() {
    assert!(matches!(choose_adapter(&[], None), Err(GpuError::NoAdapter(_))));
}

#[test]
fn software_warning_names_the_adapter_and_the_way_out() {
    let w = software_warning(&lavapipe()).expect("software adapters warn");
    assert!(w.contains("llvmpipe") && w.contains("scene-render gpus"), "{w}");
    assert!(software_warning(&info("RTX 4050", DeviceType::DiscreteGpu, Backend::Vulkan)).is_none());
}

fn wsl() -> WslProbe {
    WslProbe { dxg: true, d3d12_driver: true, nvidia: true, gallium_driver: None, d3d12_adapter: None, opt_out: false }
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn wsl2_routes_gl_through_d3d12_to_the_nvidia_gpu() {
    assert_eq!(
        wsl_environment(&wsl()),
        pairs(&[("GALLIUM_DRIVER", "d3d12"), ("MESA_D3D12_DEFAULT_ADAPTER_NAME", "NVIDIA")])
    );
}

#[test]
fn wsl2_without_nvidia_keeps_the_default_d3d12_adapter() {
    assert_eq!(wsl_environment(&WslProbe { nvidia: false, ..wsl() }), pairs(&[("GALLIUM_DRIVER", "d3d12")]));
}

#[test]
fn settings_the_user_made_are_never_overridden() {
    assert_eq!(wsl_environment(&WslProbe { gallium_driver: Some("llvmpipe".into()), ..wsl() }), pairs(&[]));
    assert_eq!(
        wsl_environment(&WslProbe { d3d12_adapter: Some("AMD".into()), ..wsl() }),
        pairs(&[("GALLIUM_DRIVER", "d3d12")])
    );
}

#[test]
fn nothing_changes_outside_wsl2_without_the_driver_or_when_opted_out() {
    assert_eq!(wsl_environment(&WslProbe { dxg: false, ..wsl() }), pairs(&[]));
    assert_eq!(wsl_environment(&WslProbe { d3d12_driver: false, ..wsl() }), pairs(&[]));
    assert_eq!(wsl_environment(&WslProbe { opt_out: true, ..wsl() }), pairs(&[]));
}

#[test]
fn a_device_reopens_on_exactly_its_own_adapter() {
    use sr_gpu::gpu::same_adapter;
    let gpu = |name: &str, device: u32, b: Backend| AdapterInfo { device, ..info(name, DeviceType::DiscreteGpu, b) };
    let found = [
        gpu("NVIDIA GeForce RTX 3080 Ti", 0x2208, Backend::Vulkan),
        gpu("NVIDIA GeForce RTX 3080", 0x2206, Backend::Dx12),
        gpu("NVIDIA GeForce RTX 3080", 0x2206, Backend::Vulkan),
        gpu("NVIDIA GeForce RTX 3080", 0x2216, Backend::Vulkan),
    ];
    for (k, a) in found.iter().enumerate() {
        assert_eq!(same_adapter(&found, a), Some(k), "{}", a.name);
    }
    assert_eq!(same_adapter(&found, &gpu("NVIDIA GeForce RTX 308", 0x2206, Backend::Vulkan)), None);
    assert_eq!(same_adapter(&found, &gpu("nvidia geforce rtx 3080", 0x2206, Backend::Vulkan)), None);
}

#[test]
fn an_adapter_that_cannot_draw_3d_says_how_to_get_one_that_can() {
    use sr_gpu::gpu::three_d_warning;
    let gl = info("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)", DeviceType::Other, Backend::Gl);
    let w = three_d_warning(&gl).expect("OpenGL adapters do not draw 3D");
    assert!(w.contains("SR_GPU_BACKEND=vulkan"), "{w}");
    assert!(w.contains("D3D12 (NVIDIA GeForce RTX 4050 Laptop GPU)") && w.contains("not drawn"), "{w}");
    assert!(w.contains("scene-render gpus"), "{w}");
    assert!(three_d_warning(&lavapipe()).is_none());
    assert!(three_d_warning(&info("RTX 4050", DeviceType::DiscreteGpu, Backend::Dx12)).is_none());
}
