"""The minimum compiler must cover every locked dependency and workspace crate."""
import json
import pathlib
import subprocess
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class MinimumRustTests(unittest.TestCase):
    def test_locked_dependencies_fit_declared_minimum(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text())
        minimum = tuple(map(int, root["workspace"]["package"]["rust-version"].split(".")[:2]))
        data = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
        for package in data["packages"]:
            version = package.get("rust_version")
            if version:
                self.assertLessEqual(tuple(map(int, version.split(".")[:2])), minimum, package["name"])
        for member in root["workspace"]["members"]:
            manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
            self.assertEqual(manifest["package"].get("rust-version"), {"workspace": True}, member)
