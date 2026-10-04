#!/usr/bin/env python3
"""Write a procedural clear-sky equirectangular Radiance .hdr for example scenes.

Row 0 is the zenith. The gradient is analytic (no measured data): a blue zenith,
a hazy horizon and a dim lower hemisphere standing in for distant sea.
Usage: make_sky_hdr.py OUT.hdr [WIDTH HEIGHT]
"""
import math
import struct
import sys


def rgbe(r, g, b):
    m = max(r, g, b)
    if m < 1e-32:
        return b"\0\0\0\0"
    f, e = math.frexp(m)
    s = f * 256.0 / m
    return struct.pack("BBBB", int(r * s), int(g * s), int(b * s), e + 128)


def sky(elevation):
    """Scene-linear radiance at an elevation in [-1, 1] (sine of the angle)."""
    zenith, horizon, sea = (0.18, 0.36, 0.80), (0.95, 0.90, 0.82), (0.04, 0.07, 0.10)
    if elevation >= 0.0:
        t = (1.0 - elevation) ** 4
        return tuple(z + (h - z) * t for z, h in zip(zenith, horizon))
    t = min(1.0, -elevation * 12.0)
    return tuple(h + (s - h) * t for h, s in zip(horizon, sea))


def main():
    out = sys.argv[1]
    w, h = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (512, 256)
    with open(out, "wb") as f:
        f.write(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n")
        f.write(f"-Y {h} +X {w}\n".encode())
        for row in range(h):
            f.write(rgbe(*sky(math.cos(math.pi * (row + 0.5) / h))) * w)


if __name__ == "__main__":
    main()
