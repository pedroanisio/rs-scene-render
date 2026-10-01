"""The vendored gpu-allocator keeps its one change and is the copy the build uses."""
import pathlib
import re
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class VendoredAllocatorTests(unittest.TestCase):
    def test_the_build_uses_the_vendored_copy(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(root["patch"]["crates-io"]["gpu-allocator"], {"path": "vendor/gpu-allocator"})
        self.assertIn("vendor", root["workspace"]["exclude"])
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        entries = [p for p in lock["package"] if p["name"] == "gpu-allocator"]
        vendored = tomllib.loads((ROOT / "vendor/gpu-allocator/Cargo.toml").read_text())["package"]["version"]
        self.assertEqual([(p["version"], p.get("source")) for p in entries], [(vendored, None)])

    def test_uploads_do_not_prefer_device_local_memory(self):
        source = (ROOT / "vendor/gpu-allocator/src/vulkan/mod.rs").read_text()
        preferred = re.search(r"let mem_loc_preferred_bits = match desc\.location \{(.*?)\n        \};", source, re.S)
        self.assertIsNotNone(preferred, "the preferred memory types of Allocator::allocate were not found")
        arm = re.search(r"MemoryLocation::CpuToGpu => \{(.*?)\}", preferred.group(1), re.S)
        self.assertIsNotNone(arm, "no CpuToGpu arm")
        self.assertIn("HOST_VISIBLE", arm.group(1))
        self.assertNotIn("DEVICE_LOCAL", arm.group(1))


if __name__ == "__main__":
    unittest.main()
