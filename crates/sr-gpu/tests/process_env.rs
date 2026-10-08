//! Tests that change the process's environment (a program found by SR_FFMPEG, SR_GPU_DEBUG): they run alone in their own process.

mod common;

#[path = "process_env/decode_warnings.rs"]
mod decode_warnings;
#[path = "process_env/gpu_debug_flags.rs"]
mod gpu_debug_flags;
