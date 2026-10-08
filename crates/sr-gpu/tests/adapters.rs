//! The adapters a renderer can open, how it behaves on each, hardening against bad input, and renderers used at the same time.

mod common;

#[path = "adapters/adapters.rs"]
mod adapters;
#[path = "adapters/concurrent_renderers.rs"]
mod concurrent_renderers;
#[path = "adapters/hardening.rs"]
mod hardening;
