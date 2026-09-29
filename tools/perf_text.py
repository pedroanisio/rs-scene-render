#!/usr/bin/env python3
"""Performance scene: 50 text layers with per-character
animators (cycling through the presets plus a wiggly selector) over a
1080p60 frame, and a burned-in karaoke caption track with word timing.

usage: tools/perf_text.py OUTDIR [--duration SECONDS]   (default 10)
then:  scene-render encode OUTDIR/perf_text.scene.xml --hw auto
"""
import os
import sys

PRESETS = ["typewriter", "fade-in", "slide-up", "pop", "scale-in", "wave", "bounce", "spin",
           "ascend", "shift", "scramble", "tracking-in", "karaoke", "letter-by-letter"]
WORDS = ("every word lights up as it is sung while fifty animated titles "
         "move above the captions in this typography stress test").split()


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "perf_text"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 10.0
    os.makedirs(out, exist_ok=True)
    assets, layers = [], []
    for k in range(50):
        c, r = k % 5, k // 5
        preset = PRESETS[k % len(PRESETS)]
        assets.append(f'    <text id="t{k}" text="Title {k:02d} {preset}" width="360" height="60" size="30" '
                      f'color="#{(k * 53) % 256:02X}{(k * 97) % 256:02X}FF" font="DejaVu Sans" weight="700"/>')
        start = (k % 10) * 0.1
        wig = ('\n      <textAnimator selector="wiggly" wiggleRate="1.5" amount="60" y="6" rotation="8"/>' if k % 3 == 0 else "")
        layers.append(f'    <layer id="l{k}" asset="t{k}" x="{40 + c * 376}" y="{40 + r * 84}">\n'
                      f'      <textAnimator preset="{preset}" presetStart="{start:.1f}" presetDuration="{min(2.0, dur):.1f}"/>{wig}\n'
                      f'    </layer>')
    # captions: pages of six words, each word 0.4 s, looping over the duration
    cues, t, i = [], 0.0, 0
    while t < dur:
        words = []
        for _ in range(6):
            w = WORDS[i % len(WORDS)]
            words.append(f'<word start="{t:.2f}" end="{t + 0.4:.2f}" text="{w}"/>')
            t += 0.4
            i += 1
        start = float(words[0].split('"')[1])
        cues.append(f'      <cue start="{start:.2f}" end="{t:.2f}">{"".join(words)}</cue>')
    xml = f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#0B0E12"/>
  <output id="perf" path="perf_text.mp4" codec="h264" preset="veryfast" crf="20" colorSpace="rec709"/>
  <assets>
{chr(10).join(assets)}
  </assets>
  <composition>
{chr(10).join(layers)}
  </composition>
  <captions>
    <captionTrack id="karaoke" language="en" preset="karaoke" activeColor="#FFD400" maxCharsPerLine="40">
{chr(10).join(cues)}
    </captionTrack>
  </captions>
</scene>
'''
    path = os.path.join(out, "perf_text.scene.xml")
    with open(path, "w") as f:
        f.write(xml)
    print(path)


if __name__ == "__main__":
    main()
