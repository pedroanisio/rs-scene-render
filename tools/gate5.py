#!/usr/bin/env python3
"""Writes the Batch 5 exit-gate scene: 2,000 animated, stroked shapes with
animated trim paths over a 1080p60 frame, plus a 30-bone skeleton whose
chain follows an IK target and deforms a skinned image layer.

usage: tools/gate5.py OUTDIR [--duration SECONDS]   (default 10)
then:  scene-render encode OUTDIR/gate5.scene.xml --hw auto
"""
import os
import subprocess
import sys


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "gate5"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 10.0
    os.makedirs(out, exist_ok=True)
    img = os.path.join(out, "ribbon.png")
    if not os.path.exists(img):
        subprocess.run([os.environ.get("SR_FFMPEG", "ffmpeg"), "-v", "error", "-y", "-f", "lavfi",
                        "-i", "testsrc2=s=900x120:d=1", "-frames:v", "1", img], check=True)
    kinds = ["ellipse", "star", "rect", "polygon"]
    shapes = []
    cols, rows = 50, 40
    for k in range(cols * rows):
        c, r = k % cols, k // cols
        x, y = 20 + c * 38, 20 + r * 26
        kind = kinds[k % 4]
        hue = (k * 37) % 360
        phase = (k % 17) / 17.0
        t0 = min(dur, 0.5 + phase)
        shapes.append(
            f'    <shape id="s{k}" shape="{kind}" x="{x}" y="{y}" width="22" height="22" anchorX="11" anchorY="11" '
            f'fill="#00000000" stroke="#{(hue * 7) % 256:02X}{(hue * 3) % 256:02X}{255 - hue % 200:02X}" strokeWidth="3" '
            f'strokeCap="round" strokeJoin="round" trimStart="0">\n'
            f'      <animate property="trimEnd"><key time="0" value="0"/><key time="{t0:.3f}" value="1"/></animate>\n'
            f'      <animate property="rotation"><key time="0" value="0"/><key time="{dur}" value="{(360 if k % 2 else -360) * dur / 10:.1f}"/></animate>\n'
            f'    </shape>')
    bones = ['      <bone id="b0" x="510" y="600" length="30"/>']
    for k in range(1, 30):
        rot = 4 if k % 2 else -4
        bones.append(
            f'      <bone id="b{k}" parent="b{k - 1}" x="30" length="30">\n'
            f'        <animate property="rotation"><key time="0" value="0"/><key time="{dur / 2}" value="{rot}"/><key time="{dur}" value="0"/></animate>\n'
            f'      </bone>')
    xml = f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#0B0E12"/>
  <output id="gate" path="gate5.mp4" codec="h264" preset="veryfast" crf="20" colorSpace="rec709"/>
  <assets>
    <image id="ribbon" src="ribbon.png" width="900" height="120"/>
  </assets>
  <composition>
{chr(10).join(shapes)}
    <shape id="goal" shape="ellipse" x="1250" y="600" width="24" height="24" anchorX="12" anchorY="12" fill="#FF3366">
      <animate property="y"><key time="0" value="480"/><key time="{dur / 2}" value="760"/><key time="{dur}" value="480"/></animate>
    </shape>
    <skeleton id="rig">
{chr(10).join(bones)}
      <transformConstraint type="ik" target="goal"/>
    </skeleton>
    <layer id="skinned" asset="ribbon" x="510" y="540">
      <deform><modifier type="skin" skeleton="rig"/></deform>
    </layer>
  </composition>
</scene>
'''
    path = os.path.join(out, "gate5.scene.xml")
    with open(path, "w") as f:
        f.write(xml)
    print(path)


if __name__ == "__main__":
    main()
