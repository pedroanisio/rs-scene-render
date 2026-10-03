"""Check generator categories without regenerating the checked-in corpus."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_corpus


class CorpusCategoriesTests(unittest.TestCase):
    def test_oracle_codes_exclude_rust_only_warnings_and_asset_checks(self):
        self.assertEqual(build_corpus.oracle_codes(["FRX4", "W01"]), ["FRX4"])
        self.assertEqual(build_corpus.oracle_codes(["S06", "A01", "W01", "FRX4"]), ["FRX4", "XSD"])
        self.assertEqual(build_corpus.oracle_codes(["S06", "FRX4"], blind=True), ["FRX4"])

    def test_case_names_are_unique_across_categories(self):
        names = list(build_corpus.VALID)
        for cases in (build_corpus.CASES, build_corpus.ASSET_CASES, build_corpus.WARN_CASES):
            names.extend(name for name, _, _ in cases)
        self.assertEqual(len(names), len(set(names)), "a category would overwrite another case")

    def test_asset_and_warning_cases_are_schema_valid(self):
        for name, _, mutate in build_corpus.ASSET_CASES + build_corpus.WARN_CASES:
            with self.subTest(case=name):
                self.assertEqual(build_corpus.verdict(mutate(build_corpus.base).encode()), ([], []))


if __name__ == "__main__":
    unittest.main()
