#!/usr/bin/env python3
"""Writes the Batch 3 exit-gate scene: 200 transformed image layers at
1920x1080, each masked with a feathered ellipse, cycling through the 33
layer-local blend modes, every layer animated so no frame can be served from a cache.

usage: tools/gate3.py OUTDIR [--layers N]
then:  scene-render render OUTDIR/gate3.scene.xml --bench --frames 0..60
"""
import math
import os
import struct
import sys
import zlib

BLENDS = ("normal dissolve add plus-lighter multiply screen overlay difference exclusion subtract divide "
          "darken lighten darker-color lighter-color color-dodge color-burn linear-dodge linear-burn soft-light "
          "hard-light linear-light vivid-light pin-light hard-mix hue saturation color luminosity stencil-alpha "
          "stencil-luma silhouette-alpha silhouette-luma alpha-add behind").split()
# stencil modes clear the whole frame outside their layer; the gate cycles through the 33 layer-local modes
BLENDS = tuple(b for b in BLENDS if not b.startswith("stencil"))


def png(path, w, h, pixel):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for x in range(w):
            raw.extend(pixel(x, y))

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(raw), 6)))
        f.write(chunk(b"IEND", b""))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "gate3"
    n = int(sys.argv[sys.argv.index("--layers") + 1]) if "--layers" in sys.argv else 200
    os.makedirs(out, exist_ok=True)
    for k in range(8):
        hue = k / 8.0

        def px(x, y, hue=hue):
            r = 0.5 + 0.5 * math.cos(6.283 * (hue + x / 256.0))
            g = 0.5 + 0.5 * math.cos(6.283 * (hue + y / 256.0 + 0.33))
            b = 0.5 + 0.5 * math.cos(6.283 * (hue + (x + y) / 512.0 + 0.66))
            a = 255 if ((x // 32 + y // 32) % 2 == 0 or k % 2 == 0) else 200
            return (int(r * 255), int(g * 255), int(b * 255), a)

        png(os.path.join(out, f"tex{k}.png"), 256, 256, px)
    assets = "\n".join(f'    <image id="t{k}" src="tex{k}.png" width="256" height="256"/>' for k in range(8))
    layers = []
    for i in range(n):
        x = 60 + (i * 97) % 1800
        y = 60 + (i * 389) % 960
        s = 0.8 + (i % 5) * 0.2
        blend = BLENDS[i % len(BLENDS)]
        layers.append(
            f'    <layer id="l{i}" asset="t{i % 8}" x="{x}" y="{y}" anchorX="128" anchorY="128" '
            f'scaleX="{s:.2f}" scaleY="{s:.2f}" blend="{blend}" opacity="0.9">\n'
            f'      <mask type="ellipse" x="16" y="16" width="224" height="224" feather="24"/>\n'
            f'      <animate property="rotation"><key time="0" value="{i * 7 % 360}"/><key time="10" value="{i * 7 % 360 + 360}"/></animate>\n'
            f'      <animate property="x"><key time="0" value="{x}"/><key time="10" value="{x + 40}"/></animate>\n'
            f'    </layer>')
    body = "\n".join(layers)
    xml = f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="10" background="#101418"/>
  <assets>
{assets}
  </assets>
  <composition>
{body}
  </composition>
</scene>
'''
    with open(os.path.join(out, "gate3.scene.xml"), "w") as f:
        f.write(xml)
    print(os.path.join(out, "gate3.scene.xml"))


if __name__ == "__main__":
    main()
