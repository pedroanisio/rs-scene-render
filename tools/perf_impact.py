#!/usr/bin/env python3
"""Impact-shot performance scene: the mix of an anime-style impact shot, without media files.

usage: tools/perf_impact.py OUTDIR [--duration SECONDS]
    Writes perf_impact.scene.xml: a 1080p60 scene with a posterize-time group (30 fps
    cadence) holding a fractal-noise backdrop, gradient shapes, 150-wedge focus lines and a
    particle emitter; a stroked title (wide and thin alpha-morphology strokes); a blurred rigid layer under 12-sample motion blur; a feathered-ring adjustment
    with a turbulent displacement; and a finishing adjustment with a grade, a vignette and
    film grain.
    then: scene-render render OUTDIR/perf_impact.scene.xml --bench --frames 0..60
"""
import math
import os
import random
import sys


def focus_lines(n=150, seed=7):
    """SVG path of `n` thin wedges converging on the centre of a 4000 px box."""
    rnd = random.Random(seed)
    c, far = 2000.0, 2900.0
    parts = []
    for i in range(n):
        th = (i + rnd.uniform(-0.35, 0.35)) / n * 2 * math.pi
        dth = rnd.uniform(0.0025, 0.011)
        r0 = rnd.uniform(430, 820)

        def p(r, a):
            return f"{c + r * math.cos(a):.1f} {c + r * math.sin(a):.1f}"

        parts.append(f"M{p(r0, th)} L{p(far, th - dth)} L{p(far, th + dth)} Z")
    return " ".join(parts)


def scene(duration):
    rnd = random.Random(11)
    shapes = []
    for k in range(12):
        x, y = rnd.uniform(0, 1700), rnd.uniform(0, 900)
        w, h = rnd.uniform(80, 300), rnd.uniform(80, 300)
        kind = "ellipse" if k % 2 else "rect"
        shapes.append(
            f'<shape id="s{k}" shape="{kind}" x="{x:.0f}" y="{y:.0f}" width="{w:.0f}" height="{h:.0f}" fill="url(#warm)">'
            f'<animate property="rotation"><key time="0" value="0"/><key time="{duration}" value="{rnd.uniform(-40, 40):.0f}"/></animate></shape>'
        )
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{duration}" seed="7" background="#090A12"/>
  <assets>
    <generator id="sky" kind="fractal-noise" width="1920" height="1080">
      <animate property="evolution"><key time="0" value="0"/><key time="{duration}" value="90"/></animate>
    </generator>
  </assets>
  <paints>
    <linearGradient id="warm" x1="0" y1="0" x2="1" y2="1"><stop offset="0" color="#FFB030"/><stop offset="1" color="#C0301A"/></linearGradient>
  </paints>
  <composition>
    <group id="rig" effects="cadence">
      <layer id="backdrop" asset="sky"/>
      {"".join(shapes)}
      <shape id="lines" shape="path" path="{focus_lines()}" width="4000" height="4000" anchorX="2000" anchorY="2000" x="1150" y="600" fill="#FFFFFF" opacity="0.6">
        <expression property="rotation" seed="22">random(0, 360)</expression>
      </shape>
      <particleEmitter id="sparks" x="1150" y="600" emitterShape="point" rate="60" lifetime="0.6" speed="1400" speedVariance="600" spread="360"
                       gravityY="700" drag="1.1" size="6" sizeEnd="1.5" color="#FFFFFF" colorEnd="#FF9A2E" opacityEnd="0" shape="streak" trail="0.07"
                       orientToVelocity="true" blend="add" seed="81"><burst time="0" count="240"/></particleEmitter>
    </group>
    <shape id="title" shape="star" points="8" innerRadius="0.6" x="1250" y="80" width="420" height="220" fill="#FFFFFF" effects="ink trim">
      <animate property="scaleX"><key time="0" value="1.4"/><key time="{duration}" value="1"/></animate>
      <animate property="scaleY"><key time="0" value="1.4"/><key time="{duration}" value="1"/></animate>
    </shape>
    <shape id="mover" shape="rect" x="100" y="760" width="360" height="200" fill="url(#warm)" effects="soft" motionBlur="on">
      <animate property="x"><key time="0" value="100"/><key time="{duration}" value="1500"/></animate>
    </shape>
    <adjustment id="shock" effects="warp">
      <mask type="ellipse" x="600" y="200" width="1100" height="800" feather="26"/>
      <mask type="ellipse" x="700" y="280" width="900" height="640" mode="subtract" feather="26"/>
    </adjustment>
    <adjustment id="finish" effects="grade vignette grain"/>
  </composition>
  <effects>
    <effect id="cadence" type="posterize-time" frequency="30"/>
    <effect id="ink" type="stroke" size="16" color="#000000"/>
    <effect id="trim" type="stroke" size="3" color="#E3261E"/>
    <effect id="soft" type="blur" radius="6"/>
    <effect id="warp" type="turbulent-displace" amount="28" size="70" speed="3"/>
    <effect id="grade" type="color-grade" saturation="1.12" contrast="1.1"/>
    <effect id="vignette" type="vignette" amount="0.3" radius="0.8"/>
    <effect id="grain" type="film-grain" amount="0.025" size="1.1" seed="3001"/>
  </effects>
</scene>
"""


def main():
    args = sys.argv[1:]
    if not args or args[0].startswith("-"):
        print(__doc__, file=sys.stderr)
        return 2
    out = args[0]
    duration = 2.0
    if "--duration" in args:
        duration = float(args[args.index("--duration") + 1])
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "perf_impact.scene.xml"), "w") as f:
        f.write(scene(duration))
    return 0


if __name__ == "__main__":
    sys.exit(main())
