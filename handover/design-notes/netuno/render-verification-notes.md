---
name: render-verification-notes
description: "Known environment failures, commit hygiene rules and shader-exactness pitfalls for rs-scene-render GPU work (observed 2026-10-04)"
metadata:
  node_type: memory
  type: reference
  originSessionId: 8e0c69e6-b97d-44af-abdc-223a64772d14
  modified: 2026-10-04T06:00:40.895Z
---

Observed while working the render front (sr-gpu, alias Mercurio) on 2026-10-04:

- **Software adapter is unreliable here.** `SR_GPU_ADAPTER=llvmpipe` (LLVM 15) darkens long volume marches in exact quarters (x0.75, x0.5) and fails these tests on the base commit too: `pathtrace_instances`, `volume::advected_cache_documents...`, 8 sr-deliver tests (access, overlay, segments, segments_parallel), and flakes `raster::...solid_fill_specialization...` under load. NVIDIA passes all. Always say which adapter each test ran on; write new volume tests with short marches so they pass on both.
- **`backends` test never finishes** (>30 min on NVIDIA): skip it, note as not run. Run test binaries directly from `target/release/deps` through `sr-gpu`, not `cargo test` under the queue.
- **Commit hygiene**: before every commit run `python3 tools/check_release_hygiene.py --staged` and `--commit-msg FILE`: no absolute local paths, `.claude/`, task or phase labels, plan references in code, tests or messages.
- **Shader "pixel-identical" claims are fragile**: even same-arithmetic reshuffles of WGSL change ~0.2% of pixels by 1 code value (compiler FMA contraction). Changes that kept the PNG hash identical only touched the voxel lookup or loop structure around untouched arithmetic. The probe's `frame_png_sha256` is the check.
- Probe: `tools/probe_render.py SCENE --times ... --size WxH`; bake a frozen plume with `scene-render bake-volume` to avoid the 35 s smoke sim per measurement.

Related: [[shared-machine-agents]]

- **Transmission in the path tracer (measured 2026-10-04, light-through-water analysis)**: shadow rays treat any shadow-casting transmissive surface as opaque, so a sun-lit seabed under transmissive water gets no direct sun (dome light still arrives by BSDF sampling and matches brute force at 57 dB). Radiance is not scaled by 1/eta^2 at refractions (balanced camera-in/light-out paths cancel), so any shadow-ray transmission through water must carry T_fresnel/eta^2 or it comes out about 1.8x too bright. Glass seen with nothing behind it picks the 2D backdrop (black) instead of the visible dome. Use 2D scene ids that are unique across the whole document (material and object ids cannot share a name). Camera pitch: negative looks down, y negative is up.

- 2026-10-07: the sr-gpu LIB test binary deadlocks in parallel mode on the NVIDIA host (twice in one day: all threads in futex_wait, several Vulkan devices open at once; passes 48/48 in 3 s with `--test-threads=1`). Root cause: seven unit tests each opened their own device; several devices used at once by threads of one process deadlock inside the NVIDIA Vulkan driver (creation was serialised, use was not). Fixed in d04584d (shared `test_gpu()` + a guard against a second device in unit tests); the template runs the lib tests in parallel again. Production still opens one device per delivery worker (`Gpu::open_like`): same risk class, open item.
