#!/usr/bin/env python3
"""Physics performance scene: simulation at 1080p60.

usage: tools/perf_physics.py OUTDIR [--duration SECONDS]
    Writes perf_physics.scene.xml: 200 rigid bodies (boxes and circles) falling in a
    frame-bounded world under a vortex field, a jelly soft body, a pinned
    cloth, and three emitters (fire, sparks that collide with the bodies, and
    snow) holding up to 12,000 live particles. Every frame advances the
    simulation, so nothing is served from caches.
    then: scene-render render OUTDIR/perf_physics.scene.xml --bench --frames 60..120
"""
import os
import sys


def scene(dur):
    bodies = []
    for k in range(200):
        x = 80 + (k % 20) * 88
        y = 60 + (k // 20) * 40
        if k % 2 == 0:
            bodies.append(f'    <shape id="b{k}" shape="rect" x="{x}" y="{y}" width="30" height="30" fill="#{(k * 37) % 200 + 40:02X}6090"><rigidBody restitution="0.3"/></shape>')
        else:
            bodies.append(f'    <shape id="b{k}" shape="ellipse" x="{x}" y="{y}" width="28" height="28" fill="#E0{(k * 53) % 200 + 40:02X}50"><rigidBody shape="circle" restitution="0.5"/></shape>')
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#101418"/>
  <composition>
{chr(10).join(bodies)}
    <shape id="jelly" shape="rect" x="860" y="100" width="200" height="200" fill="#40C080"><softBody kind="jelly" stiffness="400" mass="2" pressure="40" rows="8" cols="8"/></shape>
    <shape id="cloth" shape="rect" x="1500" y="80" width="300" height="240" fill="#C04060"><softBody kind="cloth" stiffness="20" mass="1" rows="10" cols="10" pin="top"/></shape>
    <particleEmitter id="fire" preset="fire" x="400" y="1040" emitterShape="line" emitterWidth="200" rate="1500" seed="1"/>
    <particleEmitter id="sparks" preset="sparks" x="960" y="900" rate="1500" collide="true" seed="2"/>
    <particleEmitter id="snow" preset="snow" x="960" y="-10" emitterShape="line" emitterWidth="1920" rate="800" seed="3"/>
  </composition>
  <physics bounds="frame" pixelsPerMeter="100">
    <forceField id="swirl" type="vortex" x="960" y="540" strength="0.6" radius="9" falloff="1"/>
  </physics>
</scene>
'''


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "perf_physics"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 10.0
    os.makedirs(out, exist_ok=True)
    path = os.path.join(out, "perf_physics.scene.xml")
    with open(path, "w") as f:
        f.write(scene(dur))
    print(path)


if __name__ == "__main__":
    main()
