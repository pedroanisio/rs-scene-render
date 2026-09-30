# scene-render v0.1.0

First tagged Linux x86-64 build of rs-scene-render.

This release includes fixes for parallel delivery diagnostics, marker-relative remap timing, overlay and output-caption contrast checks, and required-caption validation. The preceding delivery fixes cover strict template diagnostics, poster composition, repeated captions through remaps, and overlay asset representations.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.0-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`.

Validation: 120 delivery, CLI, and evaluator tests passed locally, along with formatting, Clippy, and release hygiene checks. The release workflow smoke-tests validation, PNG rendering, and a two-frame FFV1 encode using the packaged binary.

Known CI limitation: the existing workspace CI failed its GPU golden-image comparison before this release work (frame 0 maximum delta E 1.1755). The full workspace suite is not claimed to pass; the release build and its smoke tests are reported separately.
