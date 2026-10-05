import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import probe_render

SCENE = '<scene version="1.3">\n  <project width="3840" height="2160" fps="24" duration="6"/>\n</scene>\n'


class ResizeTests(unittest.TestCase):
    def test_the_copy_has_the_new_size_and_the_original_is_untouched(self):
        with tempfile.TemporaryDirectory() as src, tempfile.TemporaryDirectory() as work:
            scene = Path(src) / "a.scene.xml"
            scene.write_text(SCENE)
            (Path(src) / "sky.hdr").write_bytes(b"hdr")
            out = probe_render.resized_copy(str(scene), (1280, 720), work)
            text = Path(out).read_text()
            self.assertIn('width="1280" height="720"', text)
            self.assertNotIn("3840", text)
            self.assertEqual(scene.read_text(), SCENE)
            # siblings resolve beside the copy, but the copy itself is not a link to the original
            self.assertEqual((Path(out).parent / "sky.hdr").read_bytes(), b"hdr")
            self.assertFalse(os.path.islink(out))

    def test_a_scene_without_a_project_is_an_error(self):
        with tempfile.TemporaryDirectory() as src, tempfile.TemporaryDirectory() as work:
            scene = Path(src) / "a.scene.xml"
            scene.write_text("<scene/>")
            with self.assertRaises(probe_render.ProbeError):
                probe_render.resized_copy(str(scene), (1, 1), work)


class ParseTests(unittest.TestCase):
    def test_the_last_statistics_line_is_used_and_noise_is_ignored(self):
        first = json.dumps({"frame": 0, "stats": {"draws": 1}, "gpu": None})
        last = json.dumps({"frame": 0, "stats": {"draws": 2}, "gpu": None})
        got = probe_render.parse_stats(f"warning: x\n{first}\nnot json {{\n{last}\n")
        self.assertEqual(got["stats"]["draws"], 2)
        self.assertIsNone(probe_render.parse_stats("nothing here"))

    def test_gpu_passes_are_summed_by_label(self):
        gpu = {"frame_ms": 5.0, "passes": [{"label": "pathtrace trace", "ms": 1.5},
                                           {"label": "pathtrace trace", "ms": 2.0},
                                           {"label": "pathtrace denoise", "ms": 0.5}]}
        got = probe_render.sum_passes(gpu)
        self.assertEqual(got["passes_ms"], {"pathtrace trace": 3.5, "pathtrace denoise": 0.5})
        self.assertIsNone(probe_render.sum_passes(None))


if __name__ == "__main__":
    unittest.main()
