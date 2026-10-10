# scene-render v0.2.1

This patch release fixes scene evaluation, asset resolution and encode finalization.

- Motion paths now update camera and object3D positions. Camera paths with unsupported automatic orientation report a validation error.
- Rigid bodies in groups that start later enter the simulation when their group starts. Late-starting soft bodies report that they are unsupported.
- Flock sprites resolve their image assets without needing a separate image layer.
- Software H.264 and H.265 encoders use fixed thread counts so CPU core counts do not change their output. Encoded bytes can differ from earlier releases.
- Contrast analysis shares a bounded probe budget per output and runs after the encoded file is closed.
- Fuzz crash reports identify panicking worker locations and cover nonfinite ocean body inputs.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable is taken from the successful CI run for the release commit, checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.2.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

Requirements: Linux x86-64, glibc 2.39 or newer, Vulkan (including Mesa software Vulkan), FFmpeg and ffprobe 5.1 or newer, and suitable fonts. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

Windows and macOS are checked for portability; rendering equivalence across different CPUs or GPU drivers is not guaranteed. Multiple scattering is bounded by the requested collision limit and the volume quadrature resolution; its indirect walk terminates at surfaces rather than adding surface-volume interreflection.
