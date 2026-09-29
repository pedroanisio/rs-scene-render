#!/usr/bin/env python3
"""Effects performance scene and per-effect cost measurement.

usage: tools/perf_effects.py OUTDIR [--duration SECONDS]
    Writes perf_effects.scene.xml: a 1080p60 scene with 8 effects (on layers and an
    adjustment layer), a crossfade active over the whole timeline, and
    16-sample shutter motion blur on two moving layers.
    then: scene-render render OUTDIR/perf_effects.scene.xml --bench --frames 0..120

usage: tools/perf_effects.py OUTDIR --measure BIN [--frames N]
    Writes one 1080p scene per effect type (the effect at its default
    settings on a full-frame moving layer) plus a baseline without effects,
    benches each with BIN (with SR_FX_NO_CACHE=1, so every frame recomputes
    the chain) and prints every effect's cost over the baseline.
"""
import os
import re
import subprocess
import sys

EFFECTS = [
    "glow", "bloom", "blur", "color-grade", "vignette", "lens-flare", "drop-shadow", "lighting", "directional-blur", "radial-blur", "zoom-blur", "lens-blur",
    "pixel-motion-blur", "tilt-shift", "lift-gamma-gain", "cdl", "lut", "curves", "levels", "white-balance", "exposure", "hue-saturation", "tonemap", "tint",
    "tritone", "gradient-map", "grayscale", "sepia", "invert", "posterize", "threshold", "color-overlay", "gradient-overlay", "selective-color", "film-grain",
    "noise", "chromatic-aberration", "sharpen", "unsharp-mask", "halation", "light-leak", "light-sweep", "glitch", "rgb-split", "scanlines", "vhs", "halftone",
    "pixelate", "mosaic", "emboss", "bevel", "inner-shadow", "inner-glow", "long-shadow", "stroke", "outline", "echo", "posterize-time", "letterbox", "mirror",
    "kaleidoscope", "tile", "displacement-map", "turbulent-displace", "wave-warp", "ripple", "twirl", "spherize", "bulge", "lens-distortion", "heat-haze",
    "chroma-key", "luma-key", "difference-key", "spill-suppress", "matte-choke", "fill", "fractal-noise", "god-rays", "shader",
]

EXTRA = {
    "lut": ' src="grade.cube"',
    "shader": ' src="pass.glsl"',
    "curves": ' curve="0,0 0.25,0.2 0.75,0.85 1,1"',
    "gradient-map": ' paint="url(#ramp)"',
    "gradient-overlay": ' paint="url(#ramp)"',
    "displacement-map": ' source="plate"',
    "difference-key": ' source="plate"',
    "lighting": ' lights="lamp"',
}

ASSETS = '''  <assets>
    <generator id="noise" kind="fractal-noise" width="1920" height="1080" scale="160" seed="3"/>
    <generator id="check" kind="checkerboard" width="1920" height="1080" scale="60" paint="#E8E2D0" paint2="#2B3A55"/>
    <generator id="card" kind="checkerboard" width="480" height="300" scale="30" paint="#FF7A1A" paint2="#FFFFFF"/>
  </assets>
  <paints>
    <linearGradient id="ramp"><stop offset="0" color="#0A1030"/><stop offset="0.5" color="#C0406A"/><stop offset="1" color="#FFE8A0"/></linearGradient>
  </paints>'''


def write_support(out):
    n = 17
    rows = []
    for b in range(n):
        for g in range(n):
            for r in range(n):
                x = [r / (n - 1), g / (n - 1), b / (n - 1)]
                rows.append(" ".join(f"{min(1.0, v * 1.05 + 0.02):.6f}" for v in x))
    with open(os.path.join(out, "grade.cube"), "w") as f:
        f.write(f"LUT_3D_SIZE {n}\n" + "\n".join(rows) + "\n")
    with open(os.path.join(out, "pass.glsl"), "w") as f:
        f.write("uniform float amount; // = 0.5\nvec4 effect(vec2 uv) { vec4 c = getColor(uv); return vec4(mix(c.rgb, c.gbr, amount), c.a); }\n")


def perf_scene(dur):
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#000000" motionBlur="true" shutterAngle="180" shutterPhase="-90" motionBlurSamples="16"/>
{ASSETS}
  <composition>
    <layer id="bgA" asset="noise" end="{dur / 2}" effects="blur colorgrade" motionBlur="off"/>
    <layer id="bgB" asset="check" start="{dur / 2}" effects="colorgrade" motionBlur="off"/>
    <transition type="crossfade" from="bgA" to="bgB" duration="{dur}"/>
    <layer id="m1" asset="card" x="100" y="200" effects="shadow glow">
      <animate property="x"><key time="0" value="100"/><key time="{dur}" value="{100 + 1200 * dur}"/></animate>
    </layer>
    <layer id="m2" asset="card" x="1300" y="600" effects="ca streak">
      <animate property="x"><key time="0" value="1300"/><key time="{dur}" value="{1300 - 1100 * dur}"/></animate>
    </layer>
    <adjustment id="finish" effects="vignette grain"/>
  </composition>
  <effects>
    <effect id="blur" type="blur" radius="6"/>
    <effect id="colorgrade" type="color-grade" saturation="1.15" contrast="1.05"/>
    <effect id="shadow" type="drop-shadow" radius="12" offsetX="10" offsetY="14"/>
    <effect id="glow" type="glow" radius="10" threshold="0.6"/>
    <effect id="ca" type="chromatic-aberration" amount="2"/>
    <effect id="streak" type="directional-blur" radius="8" samples="12"/>
    <effect id="vignette" type="vignette" amount="0.6"/>
    <effect id="grain" type="film-grain" amount="0.3"/>
  </effects>
</scene>
'''


def effect_scene(kind, dur):
    effects = f'effects="fx"' if kind else ""
    fx = f'\n  <effects>\n    <effect id="fx" type="{kind}"{EXTRA.get(kind, "")}/>\n  </effects>' if kind else ""
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#000000"/>
{ASSETS}
  <composition>
    <layer id="plate" asset="check" visible="false"/>
    <layer id="subject" asset="noise" {effects}>
      <animate property="x"><key time="0" value="0"/><key time="{dur}" value="{60 * dur}"/></animate>
    </layer>
  </composition>
  <lights>
    <light id="lamp" type="point" x="960" y="400" intensity="1.5"/>
  </lights>{fx}
</scene>
'''


def median_ms(binary, path, frames):
    env = dict(os.environ, SR_FX_NO_CACHE="1")  # every frame recomputes the chain
    r = subprocess.run([binary, "render", path, "--bench", "--frames", f"0..{frames}"], capture_output=True, text=True, env=env)
    m = re.search(r"render: median ([0-9.]+) ms", r.stdout)
    if not m:
        raise SystemExit(f"{path}: {r.stdout}{r.stderr}")
    return float(m.group(1))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "perf_effects"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 10.0
    os.makedirs(out, exist_ok=True)
    write_support(out)
    if "--measure" in sys.argv:
        binary = sys.argv[sys.argv.index("--measure") + 1]
        frames = int(sys.argv[sys.argv.index("--frames") + 1]) if "--frames" in sys.argv else 8
        base_path = os.path.join(out, "fx_none.scene.xml")
        with open(base_path, "w") as f:
            f.write(effect_scene(None, 2.0))
        base = median_ms(binary, base_path, frames)
        print(f"baseline (one moving 1080p layer): {base:.2f} ms")
        worst = []
        for kind in EFFECTS:
            path = os.path.join(out, f"fx_{kind}.scene.xml")
            with open(path, "w") as f:
                f.write(effect_scene(kind, 2.0))
            cost = median_ms(binary, path, frames) - base
            worst.append((cost, kind))
            print(f"{kind:22s} {cost:8.2f} ms")
        worst.sort(reverse=True)
        over = [k for c, k in worst if c > 2.0]
        print(f"effects over 2 ms: {len(over)} of {len(EFFECTS)}; slowest: " + ", ".join(f"{k} {c:.1f} ms" for c, k in worst[:5]))
        return
    path = os.path.join(out, "perf_effects.scene.xml")
    with open(path, "w") as f:
        f.write(perf_scene(dur))
    print(path)


if __name__ == "__main__":
    main()
