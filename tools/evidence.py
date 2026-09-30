#!/usr/bin/env python3
"""Reference-clip evidence harness.

Every case in tools/evidence/cases.json probes one technique a reference clip depends on. The
harness renders the case's scene with `scene-render render --strict` at the times its checks
name, runs the pixel checks and classifies the result:

  native    rendered as authored (strict passes) and every check passes
  degraded  the checks pass, but the renderer reported a fallback (strict fails)
  wrong     a check fails: the frame differs from what the scene asks for
  error     the scene did not render at all
  gap       registered technique with no probe scene: the engine has no feature for it

The outcome is compared with the case's `expect`; any difference, better or worse, fails the
run so the manifest stays an accurate record. Output: report.json, report.md and
contact-sheet.png in the output directory.

Usage: tools/evidence.py [OUT_DIR] [--bin PATH] [--case ID]... [--bench]
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import time

import numpy as np
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.join(ROOT, "tools", "evidence")
PLACEHOLDERS = {
    "{MEDIA}": os.path.join(ROOT, "tests", "corpus", "media"),
    "{SHADERS}": os.path.join(ROOT, "crates", "sr-gpu", "tests", "shaders"),
    "{FIXTURES3D}": os.path.join(ROOT, "crates", "sr-3d", "tests", "fixtures"),
    "{STILLS}": os.path.join(ROOT, "crates", "sr-media", "tests", "fixtures", "still"),
    "{GEO}": os.path.join(ROOT, "crates", "sr-geo", "tests", "fixtures"),
}
OUTCOMES = ("native", "degraded", "wrong", "error", "gap")


# ---------------------------------------------------------------- generated assets


def splat_ring(path):
    """A torus of 4000 Gaussians (antimatter15 `.splat`: 32 bytes each), hue around the ring."""
    rng = np.random.default_rng(7)
    rec = bytearray()
    for _ in range(4000):
        u, v = rng.uniform(0, 2 * math.pi, 2)
        r = 1.0 + 0.3 * math.cos(v)
        pos = (r * math.cos(u), 0.3 * math.sin(v), r * math.sin(u))
        rgb = [int(255 * (0.5 + 0.5 * math.cos(u + k * 2.094))) for k in range(3)]
        rec += struct.pack("<3f3f", *pos, 0.04, 0.04, 0.04)
        rec += bytes(rgb + [255, 255, 128, 128, 128])  # rgba, then quaternion w x y z (identity)
    with open(path, "wb") as f:
        f.write(rec)


def gltf_walker(path):
    """A 1 m orange box with one animation clip, "walk", translating it ±1 m on x over 2 s."""
    pos, nrm, idx = [], [], []
    for axis in range(3):
        for sign in (-1.0, 1.0):
            n = [0.0, 0.0, 0.0]
            n[axis] = sign
            a, b = [(axis + 1) % 3, (axis + 2) % 3]
            base = len(pos)
            for s, t in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
                p = [0.0, 0.0, 0.0]
                p[axis], p[a], p[b] = 0.5 * sign, 0.5 * s, 0.5 * t
                pos.append(p)
                nrm.append(n)
            quad = [0, 1, 2, 0, 2, 3] if sign > 0 else [0, 2, 1, 0, 3, 2]
            idx += [base + q for q in quad]
    times = [0.0, 1.0, 2.0]
    moves = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]]
    chunks = [
        np.array(pos, "<f4").tobytes(),
        np.array(nrm, "<f4").tobytes(),
        np.array(idx, "<u2").tobytes(),
        np.array(times, "<f4").tobytes(),
        np.array(moves, "<f4").tobytes(),
    ]
    views, blob = [], b""
    for c in chunks:
        blob += b"\0" * (-len(blob) % 4)
        views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(c)})
        blob += c
    blob += b"\0" * (-len(blob) % 4)
    doc = {
        "asset": {"version": "2.0", "generator": "rs-scene-render tools/evidence.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "body", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.45, 0.1, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6}}],
        "buffers": [{"byteLength": len(blob)}],
        "bufferViews": views,
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3",
             "min": [-0.5] * 3, "max": [0.5] * 3},
            {"bufferView": 1, "componentType": 5126, "count": len(nrm), "type": "VEC3"},
            {"bufferView": 2, "componentType": 5123, "count": len(idx), "type": "SCALAR"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "SCALAR", "min": [0.0], "max": [2.0]},
            {"bufferView": 4, "componentType": 5126, "count": 3, "type": "VEC3"},
        ],
        "animations": [{
            "name": "walk",
            "samplers": [{"input": 3, "output": 4, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}],
        }],
    }
    js = json.dumps(doc, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    body = struct.pack("<II", len(js), 0x4E4F534A) + js + struct.pack("<II", len(blob), 0x004E4942) + blob
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, 12 + len(body)) + body)


def ocio_stub(path):
    """A minimal OCIO v2 config; its content does not matter while no OCIO library is linked."""
    with open(path, "w") as f:
        f.write("ocio_profile_version: 2\nroles:\n  default: raw\ncolorspaces:\n  - !<ColorSpace>\n    name: raw\n    isdata: true\n")


GENERATORS = {"splat_ring": splat_ring, "gltf_walker": gltf_walker, "ocio_stub": ocio_stub}


# ---------------------------------------------------------------- checks


def rgb(hexstr):
    h = hexstr.lstrip("#")
    return np.array([int(h[k:k + 2], 16) / 255.0 for k in (0, 2, 4)])


def region(px, r):
    x, y, w, h = r
    return px[y:y + h, x:x + w, :3]


def run_check(c, frames):
    """Returns (passed, measured) for one check against the rendered frames {t: HxWx4 float}."""
    kind = c["type"]
    if kind in ("color", "not_color"):
        got = frames[c["t"]][c["y"], c["x"], :3]
        dist = float(np.abs(got - rgb(c["rgb"])).max())
        near = dist <= c["tol"]
        return (near if kind == "color" else not near), "#%02X%02X%02X" % tuple(int(round(v * 255)) for v in got)
    if kind == "coverage":
        px = region(frames[c["t"]], c["region"])
        target = c.get("rgb") or c["not_rgb"]
        near = np.abs(px - rgb(target)).max(axis=2) <= c["tol"]
        frac = float((near if "rgb" in c else ~near).mean())
        ok = frac >= c.get("min", 0.0) and frac <= c.get("max", 1.0)
        return ok, round(frac, 4)
    if kind == "edge_shift":
        # the lowest row of `rgb` per column, inside vs outside a column range: refraction moves an edge
        px = frames[c["t"]][:, :, :3]
        near = np.abs(px - rgb(c["rgb"])).max(axis=2) <= c["tol"]

        def edge(cols):
            rows = [np.nonzero(near[:, x])[0] for x in range(*cols)]
            return float(np.median([r.max() for r in rows if len(r)])) if any(len(r) for r in rows) else float("nan")
        shift = abs(edge(c["inside"]) - edge(c["outside"]))
        return shift >= c["min"], round(shift, 2)
    if kind == "changes":
        a, b = (region(frames[t], c["region"]) for t in c["t"])
        delta = float(np.abs(a - b).mean())
        return c.get("min", 0.0) <= delta <= c.get("max", float("inf")), round(delta, 4)
    raise ValueError(f"unknown check type {kind}")


def check_times(case):
    ts = []
    for c in case.get("checks", []):
        for t in c["t"] if isinstance(c["t"], list) else [c["t"]]:
            if t not in ts:
                ts.append(t)
    return sorted(ts)


# ---------------------------------------------------------------- running


def prepare(case, work):
    os.makedirs(work, exist_ok=True)
    for name, gen in case.get("generate", {}).items():
        GENERATORS[gen](os.path.join(work, name))
    with open(os.path.join(HERE, case["scene"])) as f:
        text = f.read()
    for k, v in PLACEHOLDERS.items():
        text = text.replace(k, v)
    scene = os.path.join(work, os.path.basename(case["scene"]))
    with open(scene, "w") as f:
        f.write(text)
    return scene


def render(binary, scene, t, png):
    if os.path.exists(png):
        os.remove(png)
    r = subprocess.run([binary, "render", scene, "--strict", "--time", str(t), "-o", png],
                       capture_output=True, text=True)
    notes = [ln.strip() for ln in (r.stdout + r.stderr).splitlines()
             if re.match(r"\s*(note|warning|error)\b", ln) and "--strict" not in ln]
    strict_failed = r.returncode == 1 and "--strict" in r.stderr
    ok = r.returncode == 0 or (strict_failed and os.path.isfile(png))
    return ok, strict_failed, notes, (r.stdout + r.stderr).strip()


def bench(binary, scene):
    r = subprocess.run([binary, "render", scene, "--bench", "--frames", "0..48"], capture_output=True, text=True)
    m = re.search(r"render: median ([0-9.]+) ms", r.stdout)
    return float(m.group(1)) if m else None


def run_case(case, binary, out, do_bench):
    res = {k: case.get(k) for k in ("id", "reference", "technique", "expect", "known")}
    if not case.get("scene"):
        res.update(outcome="gap", checks=[], notes=[], stills=[])
        return res
    work = os.path.join(out, case["id"])
    scene = prepare(case, work)
    frames, notes, degraded, stills = {}, [], False, []
    for t in check_times(case) or [0.0]:
        png = os.path.join(work, f"t{t:07.4f}.png")
        ok, strict_failed, n, log = render(binary, scene, t, png)
        with open(png + ".log", "w") as f:
            f.write(log + "\n")
        if not ok:
            res.update(outcome="error", checks=[], notes=[log[-2000:]], stills=[])
            return res
        degraded |= strict_failed
        notes += [x for x in n if x not in notes]
        frames[t] = np.asarray(Image.open(png).convert("RGBA"), dtype=np.float64) / 255.0
        stills.append(os.path.relpath(png, out))
    checks = []
    for c in case.get("checks", []):
        passed, measured = run_check(c, frames)
        checks.append({**c, "passed": passed, "measured": measured})
    if not all(c["passed"] for c in checks):
        outcome = "wrong"
    else:
        outcome = "degraded" if degraded else "native"
    res.update(outcome=outcome, checks=checks, notes=notes, stills=stills)
    if do_bench and case.get("bench"):
        res["bench_median_ms"] = bench(binary, scene)
    return res


# ---------------------------------------------------------------- reports


def contact_sheet(results, out):
    rows = [r for r in results if r["stills"]]
    if not rows:
        return None
    tw, th, label = 256, 144, 18
    cols = max(len(r["stills"]) for r in rows)
    sheet = Image.new("RGB", (8 + cols * (tw + 8) + 200, 8 + len(rows) * (th + label + 8)), (24, 24, 28))
    d = ImageDraw.Draw(sheet)
    for i, r in enumerate(rows):
        y = 8 + i * (th + label + 8)
        d.text((8, y), f"{r['id']}  [{r['outcome']}]", fill=(230, 230, 230))
        for j, s in enumerate(r["stills"]):
            im = Image.open(os.path.join(out, s)).convert("RGB").resize((tw, th))
            sheet.paste(im, (8 + j * (tw + 8), y + label))
            d.text((12 + j * (tw + 8), y + label + 4), s.split("/")[-1][1:-4] + " s", fill=(255, 255, 255))
    path = os.path.join(out, "contact-sheet.png")
    sheet.save(path)
    return path


def markdown(report):
    lines = [
        "# Reference-clip evidence",
        "",
        f"Binary: `{report['binary']}` · backend: `{report['backend']}` · {report['date']}",
        "",
        "| Outcome | Count |",
        "|---|---|",
    ]
    for o in OUTCOMES:
        lines.append(f"| {o} | {report['counts'].get(o, 0)} |")
    lines += ["", "| Case | Reference | Outcome | Expected | Evidence |", "|---|---|---|---|---|"]
    for r in report["results"]:
        ev = []
        for c in r["checks"]:
            ev.append(("✓ " if c["passed"] else "✗ ") + f"{c['why']} ({c['measured']})")
        ev += r["notes"]
        if r.get("clip"):
            ev.append(f"[Full clip]({r['clip']}); manual review: {r.get('review', '')}")
        if r.get("known"):
            ev.append(f"known: {r['known']}")
        if r.get("bench_median_ms") is not None:
            ev.append(f"bench: median {r['bench_median_ms']} ms/frame")
        mark = "" if r["outcome"] == r["expect"] else " ⚠"
        lines.append(f"| `{r['id']}` | {r['reference']} | **{r['outcome']}**{mark} | {r['expect']} | "
                     + "<br>".join(e.replace("|", "\\|") for e in ev) + " |")
    if report["mismatches"]:
        lines += ["", "Outcomes that differ from the manifest: " + ", ".join(f"`{m}`" for m in report["mismatches"])]
    return "\n".join(lines) + "\n"


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", nargs="?", default="evidence-out")
    parser.add_argument("--bin", default=os.path.join(ROOT, "target", "release", "scene-render"))
    parser.add_argument("--manifest", default=os.path.join(HERE, "cases.json"))
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--bench", action="store_true")
    parser.add_argument("--clips", action="store_true", help="encode complete review scenes as MP4")
    args = parser.parse_args(argv)
    out, binary, do_bench = args.out, args.bin, args.bench
    with open(args.manifest) as f:
        cases = json.load(f)["cases"]
    unknown = set(args.case) - {c["id"] for c in cases}
    if unknown:
        parser.error("unknown cases: " + ", ".join(sorted(unknown)))
    if args.case:
        cases = [c for c in cases if c["id"] in args.case]
    if not cases:
        parser.error("the manifest selects no cases")
    for c in cases:
        if c.get("scene"):
            c["scene"] = os.path.abspath(os.path.join(os.path.dirname(args.manifest), c["scene"]))
    if os.path.isdir(out):
        shutil.rmtree(out)
    os.makedirs(out)
    results = []
    for case in cases:
        r = run_case(case, binary, out, do_bench)
        mark = "" if r["outcome"] == r["expect"] else f"  (expected {r['expect']})"
        print(f"{r['outcome']:9} {r['id']}{mark}", flush=True)
        if args.clips and case.get("review") and r["outcome"] in ("native", "degraded"):
            scene = os.path.join(out, case["id"], os.path.basename(case["scene"]))
            clip = os.path.join(out, case["id"], "review.mp4")
            enc = subprocess.run([binary, "encode", scene, "-o", clip, "--hw", "software", "--parallel", "1", "--strict", "--no-upload"], capture_output=True, text=True)
            with open(os.path.join(out, case["id"], "encode.log"), "w") as f:
                f.write(enc.stdout + enc.stderr)
            if enc.returncode:
                r["outcome"] = "error"
                r["notes"].append("review clip failed: " + enc.stderr[-2000:])
            else:
                r["clip"] = os.path.relpath(clip, out)
            r["review"] = case["review"]
        results.append(r)
    counts = {o: sum(r["outcome"] == o for r in results) for o in OUTCOMES}
    with open(binary, "rb") as f:
        binary_sha256 = hashlib.sha256(f.read()).hexdigest()
    report = {
        "binary": binary,
        "commit": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
        "binary_sha256": binary_sha256,
        "source_dirty": subprocess.run(["git", "diff", "--quiet", "HEAD"], cwd=ROOT).returncode != 0,
        "renderer_version": subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.strip(),
        "backend": os.environ.get("SR_GPU_BACKEND", "auto"),
        "date": time.strftime("%Y-%m-%d"),
        "counts": counts,
        "mismatches": [r["id"] for r in results if r["outcome"] != r["expect"]],
        "results": results,
    }
    with open(os.path.join(out, "report.json"), "w") as f:
        json.dump(report, f, indent=2)
    with open(os.path.join(out, "report.md"), "w") as f:
        f.write(markdown(report))
    contact_sheet(results, out)
    print(" ".join(f"{o}={counts[o]}" for o in OUTCOMES), f"-> {out}/report.md")
    return 1 if report["mismatches"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
