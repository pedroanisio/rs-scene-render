import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import evidence


class EvidenceTests(unittest.TestCase):
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
