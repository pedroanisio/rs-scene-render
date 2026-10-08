#!/usr/bin/env python3
"""cmp8.py BASE_BIN NEW_BIN: the seven identity frames and the hero 720p frame with both binaries (frames kept), then the pixels that differ, the
maximum and mean difference, the mean of each image and the path-trace time."""
import subprocess, json, sys, os
import numpy as np
from PIL import Image
S = "/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
os.chdir("/home/pals/src/rs-scene-render/.claude/worktrees/mercurio-render")
base, new = sys.argv[1:3]
scenes = {
    "plume_base": (S + "/m7/v0_base.scene.xml", "3.0", None), "plume_dome": (S + "/m7/v6_domeonly.scene.xml", "3.0", None),
    "plume_sun": (S + "/m7/v7_sunonly.scene.xml", "3.0", None), "dry_total": (S + "/r3/DRY_total.scene.xml", "0", None),
    "dry_sun": (S + "/r3/DRY_sun.scene.xml", "0", None), "dry_dome": (S + "/r3/DRY_dome.scene.xml", "0", None),
    "ball_dry": (S + "/r3/G_m0.scene.xml", "0", None),
    "hero_720p": ("examples/cinematic-impact/hero.scene.xml", "3.0", "1280x720"),
}
res = {}
for k, (sc, t, size) in scenes.items():
    out = {}
    for tag, binary in (("base", base), ("new", new)):
        work = f"{S}/cmp8/{tag}_{k}"
        os.makedirs(work, exist_ok=True)
        cmd = ["python3", "tools/probe_render.py", sc, "--times", t, "--agent", "mercurio", "--short", "--binary", binary,
               "--json", f"{work}/p.json", "--timeout", "600", "--keep-frames", "--work", work]
        if size: cmd += ["--size", size]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode: print(k, tag, "FAILED", r.stderr[-200:]); continue
        f = json.load(open(f"{work}/p.json"))["frames"][0]
        out[tag] = (np.asarray(Image.open(__import__("glob").glob(f"{work}/f_*.png")[0]).convert("RGB")).astype(float), f["frame_png_sha256"][:16], f["gpu"]["passes_ms"].get("pathtrace trace", 0) / 1000)
    if len(out) == 2:
        a, b = out["base"][0], out["new"][0]
        d = np.abs(a - b).max(2)
        res[k] = dict(differ=int((d > 0).sum()), of=int(d.size), maxdiff=float(d.max()), meandiff=float(d.mean()),
                      mean_base=float(a.mean()), mean_new=float(b.mean()), hash_base=out["base"][1], hash_new=out["new"][1],
                      trace_s_base=round(out["base"][2], 2), trace_s_new=round(out["new"][2], 2))
        print(k, res[k])
json.dump(res, open(S + "/cmp8.json", "w"), indent=1)
