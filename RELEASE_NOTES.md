# scene-render v0.1.1

Linux x86-64 update with rendering performance improvements, stronger visual checks and fixes for portability, evidence reports and accessibility.

- Executable discovery supports Windows `PATHEXT` and preserves Unix execute-permission checks. Portability CI compiles all targets and runs provider unit tests.
- Evidence clips use absolute output paths and must exist before the report links them. Renderer commands found through `PATH` can be hashed and reported.
- Text matching its backdrop reports 1:1 contrast when rendered coverage is present, including isolated groups, overlays and captions. Transparent, covered, offscreen and empty text remain excluded.
- Image-size validation accounts for EXIF orientation, and benchmark renders propagate errors during warm-up and measurement.
- Rendering selects hardware adapters ahead of software adapters, bounds and reuses effect targets, and reduces redundant passes. Particle bursts follow composition time.
- Missing goldens fail instead of being accepted automatically. CI runs evidence probes and complete review clips independently from the test, performance and oracle jobs.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.1-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`.

Validation before the version bump: 615 Rust tests passed, with 4 ignored, and 6 Python tests passed. Formatting, Clippy and golden comparisons passed. All 24 evidence cases matched their expected outcomes (22 native, 2 degraded); both review clips were verified as 3-second, 640×360, 24 fps videos with valid report links. The release workflow smoke-tests validation, PNG rendering and a two-frame FFV1 encode using the packaged binary.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.
