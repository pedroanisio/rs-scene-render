"""Writes glossy.ply: a 3D Gaussian Splatting file (the format of the reference implementation,
nerfstudio's splatfacto exports and most capture apps) of a glossy ceramic vase, with degree-3
spherical harmonics that brighten the side facing +x, standing in for a photographic capture.

    python examples/splat/make_splat.py      (needs numpy)

The splats are in metres, y up, like a capture reoriented upright; the scene places the object
with object3D's position, rotation and scale.
"""
import math
import os
import struct

import numpy as np

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "glossy.ply")
C0 = 0.28209479177387814
rng = np.random.default_rng(3)

pts, cols, shs = [], [], []
for k in range(24000):
    # a vase profile: radius as a function of height
    h = rng.uniform(0.0, 0.5)
    r = 0.12 + 0.08 * math.sin(h / 0.5 * math.pi * 1.5) + 0.02 * math.cos(h * 40)
    a = rng.uniform(0, math.tau)
    n = np.array([math.cos(a), 0.0, math.sin(a)])
    p = np.array([r * math.cos(a), h, r * math.sin(a)])
    base = np.array([0.55, 0.22, 0.12]) * (0.85 + 0.15 * math.sin(a * 6))
    pts.append(p)
    cols.append(base)
    # degree-1 lobe toward the light at +x plus a little degree 2/3 texture; coefficient k of the
    # basis multiplies the direction terms of the 3DGS basis (see the renderer's three_splat.wgsl)
    sh = np.zeros((15, 3))
    sh[2] = -0.25 * n[0]  # −C1·x term: brighter when seen along −x, i.e. the +x side facing the camera
    sh[4:15] = rng.normal(0, 0.03, (11, 3))
    shs.append(sh)

props = ["x", "y", "z", "nx", "ny", "nz", "f_dc_0", "f_dc_1", "f_dc_2"] + [f"f_rest_{k}" for k in range(45)]
props += ["opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2", "rot_3"]
with open(OUT, "wb") as f:
    f.write(f"ply\nformat binary_little_endian 1.0\nelement vertex {len(pts)}\n".encode())
    for p in props:
        f.write(f"property float {p}\n".encode())
    f.write(b"end_header\n")
    for p, c, sh in zip(pts, cols, shs):
        dc = (c - 0.5) / C0
        rest = sh.T.reshape(-1)  # channel-major, as the reference implementation writes it
        opacity = 4.0  # logit
        scale = [math.log(0.006)] * 3
        vals = [*p, 0.0, 0.0, 0.0, *dc, *rest, opacity, *scale, 1.0, 0.0, 0.0, 0.0]
        f.write(struct.pack(f"<{len(vals)}f", *vals))
print("wrote", OUT, len(pts), "splats")
