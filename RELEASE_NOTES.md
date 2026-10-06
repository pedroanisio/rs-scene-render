# scene-render v0.1.4

Reliability fixes for incremental rendering, volume loading, media metadata, delivery and data parsing.

- Delivery URLs escape filenames containing spaces, punctuation and Unicode.
- Adjacent baked volume frames share decoded payloads; incremental rendering tracks baked payload changes and missing files.
- Spherical MP4 metadata rejects invalid extended sizes, removes obsolete projection metadata and preserves permissions through secure temporary files.
- AV1 two-pass encoding verifies SVT-AV1 statistics support and falls back to libaom-av1 when the installed FFmpeg build ignores pass flags. Builds with neither implementation report an error.
- HEIF colour detection respects the primary image property associations.
- PMTiles rejects oversized sections before allocating memory.
- CSV and TSV preserve quoted empty records while skipping blank physical lines.

The reliability fixes include regressions that failed before their implementation changes and passed afterward. Encoder checks include both modern and older FFmpeg builds. The release workflow runs workspace tests, Clippy, portability checks, evidence and performance checks before packaging and smoke-testing the validated binary. Numerical tests use optimized builds with debug assertions and overflow checks enabled. The smoke and fuzz checks reuse the same binaries built by the evidence job.

This release also includes the mesh, path tracing, simulation and cinematic rendering changes accumulated since v0.1.2. These features require scene-specific visual review; this release does not establish production readiness for every cinematic workflow.

Download the Linux x86-64 archive and checksum. Run `sha256sum -c scene-render-v0.1.4-linux-x86_64.tar.gz.sha256`, extract the archive, then run `./scene-render --help`. The archive includes the binary, README, license and source commit identifier.

The binary requires Linux x86-64 with glibc 2.39 or newer, a Vulkan driver, FFmpeg and ffprobe 5.1 or newer, and suitable fonts. Mesa software Vulkan is supported. Ubuntu 24.04 dependencies: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.

## Transmission rendering

Image changes in the path tracer for scenes with transmissive materials (`transmission` above 0); scenes without one render the same bytes at the same speed.

- Surfaces seen through water or glass now receive the analytic lights: the shadow ray is refracted at the interface instead of being blocked by it, with Fresnel transmittance and absorption. Before, a floor under water got no direct light at all. In `examples/cinematic-impact/impact.scene.xml` the mean brightness at t = 2.0 goes from 0.00663 to 0.00909 (plume and ejecta removed for the measurement). Shadows that glass casts on surfaces in air stay black.
- `attenuationColor` and `attenuationDistance` now take effect in path-traced renders (they already did in raster): after one attenuation distance the light left is the colour, per channel.
- Glass or water over nothing now shows the visible dome where it showed black, including the dome's reflection on water at the horizon.
- The ocean's default spray is transmissive, so frames with whitewater change slightly around the spray (a UHD hero frame: 394 pixels, at most 28 code values).
- Limits: a camera that starts under the water does not see the sun on the floor, caustics are not produced, and very steep waves can leave samples dark. See the cinematic impact SREP, "Light through water and glass".
