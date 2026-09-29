#!/usr/bin/env python3
"""Byte-exact oracle: renders the perf fixtures with two scene-render binaries and
compares the decoded pixels of every frame, and the audio mix, byte for byte.

usage: tools/equivalence.py --baseline BIN --candidate BIN [--work DIR] [--frames N]
                            [--only NAME,...] [--beta SCENE] [--keep] [--jobs N]

For each of perf_layers, perf_vector, perf_text, perf_effects, perf_3d, perf_physics
and perf_video (and the --beta scene) both binaries render N frames (default 12;
--frames 3 for a smoke run) as 16-bit PNG: frames 0..N, except perf_physics 60..60+N
and beta 0..N plus 3000..3000+N. The PNGs are decoded here (zlib and PNG filter
reconstruction; 8/16-bit grey, RGB and RGBA, no interlace) and the SHA-256 of the raw
decoded pixel bytes is compared frame by frame. perf_video and beta also produce the
audio-only mix (encode -o mix.wav) with both binaries; the SHA-256 of the WAV data
chunk is compared, header and other chunks ignored.

Prints one line per fixture: IDENTICAL, or DIFFERS with the first differing frame, its
number of differing pixels and the largest per-channel difference, or ERROR when a
render failed. Exit code 0 when everything is identical, 1 otherwise. Fixtures are
reused from WORK/<name>/ (WORK defaults to /tmp/perf_suite) or /tmp/<name>/, else
generated with tools/<name>.py. Rendered files go under WORK/equivalence/<name>/ and
are deleted when identical unless --keep; differing or failed fixtures are kept.
Every run gets SR_GPU_BACKEND=vulkan unless the variable is already set.
"""
import argparse
import concurrent.futures
import hashlib
import operator
import os
import shutil
import struct
import subprocess
import sys
import zlib
from array import array

import perf_common as pc

SIG = b"\x89PNG\r\n\x1a\n"
CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}
COLOUR = {0: "grey", 2: "RGB", 4: "grey+alpha", 6: "RGBA"}
MIXED = {"perf_video", "beta"}  # fixtures whose audio mix is compared too


class ToolError(Exception):
    pass


# --- PNG decoding -------------------------------------------------------------
# Sub and Up filters are reconstructed with big-integer arithmetic on 16-bit lanes
# (one byte per lane, so lane sums never carry) and Average and Paeth, which are not
# linear, byte by byte.

def _widen(b):
    return b.decode("latin-1").encode("utf-16-le")


def _narrow(s, n):
    return s.to_bytes(2 * n, "little")[0::2]


def _sub(row, bpp, mask):
    n = len(row)
    s = int.from_bytes(_widen(row), "little")
    k = bpp  # prefix sum over every bpp-th byte by doubling; masking keeps each lane mod 256
    while k < n:
        s = (s + (s << (16 * k))) & mask
        k <<= 1
    return _narrow(s, n)


def _up(row, prev, mask):
    s = (int.from_bytes(_widen(row), "little") + int.from_bytes(_widen(prev), "little")) & mask
    return _narrow(s, len(row))


def _average(row, prev, bpp):
    out = bytearray(row)
    for i in range(bpp):
        out[i] = (row[i] + (prev[i] >> 1)) & 255
    for i in range(bpp, len(row)):
        out[i] = (row[i] + ((out[i - bpp] + prev[i]) >> 1)) & 255
    return bytes(out)


def _paeth(row, prev, bpp):
    out = bytearray(row)
    for i in range(bpp):
        out[i] = (row[i] + prev[i]) & 255
    for i in range(bpp, len(row)):
        a, b, c = out[i - bpp], prev[i], prev[i - bpp]
        p = a + b - c
        pa, pb, pc_ = abs(p - a), abs(p - b), abs(p - c)
        if pa <= pb and pa <= pc_:
            out[i] = (row[i] + a) & 255
        elif pb <= pc_:
            out[i] = (row[i] + b) & 255
        else:
            out[i] = (row[i] + c) & 255
    return bytes(out)


def unfilter(raw, height, stride, bpp):
    out = bytearray(height * stride)
    prev = bytes(stride)
    mask = int.from_bytes(b"\xff\x00" * stride, "little")
    view = memoryview(raw)
    for y in range(height):
        at = y * (stride + 1)
        f = raw[at]
        row = view[at + 1:at + 1 + stride].tobytes()
        if f == 0:
            cur = row
        elif f == 1:
            cur = _sub(row, bpp, mask)
        elif f == 2:
            cur = _up(row, prev, mask)
        elif f == 3:
            cur = _average(row, prev, bpp)
        elif f == 4:
            cur = _paeth(row, prev, bpp)
        else:
            raise ToolError(f"row {y}: unknown filter type {f}")
        out[y * stride:(y + 1) * stride] = cur
        prev = cur
    return bytes(out)


def decode_png(path):
    """(meta, pixels): the raw big-endian samples of a non-interlaced 8/16-bit grey/RGB/RGBA PNG."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != SIG:
        raise ToolError(f"{path}: not a PNG")
    pos, idat, hdr = 8, [], None
    while pos + 8 <= len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
        pos += 12 + n
    if hdr is None:
        raise ToolError(f"{path}: no IHDR")
    w, h, depth, ctype, _, _, interlace = hdr
    if depth not in (8, 16) or ctype not in CHANNELS or interlace != 0:
        raise ToolError(f"{path}: unsupported PNG (depth {depth}, colour type {ctype}, interlace {interlace})")
    ch = CHANNELS[ctype]
    bpp = ch * depth // 8
    stride = w * bpp
    raw = zlib.decompress(b"".join(idat))
    if len(raw) != h * (stride + 1):
        raise ToolError(f"{path}: {len(raw)} bytes of image data, expected {h * (stride + 1)}")
    meta = {"width": w, "height": h, "depth": depth, "channels": ch, "colour": COLOUR[ctype]}
    return meta, unfilter(raw, h, stride, bpp)


def hash_png(path):
    meta, pixels = decode_png(path)
    return meta, hashlib.sha256(pixels).hexdigest()


def hash_all(paths, jobs):
    if jobs <= 1 or len(paths) <= 1:
        return [hash_png(p) for p in paths]
    with concurrent.futures.ProcessPoolExecutor(max_workers=min(jobs, len(paths))) as ex:
        return list(ex.map(hash_png, paths))


def pixel_diff(path_a, path_b):
    """(differing pixels, largest per-channel absolute difference, pixels in all)."""
    meta, a = decode_png(path_a)
    _, b = decode_png(path_b)
    n = len(a)
    xor = (int.from_bytes(a, "big") ^ int.from_bytes(b, "big")).to_bytes(n, "big")
    ps = meta["channels"] * meta["depth"] // 8
    fmt = {1: "B", 2: "H", 4: "I", 8: "Q"}.get(ps)
    if fmt:
        lanes = memoryview(xor).cast(fmt).tolist()
        differing = len(lanes) - lanes.count(0)
    else:
        differing = len({i // ps for i, v in enumerate(xor) if v})
    if meta["depth"] == 16:
        sa, sb = array("H"), array("H")
        sa.frombytes(a)
        sb.frombytes(b)
        if sys.byteorder == "little":
            sa.byteswap()
            sb.byteswap()
    else:
        sa, sb = a, b
    largest = max(map(abs, map(operator.sub, sa, sb)))
    return differing, largest, meta["width"] * meta["height"]


# --- WAV ---------------------------------------------------------------------------

def wav_data(path):
    """(data chunk bytes, fmt dict) of a RIFF/WAVE or RF64 file."""
    with open(path, "rb") as f:
        d = f.read()
    if d[:4] not in (b"RIFF", b"RF64") or d[8:12] != b"WAVE":
        raise ToolError(f"{path}: not a WAVE file")
    pos, fmt, ds64_data = 12, None, None
    while pos + 8 <= len(d):
        cid = d[pos:pos + 4]
        n = int.from_bytes(d[pos + 4:pos + 8], "little")
        if cid == b"ds64":
            ds64_data = int.from_bytes(d[pos + 16:pos + 24], "little")
        elif cid == b"fmt ":
            body = d[pos + 8:pos + 8 + n]
            fmt = {"channels": int.from_bytes(body[2:4], "little"), "rate": int.from_bytes(body[4:8], "little"),
                   "block_align": int.from_bytes(body[12:14], "little"), "bits": int.from_bytes(body[14:16], "little")}
        elif cid == b"data":
            if n == 0xFFFFFFFF and ds64_data is not None:
                n = ds64_data
            n = min(n, len(d) - pos - 8)
            return d[pos + 8:pos + 8 + n], fmt or {}
        pos += 8 + n + (n & 1)
    raise ToolError(f"{path}: no data chunk")


# --- runs ----------------------------------------------------------------------------

def run(cmd, env):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, env=env)
    except OSError as e:
        raise ToolError(f"cannot run {cmd[0]}: {e}")
    if r.returncode != 0:
        raise ToolError(f"{os.path.basename(cmd[0])} {cmd[1]} exited {r.returncode}: {pc.tail(r.stderr or r.stdout, 6)}")
    return r


def render(binary, scene, ranges, out_dir, env):
    os.makedirs(out_dir, exist_ok=True)
    for a, b in ranges:
        run([binary, "render", scene, "--frames", f"{a}..{b}", "--bit-depth", "16",
             "-o", os.path.join(out_dir, "f_%04d.png")], env)


def mix(binary, scene, out_dir, env):
    os.makedirs(out_dir, exist_ok=True)
    wav = os.path.join(out_dir, "mix.wav")
    run([binary, "encode", scene, "--hw", "software", "-o", wav, "--json"], env)
    if not os.path.isfile(wav):
        raise ToolError(f"{wav} was not written")
    return wav


def frame_no(name):
    try:
        return int(name[2:-4])
    except ValueError:
        return name


def compare_frames(base_dir, cand_dir, jobs):
    names_b = sorted(f for f in os.listdir(base_dir) if f.endswith(".png"))
    names_c = sorted(f for f in os.listdir(cand_dir) if f.endswith(".png"))
    if names_b != names_c:
        return "DIFFERS", f"frame files differ: baseline wrote {len(names_b)}, candidate {len(names_c)}"
    if not names_b:
        return "ERROR", "no frames written"
    hb = hash_all([os.path.join(base_dir, n) for n in names_b], jobs)
    hc = hash_all([os.path.join(cand_dir, n) for n in names_c], jobs)
    for name, (mb, db), (mc, dc) in zip(names_b, hb, hc):
        if mb != mc:
            return "DIFFERS", (f"first differing frame {frame_no(name)}: format differs, baseline {mb['width']}x{mb['height']} "
                               f"{mb['colour']} {mb['depth']}-bit, candidate {mc['width']}x{mc['height']} {mc['colour']} {mc['depth']}-bit")
        if db != dc:
            differing, largest, total = pixel_diff(os.path.join(base_dir, name), os.path.join(cand_dir, name))
            return "DIFFERS", (f"first differing frame {frame_no(name)}: {differing} of {total} pixels differ, "
                               f"max per-channel difference {largest} of {(1 << mb['depth']) - 1}")
    m = hb[0][0]
    return "IDENTICAL", f"{len(names_b)} frames, {m['width']}x{m['height']} {m['colour']} {m['depth']}-bit"


def compare_mix(base_wav, cand_wav):
    db, fb = wav_data(base_wav)
    dc, fc = wav_data(cand_wav)
    what = (f"{fb.get('channels')} ch, {fb.get('rate')} Hz, {fb.get('bits')}-bit" if fb else "unknown format")
    if fb != fc:
        return "DIFFERS", f"format differs: baseline {fb}, candidate {fc}"
    if hashlib.sha256(db).digest() == hashlib.sha256(dc).digest():
        return "IDENTICAL", f"data chunk {len(db)} bytes ({what})"
    if len(db) != len(dc):
        return "DIFFERS", f"data chunk length differs: baseline {len(db)} bytes, candidate {len(dc)} bytes ({what})"
    xor = (int.from_bytes(db, "big") ^ int.from_bytes(dc, "big")).to_bytes(len(db), "big")
    first = len(xor) - len(xor.lstrip(b"\x00"))
    nbytes = len(xor) - xor.count(0)
    align, rate = fb.get("block_align") or 1, fb.get("rate") or 0
    at = f"sample frame {first // align}" + (f" ({first // align / rate:.3f} s)" if rate else "")
    return "DIFFERS", f"{nbytes} of {len(db)} data bytes differ, first at {at} ({what})"


def report(name, status, detail):
    print(f"{name:<16} {status:<9} {detail}", flush=True)


def check(name, scene, ranges, args, env):
    root = os.path.join(args.work, "equivalence", name)
    shutil.rmtree(root, ignore_errors=True)
    sides = (("baseline", args.baseline), ("candidate", args.candidate))
    try:
        for side, binary in sides:
            render(binary, scene, ranges, os.path.join(root, side), env)
        status, detail = compare_frames(os.path.join(root, "baseline"), os.path.join(root, "candidate"), args.jobs)
    except ToolError as e:
        status, detail = "ERROR", str(e)
    report(name, status, detail)
    ok = status == "IDENTICAL"
    if name in MIXED:
        try:
            wavs = [mix(binary, scene, os.path.join(root, side + "_mix"), env) for side, binary in sides]
            status, detail = compare_mix(*wavs)
        except ToolError as e:
            status, detail = "ERROR", str(e)
        report(name + " mix", status, detail)
        ok = ok and status == "IDENTICAL"
    if ok and not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    else:
        report("", "", f"kept {root}")
    return ok


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--baseline", required=True, metavar="BIN", help="reference scene-render binary")
    ap.add_argument("--candidate", required=True, metavar="BIN", help="scene-render binary under test")
    ap.add_argument("--work", default=pc.DEFAULT_WORK, help="fixture and output directory (default: %(default)s)")
    ap.add_argument("--frames", type=int, default=12, help="frames per range (default: %(default)s)")
    ap.add_argument("--only", metavar="NAME,...", help="fixture names to check (perf_text, beta, ...)")
    ap.add_argument("--beta", metavar="SCENE", help="a real scene to check as well")
    ap.add_argument("--keep", action="store_true", help="keep the rendered files of identical fixtures too")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1, help="PNG decoders in parallel (default: %(default)s)")
    args = ap.parse_args()
    if args.frames < 1:
        sys.exit("--frames must be at least 1")
    only = pc.parse_only(args.only)
    names = [n for n in pc.FIXTURES + (["beta"] if args.beta else []) if only is None or n in only]
    if not names:
        sys.exit(f"--only {args.only}: nothing to check")
    if args.beta and not os.path.isfile(args.beta):
        sys.exit(f"--beta {args.beta}: no such file")
    env = pc.run_env()
    n = args.frames
    all_ok = True
    for name in names:
        if name == "beta":
            scene, ranges = os.path.abspath(args.beta), [(0, n), (3000, 3000 + n)]
        else:
            try:
                scene = pc.fixture_scene(args.work, name, log=lambda m: print(m, file=sys.stderr, flush=True))
            except pc.FixtureError as e:
                report(name, "ERROR", f"fixture: {e}")
                all_ok = False
                continue
            ranges = [(60, 60 + n)] if name == "perf_physics" else [(0, n)]
        all_ok = check(name, scene, ranges, args, env) and all_ok
    print("all identical" if all_ok else "differences found")
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
