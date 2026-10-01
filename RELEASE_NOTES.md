# scene-render v0.1.2

Linux x86-64 update with quality tiers, incremental rendering, watch mode, GPU timing and fixes for rendering correctness and accessibility.

- `watch` reloads changed image files, and `--changed-only` includes active transition types, parameters and static attributes in frame fingerprints.
- Incremental renders retain strict validation: frames with unsupported content are rendered again so shader failures cannot disappear on a subsequent run.
- Effect caches follow animated generator paints, including `paint2` and paint references nested inside assets.
- Wide strokes, outlines and matte-choke preserve partial alpha. Wide gathers can cost more at large radii on translucent inputs; their cost grows with radius.
- Caption burn-in and authored text layers have distinct contrast identities. A layer named `captions` receives normal contrast checks, and text covered by later layers is measured in the final picture.
- Linux CI installs the HEVC decoder required by HEIC fixtures and checks decoding before building.
- Quality tiers, incremental rendering, watch mode, GPU timing and performance comparisons are included from the changes since v0.1.1.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.2-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding also requires `libheif-examples libheif-plugin-libde265`.

Validation before the version bump: 637 Rust tests passed, with 4 benchmarks ignored, and 14 Python tests passed. Seven new regression tests failed before their fixes and passed afterward. Formatting, Clippy and golden comparisons passed without replacing references. The `still-formats` evidence case passed locally, including HEIC decoding. The release workflow smoke-tests validation, PNG rendering and a two-frame FFV1 encode using the release binary.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.
