//! GPU availability is mandatory in renderer CI and optional on developer machines.
use std::sync::OnceLock;
pub fn gpu() -> Option<sr_gpu::Gpu> {
    static GPU: OnceLock<Option<sr_gpu::Gpu>> = OnceLock::new();
    GPU.get_or_init(|| match sr_gpu::Gpu::new() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "required GPU unavailable: {e}");
            eprintln!("skipping GPU tests: {e}");
            None
        }
    })
    .clone()
}
