# scene-render v0.3.0

This release adds scene document version 1.6 and integrates the following rendering features:

- Build-time and stepping WebAssembly programs with deterministic host functions, fuel limits and pinned output hashes.
- Compute histograms, density tonemapping, iterated effects, error diffusion and segmented sorting.
- Node-local shader stepping, prewarming and checkpoints.
- Parametric paths, parametric surfaces and heightfields.
- Shader content rectangles, supersampling, per-node edge blending and optional float32 working textures.
- Float32 working precision is preserved through CPU effects and checkpoint restores, with format conversion for output joins.
- Upright flock sprites through `orientToVelocity="false"`.
- Faster bounded texture-coordinate conversion in the depth-of-field shader, checked against the original shader.

The schema retains the vendored sr-core 1.5.0 base with the local 1.6 extensions documented in `schema/UPSTREAM`. The compute histogram carry tests use llvmpipe; discrete-GPU carry remains unverified.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.3.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.
