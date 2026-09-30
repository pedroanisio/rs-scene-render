import os
import stat
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import perf_gate

TOOLS = Path(__file__).resolve().parents[1]


def bench_line(ms):
    return (
        f"x.scene.xml: 30 frames on llvmpipe (Vulkan, Cpu)\n"
        f"render: median {ms:.3f} ms ({1000 / ms:.1f} fps), p95 {ms * 1.2:.3f} ms; "
        f"of which CPU planning and submission 1.000 ms; evaluation 0.100 ms\n"
    )


class VerdictTests(unittest.TestCase):
    def test_a_slowdown_over_the_threshold_is_a_regression(self):
        v = perf_gate.verdict([100.0, 102.0, 101.0], [117.0, 118.0, 120.0], 0.15)
        self.assertTrue(v["regressed"])
        self.assertAlmostEqual(v["ratio"], 1.17)

    def test_a_slowdown_within_the_threshold_passes(self):
        self.assertFalse(perf_gate.verdict([100.0], [114.0], 0.15)["regressed"])

    def test_the_best_round_of_each_binary_is_compared(self):
        # one slow round of either binary (a busy runner) does not decide the verdict
        v = perf_gate.verdict([100.0, 180.0], [101.0, 150.0], 0.15)
        self.assertFalse(v["regressed"])
        self.assertEqual((v["base_ms"], v["head_ms"]), (100.0, 101.0))

    def test_faster_is_never_a_regression(self):
        self.assertFalse(perf_gate.verdict([100.0], [60.0], 0.15)["regressed"])

    def test_the_median_is_read_from_bench_output(self):
        self.assertAlmostEqual(perf_gate.median_ms(bench_line(42.5)), 42.5)
        with self.assertRaises(ValueError):
            perf_gate.median_ms("no bench line here")


class FixtureTests(unittest.TestCase):
    def test_the_impact_fixture_is_a_well_formed_scene_with_the_features_it_measures(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([sys.executable, str(TOOLS / "perf_impact.py"), d], check=True, capture_output=True)
            path = Path(d) / "perf_impact.scene.xml"
            root = ET.parse(path).getroot()
            text = path.read_text()
            for feature in ["posterize-time", 'type="stroke"', "<particleEmitter", 'motionBlur="on"', "<adjustment", "film-grain"]:
                self.assertTrue(feature in text, f"the fixture has {feature}")
            project = root.find("project")
            self.assertEqual((project.get("width"), project.get("height")), ("1920", "1080"))


def fake_binary(d, name, ms):
    """A stand-in scene-render that answers `render ... --bench` with a fixed median."""
    p = Path(d) / name
    p.write_text(f"#!/bin/sh\nprintf '%s' '{bench_line(ms)}'\n")
    p.chmod(p.stat().st_mode | stat.S_IEXEC)
    return str(p)


class GateTests(unittest.TestCase):
    def run_gate(self, base_ms, head_ms):
        with tempfile.TemporaryDirectory() as d:
            base = fake_binary(d, "base", base_ms)
            head = fake_binary(d, "head", head_ms)
            out = Path(d) / "gate.json"
            r = subprocess.run(
                [sys.executable, str(TOOLS / "perf_gate.py"), base, head, "--rounds", "2", "--work", d, "--json", str(out)],
                capture_output=True,
                text=True,
            )
            return r, out.read_text() if out.exists() else ""

    def test_the_gate_fails_on_a_regression_and_writes_its_numbers(self):
        r, report = self.run_gate(100.0, 130.0)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("REGRESSION", r.stdout)
        self.assertIn('"regressed": true', report)

    def test_the_gate_passes_without_one(self):
        r, _ = self.run_gate(100.0, 104.0)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
