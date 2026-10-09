# scene-render v0.2.0

This release integrates the outstanding scene-format implementations and cinematic rendering work, with fixes for nonfinite physics inputs and path-traced water.

- Repeat copies can use generated points; connectors follow their endpoints; PDF assets and text-anchored regions are supported in the packaged executable.
- Render reports include accessibility findings, legibility checks, pinned-font findings and capability notices. Caption layouts can be exported with the captions dump command.
- Text uses consistent line seating, rectangle corner radii follow the format contract, selective-color strength mixes display-encoded values, and missing animation blend targets use the rest pose.
- Voxel bodies can fracture under stress, and crater placement accounts for curved terrain. The ocean solver includes an absorbing boundary mode.
- Volumes support bounded multiple scattering in the path tracer. One scattering bounce remains the default; zero retains absorption and emission without scattering.
- Schema support includes generated marker references and the 1.5 reference checks and conditional capability manifest format, while retaining the documented cinematic extensions.
- Rendering reuses contrast work and specializes raster passes to reduce repeated work.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable is taken from the successful CI run for the release commit, checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.2.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

Requirements: Linux x86-64, glibc 2.39 or newer, Vulkan (including Mesa software Vulkan), FFmpeg and ffprobe 5.1 or newer, and suitable fonts. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

Windows and macOS are checked for portability; rendering equivalence across different CPUs or GPU drivers is not guaranteed. Multiple scattering is bounded by the requested collision limit and the volume quadrature resolution; its indirect walk terminates at surfaces rather than adding surface-volume interreflection.
