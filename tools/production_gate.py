#!/usr/bin/env python3
"""Fail-closed technical and premium acceptance gates. Never uploads or scores a film."""
import argparse
import datetime
from decimal import Decimal, ROUND_DOWN
from fractions import Fraction
import hashlib
import html
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

CATEGORIES = (
    "Visual quality", "Animation / VFX", "Cinematography", "Editing / pacing",
    "Graphics / overlays", "Voiceover", "Sound design", "Music / score",
    "Narrative / educational clarity", "Overall YouTube production quality",
)
ROLES = (
    "pals-coordinator", "pals-film-producer", "pals-art-director", "pals-sound-narration",
    "pals-researcher", "pals-pauta-editor", "pals-growth", "pals-reviewer",
    "pals-resource-manager", "pals-engine-maintainer", "build-ci-cd", "pals-publishing-editor", "pals-improvement",
)
REPORTS = ("REPORT.md", "ISSUES.md", "SAMPLING.md", "VISUAL.md", "AUDIO.md", "FACTS.md", "NARRATIVE.md", "EDITORIAL.md")
OWNERS = (
    "pals-art-director", "pals-art-director", "pals-art-director", "pals-film-producer",
    "pals-art-director", "pals-sound-narration", "pals-sound-narration", "pals-sound-narration",
    "pals-film-producer", "pals-coordinator",
)
SKIP = {"out", "work", "qa", ".git", "__pycache__", ".venv", "node_modules"}


class GateError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise GateError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def artifact(path):
    p = Path(path).resolve(strict=True)
    require(p.is_file() and p.stat().st_size > 0, f"Missing/empty artifact: {p}")
    return {"path": str(p), "sha256": digest(p)}


def verify_artifact(item):
    require(isinstance(item, dict) and set(item) >= {"path", "sha256"}, "Malformed artifact record")
    require(artifact(item["path"]) == item, f"Artifact changed: {item['path']}")


def inputs(project):
    root = Path(project).resolve()
    result = {}
    for p in sorted(root.rglob("*")):
        if any(part in SKIP for part in p.relative_to(root).parts) or p.name.startswith(".sr-"):
            continue
        if p.is_file():
            require(p.resolve().is_relative_to(root), f"Vendor external assets before review: {p}")
            result[str(p.relative_to(root))] = digest(p)
    return result


def vendored_scene(scene, project, seen=None):
    seen = set() if seen is None else seen
    scene, project = Path(scene).resolve(), Path(project).resolve()
    if scene in seen:
        return
    seen.add(scene)
    for node in ET.parse(scene).iter():
        for name in ("src", "fontFile"):
            source = node.get(name)
            if not source or source.startswith("data:"):
                continue
            require(not re.match(r"^[A-Za-z][\w+.-]*://", source) and not source.startswith("~"),
                    f"Vendor remote/home asset before review: {source}")
            path = (scene.parent / source).resolve()
            require(path.is_relative_to(project), f"Asset outside project: {source}")
            if node.tag.rsplit("}", 1)[-1] == "include" and name == "src":
                vendored_scene(path, project, seen)


def write_json(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + ".tmp")
    temp.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")
    temp.replace(path)


def run(argv, *, env=None):
    result = subprocess.run([str(x) for x in argv], capture_output=True, text=True, env=env, timeout=3600)
    return {"command": [str(x) for x in argv], "exit": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}


def successful(argv, *, env=None):
    result = run(argv, env=env)
    require(result["exit"] == 0, f"Check failed (exit {result['exit']}): {' '.join(result['command'])}\n"
            + (result["stderr"] or result["stdout"])[-2000:])
    return result


def reports(path):
    text = Path(path).read_text()
    try:
        data = json.loads(text)
        return data if isinstance(data, list) else [data]
    except json.JSONDecodeError:
        return [json.loads(line) for line in text.splitlines() if line.strip()]


def probe(path):
    result = successful([os.environ.get("SR_FFPROBE", "ffprobe"), "-v", "error", "-count_frames",
                         "-show_streams", "-show_format", "-of", "json", path])
    return json.loads(result["stdout"])


def check_video(report, video, data, *, final=True):
    for key in ("unsupported", "accessibility", "evaluation_warnings", "warnings", "uploads"):
        require(not report.get(key), f"Encode report has {key}; resolve before release")
    require(not final or report.get("quality") == "final", "Publication requires final quality")
    streams = data.get("streams", [])
    vs = [s for s in streams if s.get("codec_type") == "video"]
    require(len(vs) == 1, f"Expected one video stream in {video}")
    v = vs[0]
    require(int(v.get("nb_read_frames", -1)) == report["frames"],
            f"Frame count mismatch: decoded {v.get('nb_read_frames')}, rendered {report['frames']}")
    require([v["width"], v["height"]] == report["size"], "Encoded dimensions differ from render report")
    fps = float(Fraction(v["avg_frame_rate"]))
    require(math.isfinite(fps) and abs(fps - float(report["fps"])) < 0.001, "Encoded FPS mismatch")
    duration = report["frames"] / fps
    for a in (s for s in streams if s.get("codec_type") == "audio"):
        # AAC priming/padding can occupy one codec frame beyond the PCM range.
        require(abs(float(a.get("duration", data["format"]["duration"])) - duration)
                <= 1 / fps + 2048 / int(a.get("sample_rate", 48000)), "Video/audio duration mismatch")
    return duration


def frame_gate(path):
    count = 0
    for report in reports(path):
        if not report.get("frames") or not report.get("fps"):
            continue
        video = Path(report["path"])
        if "%" in str(video) or "{frame}" in str(video):
            continue
        check_video(report, video, probe(video), final=False)
        count += 1
    require(count > 0, "No encoded video report found")
    return count


def captions(path, duration):
    text = Path(path).read_text(encoding="utf-8-sig").replace("\r\n", "\n")
    require(Path(path).suffix.lower() in {".vtt", ".srt"}, "Final captions must be VTT or SRT")
    def seconds(value):
        fields = value.replace(",", ".").split(":")
        require(len(fields) in (2, 3), "Invalid caption timestamp")
        return sum(float(field) * 60 ** i for i, field in enumerate(reversed(fields)))
    count, previous = 0, -1.0
    for block in re.split(r"\n\s*\n", text.strip()):
        lines = block.splitlines()
        indices = [i for i, line in enumerate(lines) if "-->" in line]
        if not indices:
            require(block.startswith(("WEBVTT", "NOTE", "STYLE", "REGION")), "Malformed caption block")
            continue
        require(len(indices) == 1, "Malformed caption cue")
        i = indices[0]
        times = lines[i].split("-->")
        start, end = seconds(times[0].strip()), seconds(times[1].strip().split()[0])
        require(math.isfinite(start) and math.isfinite(end) and 0 <= start < end <= duration + 0.05,
                "Caption outside final video timeline")
        require(start >= previous, "Overlapping or unordered caption cues")
        previous = end
        content = [html.unescape(re.sub(r"<[^>]*>", "", line)) for line in lines[i + 1:]]
        require(1 <= len(content) <= 2 and all(0 < len(line) <= 42 for line in content),
                "Captions exceed two 42-character lines")
        require(sum(len(line) for line in content) / (end - start) <= 17.0,
                "Captions exceed 17 characters per second")
        count += 1
    require(count > 0, "No caption cues found")
    return {"exit": 0, "cues": count}


def technical(args):
    project = Path(args.project).resolve(strict=True)
    scene, video, engine = artifact(args.scene), artifact(args.video), artifact(args.engine)
    require(Path(scene["path"]).is_relative_to(project), "Scene must belong to project")
    vendored_scene(scene["path"], project)
    matching = [r for r in reports(args.encode_report) if Path(r["path"]).resolve() == Path(video["path"])]
    require(len(matching) == 1, "Encode report must identify the candidate exactly once")
    data = probe(video["path"])
    duration = check_video(matching[0], video["path"], data)
    require(math.isfinite(args.voice_end) and 0 <= args.voice_end < duration,
            "Last spoken word must end within the final video")
    require(any(s.get("codec_type") == "audio" for s in data["streams"]), "Narrated film needs audio")
    before = inputs(project)
    sidecars = [artifact(path) for path in args.captions]
    checks = {"captions": {"exit": 0, "files": [captions(item["path"], duration) for item in sidecars]}}
    checks["validate"] = successful([engine["path"], "validate", scene["path"], "--deny-warnings"])
    checks["resolve"] = successful([engine["path"], "resolve", scene["path"], "--check"])
    env = dict(os.environ, SR=engine["path"])
    for name in ("sr-audiocheck", "sr-margin", "sr-sibilance"):
        command = Path(args.tools_dir) / name
        argv = [command, project]
        if name == "sr-audiocheck":
            argv.append(video["path"])
        if name != "sr-sibilance":
            argv += ["--scene", scene["path"]]
        checks[name] = successful(argv, env=env)
    tail = args.tail_check or Path(args.tools_dir).resolve().parent / "lib" / "tools" / "tail_check.py"
    checks["audio_tail"] = successful([args.audio_python, tail, video["path"],
        "--voice-end", str(args.voice_end), "--scene", scene["path"]], env=env)
    ffmpeg = os.environ.get("SR_FFMPEG", "ffmpeg")
    checks["decoded_audio"] = successful([ffmpeg, "-nostdin", "-v", "info", "-i", video["path"],
                                           "-vn", "-af", "ebur128=peak=true", "-f", "null", "-"])
    summary = checks["decoded_audio"]["stderr"].rsplit("Summary:", 1)[-1]
    loud = re.search(r"I:\s*(-?[\d.]+) LUFS", summary)
    peak = re.search(r"Peak:\s*(-?[\d.]+) dBFS", summary)
    require(loud and peak, "Cannot measure decoded loudness/true peak")
    require(abs(float(loud[1]) - args.loudness_target) <= 1.0, "Decoded loudness is outside target ±1 LU")
    require(float(peak[1]) <= -1.0, "Decoded true peak exceeds -1 dBTP")
    checks["black_freeze"] = successful([ffmpeg, "-nostdin", "-v", "info", "-i", video["path"], "-an",
        "-vf", "scale=320:-2,blackdetect=d=0.3:pix_th=0.06,freezedetect=n=0.001:d=2", "-f", "null", "-"])
    after = inputs(project)
    require(before == after, "Project changed during technical checks; rerun")
    require(artifact(video["path"]) == video, "Candidate changed during checks")
    for item in (scene, engine, *sidecars):
        verify_artifact(item)
    report = {"version": 1, "status": "TECHNICAL PASS", "project": str(project), "scene": scene,
              "video": video, "engine": engine, "encode_report": artifact(args.encode_report),
              "inputs": after, "duration": duration, "probe": data, "checks": checks, "sidecars": sidecars,
              "encoded_utc": datetime.datetime.fromtimestamp(Path(args.encode_report).stat().st_mtime,
                                                              datetime.timezone.utc).isoformat(),
              "voice_end": args.voice_end, "loudness_target": args.loudness_target,
              "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    write_json(project / "qa" / "TECHNICAL.json", report)
    return report


def template(args):
    root = Path(args.project).resolve(strict=True)
    path = root / "qa" / "PREMIUM.json"
    require(not path.exists(), "PREMIUM.json already exists; preserve its review history")
    tech = root / "qa" / "TECHNICAL.json"
    require(tech.exists(), "Run technical checks before preparing the premium review")
    t = json.loads(tech.read_text())
    data = {"version": 1, "standard": "premium", "status": "NOT READY", "technical": artifact(tech),
            "video": t["video"], "sidecars": t["sidecars"], "producer_session": "", "reviewer_session": "",
            "reports": {}, "roles": {r: {"session": "", "applicable": True, "reason": ""} for r in ROLES},
            "categories": [{"name": n, "owner": owner, "score": None, "inspection": None,
                            "evidence": [], "rationale": "", "reviewed_by": "", "reference": ""}
                           for n, owner in zip(CATEGORIES, OWNERS)],
            "sampling": [], "full_playback": {"reviewer": "", "method": "", "evidence": []},
            "regression": {"reviewer": "", "method": "", "evidence": []},
            "coordinator_acceptance": {"session": "", "video": t["video"], "evidence": []},
            "professional_references": [], "issues": []}
    data["art_direction"] = {"session": "", "signature_still": None, "approved_utc": "",
                             "reference_board": [], "visible_advance": "", "evidence": []}
    data["editorial_review"] = {"reviewer": "", "method": "", "failures": None, "evidence": []}
    write_json(path, data)
    return path


def evidence(items):
    require(isinstance(items, list) and len(items) > 0, "Missing rendered review evidence")
    for item in items:
        verify_artifact(item)


def premium(root):
    root = Path(root).resolve(strict=True)
    m = json.loads((root / "qa" / "PREMIUM.json").read_text())
    require(m.get("version") == 1 and m.get("standard") == "premium", "Unsupported premium contract")
    verify_artifact(m["technical"])
    t = json.loads(Path(m["technical"]["path"]).read_text())
    require(t["status"] == "TECHNICAL PASS" and Path(t["project"]) == root, "Invalid technical result")
    require(set(t["checks"]) >= {"validate", "resolve", "sr-audiocheck", "sr-margin", "sr-sibilance",
                                "decoded_audio", "black_freeze", "captions", "audio_tail"}, "Incomplete technical checks")
    require(all(c["exit"] == 0 for c in t["checks"].values()), "A technical check failed")
    for key in ("scene", "video", "engine", "encode_report"):
        verify_artifact(t[key])
    require(inputs(root) == t["inputs"], "Sources/assets changed since technical review")
    require(m["video"] == t["video"], "Review belongs to a different candidate")
    require(m.get("sidecars"), "Captions/sidecars must be identified and reviewed")
    require(m["sidecars"] == t["sidecars"], "Captions differ from technical candidate")
    for item in m["sidecars"]:
        verify_artifact(item)
    require(set(m["reports"]) == set(REPORTS), "Missing mandatory premium QA reports")
    for name, item in m["reports"].items():
        require(Path(item["path"]).resolve() == root / "qa" / name, "QA report belongs to another project")
        verify_artifact(item)
    producer, reviewer = m["producer_session"], m["reviewer_session"]
    require(producer and reviewer and producer != reviewer, "Review must be independent of producer")
    require(set(m["roles"]) == set(ROLES), "Account for all thirteen roles")
    for role, item in m["roles"].items():
        require(item.get("session") if item.get("applicable") is True else
                item.get("applicable") is False and item.get("reason"), f"Unassigned role: {role}")
    require(m["roles"]["pals-film-producer"]["session"] == producer and
            m["roles"]["pals-reviewer"]["session"] == reviewer, "Review sessions differ from role assignments")
    acceptance = m["coordinator_acceptance"]
    require(acceptance["session"] == m["roles"]["pals-coordinator"]["session"] and acceptance["session"],
            "Missing coordinator acceptance")
    require(acceptance["video"] == m["video"], "Coordinator accepted another candidate")
    evidence(acceptance["evidence"])
    art = m["art_direction"]
    require(art["session"] == m["roles"]["pals-art-director"]["session"] and art["session"],
            "Missing art-director approval")
    verify_artifact(art["signature_still"])
    approved = datetime.datetime.fromisoformat(art["approved_utc"].replace("Z", "+00:00"))
    encoded = datetime.datetime.fromisoformat(t["encoded_utc"])
    require(approved.tzinfo is not None and encoded.tzinfo is not None and approved <= encoded,
            "Signature still must be approved before encoding")
    evidence(art["reference_board"])
    require(art["visible_advance"], "Name a visible advance over the channel's best reference")
    evidence(art["evidence"])
    editorial = m["editorial_review"]
    require(editorial["reviewer"] == reviewer and editorial["method"] == "sentence-and-silence-frame-review"
            and type(editorial["failures"]) is int and editorial["failures"] == 0,
            "Missing independent editorial review with no FAIL moments")
    evidence(editorial["evidence"])
    require(m.get("professional_references"), "Missing professional reference comparison")
    reference_ids = set()
    for reference in m["professional_references"]:
        require(reference.get("id") and reference.get("title") and reference.get("source") and
                reference.get("comparison") and reference.get("reviewer") == reviewer,
                "Incomplete independent professional reference comparison")
        require(reference["id"] not in reference_ids, "Duplicate professional reference")
        reference_ids.add(reference["id"])
        evidence(reference["evidence"])
    rows = m["categories"]
    require([r["name"] for r in rows] == list(CATEGORIES), "Exactly ten fixed categories are required")
    scores = []
    for i, row in enumerate(rows):
        require(row["owner"] == OWNERS[i], f"Wrong category owner: {row['name']}")
        require(m["roles"][row["owner"]]["applicable"] is True, "Score owner cannot be not applicable")
        require(type(row["score"]) in (int, float, str), f"UNASSESSED: {row['name']}")
        score = Decimal(str(row["score"]))
        floor = Decimal("9.0") if i in (0, 2) else Decimal("8.5")
        require(score.is_finite() and floor <= score <= 10, f"Score below premium bar: {row['name']}")
        scores.append(score)
        require(row.get("rationale"), f"Missing rationale: {row['name']}")
        require(row.get("reference") in reference_ids, f"No professional comparison: {row['name']}")
        require(row.get("reviewed_by") == reviewer, f"Category lacks independent acceptance: {row['name']}")
        methods = row.get("inspection") or []
        required = "direct-listening" if 5 <= i <= 7 else "direct-playback"
        require(required in methods, f"Missing {required}: {row['name']}")
        evidence(row["evidence"])
    overall = sum(scores) / Decimal(len(scores))
    require(overall >= 9, "Overall is below 9.0")
    tags = set()
    for sample in m["sampling"]:
        require(sample["reviewer"] == reviewer, "Sampling needs independent reviewer")
        require(0 <= sample["start"] < sample["end"] <= t["duration"] + 0.001, "Invalid sample range")
        require(sample.get("reason"), "Sample has no reason")
        evidence(sample["evidence"])
        tags.update(sample["tags"])
    require(tags >= {"hook", "quarter", "middle", "three-quarter", "ending", "transitions", "complex",
                     "modified", "black-freeze-review", "caption-sync"}, "Incomplete runtime/regression sampling")
    for tag, fraction in (("hook", 0), ("quarter", 0.25), ("middle", 0.5),
                          ("three-quarter", 0.75), ("ending", 1)):
        time = t["duration"] * fraction
        require(any(tag in sample["tags"] and sample["start"] <= time <= sample["end"] + 0.001
                    for sample in m["sampling"]), f"Sampling does not cover the {tag}")
    for key in ("full_playback", "regression"):
        record = m[key]
        require(record["reviewer"] == reviewer and record["method"] == "direct-audiovisual-playback",
                f"Missing direct audiovisual {key} review")
        evidence(record["evidence"])
    for issue in m["issues"]:
        require(issue.get("id") and issue.get("owner") and issue.get("root_cause"), "Incomplete issue ownership")
        require(issue.get("classification") in ("Blocking", "Minor / acceptable"), "Unclassified issue")
        require(issue.get("status") == "ACCEPTED" and issue.get("accepted_by") == reviewer,
                f"Unresolved issue: {issue['id']}")
        require(0 <= issue["start"] < issue["end"] <= t["duration"] + 0.001, "Issue needs exact timestamps")
        require(any(s["start"] <= issue["start"] and s["end"] >= issue["end"]
                    and "modified" in s["tags"] for s in m["sampling"]),
                f"No final regression sample for issue: {issue['id']}")
        evidence(issue["before"])
        evidence(issue["after"])
    return {"verdict": "READY TO PUBLISH", "overall": str(overall.quantize(Decimal("0.01"), rounding=ROUND_DOWN)),
            "video": m["video"], "sidecars": m["sidecars"], "reviewer": reviewer,
            "note": "Quality acceptance only. Upload/publication still requires the owner's instruction."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    f = sub.add_parser("frames", help="Check decoded video against engine JSON delivery reports")
    f.add_argument("report")
    p = sub.add_parser("technical", help="Run checks against the exact final narrated candidate")
    p.add_argument("project")
    for name in ("scene", "video", "engine", "encode-report", "tools-dir"):
        p.add_argument("--" + name, required=True)
    p.add_argument("--loudness-target", type=float, default=-14.0)
    p.add_argument("--captions", action="append", required=True, help="Final VTT/SRT; repeat for multiple languages")
    p.add_argument("--voice-end", type=float, required=True, help="Final word end in encoded-video seconds")
    p.add_argument("--tail-check", help="Default: ../lib/tools/tail_check.py relative to --tools-dir")
    voice_python = Path.home() / ".venvs" / "voice" / "bin" / "python"
    p.add_argument("--audio-python", default=str(voice_python) if voice_python.exists() else sys.executable)
    for action in ("init", "check"):
        sub.add_parser(action).add_argument("project")
    args = parser.parse_args()
    try:
        if args.action == "frames":
            print(f"Encoded frame verification PASS: {frame_gate(args.report)} video(s)")
        elif args.action == "technical":
            technical(args)
            print("TECHNICAL PASS; perceptual review still required")
        elif args.action == "init":
            print(f"NOT READY: complete the review in {template(args)}")
        else:
            print(json.dumps(premium(args.project), indent=2))
        return 0
    except (GateError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, ArithmeticError, ET.ParseError) as e:
        print(f"NOT READY: {e}", file=sys.stderr)
        return 1 if isinstance(e, GateError) else 2


if __name__ == "__main__":
    sys.exit(main())
