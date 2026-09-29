#!/usr/bin/env python3
"""Performance scene and media: four 1920x1080 60 fps
H.264 sources with AAC audio, a music track, and a 1080p60 H.264 output
of four transformed video layers with the mixed, loudness-normalised audio.

usage: tools/perf_video.py OUTDIR [--duration SECONDS]   (default 60)
then:  scene-render encode OUTDIR/perf_video.scene.xml --hw auto
"""
import os
import subprocess
import sys


def ff(*args):
    subprocess.run([os.environ.get("SR_FFMPEG", "ffmpeg"), "-v", "error", "-y", *args], check=True)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "perf_video"
    dur = float(sys.argv[sys.argv.index("--duration") + 1]) if "--duration" in sys.argv else 60.0
    os.makedirs(out, exist_ok=True)
    sources = ["testsrc2", "smptehdbars", "testsrc", "rgbtestsrc"]
    for k, src in enumerate(sources):
        path = os.path.join(out, f"src{k}.mp4")
        if not os.path.exists(path):
            ff("-f", "lavfi", "-i", f"{src}=s=1920x1080:r=60:d={dur}",
               "-f", "lavfi", "-i", f"sine=frequency={220 * (k + 1)}:sample_rate=48000:duration={dur}",
               "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-g", "120", "-pix_fmt", "yuv420p",
               "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
               "-c:a", "aac", "-b:a", "128k", "-shortest", path)
    music = os.path.join(out, "music.wav")
    if not os.path.exists(music):
        ff("-f", "lavfi", "-i", f"sine=frequency=110:sample_rate=48000:duration={dur}:beep_factor=2", "-ac", "2", music)
    assets = "\n".join(
        f'    <video id="v{k}" src="src{k}.mp4" width="1920" height="1080" fps="60" duration="{dur}" hasAudio="true" colorSpace="rec709"/>'
        for k in range(4))
    quads = [(480, 270), (1440, 270), (480, 810), (1440, 810)]
    layers = []
    for k, (x, y) in enumerate(quads):
        extra = ""
        if k == 1:
            extra = f'\n      <animate property="rotation"><key time="0" value="0"/><key time="{dur}" value="{dur * 30}"/></animate>'
        if k == 2:
            extra = '\n      <mask type="ellipse" x="160" y="90" width="1600" height="900" feather="120"/>'
        blend = "screen" if k == 3 else "normal"
        layers.append(
            f'    <layer id="L{k}" asset="v{k}" x="{x}" y="{y}" anchorX="960" anchorY="540" scaleX="0.5" scaleY="0.5" '
            f'blend="{blend}" volume="0.5">{extra}\n    </layer>')
    xml = f'''<?xml version="1.0" encoding="UTF-8"?>
<scene version="1.1">
  <project width="1920" height="1080" fps="60" duration="{dur}" background="#101418"/>
  <output id="perf" path="perf_video.mp4" codec="h264" preset="veryfast" crf="20" colorSpace="rec709"/>
  <assets>
{assets}
    <audio id="musicA" src="music.wav"/>
  </assets>
  <composition>
{chr(10).join(layers)}
  </composition>
  <audioMix sampleRate="48000">
    <audioTrack id="music" asset="musicA" volume="0.8" fadeIn="1" fadeOut="1" duckUnder="L0" duckAmount="-6"/>
    <master normalize="integrated" loudness="-14" truePeak="-1"/>
  </audioMix>
</scene>
'''
    path = os.path.join(out, "perf_video.scene.xml")
    with open(path, "w") as f:
        f.write(xml)
    print(path)


if __name__ == "__main__":
    main()
