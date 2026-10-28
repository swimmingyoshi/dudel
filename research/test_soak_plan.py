"""A short or mismatched native run cannot satisfy the requested long workload."""
import copy
import argparse
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

spec = importlib.util.spec_from_file_location("soak_controller_for_test", Path(__file__).resolve().parents[1] / "scripts/run_soak.py")
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


def fixture():
    args = SimpleNamespace(rounds=30, layers=32, density="noise", minimum_seconds=7200, brushes="mixed")
    report = {"rounds": 43, "layers": 32, "fixture_options": {"density": "Noise"},
              "elapsed_seconds": 7202, "observed_session_seconds": 7201.005, "configured_minimum_rounds": 30,
              "session_plan": {"minimum_seconds": 7200, "mixed_brushes": True},
              "workload_brushes": [{"name": name} for name in ["G-pen", "Pencil", "QA textured stamp"]],
              "generated_brush_samples_per_round": 240,
              "frames": [{"callback_interval_ms": 1000, "diagnostic_driver_ms": 5,
                          "ui_update_before_diagnostic_ms": 5} for _ in range(7202)]}
    return report, args


class SoakPlanTests(unittest.TestCase):
    def test_buffered_frame_logging_keeps_complete_continuity_and_visible_io_overhead(self):
        report, args = fixture()
        report.update(version=3, frame_samples_recorded=len(report["frames"]),
                      diagnostic_final_frame_log_ms=0)
        for frame in report["frames"]:
            frame["diagnostic_previous_frame_log_ms"] = 0
        controller.validate_requested_plan(report, args)
        report["frames"][-1]["callback_interval_ms"] += 29000
        report["observed_session_seconds"] += 29
        report["elapsed_seconds"] += 29
        with self.assertRaisesRegex(ValueError, "gap"):
            controller.validate_requested_plan(report, args)
        report["frames"][-1]["diagnostic_previous_frame_log_ms"] = 29000
        controller.validate_requested_plan(report, args)
        for broken in [dict(report, frame_samples_recorded=0),
                       dict(report, diagnostic_final_frame_log_ms=-1)]:
            with self.assertRaises(ValueError):
                controller.validate_requested_plan(broken, args)
        invalid_final = copy.deepcopy(report)
        invalid_final["frames"][-1]["diagnostic_driver_ms"] = -1
        invalid_final["diagnostic_final_frame_log_ms"] = 1
        with self.assertRaisesRegex(ValueError, "Invalid final"):
            controller.validate_requested_plan(invalid_final, args)
        del report["frames"][-1]["diagnostic_previous_frame_log_ms"]
        with self.assertRaisesRegex(ValueError, "Incomplete buffered"):
            controller.validate_requested_plan(report, args)

    def test_short_or_mismatched_duration_cannot_satisfy_a_long_request(self):
        report, args = fixture()
        controller.validate_requested_plan(report, args)
        for change in [{"rounds": 29}, {"elapsed_seconds": 7199}, {"observed_session_seconds": 7199}, {"configured_minimum_rounds": 29},
                       {"session_plan": {"minimum_seconds": 60, "mixed_brushes": True}}]:
            with self.assertRaises(ValueError):
                controller.validate_requested_plan(dict(report, **change), args)

    def test_mixed_request_cannot_be_satisfied_by_a_controlled_or_incomplete_schedule(self):
        report, args = fixture()
        for change in [{"session_plan": {"minimum_seconds": 7200, "mixed_brushes": False}},
                       {"workload_brushes": report["workload_brushes"][:2]},
                       {"generated_brush_samples_per_round": 0}]:
            with self.assertRaises(ValueError):
                controller.validate_requested_plan(dict(report, **change), args)

    def test_default_request_remains_compatible_and_duration_input_is_bounded(self):
        report, args = fixture()
        args.minimum_seconds = 0
        args.brushes = "controlled"
        del report["session_plan"]
        controller.validate_requested_plan(report, args)
        wrong = copy.deepcopy(report)
        wrong["fixture_options"]["density"] = "Sparse"
        with self.assertRaises(ValueError):
            controller.validate_requested_plan(wrong, args)
        for value in ["0", "60", "7200", "21600"]:
            self.assertEqual(controller.duration_seconds(value), int(value))
        for value in ["-1", "59", "21601"]:
            with self.assertRaises(argparse.ArgumentTypeError):
                controller.duration_seconds(value)

    def test_continuity_rejects_a_pause_but_accounts_for_known_diagnostic_work(self):
        report, args = fixture()
        report["frames"][-1]["callback_interval_ms"] += 29000
        report["observed_session_seconds"] += 29
        report["elapsed_seconds"] += 29
        with self.assertRaisesRegex(ValueError, "gap"):
            controller.validate_requested_plan(report, args)
        report["frames"][-2]["diagnostic_driver_ms"] += 29000
        controller.validate_requested_plan(report, args)
        report["observed_session_seconds"] += 2
        with self.assertRaisesRegex(ValueError, "span"):
            controller.validate_requested_plan(report, args)

    def test_execution_request_restores_prior_flags_even_when_work_fails(self):
        calls = []
        def setter(flags):
            calls.append(flags)
            return 0x80000000
        with self.assertRaisesRegex(RuntimeError, "work interrupted"):
            with controller.execution_request(True, setter=setter) as state:
                self.assertTrue(state["requested"])
                raise RuntimeError("work interrupted")
        self.assertEqual(calls, [0x80000003, 0x80000000])
        self.assertTrue(state["restored"])
        with controller.execution_request(False, setter=setter) as disabled:
            self.assertFalse(disabled["requested"])
        self.assertEqual(len(calls), 2)

    def test_suspend_time_cannot_count_as_native_work_even_inside_a_driver_step(self):
        start = {"biased_seconds": 100, "awake_seconds": 90}
        controller.clock_delta(start, {"biased_seconds": 200, "awake_seconds": 190})
        with self.assertRaisesRegex(ValueError, "suspension"):
            controller.clock_delta(start, {"biased_seconds": 800, "awake_seconds": 190})

    def test_small_probe_cannot_be_presented_as_resource_acceptance(self):
        args = SimpleNamespace(rounds=2, layers=4, probe_only=False, require_job_peaks=False)
        with self.assertRaises(ValueError):
            controller.validate_mode(args)
        args.probe_only = True
        controller.validate_mode(args)
        args.require_job_peaks = True
        with self.assertRaises(ValueError):
            controller.validate_mode(args)


if __name__ == "__main__":
    unittest.main()
