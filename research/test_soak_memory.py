"""Acceptance arithmetic must reject growth and incomplete/diagnostic evidence."""
import copy
import unittest
from assess_soak_memory import assess, STAGES


def fixture(slope):
    report = {"layers": 20, "profile": "release", "fixture_options": {"rounds": 30, "layers": 20}}
    summary = {"rounds": 30, "renderer": "Dx12 fixture", "elapsed_seconds": 1000, "idle_series": {}}
    for label in STAGES:
        summary["idle_series"][label] = {"samples": [
            {"round": i, "private_committed_mib": 500 + i * slope,
             "registered_snapshot_total_mib": 100, "display_rgba_mib": 25,
             "display_prefix_mib": 0, "history_mib": 0} for i in range(30)]}
    return report, summary


class SoakMemoryTests(unittest.TestCase):
    def test_independent_tail_arithmetic_and_no_overall_acceptance(self):
        report, summary = fixture(0.5)
        result = assess(report, summary)
        self.assertTrue(result["per_run_memory_targets_pass"])
        self.assertFalse(result["release_gate_accepted"])
        stage = result["stages"][STAGES[-1]]
        self.assertEqual(stage["private_net_mib"], 12)
        self.assertEqual(stage["private_slope_mib_per_round"], 0.5)
        self.assertEqual(stage["samples"], 25)

    def test_sustained_growth_and_diagnostic_build_cannot_pass(self):
        report, summary = fixture(1.5)
        self.assertFalse(assess(report, summary)["per_run_memory_targets_pass"])
        report, summary = fixture(0)
        for flag in ("allocation_profile_enabled", "allocation_stack_profile_enabled", "gpu_allocation_profile_enabled"):
            modified = copy.deepcopy(report)
            modified[flag] = True
            self.assertFalse(assess(modified, summary)["per_run_memory_targets_pass"])

    def test_missing_round_nonfinite_measurement_or_retained_history_is_rejected(self):
        report, summary = fixture(0)
        del summary["idle_series"][STAGES[0]]["samples"][7]
        with self.assertRaisesRegex(ValueError, "matching idle"):
            assess(report, summary)
        report, summary = fixture(0)
        summary["idle_series"][STAGES[-1]]["samples"][-1]["private_committed_mib"] = float("nan")
        with self.assertRaisesRegex(ValueError, "Invalid process"):
            assess(report, summary)
        report, summary = fixture(0)
        summary["idle_series"][STAGES[-1]]["samples"][12]["history_mib"] = 1
        self.assertFalse(assess(report, summary)["per_run_memory_targets_pass"])


class FailedSessionTests(unittest.TestCase):
    def test_explicit_failure_cannot_pass_otherwise_valid_idle_evidence(self):
        report, summary = fixture(0)
        self.assertTrue(assess(report, summary)["per_run_memory_targets_pass"])
        report["complete"] = False
        with self.assertRaisesRegex(ValueError, "Incomplete"):
            assess(report, summary)

    def test_summary_rejects_failure_before_reading_completed_workload_fields(self):
        import json
        from pathlib import Path
        import tempfile
        from analyze_native_session import summarize
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "session.incomplete.json"
            path.write_text(json.dumps({"complete": False, "error": "diagnostic timeout"}))
            with self.assertRaisesRegex(ValueError, "Incomplete"):
                summarize(path)


if __name__ == "__main__":
    unittest.main()
