#!/usr/bin/env python3
"""Performance suite: benches and encodes the perf fixtures with one scene-render
binary, writes the numbers as JSON and, given an earlier JSON, compares against it.

usage: tools/perf_suite.py OUT.json [--binary PATH] [--baseline BASE.json] [--work DIR]
                           [--beta SCENE] [--adapter-label TEXT] [--quick] [--only NAME,...]
                           [--timeout SECONDS]

Runs, in this order: bench (render --bench --stats) perf_layers, perf_vector, perf_text,
perf_effects and perf_3d over frames 0..60 and perf_physics over 60..120; encode
(--hw software --end 5) perf_video and perf_text; and, with --beta SCENE, bench 0..60
and 3000..3060, encode --start 100 --end 105 and the audio-only mix of that scene.
--quick benches 12 frames and encodes one second. --only takes fixture names
(perf_text, beta) or run keys (perf_text_bench, beta_mix).

Fixtures are reused from WORK/<name>/ (WORK defaults to /tmp/perf_suite) or /tmp/<name>/,
otherwise generated with tools/<name>.py. Every run gets SR_GPU_BACKEND=vulkan unless
the variable is already set. The binary defaults to $SCENE_RENDER, the workspace's
target/release build, or scene-render on PATH.

OUT.json is {"meta": {...}, "fixtures": {"<key>": {"metric": ..., "value": ..., ...}}}.
"value" is the median frame time in ms for a bench (metric "median_ms"), the encode
wall time in seconds for an encode ("seconds") and audio_seconds for the mix
("audio_seconds"); a run that failed records "error" instead of "value" and the suite
carries on. A bench also records p95_ms, cpu_ms, eval_ms, fps, adapter and the last
frame's stats (draws, targets, fx_passes, backdrop_copies, vector_seconds, subframes);
an encode records frames, render_fps, stage_seconds (evaluate, render, readback_wait,
encoder_blocked), decode_wait_seconds, vector_seconds and audio_seconds. Every run
records wall_seconds and its command line.

With --baseline: prints baseline, new and ratio = baseline / new per fixture (above 1
is faster) and the geometric mean of the ratios over the fixtures present in both;
a ratio below 0.95 is flagged REGRESSION and makes the exit code 1.
"""
import argparse
import json
import math
import os
import platform
import re
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone

import perf_common as pc

REGRESSION_RATIO = 0.95
BENCH_SCENES = ["perf_layers", "perf_vector", "perf_text", "perf_effects", "perf_3d"]
STAT_KEYS = ["draws", "targets", "fx_passes", "backdrop_copies", "vector_seconds", "subframes"]
STAGES = ["evaluate", "render", "readback_wait", "encoder_blocked"]
NUM = r"([0-9]+(?:\.[0-9]+)?)"


class RunError(Exception):
    def __init__(self, message, wall=None):
        super().__init__(message)
        self.wall = wall


def plan(quick, beta):
    n = 12 if quick else 60
    end = 1.0 if quick else 5.0
    runs = [dict(key=f"{s}_bench", scene=s, kind="bench", frames=(0, n)) for s in BENCH_SCENES]
    runs.append(dict(key="perf_physics_bench", scene="perf_physics", kind="bench", frames=(60, 60 + n)))
    runs.append(dict(key="perf_video_encode", scene="perf_video", kind="encode", start=None, end=end))
    runs.append(dict(key="perf_text_encode", scene="perf_text", kind="encode", start=None, end=end))
    if beta:
        runs += [dict(key="beta_bench_0", scene="beta", kind="bench", frames=(0, n)),
                 dict(key="beta_bench_3000", scene="beta", kind="bench", frames=(3000, 3000 + n)),
                 dict(key="beta_encode", scene="beta", kind="encode", start=100.0, end=100.0 + end),
                 dict(key="beta_mix", scene="beta", kind="mix")]
    return runs


def progress(msg):
    print(msg, file=sys.stderr, flush=True)


def execute(cmd, env, timeout):
    t0 = time.monotonic()
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise RunError(f"timeout after {timeout:.0f} s", time.monotonic() - t0)
    except OSError as e:
        raise RunError(f"cannot run {cmd[0]}: {e}", time.monotonic() - t0)
    wall = time.monotonic() - t0
    if r.returncode != 0:
        raise RunError(f"exit {r.returncode}: {pc.tail(r.stderr or r.stdout)}", wall)
    return r, wall


def adapter_of(text):
    m = re.search(r"\b[0-9]+ frames? on (.+?)\s*$", text or "", re.M)
    return m.group(1) if m else None


def last_json(text, key):
    """The last line of TEXT that is a JSON object with KEY, or None."""
    for line in reversed((text or "").splitlines()):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            continue
        if isinstance(obj, dict) and key in obj:
            return obj
    return None


def parse_bench(stdout, stderr):
    m = re.search(rf"render: median {NUM} ms(?: \({NUM} fps\))?, p95 {NUM} ms", stdout)
    if not m:
        raise RunError("no 'render: median' line in the output: " + pc.tail(stdout + "\n" + stderr))
    rec = {"median_ms": float(m.group(1)), "fps": float(m.group(2)) if m.group(2) else None,
           "p95_ms": float(m.group(3))}
    m = re.search(rf"CPU planning and submission {NUM} ms", stdout)
    rec["cpu_ms"] = float(m.group(1)) if m else None
    m = re.search(rf"evaluation {NUM} ms", stdout)
    rec["eval_ms"] = float(m.group(1)) if m else None
    rec["adapter"] = adapter_of(stdout)
    frame = last_json(stderr, "stats")
    if frame is not None and isinstance(frame["stats"], dict):
        rec["stats"] = {k: frame["stats"].get(k) for k in STAT_KEYS}
    else:  # no --stats lines: fall back to the summary line
        m = re.search(r"last frame: (\d+) draws, (\d+) targets, .*?(\d+) backdrop copies, (\d+) effect passes, "
                      r"(\d+) sub-frames", stdout)
        rec["stats"] = ({"draws": int(m.group(1)), "targets": int(m.group(2)), "fx_passes": int(m.group(4)),
                         "backdrop_copies": int(m.group(3)), "vector_seconds": None, "subframes": int(m.group(5))}
                        if m else {k: None for k in STAT_KEYS})
    return rec


def parse_encode(stdout, stderr):
    j = last_json(stdout, "seconds")
    if j is None:
        raise RunError("no JSON report on stdout: " + pc.tail(stdout + "\n" + stderr))
    stages = j.get("stage_seconds") or []
    return {"seconds": j["seconds"], "frames": j.get("frames"), "render_fps": j.get("render_fps"),
            "stage_seconds": {k: (stages[i] if i < len(stages) else None) for i, k in enumerate(STAGES)},
            "decode_wait_seconds": j.get("decode_wait_seconds"), "vector_seconds": j.get("vector_seconds"),
            "audio_seconds": j.get("audio_seconds"), "encoder": j.get("encoder")}


def run_one(run, scene, binary, env, timeout, out_root):
    kind = run["kind"]
    rec = {"kind": kind, "scene": scene}
    if kind == "bench":
        a, b = run["frames"]
        rec["metric"], rec["frames"] = "median_ms", f"{a}..{b}"
        cmd = [binary, "render", scene, "--bench", "--stats", "--frames", f"{a}..{b}"]
    else:
        out_dir = os.path.join(out_root, run["key"])
        os.makedirs(out_dir, exist_ok=True)
        if kind == "encode":
            rec["metric"], rec["start"], rec["end"] = "seconds", run["start"], run["end"]
            cmd = [binary, "encode", scene, "--hw", "software", "--out-dir", out_dir, "--json", "--end", str(run["end"])]
            if run["start"] is not None:
                cmd += ["--start", str(run["start"])]
        else:
            rec["metric"] = "audio_seconds"
            cmd = [binary, "encode", scene, "--hw", "software", "-o", os.path.join(out_dir, "mix.wav"), "--json"]
    rec["command"] = cmd
    try:
        r, wall = execute(cmd, env, timeout)
        rec["wall_seconds"] = round(wall, 3)
        if kind == "bench":
            rec.update(parse_bench(r.stdout, r.stderr))
            rec["value"] = rec["median_ms"]
        else:
            rec.update(parse_encode(r.stdout, r.stderr))
            rec["value"] = rec["seconds"] if kind == "encode" else rec["audio_seconds"]
        if rec["value"] is None:
            raise RunError(f"the report has no {rec['metric']}", wall)
    except RunError as e:
        rec["error"] = str(e)
        if e.wall is not None:
            rec["wall_seconds"] = round(e.wall, 3)
    return rec


def write_json(path, results):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(results, f, indent=1, sort_keys=False)
        f.write("\n")
    os.replace(tmp, path)


def fmt(v):
    return "-" if v is None else f"{v:.3f}"


def detail_of(rec):
    if "error" in rec:
        return "error: " + rec["error"].strip().splitlines()[0][:100]
    if rec["kind"] == "bench":
        s = rec["stats"]
        return (f"p95 {fmt(rec['p95_ms'])} ms, cpu {fmt(rec['cpu_ms'])} ms, eval {fmt(rec['eval_ms'])} ms; "
                f"{s['draws']} draws, {s['targets']} targets, {s['fx_passes']} fx passes, "
                f"{s['backdrop_copies']} backdrop copies, {s['subframes']} sub-frames")
    if rec["kind"] == "encode":
        st = rec["stage_seconds"]
        return (f"{rec['frames']} frames at {fmt(rec['render_fps'])} fps; evaluate {fmt(st['evaluate'])} s, "
                f"render {fmt(st['render'])} s, readback {fmt(st['readback_wait'])} s, "
                f"encoder blocked {fmt(st['encoder_blocked'])} s, audio {fmt(rec['audio_seconds'])} s")
    return f"whole programme; {fmt(rec['seconds'])} s in all"


def print_table(results):
    print(f"{'fixture':<22} {'metric':<14} {'value':>10} {'wall s':>8}  detail")
    for key, rec in results["fixtures"].items():
        value = "error" if "error" in rec else fmt(rec["value"])
        wall = rec.get("wall_seconds")
        print(f"{key:<22} {rec['metric']:<14} {value:>10} {(f'{wall:.1f}' if wall is not None else '-'):>8}  {detail_of(rec)}")


def compare(base, new):
    """Prints the comparison table; returns (regressions, geometric mean or None)."""
    bf, nf = base.get("fixtures", {}), new["fixtures"]
    keys = list(nf) + [k for k in bf if k not in nf]
    print(f"{'fixture':<22} {'metric':<14} {'baseline':>10} {'new':>10} {'ratio':>8}  note")
    ratios, regressions = [], []
    for k in keys:
        b, n = bf.get(k), nf.get(k)
        bv = b.get("value") if b else None
        nv = n.get("value") if n else None
        metric = (n or b).get("metric", "")
        ratio, note = None, ""
        if b is None:
            note = "not in baseline"
        elif n is None:
            note = "not run"
        elif bv is None or nv is None or nv <= 0 or bv <= 0:
            note = "error: " + ((n.get("error") or b.get("error") or "no value").strip().splitlines()[0][:80])
        else:
            ratio = bv / nv
            ratios.append(ratio)
            if ratio < REGRESSION_RATIO:
                regressions.append((k, ratio))
                note = "REGRESSION"
        print(f"{k:<22} {metric:<14} {fmt(bv):>10} {fmt(nv):>10} {(f'{ratio:.3f}' if ratio else '-'):>8}  {note}")
    geomean = math.exp(sum(math.log(r) for r in ratios) / len(ratios)) if ratios else None
    if geomean is not None:
        print(f"geometric mean of ratios over {len(ratios)} fixture{'s' if len(ratios) != 1 else ''}: "
              f"{geomean:.3f} (baseline / new; above 1 is faster)")
    else:
        print("no fixture with a value in both runs")
    for k, r in regressions:
        print(f"REGRESSION: {k} ratio {r:.3f} is below {REGRESSION_RATIO}")
    return regressions, geomean


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", metavar="OUT.json")
    ap.add_argument("--binary", default=pc.default_binary(), help="scene-render binary (default: %(default)s)")
    ap.add_argument("--baseline", metavar="BASE.json", help="earlier OUT.json to compare against")
    ap.add_argument("--work", default=pc.DEFAULT_WORK, help="fixture and output directory (default: %(default)s)")
    ap.add_argument("--beta", metavar="SCENE", help="a real scene to bench, encode and mix as well")
    ap.add_argument("--adapter-label", metavar="TEXT", help="human label of the GPU, recorded in meta")
    ap.add_argument("--quick", action="store_true", help="12 bench frames and one encoded second")
    ap.add_argument("--only", metavar="NAME,...", help="fixture names or run keys to run")
    ap.add_argument("--timeout", type=float, default=3600.0, help="seconds per run (default: %(default)s)")
    args = ap.parse_args()

    only = pc.parse_only(args.only)
    runs = [r for r in plan(args.quick, args.beta) if only is None or r["key"] in only or r["scene"] in only]
    if not runs:
        sys.exit(f"--only {args.only}: nothing to run")
    if args.beta and not os.path.isfile(args.beta):
        sys.exit(f"--beta {args.beta}: no such file")
    env = pc.run_env()
    started = time.monotonic()
    meta = {"schema": 1, "timestamp": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "hostname": socket.gethostname(), "platform": platform.platform(), "commit": pc.commit(),
            "binary": os.path.abspath(args.binary) if os.path.exists(args.binary) else args.binary,
            "version": pc.version(args.binary), "adapter": None, "adapter_label": args.adapter_label,
            "quick": args.quick, "work": os.path.abspath(args.work),
            "beta": os.path.abspath(args.beta) if args.beta else None,
            "env": {"SR_GPU_BACKEND": env.get("SR_GPU_BACKEND")}, "argv": sys.argv[1:], "total_wall_seconds": None}
    results = {"meta": meta, "fixtures": {}}
    progress(f"{meta['version'] or args.binary} at {meta['commit'] or '?'} on {meta['hostname']}"
             f"{' (' + args.adapter_label + ')' if args.adapter_label else ''}: {len(runs)} runs, work {meta['work']}")
    out_root = os.path.join(args.work, "out")
    scenes = {}
    for run in runs:
        key, name = run["key"], run["scene"]
        try:
            if name == "beta":
                scene = os.path.abspath(args.beta)
            elif name in scenes:
                scene = scenes[name]
            else:
                scene = scenes[name] = pc.fixture_scene(args.work, name, log=progress)
        except pc.FixtureError as e:
            results["fixtures"][key] = {"kind": run["kind"], "scene": None, "metric": None, "error": f"fixture: {e}"}
            progress(f"{key}: fixture error: {e}")
            write_json(args.out, results)
            continue
        progress(f"{key}: {run['kind']} {os.path.basename(scene)}"
                 + (f" --frames {run['frames'][0]}..{run['frames'][1]}" if run["kind"] == "bench" else "")
                 + (f" --end {run['end']}" + (f" --start {run['start']}" if run["start"] is not None else "")
                    if run["kind"] == "encode" else ""))
        rec = run_one(run, scene, args.binary, env, args.timeout, out_root)
        results["fixtures"][key] = rec
        if meta["adapter"] is None and rec.get("adapter"):
            meta["adapter"] = rec["adapter"]
        progress(f"{key}: " + ("error: " + rec["error"].strip().splitlines()[0] if "error" in rec
                               else f"{rec['metric']} {fmt(rec['value'])} in {rec['wall_seconds']:.1f} s"))
        write_json(args.out, results)
    meta["total_wall_seconds"] = round(time.monotonic() - started, 3)

    regressions = []
    if args.baseline:
        with open(args.baseline) as f:
            base = json.load(f)
        regressions, geomean = compare(base, results)
        meta["comparison"] = {"baseline": os.path.abspath(args.baseline), "geomean": geomean,
                              "regressions": [k for k, _ in regressions], "threshold": REGRESSION_RATIO}
    else:
        print_table(results)
    write_json(args.out, results)
    failed = [k for k, r in results["fixtures"].items() if "error" in r]
    if failed:
        print(f"failed runs: {', '.join(failed)}")
    print(f"wrote {args.out}")
    sys.exit(1 if regressions else 0)


if __name__ == "__main__":
    main()
