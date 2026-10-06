import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "gpu_shards", Path(__file__).resolve().parents[1] / "gpu_shards.py"
)
gpu_shards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gpu_shards)


class GpuShardsTest(unittest.TestCase):
    def test_every_integration_target_runs_exactly_once(self):
        names = [f"suite_{i:02}" for i in range(61)]
        shards = [gpu_shards.select_targets(names, i, 6) for i in range(6)]
        flattened = [name for shard in shards for name in shard]
        self.assertCountEqual(flattened, names)
        self.assertEqual(len(flattened), len(set(flattened)))
        self.assertLessEqual(max(map(len, shards)) - min(map(len, shards)), 1)

    def test_selection_is_independent_of_metadata_order(self):
        names = ["volume", "backends", "effects", "water"]
        self.assertEqual(gpu_shards.select_targets(names, 0, 2),
                         gpu_shards.select_targets(names[::-1], 0, 2))

    def test_invalid_shard_is_rejected(self):
        for index, count in [(-1, 6), (6, 6), (0, 0)]:
            with self.assertRaises(ValueError):
                gpu_shards.select_targets(["backends"], index, count)


if __name__ == "__main__":
    unittest.main()
