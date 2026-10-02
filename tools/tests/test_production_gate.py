"""Acceptance gates reject missing perception, stale artifacts and mux truncation."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("production_gate", Path(__file__).parents[1] / "production_gate.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class CaptionGateTests(unittest.TestCase):
    def test_external_assets_must_be_vendored(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            scene = root / "film.scene.xml"
            scene.write_text('<scene><assets><image src="../external.png"/></assets></scene>')
            with self.assertRaisesRegex(gate.GateError, "outside project"):
                gate.vendored_scene(scene, root)
            scene.write_text('<scene><assets><image src="https://example.org/image.png"/></assets></scene>')
            with self.assertRaisesRegex(gate.GateError, "Vendor remote"):
                gate.vendored_scene(scene, root)
            scene.write_text('<scene><assets><image src="assets/image.png"/></assets></scene>')
            gate.vendored_scene(scene, root)

    def test_captions_must_fit_the_final_timeline_and_reading_area(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "film.vtt"
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\nA readable line.\n")
            self.assertEqual(gate.captions(path, 2)["cues"], 1)
            with self.assertRaisesRegex(gate.GateError, "timeline"):
                gate.captions(path, 1)
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\n" + "x" * 43 + "\n")
            with self.assertRaisesRegex(gate.GateError, "42-character"):
                gate.captions(path, 2)
            path.write_text("WEBVTT\n\n00:00.000 --> 00:02.000\nOne\nTwo\nThree\n")
            with self.assertRaisesRegex(gate.GateError, "42-character"):
                gate.captions(path, 2)


class PremiumGateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for folder in ("qa", "out", "work", "assets"):
            (self.root / folder).mkdir()
        for name in ("film.scene.xml", "out/film.mp4", "out/film.vtt", "assets/image.png",
                     "work/encode.json", "work/engine", "work/review.txt"):
            (self.root / name).write_text(name)
        self.review = gate.artifact(self.root / "work/review.txt")
        checks = {name: {"exit": 0} for name in ("validate", "resolve", "sr-audiocheck", "sr-margin",
                   "sr-sibilance", "decoded_audio", "black_freeze", "captions", "audio_tail")}
        self.tech = {"version": 1, "status": "TECHNICAL PASS", "project": str(self.root), "duration": 100,
                     "inputs": gate.inputs(self.root), "checks": checks,
                     "sidecars": [gate.artifact(self.root / "out/film.vtt")]}
        self.tech["encoded_utc"] = "2026-10-02T12:00:00+00:00"
        for key, name in (("scene", "film.scene.xml"), ("video", "out/film.mp4"),
                          ("engine", "work/engine"), ("encode_report", "work/encode.json")):
            self.tech[key] = gate.artifact(self.root / name)
        gate.write_json(self.root / "qa/TECHNICAL.json", self.tech)
        gate.template(type("Args", (), {"project": self.root}))
        self.manifest = json.loads((self.root / "qa/PREMIUM.json").read_text())

    def complete(self):
        m = self.manifest
        m["producer_session"], m["reviewer_session"] = "producer", "reviewer"
        m["sidecars"] = [gate.artifact(self.root / "out/film.vtt")]
        for name in gate.REPORTS:
            p = self.root / "qa" / name
            p.write_text("Actual review evidence")
            m["reports"][name] = gate.artifact(p)
        for role, row in m["roles"].items():
            row["session"] = "reviewer" if role == "pals-reviewer" else "producer" if role == "pals-film-producer" else role
        for row in m["categories"]:
            row.update(score=9, inspection=["direct-playback", "direct-listening"],
                       evidence=[self.review], rationale="Reviewed final cut", reviewed_by="reviewer", reference="ref")
        m["coordinator_acceptance"].update(session="pals-coordinator", evidence=[self.review])
        m["art_direction"].update(session="pals-art-director", signature_still=self.review,
            approved_utc="2026-10-02T11:00:00+00:00", reference_board=[self.review],
            visible_advance="Specific improvement over reference", evidence=[self.review])
        m["editorial_review"].update(reviewer="reviewer", method="sentence-and-silence-frame-review",
                                     failures=0, evidence=[self.review])
        m["professional_references"] = [{"id": "ref", "title": "Selected professional benchmark",
            "source": "Reference source and timestamp", "comparison": "Documented comparison",
            "reviewer": "reviewer", "evidence": [self.review]}]
        m["sampling"] = [{"start": 0, "end": 100, "reviewer": "reviewer", "reason": "Complete film review",
                          "tags": ["hook", "quarter", "middle", "three-quarter", "ending", "transitions",
                                   "complex", "modified", "black-freeze-review", "caption-sync"],
                          "evidence": [self.review]}]
        for key in ("full_playback", "regression"):
            m[key].update(reviewer="reviewer", method="direct-audiovisual-playback", evidence=[self.review])

    def check(self):
        gate.write_json(self.root / "qa/PREMIUM.json", self.manifest)
        return gate.premium(self.root)

    def test_template_never_certifies_quality(self):
        with self.assertRaises(gate.GateError):
            self.check()

    def test_complete_independent_review_passes(self):
        self.complete()
        self.assertEqual(self.check()["overall"], "9.00")

    def test_proxy_scores_cannot_replace_perceptual_evidence(self):
        self.complete()
        self.manifest["categories"][5]["inspection"] = ["ASR", "loudness"]
        with self.assertRaisesRegex(gate.GateError, "direct-listening"):
            self.check()

    def test_changed_caption_invalidates_review(self):
        self.complete()
        (self.root / "out/film.vtt").write_text("Changed captions")
        with self.assertRaisesRegex(gate.GateError, "Artifact changed"):
            self.check()

    def test_changed_asset_invalidates_technical_review(self):
        self.complete()
        (self.root / "assets/image.png").write_text("Changed pixels")
        with self.assertRaisesRegex(gate.GateError, "Sources/assets changed"):
            self.check()

    def test_changed_encode_or_engine_invalidates_review(self):
        for name in ("out/film.mp4", "work/engine", "work/encode.json"):
            with self.subTest(name=name):
                self.complete()
                p = self.root / name
                old = p.read_text()
                p.write_text("Changed")
                with self.assertRaisesRegex(gate.GateError, "Artifact changed"):
                    self.check()
                p.write_text(old)

    def test_high_average_cannot_hide_weak_category(self):
        self.complete()
        for row in self.manifest["categories"]:
            row["score"] = 10
        self.manifest["categories"][0]["score"] = 8.49
        with self.assertRaisesRegex(gate.GateError, "Score below"):
            self.check()

    def test_visual_and_cinematography_require_nine(self):
        self.complete()
        self.manifest["categories"][0]["score"] = 8.9
        with self.assertRaisesRegex(gate.GateError, "Visual quality"):
            self.check()
        self.complete()
        self.manifest["categories"][2]["score"] = 8.9
        with self.assertRaisesRegex(gate.GateError, "Cinematography"):
            self.check()

    def test_signature_approval_after_encode_and_editorial_fail_block_release(self):
        self.complete()
        self.manifest["art_direction"]["approved_utc"] = "2026-10-02T13:00:00+00:00"
        with self.assertRaisesRegex(gate.GateError, "before encoding"):
            self.check()
        self.complete()
        self.manifest["editorial_review"]["failures"] = 1
        with self.assertRaisesRegex(gate.GateError, "editorial review"):
            self.check()

    def test_overall_never_rounds_up_to_threshold(self):
        self.complete()
        self.manifest["categories"][1]["score"] = 8.99
        with self.assertRaisesRegex(gate.GateError, "Overall is below"):
            self.check()

    def test_nan_and_boolean_scores_are_rejected(self):
        self.complete()
        for value in (True, "NaN", "Infinity"):
            self.manifest["categories"][0]["score"] = value
            with self.assertRaises((gate.GateError, ValueError)):
                self.check()

    def test_producer_cannot_accept_own_review(self):
        self.complete()
        self.manifest["reviewer_session"] = "producer"
        with self.assertRaisesRegex(gate.GateError, "independent"):
            self.check()

    def test_professional_benchmark_and_coordinator_acceptance_are_required(self):
        self.complete()
        self.manifest["professional_references"] = []
        with self.assertRaisesRegex(gate.GateError, "professional reference"):
            self.check()
        self.complete()
        self.manifest["coordinator_acceptance"]["session"] = ""
        with self.assertRaisesRegex(gate.GateError, "coordinator acceptance"):
            self.check()

    def test_missing_middle_sample_blocks_release(self):
        self.complete()
        self.manifest["sampling"][0]["end"] = 20
        with self.assertRaisesRegex(gate.GateError, "quarter"):
            self.check()

    def test_open_issue_or_unverified_fix_blocks_release(self):
        self.complete()
        self.manifest["issues"] = [{"id": "V1", "owner": "producer", "root_cause": "camera",
            "classification": "Blocking", "status": "FIXED", "accepted_by": "reviewer"}]
        with self.assertRaisesRegex(gate.GateError, "Unresolved issue"):
            self.check()

    def test_new_source_file_invalidates_review(self):
        self.complete()
        (self.root / "assets/new.png").write_text("New image")
        with self.assertRaisesRegex(gate.GateError, "Sources/assets changed"):
            self.check()


class EncodedFrameTests(unittest.TestCase):
    def test_actual_decoded_frames_must_match_render_count(self):
        report = {"quality": "final", "frames": 1327, "size": [1080, 1920], "fps": 30}
        probe = {"streams": [{"codec_type": "video", "width": 1080, "height": 1920,
                  "avg_frame_rate": "30/1", "nb_read_frames": "1324"}], "format": {"duration": "44.233"}}
        with self.assertRaisesRegex(gate.GateError, "Frame count mismatch"):
            gate.check_video(report, "short.mp4", probe)
        probe["streams"][0]["nb_read_frames"] = "1327"
        self.assertAlmostEqual(gate.check_video(report, "short.mp4", probe), 1327 / 30)

    def test_degraded_output_is_not_certified(self):
        with self.assertRaisesRegex(gate.GateError, "unsupported"):
            gate.check_video({"unsupported": ["missing shader"]}, "film.mp4", {})


if __name__ == "__main__":
    unittest.main()
