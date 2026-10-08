import subprocess, json, glob, os, sys
import numpy as np
from PIL import Image
S = "/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad"
os.chdir("/home/pals/src/rs-scene-render/.claude/worktrees/mercurio-base1")
binary = sys.argv[1]; tag = sys.argv[2]
def render(scene, t, name):
    work = f"{S}/hf/{tag}_{name}"; os.makedirs(work, exist_ok=True)
    r = subprocess.run(["python3", "tools/probe_render.py", scene, "--times", t, "--size", "1280x720", "--agent", "mercurio", "--short",
        "--binary", binary, "--json", f"{work}/p.json", "--timeout", "900", "--keep-frames", "--work", work], capture_output=True, text=True)
    if r.returncode: print(name, "FAILED", r.stdout[-300:], r.stderr[-300:]); return None
    f = json.load(open(f"{work}/p.json"))["frames"][0]
    png = glob.glob(f"{work}/f_*.png")[0]
    print(tag, name, f["frame_png_sha256"], "wall", f["wall_seconds"], "trace_s", round(f["gpu"]["passes_ms"].get("pathtrace trace", 0)/1000, 2), "rss", f["peak_rss_mib"], flush=True)
    return png
hero = "examples/cinematic-impact/hero.scene.xml"; nof = "examples/cinematic-impact/zz_nofollow.scene.xml"
out = {}
out["t3"] = render(hero, "3.0", "hero_t3.0")
out["f120_follow"] = render(hero, "5.0", "f120_follow")
out["f120_nofollow"] = render(nof, "5.0", "f120_nofollow")
out["f143_follow"] = render(hero, "5.96", "f143_follow")
out["f143_nofollow"] = render(nof, "5.96", "f143_nofollow")
json.dump(out, open(f"{S}/hf/{tag}_paths.json", "w"), indent=1)
