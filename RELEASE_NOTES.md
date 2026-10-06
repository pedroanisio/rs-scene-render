# scene-render v0.1.3

Reliability fixes for incremental rendering, volume loading, media metadata, delivery and data parsing.

- Delivery URLs escape filenames containing spaces, punctuation and Unicode.
- Adjacent baked volume frames share decoded payloads; incremental rendering tracks baked payload changes and missing files.
- Spherical MP4 metadata rejects invalid extended sizes, removes obsolete projection metadata and preserves permissions through secure temporary files.
- SVT-AV1 encoding supplies two-pass statistics options.
- HEIF colour detection respects the primary image property associations.
- PMTiles rejects oversized sections before allocating memory.
- CSV and TSV preserve quoted empty records while skipping blank physical lines.

Each of these ten fixes includes a regression that failed before the implementation change and passed afterward. Targeted Rust tests and formatting checks passed locally. The release workflow runs workspace tests, Clippy, portability checks, evidence and performance checks before building and smoke-testing the binary.

This release also includes the mesh, path tracing, simulation and cinematic rendering changes accumulated since v0.1.2. These features require scene-specific visual review; this release does not establish production readiness for every cinematic workflow.

Download the Linux x86-64 archive and checksum. Run `sha256sum -c scene-render-v0.1.3-linux-x86_64.tar.gz.sha256`, extract the archive, then run `./scene-render --help`. The archive includes the binary, README, license and source commit identifier.

The binary requires Linux x86-64 with glibc 2.39 or newer, a Vulkan driver, FFmpeg and ffprobe 5.1 or newer, and suitable fonts. Mesa software Vulkan is supported. Ubuntu 24.04 dependencies: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.
