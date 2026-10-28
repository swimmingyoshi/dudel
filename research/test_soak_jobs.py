import copy
import unittest
from assess_soak_jobs import assess


def fixture():
    return {"rounds": 5, "layers": 20, "canvas": [2048, 2048], "profile": "release",
            "fixture_options": {"density": "Sparse"}, "initial_raster_and_layer_mask_payload_bytes": 64 * 1024**2,
            "checkpoints": [{"process_memory": {"private_committed_bytes": 500 * 1024**2,
                              "peak_private_committed_bytes": (700 + i) * 1024**2}} for i in range(5)],
            "completed_jobs": [{"round": i, "operation": operation, "command_to_editor_ready_ms": 1000 + i}
                               for i in range(5) for operation in ("Save", "Open")]}


class SoakJobTests(unittest.TestCase):
    def test_full_samples_pass_warm_subtargets_without_certifying_release(self):
        result = assess(fixture())
        self.assertTrue(result["per_run_job_peak_targets_pass"])
        self.assertEqual(result["completed_jobs_ms"]["Save"]["p95_nearest_rank"], 1004)
        self.assertEqual(result["private_commit_lifetime_peak_bytes"], 704 * 1024**2)
        self.assertFalse(result["release_gate_accepted"])

    def test_transient_peak_and_completed_wall_time_can_fail_independently(self):
        report = fixture()
        for point in report["checkpoints"]:
            point["process_memory"]["peak_private_committed_bytes"] = 3 * 1024**3
        result = assess(report)
        self.assertFalse(result["peak_target_pass"])
        self.assertTrue(result["completed_job_target_pass"])
        report = fixture()
        report["completed_jobs"][-1]["command_to_editor_ready_ms"] = 6000
        result = assess(report)
        self.assertTrue(result["peak_target_pass"])
        self.assertFalse(result["completed_job_target_pass"])

    def test_missing_duplicate_nonfinite_and_nonmonotonic_data_cannot_pass(self):
        historical = fixture()
        del historical["completed_jobs"]
        self.assertFalse(assess(historical)["available"])
        for kind in ("missing", "duplicate", "nonfinite", "decreasing"):
            report = fixture()
            if kind == "missing": report["completed_jobs"].pop()
            elif kind == "duplicate": report["completed_jobs"][-1] = copy.deepcopy(report["completed_jobs"][0])
            elif kind == "nonfinite": report["completed_jobs"][0]["command_to_editor_ready_ms"] = float("nan")
            else: report["checkpoints"][-1]["process_memory"]["peak_private_committed_bytes"] = 600 * 1024**2
            with self.assertRaises(ValueError, msg=kind): assess(report)

    def test_fixture_and_profiling_requirements_prevent_false_acceptance(self):
        for key, value in (("layers", 4), ("profile", "debug"), ("allocation_profile_enabled", True),
                           ("initial_raster_and_layer_mask_payload_bytes", 513 * 1024**2)):
            report = fixture(); report[key] = value
            self.assertFalse(assess(report)["per_run_job_peak_targets_pass"], key)
        report = fixture(); report["layers"] = 32; report["fixture_options"]["density"] = "Noise"
        self.assertTrue(assess(report)["per_run_job_peak_targets_pass"])


class FailedSessionTests(unittest.TestCase):
    def test_explicit_failure_cannot_pass_otherwise_valid_job_and_peak_evidence(self):
        report = fixture()
        self.assertTrue(assess(report)["per_run_job_peak_targets_pass"])
        report["complete"] = False
        with self.assertRaisesRegex(ValueError, "Incomplete"):
            assess(report)


if __name__ == "__main__": unittest.main()
