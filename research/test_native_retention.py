"""Retention evidence must use the default-heap walk, not heap ordering."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from analyze_native_retention import analyze, SIZE_KEY


class RetentionTests(unittest.TestCase):
    def fixture(self, directory):
        path = Path(directory) / "session.json"
        points = []
        walks = []
        for i in range(4):
            points.append({
                "label": "final-idle-history-released", "round": i,
                "history_retained_bytes": 0,
                "memory_regions": {"heaps_omitted": 0, "heaps": [{"allocated_bytes": 99999999}]},
                "process_memory": {"private_committed_bytes": (10 + 2 * i) * 1048576},
                "rust_allocations": {"live_requested_bytes": 1048576},
                "gpu_allocations": None,
            })
            walks.append({"round": i, "walked": True, "overflow_blocks": 0,
                          "busy_bytes": (2 + i) * 1048576,
                          "slots": [{"key": SIZE_KEY, "count": i * 10}]})
        report = {"rounds": 4, "renderer": "fixture", "checkpoints": points, "elapsed_seconds": 1}
        path.write_text(json.dumps(report), encoding="utf-8")
        path.with_suffix(".heapwalk.json").write_text(json.dumps({"walks": walks}), encoding="utf-8")
        return path, walks

    @patch("analyze_native_retention.summarize", return_value={"mask_edits_verified": 64, "brush_strokes_verified": 4})
    def test_tail_arithmetic_uses_walked_default_heap(self, _validated):
        with tempfile.TemporaryDirectory() as directory:
            path, _ = self.fixture(directory)
            result = analyze(path, warmup=1)
            self.assertEqual(result["tail"]["default_heap_busy_mib"], {"net": 2, "slope_per_round": 1})
            self.assertEqual(result["tail"]["private_mib"], {"net": 4, "slope_per_round": 2})
            self.assertEqual(result["tail"]["size_184_blocks"], {"net": 20, "slope_per_round": 10})

    @patch("analyze_native_retention.summarize", return_value={"mask_edits_verified": 64, "brush_strokes_verified": 4})
    def test_partial_or_overflowing_walk_cannot_become_evidence(self, _validated):
        with tempfile.TemporaryDirectory() as directory:
            path, walks = self.fixture(directory)
            walks[2]["overflow_blocks"] = 1
            path.with_suffix(".heapwalk.json").write_text(json.dumps({"walks": walks}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "overflowing"):
                analyze(path, warmup=1)
            walks[2]["overflow_blocks"] = 0
            path.with_suffix(".heapwalk.json").write_text(json.dumps({"walks": walks[:-1]}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "every round"):
                analyze(path, warmup=1)


if __name__ == "__main__":
    unittest.main()
