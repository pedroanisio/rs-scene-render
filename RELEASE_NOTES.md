# scene-render v0.3.1

Scenes with up to four ambient or directional lights use specialized shader pipelines that preserve light order and rendered output. Larger light lists and local lights retain the general lighting path.

The optimization measured 6.7% and 10.9% higher frame throughput in two 24-frame Inova benchmark windows against v0.3.0. These are short-window measurements, not a full-package speedup claim. Two encoded clips matched byte for byte, including decoded video, audio and diagnostics. The GPU oracle checks all 31 light arrangements through four lights, opaque and blended materials, and fallback paths.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.3.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.
