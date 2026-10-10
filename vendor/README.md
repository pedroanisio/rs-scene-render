# Vendored dependencies

## Disclaimer

This work is subject to the methodological caveats and commitments described in [@DISCLAIMER.md](../DISCLAIMER.md).
> No statement or premise not backed by a real logical definition or verifiable reference should be taken for granted.

Crates kept in the repository because the build needs a change that their released versions do not have.
They are used through `[patch.crates-io]` in the workspace `Cargo.toml` and are not workspace members:
formatting, lints and tests of the workspace do not apply to them, and their own formatting is left as
released so they can be compared with upstream.

## gpu-allocator 0.28.0

wgpu's Vulkan backend allocates all device memory through this crate. Upstream: Traverse Research,
<https://github.com/Traverse-Research/gpu-allocator>, MIT OR Apache-2.0 (both licence files are kept).

**The change** is in `src/vulkan/mod.rs`, `Allocator::allocate`: for `MemoryLocation::CpuToGpu` (mappable
buffers and the staging buffers behind `create_buffer_init`, `Queue::write_buffer` and `Queue::write_texture`)
the selector in `src/vulkan/memory_type.rs` explicitly prefers compatible
`HOST_VISIBLE | HOST_COHERENT` types without `DEVICE_LOCAL`. Device-local host-visible memory remains
a fallback for UMA, resource compatibility, or a failed host allocation. A retry excludes the failed
type. Behavioral tests compile the same policy module from `tools/tests/test_vendor.py`.

**Why.** On NVIDIA the type that is both host-visible and device-local is device memory mapped through the
BAR1 window, 256 MiB on the card this was measured on, shared by every process using the GPU. Each device
takes a 64 MiB block of it with its first upload, so three devices fill it. With the window at its limit and
other processes starting or stopping, the driver can hand a process a block that it then cannot map there
(the kernel logs `NVRM: dmaAllocMapping_GM107: can't alloc VA space for mapping`), and every write to that
block is about a thousand times slower until the process exits: a render at 2 s per frame instead of 50 ms,
with the CPU at 100 % in `memset` and `memcpy` and the GPU idle. A block that does not fit at all fails
cleanly and falls back to host memory, which is why the fault is intermittent. With upload memory in
ordinary host memory the window is not used and the state cannot occur. Measured cost for a process that
is alone: about 3 ms more per 30 MiB uploaded.

**When wgpu is upgraded** and asks for another gpu-allocator version, vendor that version and carry the
change over, or drop this directory if upstream has an option for it by then. `tools/tests/test_vendor.py`
fails if the patch entry or the change is missing.

## wasmtime-internal-cranelift 41.0.4

Wasmtime's compiler integration, from the Bytecode Alliance's
<https://github.com/bytecodealliance/wasmtime>, under Apache-2.0 WITH LLVM-exception
(the licence is kept in the crate and in `THIRD-PARTY-NOTICES.md`).

The single source change is in `src/func_environ.rs`: `fuel_consumed` starts at zero
instead of one. The program runtime charges executed WebAssembly instructions;
Wasmtime's additional charge for every function entry is not part of that contract.
Calls still cost one unit, including recursive calls. All instruction costs and fuel
checks remain as released upstream.

`crates/sr-wasm/tests/runtime.rs` checks exact budgets for straight-line code, loops,
nested functions and host calls. Preserve those boundaries when upgrading Wasmtime,
and drop the patch if upstream provides a configurable function-entry cost.
