#!/usr/bin/env python3
"""ident.py BIN TAG: frame hashes of glass-free path-traced scenes with BIN, written to ident_TAG.json."""
import subprocess, json, sys, os
S = "/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
os.chdir("/home/pals/src/rs-scene-render/.claude/worktrees/mercurio-voxel")
binary, tag = sys.argv[1:3]
scenes = {
    "plume_base": S + "/m7/v0_base.scene.xml", "plume_dome": S + "/m7/v6_domeonly.scene.xml",
    "plume_sun": S + "/m7/v7_sunonly.scene.xml", "dry_total": S + "/r3/DRY_total.scene.xml",
    "dry_sun": S + "/r3/DRY_sun.scene.xml", "dry_dome": S + "/r3/DRY_dome.scene.xml",
    "ball_dry": S + "/r3/G_m0.scene.xml",
}
out = {}
for k, sc in scenes.items():
    t = "3.0" if k.startswith("plume") else "0"
    r = subprocess.run(["python3", "tools/probe_render.py", sc, "--times", t, "--agent", "mercurio", "--short", "--binary", binary,
                        "--json", f"{S}/ident_{tag}_{k}.json", "--timeout", "300"], capture_output=True, text=True)
    if r.returncode: print(k, "FAILED", r.stderr[-200:]); continue
    f = json.load(open(f"{S}/ident_{tag}_{k}.json"))["frames"][0]
    out[k] = (f["frame_png_sha256"][:16], round(f["gpu"]["passes_ms"].get("pathtrace trace", 0) / 1000, 2))
json.dump(out, open(f"{S}/ident_{tag}.json", "w"), indent=1)
for k, v in out.items(): print(tag, k, v)
