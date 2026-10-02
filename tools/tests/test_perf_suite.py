import json
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


TOOLS = Path(__file__).resolve().parents[1]


class SuiteTests(unittest.TestCase):
    def test_failed_workload_fails_the_suite_and_preserves_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            fixture = work / "perf_text"
            fixture.mkdir()
            (fixture / "perf_text.scene.xml").write_text('<scene version="1.2"/>')
            binary = work / "renderer"
            binary.write_text("#!/bin/sh\necho 'deliberate renderer failure' >&2\nexit 7\n")
            binary.chmod(binary.stat().st_mode | stat.S_IEXEC)
            report = work / "result.json"
            result = subprocess.run(
                [sys.executable, str(TOOLS / "perf_suite.py"), str(report),
                 "--binary", str(binary), "--work", str(work), "--only", "perf_text_bench"],
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            record = json.loads(report.read_text())["fixtures"]["perf_text_bench"]
            self.assertIn("deliberate renderer failure", record["error"])
            self.assertNotIn("value", record)


if __name__ == "__main__":
    unittest.main()
