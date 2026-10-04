#!/usr/bin/env python3
"""Render probe: time and memory of single frames of a scene, per stage, as JSON.

usage: tools/probe_render.py SCENE --times 0.8,1.5,3.0 [--size 1280x720] [--json OUT]
                             [--binary BIN] [--work DIR] [--agent NAME] [--short]
                             [--strict] [--keep-frames] [--timeout SECONDS]

Renders each time T of SCENE as its own `scene-render render --time T --stats` run through
the shared GPU queue (`~/.local/share/scene-render/bin/sr-gpu`, `SR_AGENT` from --agent),
and records per frame:

  wall_seconds    wall clock of the scene-render process alone: queue wait and the wrapper's
                  start-up pause are excluded, because the timed child starts after both
  peak_rss_mib    peak resident set of that process (and anything it waited for)
  stats           RenderStats as printed by --stats: seconds per solver (sim_*_seconds),
                  draw preparation, and the path tracer's CPU stages (pt_*_seconds)
  gpu             GPU milliseconds: the whole frame and `passes_ms`, the passes summed by
                  label (`pathtrace trace`, `pathtrace denoise`, effect passes); absent where
                  the adapter has no timestamp queries
  frame_png_sha256  hash of the written frame, to tell two builds' pixels apart

--size WxH renders a copy of the scene whose <project> has that size (the scene's own file is
never edited; its sibling files are linked beside the copy so relative paths resolve).
--stats makes the renderer time every GPU pass, which costs a little itself.
A failed render or a missing statistics line is an error (exit 1), never a partial result.
Standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import resource
import shutil
import subprocess
import sys
import tempfile
import time

import perf_common

SR_GPU = os.path.expanduser("~/.local/share/scene-render/bin/sr-gpu")
PROJECT = re.compile(r"<project\b[^>]*>")


class ProbeError(Exception):
    pass


def resized_copy(scene, size, work):
    """Path of a copy of SCENE with its <project> resized, beside links to its siblings."""
    src_dir = os.path.dirname(os.path.abspath(scene))
    dst_dir = os.path.join(work, "scene")
    os.makedirs(dst_dir, exist_ok=True)
    name = os.path.basename(scene)
    for entry in os.listdir(src_dir):
        link = os.path.join(dst_dir, entry)
        if entry == name or os.path.lexists(link):
            continue
        os.symlink(os.path.join(src_dir, entry), link)
    with open(scene, encoding="utf-8") as f:
        text = f.read()
    match = PROJECT.search(text)
    if not match:
        raise ProbeError(f"{scene}: no <project> element to resize")
    tag = match.group(0)
    for attr, value in zip(("width", "height"), size):
        tag, n = re.subn(rf'\b{attr}="[^"]*"', f'{attr}="{value}"', tag)
        if n != 1:
            raise ProbeError(f"{scene}: <project> has no single {attr} attribute")
    out = os.path.join(dst_dir, name)
    with open(out, "w", encoding="utf-8") as f:
        f.write(text[: match.start()] + tag + text[match.end():])
    return out


def child(args):
    """Timed run of one render; prints one JSON object. Runs inside the GPU queue."""
    cmd = [args.binary, "render", args.scene, "--time", repr(args.time), "-o", args.out, "--stats"]
    if args.strict:
        cmd.append("--strict")
    started = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                            env=perf_common.run_env())
    try:
        _, stderr = proc.communicate(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        _, stderr = proc.communicate()
        print(json.dumps({"error": f"timed out after {args.timeout}s", "stderr_tail": perf_common.tail(stderr)}))
        return 1
    wall = time.perf_counter() - started
    # peak resident set of the render and anything it waited for, in KiB on Linux
    rss_kib = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    print(json.dumps({"returncode": proc.returncode, "wall_seconds": wall, "peak_rss_mib": rss_kib / 1024.0,
                      "stderr": stderr}))
    return 0


def parse_stats(stderr):
    """The last `{"frame":…,"stats":…}` line of the renderer's standard error."""
    found = None
    for line in stderr.splitlines():
        line = line.strip()
        if line.startswith("{") and '"stats"' in line:
            try:
                found = json.loads(line)
            except ValueError:
                continue
    return found


def sum_passes(gpu):
    if not gpu:
        return None
    sums = {}
    for p in gpu.get("passes", []):
        sums[p["label"]] = sums.get(p["label"], 0.0) + p["ms"]
    return {"frame_ms": gpu.get("frame_ms"), "passes_ms": sums}


def probe_one(args, scene, t, work):
    png = os.path.join(work, f"f_{t}.png")
    env = dict(os.environ)
    env["SR_AGENT"] = args.agent
    if args.short:
        env["SR_GPU_SHORT"] = "1"
    cmd = [SR_GPU, sys.executable, os.path.abspath(__file__), "--child", "--binary", args.binary,
           "--time", repr(t), "--out", png, "--timeout", str(args.timeout)]
    if args.strict:
        cmd.append("--strict")
    cmd.append(scene)
    r = subprocess.run(cmd, capture_output=True, text=True, env=env)
    lines = [ln for ln in r.stdout.splitlines() if ln.startswith("{")]
    if r.returncode != 0 or not lines:
        raise ProbeError(f"t={t}: probe child failed (exit {r.returncode}): {perf_common.tail(r.stderr or r.stdout)}")
    got = json.loads(lines[-1])
    if "error" in got:
        raise ProbeError(f"t={t}: {got['error']}\n{got.get('stderr_tail', '')}")
    if got["returncode"] != 0:
        raise ProbeError(f"t={t}: scene-render exit {got['returncode']}:\n{perf_common.tail(got['stderr'])}")
    parsed = parse_stats(got["stderr"])
    if parsed is None:
        raise ProbeError(f"t={t}: no --stats line in the renderer's output:\n{perf_common.tail(got['stderr'])}")
    stats = parsed["stats"]
    if stats.get("errors"):
        raise ProbeError(f"t={t}: the renderer reported errors: {stats['errors']}")
    with open(png, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    if not args.keep_frames:
        os.remove(png)
    return {
        "time": t,
        "wall_seconds": round(got["wall_seconds"], 3),
        "peak_rss_mib": round(got["peak_rss_mib"], 1),
        "stats": stats,
        "gpu": sum_passes(parsed.get("gpu")),
        "frame_png_sha256": digest,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("scene", nargs="?")
    ap.add_argument("--times", default="0")
    ap.add_argument("--size", help="WxH of the rendered frame (default: the scene's own)")
    ap.add_argument("--json", help="write the report here as well as to standard output")
    ap.add_argument("--binary", default=perf_common.default_binary())
    ap.add_argument("--work", help="directory for the resized copy and frames (default: a new temporary one)")
    ap.add_argument("--agent", default=os.environ.get("SR_AGENT", "probe"))
    ap.add_argument("--short", action="store_true", help="queue each frame as a short job (under about two minutes)")
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--keep-frames", action="store_true")
    ap.add_argument("--timeout", type=float, default=1800.0)
    ap.add_argument("--child", action="store_true", help=argparse.SUPPRESS)
    ap.add_argument("--out", help=argparse.SUPPRESS)
    ap.add_argument("--time", type=float, help=argparse.SUPPRESS)
    args = ap.parse_args()
    if args.child:
        return child(args)
    if not args.scene:
        ap.error("SCENE is required")
    try:
        times = [float(v) for v in args.times.split(",") if v.strip()]
        if not times:
            raise ValueError("empty")
    except ValueError:
        ap.error("--times must be comma-separated seconds")
    size = None
    if args.size:
        m = re.fullmatch(r"(\d+)x(\d+)", args.size)
        if not m:
            ap.error("--size must be WxH")
        size = (int(m.group(1)), int(m.group(2)))
    if not os.access(SR_GPU, os.X_OK):
        print(f"error: {SR_GPU} not found; the GPU queue is required", file=sys.stderr)
        return 1
    if not os.path.isfile(args.binary):
        print(f"error: scene-render binary not found: {args.binary}", file=sys.stderr)
        return 1
    work = args.work or tempfile.mkdtemp(prefix="probe_render_")
    os.makedirs(work, exist_ok=True)
    try:
        scene = os.path.abspath(args.scene)
        if size:
            scene = resized_copy(scene, size, work)
        frames = []
        for t in times:
            print(f"probing t={t} …", file=sys.stderr)
            frames.append(probe_one(args, scene, t, work))
    except ProbeError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    finally:
        if not args.work and not args.keep_frames:
            shutil.rmtree(work, ignore_errors=True)
    report = {
        "commit": perf_common.commit(),
        "binary": os.path.abspath(args.binary),
        "scene": os.path.abspath(args.scene),
        "size": args.size,
        "frames": frames,
    }
    text = json.dumps(report, indent=2)
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
