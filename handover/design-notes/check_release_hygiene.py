#!/usr/bin/env python3
"""Checks that the tracked tree carries only releasable material.

usage: tools/check_release_hygiene.py --all             every tracked file
       tools/check_release_hygiene.py --staged          files staged for commit
       tools/check_release_hygiene.py --dir PATH        an unpacked release archive or crate package
       tools/check_release_hygiene.py --commit-msg FILE a commit message

Rule A rejects files that are working material (agent instructions, plans, notes, logs).
Rule B rejects text that refers to private locations, delivery phases, documents that are not
shipped, or the development session. Rule D lists past-tense phrases for a human to judge.
Exit status 1 when a rule A or B match remains. Legitimate exceptions go in ALLOW with a reason.
"""
import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PATH_RULES = [
    ("A1", r"(^|/)(CLAUDE|AGENTS|GEMINI)\.md$", "agent instruction file"),
    ("A2", r"(^|/)\.(claude|cursor|aider|windsurf|continue)(/|[^/]*$)", "agent tooling state"),
    ("A3", r"(^|/)\.github/copilot-instructions\.md$", "agent instruction file"),
    ("A4", r"(^|/)(prompts?|skills?)/", "prompt or skill directory"),
    ("A5", r"(^|/)(PLAN|TODO|NOTES|SCRATCH|HANDOFF|PROGRESS|FINDINGS)[^/]*$", "plan, notes or progress file"),
    ("A6", r"\.(session|transcript)\.[^/]+$|\.log$", "session record or log"),
    ("A7", r"(^|/)[^/]*-(findings|report|audit)\.[^/]+$", "findings or report file"),
]

TEXT_RULES = [
    ("B1", r"/home/[A-Za-z0-9_.-]+/|/Users/[A-Za-z0-9_.-]+/|[A-Za-z]:\\Users\\", "local absolute path"),
    ("B2", r"~/\.claude|\.claude/|\.cursor/|-private\b", "private tooling or storage location"),
    ("B3", r"\bBatch [0-9]+\b|\bPhase [0-9]+\b|\b(exit|release) gate\b|\bthe plan\b|\bfrom the plan\b|\bitem [0-9]+[a-z]?\b",
     "delivery-phase label; name the subject instead"),
    ("B4", r"\bCONVENTIONS\b|\bscenerender/|\b[Cc] renderer\b|\bPython (renderer|engine)\b",
     "citation of a document or project that is not shipped; state the rule inline"),
    ("B5", r"\bD(?:[1-9]|[12][0-9]|3[0-9])\b", "decision code of an unshipped document; state the rule inline"),
    ("B6", r"\bnot verified on\b|\(was (gap|wrong|degraded)\)|\bnot met\b\.?\s*The", "status diary"),
    ("B7", r"\bin this session\b|\bas (requested|discussed|agreed)\b|\bClaude\b|\bCopilot\b|\bChatGPT\b",
     "development-session narration"),
]

TRIPWIRE = r"\b(used to|no longer|now (passes|works|reads)|since the [a-z -]+ work)\b"

COMMIT_RULES = [("C1", r"\b(Phase|Batch|item) [0-9]+\b|\bWIP\b", "delivery-phase label or WIP in the subject")]

# (path prefix, rule) -> reason. Keep entries narrow.
ALLOW = {}

SKIP = ("Cargo.lock", "tests/corpus/", "tools/check_release_hygiene.py")
TEXT_EXT = {".md", ".rs", ".wgsl", ".glsl", ".fs", ".py", ".toml", ".yml", ".yaml", ".xml", ".xsd", ".sch",
            ".json", ".in", ".txt", ".sh"}


def allowed(path, rule):
    return any(path.startswith(p) and r == rule for (p, r) in ALLOW)


def files_from_git(staged):
    cmd = ["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR"] if staged else ["git", "ls-files"]
    out = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=True).stdout
    return [f for f in out.splitlines() if f]


def files_from_dir(d):
    return [os.path.relpath(os.path.join(r, f), d) for r, _, fs in os.walk(d) for f in fs]


def check(paths, base):
    bad = []
    warn = []
    for p in paths:
        for rid, pat, why in PATH_RULES:
            if re.search(pat, p) and not allowed(p, rid):
                bad.append(f"{p}: {rid} {why}; move it out of the repository")
        if p.startswith(SKIP) or os.path.splitext(p)[1] not in TEXT_EXT:
            continue
        full = os.path.join(base, p)
        if not os.path.isfile(full):
            continue
        try:
            lines = open(full, encoding="utf-8").read().splitlines()
        except UnicodeDecodeError:
            continue
        for n, line in enumerate(lines, 1):
            code = os.path.splitext(p)[1] in (".rs", ".wgsl", ".glsl")
            comment = line.split("//", 1)[1] if "//" in line else ""
            for rid, pat, why in TEXT_RULES:
                # decision codes look like identifiers in code (D2, D65); judge them in comments only
                m = re.search(pat, comment if code and rid == "B5" else line)
                if m and not allowed(p, rid):
                    bad.append(f'{p}:{n}: {rid} {why}: "{m.group(0)}"')
            m = re.search(TRIPWIRE, line)
            if m:
                warn.append(f'{p}:{n}: D1 review: "{m.group(0)}" (keep only if it states a current rule)')
    return bad, warn


def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--all", action="store_true")
    g.add_argument("--staged", action="store_true")
    g.add_argument("--dir")
    g.add_argument("--commit-msg")
    a = ap.parse_args()
    if a.commit_msg:
        subject = open(a.commit_msg, encoding="utf-8").readline()
        bad = [f'commit subject: {rid} {why}: "{m.group(0)}"'
               for rid, pat, why in COMMIT_RULES for m in [re.search(pat, subject)] if m]
        warn = []
    elif a.dir:
        bad, warn = check(files_from_dir(a.dir), a.dir)
        if not any(re.match(r"LICENSE", os.path.basename(f)) for f in files_from_dir(a.dir)):
            bad.append(f"{a.dir}: E1 no LICENSE file in the package")
    else:
        bad, warn = check(files_from_git(a.staged), ROOT)
    for w in warn:
        print("warning:", w)
    for b in bad:
        print("error:", b)
    if bad:
        print(f"{len(bad)} hygiene error(s)", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
