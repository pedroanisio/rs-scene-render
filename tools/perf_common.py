#!/usr/bin/env python3
"""Shared helpers of tools/perf_suite.py and tools/equivalence.py: which scene-render
binary to use, the perf fixtures (reused when present, else generated) and the
environment every run gets. Standard library only.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent
FIXTURES = ["perf_layers", "perf_vector", "perf_text", "perf_effects", "perf_3d", "perf_physics", "perf_video"]
DEFAULT_WORK = "/tmp/perf_suite"


class FixtureError(Exception):
    pass


def default_binary():
    """$SCENE_RENDER, else the workspace's release or debug build, else scene-render on PATH."""
    env = os.environ.get("SCENE_RENDER")
    if env:
        return env
    for p in (ROOT / "target" / "release" / "scene-render", ROOT / "target" / "debug" / "scene-render"):
        if p.is_file():
            return str(p)
    return shutil.which("scene-render") or "scene-render"


def run_env():
    """Environment of every scene-render run: SR_GPU_BACKEND=vulkan unless the caller set it."""
    env = dict(os.environ)
    env.setdefault("SR_GPU_BACKEND", "vulkan")
    return env


def parse_only(text):
    """--only NAME,... as a set, or None when not given."""
    if not text:
        return None
    return {n.strip() for n in text.split(",") if n.strip()}


def tail(text, lines=12):
    return "\n".join((text or "").strip().splitlines()[-lines:])


def fixture_scene(work, name, log=None):
    """Absolute path of NAME.scene.xml: reused from WORK/NAME/ or /tmp/NAME/ when one
    exists there, otherwise generated into WORK/NAME/ with tools/NAME.py."""
    for d in (os.path.join(work, name), os.path.join("/tmp", name)):
        p = os.path.join(d, f"{name}.scene.xml")
        if os.path.isfile(p):
            return os.path.abspath(p)
    gen = TOOLS / f"{name}.py"
    if not gen.is_file():
        raise FixtureError(f"no generator {gen}")
    out = os.path.join(work, name)
    if log:
        log(f"generating {name} into {out} with {gen.name}")
    r = subprocess.run([sys.executable, str(gen), out], capture_output=True, text=True)
    if r.returncode != 0:
        raise FixtureError(f"{gen.name} failed (exit {r.returncode}): {tail(r.stderr or r.stdout)}")
    p = os.path.join(out, f"{name}.scene.xml")
    if not os.path.isfile(p):
        raise FixtureError(f"{gen.name} did not write {p}")
    return os.path.abspath(p)


def commit():
    """Short commit of the checkout the tools live in, or None outside git."""
    try:
        r = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=str(ROOT),
                           capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout.strip() if r.returncode == 0 and r.stdout.strip() else None


def version(binary):
    try:
        r = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return (r.stdout or r.stderr).strip() or None
