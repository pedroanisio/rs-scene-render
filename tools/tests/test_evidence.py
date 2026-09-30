import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import evidence


class EvidenceTests(unittest.TestCase):
    def run_manifest(self, root, binary, clips=False, writes_clip=True):
        manifest = root / "manifest.json"
        manifest.write_text(json.dumps({"cases": [{"id": "probe", "scene": "scene.xml", "expect": "native", "review": "Inspect timing"}]}))
        output = root / "output"
        def run_case(case, binary, out, bench):
            work = Path(out, case["id"])
            work.mkdir(parents=True)
            (work / "scene.xml").write_text("<scene/>")
            return {"id": "probe", "reference": "probe", "expect": "native", "outcome": "native", "checks": [], "notes": [], "stills": []}
        real_run = evidence.subprocess.run
        def run(cmd, **kwargs):
            if len(cmd) > 1 and cmd[1] == "encode":
                # Delivery resolves relative -o paths against the scene directory.
                dest = Path(cmd[2]).resolve().parent / cmd[cmd.index("-o") + 1]
                if writes_clip:
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(b"clip")
                return evidence.subprocess.CompletedProcess(cmd, 0, "", "")
            return real_run(cmd, **kwargs)
        args = [str(output), "--bin", binary, "--manifest", str(manifest)]
        if clips:
            args.append("--clips")
        with patch.object(evidence, "run_case", side_effect=run_case), patch.object(evidence.subprocess, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
            code = evidence.main(args)
        return code, output

    def test_binary_from_path_is_hashed_and_reported(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"PATH": str(Path(sys.executable).parent) + os.pathsep + os.environ.get("PATH", "")}):
            code, out = self.run_manifest(Path(tmp), Path(sys.executable).name)
            self.assertEqual(code, 0)
            self.assertTrue(json.loads((out / "report.json").read_text())["binary_sha256"])

    def test_relative_output_clip_exists_at_reported_path(self):
        with tempfile.TemporaryDirectory(dir=".") as tmp:
            code, out = self.run_manifest(Path(os.path.relpath(tmp)), sys.executable, clips=True)
            self.assertEqual(code, 0)
            report = json.loads((out / "report.json").read_text())
            self.assertTrue((out / report["results"][0]["clip"]).is_file())

    def test_successful_encoder_without_clip_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            code, out = self.run_manifest(Path(tmp), sys.executable, clips=True, writes_clip=False)
            self.assertEqual(code, 1)
            report = json.loads((out / "report.json").read_text())
            self.assertEqual(report["results"][0]["outcome"], "error")
            self.assertNotIn("clip", report["results"][0])

    def test_changes_upper_bound_detects_a_broken_hold(self):
        frames = {0: np.zeros((2, 2, 4)), 1: np.ones((2, 2, 4))}
        check = {"type": "changes", "t": [0, 1], "region": [0, 0, 2, 2], "max": 0.01}
        self.assertFalse(evidence.run_check(check, frames)[0])
        frames[1] = frames[0].copy()
        self.assertTrue(evidence.run_check(check, frames)[0])

    def test_unknown_case_fails_before_deleting_existing_output(self):
        with tempfile.TemporaryDirectory() as out:
            sentinel = Path(out, "keep.txt")
            sentinel.write_text("keep")
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                try:
                    code = evidence.main([out, "--case", "misspelled-case"])
                except SystemExit as e:
                    code = e.code
            self.assertNotEqual(code, 0)
            self.assertEqual(sentinel.read_text(), "keep")

    def test_render_does_not_accept_a_stale_png_after_failure(self):
        with tempfile.TemporaryDirectory() as out:
            png = Path(out, "frame.png")
            png.write_bytes(b"stale")
            with patch.object(evidence.subprocess, "run") as run:
                run.return_value.returncode = 1
                run.return_value.stdout = ""
                run.return_value.stderr = "error: --strict: failed"
                self.assertFalse(evidence.render("renderer", "scene.xml", 0, str(png))[0])

if __name__ == '__main__':
    unittest.main()
