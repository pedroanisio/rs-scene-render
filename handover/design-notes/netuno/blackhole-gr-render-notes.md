---
name: blackhole-gr-render-notes
description: "Mercurio's black-hole geodesic pass in sr-gpu: where it lives, how it was verified, measured numbers, and the engine findings handed to Saturno (2026-10-06)"
metadata:
  node_type: memory
  type: project
  originSessionId: 8e0c69e6-b97d-44af-abdc-223a64772d14
---

Schwarzschild black hole (G = c = 1) rendered by geodesic tracing, merged into local main as e19e541 (feat/blackhole-gr, my commits 99f4d2e, 74ebc2d, 7df721d, e4b1f85 on top of Saturno's schema b385cb8 and Netuno's `sr_sim::gr` 28799fd).

- Code: `crates/sr-gpu/src/geodesic.rs` + `geodesic.wgsl` (fragment pass, RK4 in phi on u''=-u+3Mu^2, step 0.02, 4096 steps, crossings at phi0+k*pi, opaque first-hit disk, procedural stars); engine hook `Scene3.geodesic` in `three.rs` and `geodesic_scene` in `render_three.rs`; `blackHole`/`accretionDisk` are in `sr_eval::THREE_D_DRAWN`.
- g = sqrt(1-3M/r)/(1+Omega*b*h), h=(e1 x e2).axis of the TRACED path (camera outward); the physical photon runs the other way.
- Measured: shadow 39.99 px for 40; agreement with the f64 CPU image 5760/5760 pixels, worst 1.2e-5; f32 reference 1e-6 not reached everywhere (GPU contraction): 2528/3186 within 1e-6; cost 4.3 ms at 1280x800 16 spp, 17 ms at 64 spp on the RTX 6000 Ada.
- Films: `/home/pals/renders/cinematic-impact/blackhole-gr/` (HD 64 spp, pixel art 16 colours); pattern `timeScale` 10 winds the clumps into fine rings in seconds, 4 looks right.
- Engine findings given to Saturno: particles3D dt=1/24 fails at frame times that coincide with a birth (his fix: dt is 1/24 + 3e-17); emitterMesh emits (objects3d identical) but no particle appears where the mesh is.
- Known environment failure: cli test `a_flat_document_still_renders_on_opengl` ("Parent device is lost" at GL device creation) fails on this machine with any binary.

**Why:** a later session continuing the black-hole or review work needs the layout and the honest numbers.

**How to apply:** read the doc comment of `geodesic.wgsl` and the SREP section on the black hole before changing the pass; keep the sign convention above.

Related: [[render-verification-notes]], [[shared-machine-agents]]
