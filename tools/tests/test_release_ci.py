import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import tempfile

spec = importlib.util.spec_from_file_location(
    "release_ci", Path(__file__).resolve().parents[1] / "release_ci.py"
)
release_ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_ci)


class ReleaseCiTest(unittest.TestCase):
    def run_record(self, **changes):
        record = dict(id=10, head_sha="abc", head_branch="main", event="push",
                      path=".github/workflows/ci.yml", status="completed",
                      conclusion="success")
        record.update(changes)
        return record

    def test_only_exact_main_push_ci_can_validate_release(self):
        for changes in [dict(head_sha="other"), dict(head_branch="feature"),
                        dict(event="pull_request"), dict(path="other.yml")]:
            self.assertIsNone(release_ci.select_run([self.run_record(**changes)], "abc"))
        self.assertEqual(release_ci.select_run([self.run_record()], "abc")["id"], 10)

    def test_latest_attempt_wins_even_when_older_attempt_succeeded(self):
        newer = self.run_record(id=20, status="in_progress", conclusion=None)
        self.assertEqual(release_ci.select_run([self.run_record(), newer], "abc"), newer)

    def test_binary_artifact_must_exist_and_not_be_expired(self):
        valid = dict(name="validated-linux-tools", expired=False)
        self.assertTrue(release_ci.has_validated_tools([valid]))
        self.assertFalse(release_ci.has_validated_tools([dict(valid, expired=True)]))
        self.assertFalse(release_ci.has_validated_tools([dict(valid, name="other")]))
        self.assertFalse(release_ci.has_validated_tools([]))

    def run_discovery(self, responses):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            with patch.dict(release_ci.os.environ, dict(
                    GITHUB_REPOSITORY="owner/repo", GITHUB_SHA="abc",
                    GITHUB_OUTPUT=str(output))), \
                    patch.object(release_ci, "api", side_effect=responses), \
                    patch.object(release_ci.time, "sleep") as sleep:
                release_ci.main()
                return output.read_text(), sleep.call_count

    def test_waits_for_validation_before_reusing_binary(self):
        output, sleeps = self.run_discovery([
            {"workflow_runs": [self.run_record(status="queued", conclusion=None)]},
            {"workflow_runs": [self.run_record()]},
            {"artifacts": [dict(name="validated-linux-tools", expired=False)]},
        ])
        self.assertEqual(output, "run-id=10\n")
        self.assertEqual(sleeps, 1)

    def test_missing_ci_or_artifacts_requires_full_validation(self):
        for responses in [[{"workflow_runs": []}],
                          [{"workflow_runs": [self.run_record()]}, {"artifacts": []}]]:
            self.assertEqual(self.run_discovery(responses), ("run-id=\n", 0))

    def test_failed_main_ci_blocks_release(self):
        with self.assertRaisesRegex(RuntimeError, "did not pass"):
            self.run_discovery([
                {"workflow_runs": [self.run_record(conclusion="failure")]}
            ])


if __name__ == "__main__":
    unittest.main()
