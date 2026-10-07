"""The release hygiene checker: every rule catches what it names, the staged check reads the index and not the
working tree, and the review tripwire skips the phrases that are not a diary."""
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "check_release_hygiene.py"
spec = importlib.util.spec_from_file_location("hygiene", SCRIPT)
hygiene = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hygiene)


def run(*args, cwd=None):
    return subprocess.run([sys.executable, str(SCRIPT), *args], cwd=cwd, capture_output=True, text=True)


class Repo:
    """A throwaway git repository the checker is pointed at with --root."""

    def __init__(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.dir.name)
        for cmd in (["init", "-q"], ["config", "user.email", "t@example.com"], ["config", "user.name", "t"]):
            subprocess.run(["git", *cmd], cwd=self.path, check=True)

    def write(self, name, text):
        target = self.path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def stage(self, name):
        subprocess.run(["git", "add", name], cwd=self.path, check=True)

    def check(self, *args):
        return run("--root", str(self.path), *args)


def findings(path, text):
    """The checker's errors and warnings for one file with this text."""
    with tempfile.TemporaryDirectory() as d:
        full = pathlib.Path(d) / path
        full.parent.mkdir(parents=True, exist_ok=True)
        full.write_text(text)
        return hygiene.check([path], d)


class PathRules(unittest.TestCase):
    def test_each_path_rule_names_what_it_rejects(self):
        for rule, path in [
            ("A1", "CLAUDE.md"), ("A2", ".claude/settings.json"), ("A3", ".github/copilot-instructions.md"),
            ("A4", "docs/prompts/x.md"), ("A5", "notes/PLAN-x.md"), ("A6", "build.log"), ("A7", "x-findings.md"),
        ]:
            bad, _ = hygiene.check([path], "/nonexistent")
            self.assertTrue(any(f" {rule} " in b for b in bad), f"{rule} on {path}: {bad}")


class TextRules(unittest.TestCase):
    def test_each_text_rule_names_what_it_rejects(self):
        for rule, line in [
            ("B1", "see /home/someone/work/file"), ("B2", "kept in .claude/state"), ("B3", "done in Phase 2"),
            ("B4", "as in the Python renderer"), ("B5", "see D12 for why"), ("B6", "not verified on the target"),
            ("B7", "decided in this session"),
        ]:
            bad, _ = findings("notes.md", line + "\n")
            self.assertTrue(any(f": {rule} " in b for b in bad), f"{rule} on {line!r}: {bad}")

    def test_a_decision_code_in_code_is_judged_only_in_a_comment(self):
        bad, _ = findings("x.rs", "let D2 = 1;\n")
        self.assertEqual(bad, [])
        bad, _ = findings("x.rs", "let a = 1; // see D2\n")
        self.assertTrue(any(": B5 " in b for b in bad), bad)

    def test_slashes_inside_a_string_or_a_url_are_not_a_comment(self):
        bad, _ = findings("x.rs", 'let u = "http://example.com/D2"; let a = "a // D2";\n')
        self.assertEqual(bad, [])
        bad, _ = findings("x.rs", 'let s = "x"; // D2 explains it\n')
        self.assertTrue(any(": B5 " in b for b in bad), bad)

    def test_the_vendored_sources_are_not_read(self):
        bad, warn = findings("vendor/gpu-allocator/src/lib.rs", "// it used to work, see Phase 2 in /home/a/b/\n")
        self.assertEqual((bad, warn), ([], []))


class Tripwire(unittest.TestCase):
    def warns(self, text):
        return bool(findings("x.md", text + "\n")[1])

    def test_a_diary_is_flagged(self):
        for text in ["The loader used to reject it.", "It no longer rejects it.", "This now passes.", "since the loader work"]:
            self.assertTrue(self.warns(text), text)

    def test_a_use_or_a_current_rule_is_not(self):
        for text in [
            "FNV-1a over bytes, used to derive element seeds from ids.",
            "a base that is used to share child enums.",
            "it is used to start the run.",
            "must depend only on records that no longer change.",
            "a solver that is no longer deterministic.",
        ]:
            self.assertFalse(self.warns(text), text)


class CommitSubjects(unittest.TestCase):
    def subject(self, text):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(text + "\n\nbody\n")
        return run("--commit-msg", f.name)

    def test_labels_of_a_phase_a_batch_an_item_or_wip_are_refused_in_any_case(self):
        for subject in ["Phase 2 work", "phase2/cosim merge", "phase-3 fix", "Batch 4", "item 5", "wip: x", "WIP x"]:
            self.assertEqual(self.subject(subject).returncode, 1, subject)

    def test_an_ordinary_subject_passes(self):
        for subject in ["fix(eval): a smoke fails", "feat: the phase of the moon", "docs: items in the list"]:
            self.assertEqual(self.subject(subject).returncode, 0, subject)


class Staged(unittest.TestCase):
    def test_the_check_reads_the_index_and_not_the_working_tree(self):
        repo = Repo()
        repo.write("a.md", "kept in /home/someone/private/\n")
        repo.stage("a.md")
        repo.write("a.md", "clean now\n")
        result = repo.check("--staged")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("B1", result.stdout)
        # and the converse: a clean index with a dirty working tree passes
        other = Repo()
        other.write("b.md", "clean\n")
        other.stage("b.md")
        other.write("b.md", "kept in /home/someone/private/\n")
        self.assertEqual(other.check("--staged").returncode, 0)


LEDGER = "tools/evidence/cinematic-impact.json"


def milestone(called="one", without=()):
    entry = {"name": called, "evidence": ["a.rs"], "validation": "v", "limits": "l"}
    for key in without:
        del entry[key]
    return entry


class Ledger(unittest.TestCase):
    """The acceptance ledger is read by a parser that refuses a repeated key: a merge that splices two lists of milestones can leave a file that loads
    and has lost an entry (the second copy of a key wins), and a file that loads is not a file that is whole."""

    def check_text(self, text):
        repo = Repo()
        repo.write(LEDGER, text)
        repo.write("a.md", "clean\n")
        repo.stage("a.md")
        return repo.check("--all")

    def check_milestones(self, milestones):
        import json
        return self.check_text(json.dumps({"milestones": milestones}, indent=2))

    def test_a_whole_ledger_passes(self):
        result = self.check_milestones([milestone("one"), milestone("two")])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_ledger_of_this_repository_is_whole(self):
        self.assertEqual(hygiene.check_ledger(str(ROOT)), [])

    def test_a_key_that_is_repeated_in_an_object_is_refused(self):
        text = '{"milestones": [{"name": "a", "evidence": [], "validation": "v", "limits": "l", "name": "b"}]}'
        result = self.check_text(text)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L1", result.stdout)
        self.assertIn("name", result.stdout)

    def test_a_milestone_that_lacks_a_field_or_a_name_is_refused(self):
        for missing in ("name", "evidence", "validation", "limits"):
            result = self.check_milestones([milestone("one"), milestone("two", without=(missing,))])
            self.assertEqual(result.returncode, 1, f"{missing}: {result.stdout}{result.stderr}")
            self.assertIn("L2", result.stdout, missing)
            self.assertIn(missing, result.stdout)

    def test_two_milestones_of_one_name_are_refused(self):
        result = self.check_milestones([milestone("one"), milestone("two"), milestone("one")])
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L3", result.stdout)

    def test_a_ledger_that_does_not_parse_is_refused_by_name(self):
        result = self.check_text('{"milestones": [')
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("L1", result.stdout)

    def test_a_repository_with_no_ledger_is_not_asked_for_one(self):
        repo = Repo()
        repo.write("a.md", "clean\n")
        repo.stage("a.md")
        self.assertEqual(repo.check("--all").returncode, 0)


if __name__ == "__main__":
    unittest.main()
