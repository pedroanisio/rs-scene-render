#!/usr/bin/env python3
"""Batch 8 exit gate.

usage: tools/gate8.py OUTDIR [--duration SECONDS]
    Writes gate8.scene.xml: 1080p60 with 207,936 triangles (a 114-segment
    sphere drawn as 16 instances placed on a 4×4 grid by `index`), a floor,
    a glass slab (transmission), 4 spot lights casting 2048² shadows, and a
    camera with depth of field.
    The spheres turn and the lights orbit, so every frame is new work.
    then: scene-render render OUTDIR/gate8.scene.xml --bench --frames 0..120

usage: tools/gate8.py OUTDIR --measure BIN [--frames N]
    Benches the gate scene and variants without shadows, transmission and
    depth of field, and prints each feature's cost.
"""
import os
import re
import subprocess
import sys


def scene(dur, shadows=True, glass=True, dof=True):
    cast = "true" if shadows else "false"
    lights = []
    for k, (x, z) in enumerate([(360, -200), (1560, -200), (360, 900), (1560, 900)]):
        lights.append(
            f'    <light id="spot{k}" type="spot" x="{x}" y="-300" z="{z}" pitch="-50" yaw="{(-30 if x > 960 else 30) + (0 if z < 0 else 180)}" '
            f'spotAngle="70" intensity="800" castShadow="{cast}" shadowMapSize="2048" range="4000">\n'
            f'      <animate property="x"><key time="0" value="{x}"/><key time="{dur}" value="{x + 200}"/></animate>\n'
            f'    </light>'
        )
    glass_obj = (
        '    <object3D id="glass" primitive="box" width="700" height="360" depth="30" x="960" y="620" z="-120" material="m-glass"/>\n' if glass else ""
    )
    camera = (
        f'    <camera id="cam" fov="45" y="-120" pitch="-8" depthOfField="{"true" if dof else "false"}" fStop="2" focusTarget="spheres" focalLength="50"/>\n'
    )
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#101418"/>
  <materials>
    <material id="m-chrome" baseColor="#D8D8E0" metallic="1" roughness="0.25" clearcoat="0.5"/>
    <material id="m-floor" baseColor="#607080" roughness="0.8"/>
    <material id="m-glass" transmission="1" roughness="0.05" ior="1.5" thickness="30" dispersion="0.5"/>
  </materials>
  <composition>
{camera}    <object3D id="floor" primitive="plane" width="6000" height="6000" x="960" y="900" z="600" rotationX="-90" material="m-floor"/>
    <object3D id="spheres" primitive="sphere" radius="60" segments="114" instances="16" z="500" material="m-chrome">
      <expression property="x">960 + (index % 4 - 1.5) * 150</expression>
      <expression property="y">620 + (Math.floor(index / 4) - 1.5) * 150</expression>
      <animate property="rotationY"><key time="0" value="0"/><key time="{dur}" value="{36 * dur}"/></animate>
    </object3D>
{glass_obj}  </composition>
  <lights>
    <light id="sky" type="ambient" intensity="0.15"/>
{chr(10).join(lights)}
  </lights>
</scene>
'''


def median_ms(binary, path, frames):
    r = subprocess.run([binary, "render", path, "--bench", "--frames", f"0..{frames}"], capture_output=True, text=True)
    m = re.search(r"render: median ([0-9.]+) ms", r.stdout)
    if not m:
        raise SystemExit(f"{path}: {r.stdout}{r.stderr}")
    return float(m.group(1))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "gate8"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 10.0
    os.makedirs(out, exist_ok=True)
    if "--measure" in sys.argv:
        binary = sys.argv[sys.argv.index("--measure") + 1]
        frames = int(sys.argv[sys.argv.index("--frames") + 1]) if "--frames" in sys.argv else 8
        variants = [
            ("full", dict()),
            ("no shadows", dict(shadows=False)),
            ("no transmission", dict(glass=False)),
            ("no depth of field", dict(dof=False)),
            ("geometry only", dict(shadows=False, glass=False, dof=False)),
        ]
        times = {}
        for name, kw in variants:
            path = os.path.join(out, f"gate8_{name.replace(' ', '_')}.scene.xml")
            with open(path, "w") as f:
                f.write(scene(2.0, **kw))
            times[name] = median_ms(binary, path, frames)
            print(f"{name:20s} {times[name]:9.1f} ms  ({1000.0 / times[name]:.1f} fps)")
        full = times["full"]
        print(f"shadows {full - times['no shadows']:.1f} ms, transmission {full - times['no transmission']:.1f} ms, "
              f"depth of field {full - times['no depth of field']:.1f} ms, geometry and lighting {times['geometry only']:.1f} ms")
        return
    path = os.path.join(out, "gate8.scene.xml")
    with open(path, "w") as f:
        f.write(scene(dur))
    print(path)


if __name__ == "__main__":
    main()
