"""Run isolated native memory sessions sequentially from one frozen executable."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys

REPOSITORY = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPOSITORY / "research"))
from analyze_native_session import summarize
from assess_soak_memory import assess
from assess_soak_jobs import assess as assess_jobs


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def write_state(folder, state):
    pending = folder / "STATE.pending.json"
    pending.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    pending.replace(folder / "STATE.json")


def duration_seconds(value):
    seconds = int(value)
    if seconds != 0 and not 60 <= seconds <= 21600:
        raise argparse.ArgumentTypeError("Duration must be 0 or 60..21600 seconds")
    return seconds


@contextmanager
def execution_request(enabled, setter=None):
    info = {"requested": False, "restored": True, "flags": 0x80000003}
    if not enabled:
        yield info
        return
    if setter is None:
        if os.name != "nt":
            raise RuntimeError("Native soak execution requests require Windows")
        import ctypes
        from ctypes import wintypes
        setter = ctypes.WinDLL("kernel32", use_last_error=True).SetThreadExecutionState
        setter.argtypes = [wintypes.DWORD]
        setter.restype = wintypes.DWORD
    previous = setter(info["flags"])
    if not previous:
        raise RuntimeError("Could not request temporary system/display activity")
    info.update(requested=True, restored=False)
    try:
        yield info
    finally:
        info["restored"] = bool(setter(previous))


def system_clock():
    import ctypes
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    biased = kernel.GetTickCount64
    biased.argtypes = []
    biased.restype = ctypes.c_ulonglong
    awake = kernel.QueryUnbiasedInterruptTime
    awake.argtypes = [ctypes.POINTER(ctypes.c_ulonglong)]
    awake.restype = wintypes.BOOL
    biased_seconds = biased() / 1000.0
    counter = ctypes.c_ulonglong()
    if not awake(ctypes.byref(counter)):
        raise ctypes.WinError(ctypes.get_last_error())
    return {"biased_seconds": biased_seconds, "awake_seconds": counter.value / 10_000_000.0}


def clock_delta(start, end):
    wall = end["biased_seconds"] - start["biased_seconds"]
    awake = end["awake_seconds"] - start["awake_seconds"]
    if not all(math.isfinite(value) and value >= 0 for value in (wall, awake)):
        raise ValueError("Invalid native run clock evidence")
    suspension = max(0, wall - awake)
    if suspension > 2:
        raise ValueError(f"System suspension invalidates the native run ({suspension:.2f}s)")
    return {"biased_elapsed_seconds": wall, "awake_elapsed_seconds": awake,
            "estimated_suspend_seconds": suspension, "suspension_tolerance_seconds": 2}


def callback_continuity(report):
    frames = report.get("frames", [])
    if len(frames) < 2:
        raise ValueError("Insufficient continuous callback evidence")
    if report.get("version", 0) >= 3 and (
            report.get("frame_samples_recorded") != len(frames)
            or "diagnostic_final_frame_log_ms" not in report
            or any("diagnostic_previous_frame_log_ms" not in f for f in frames)):
        raise ValueError("Incomplete buffered frame logging evidence")
    residuals = []
    final_logging = report.get("diagnostic_final_frame_log_ms", 0)
    final_driver = frames[-1]["diagnostic_driver_ms"]
    if not all(math.isfinite(value) and value >= 0 for value in (final_driver, final_logging)):
        raise ValueError("Invalid final diagnostic duration")
    traced_ms = final_driver + final_logging
    for before, current in zip(frames, frames[1:]):
        values = [current["callback_interval_ms"], before["diagnostic_driver_ms"],
                  current["ui_update_before_diagnostic_ms"],
                  current.get("diagnostic_previous_frame_log_ms", 0)]
        if not all(math.isfinite(value) and value >= 0 for value in values):
            raise ValueError("Invalid callback continuity values")
        traced_ms += values[0]
        residuals.append(max(0, values[0] - values[1] - values[2] - values[3]))
    if abs(traced_ms / 1000.0 - report["observed_session_seconds"]) > 1:
        raise ValueError("Callback trace does not span the observed native session")
    if max(residuals) > 2000:
        raise ValueError("Long unexplained callback gap invalidates continuity")
    return {"maximum_unexplained_callback_ms": max(residuals), "gap_limit_ms": 2000,
            "traced_seconds": traced_ms / 1000.0,
            "scope": "Continuity only; subtracts known diagnostic driver, buffered logging and current UI prefix. Not frame/queue performance or physical latency acceptance."}


def validate_requested_plan(report, args):
    if (report["rounds"] < args.rounds or report["layers"] != args.layers
            or report.get("fixture_options", {}).get("density") != args.density.capitalize()):
        raise ValueError("Completed fixture differs from the requested workload")
    plan = report.get("session_plan", {})
    if args.minimum_seconds and (not math.isfinite(report["elapsed_seconds"])
            or report["elapsed_seconds"] < args.minimum_seconds
            or not math.isfinite(report.get("observed_session_seconds", 0))
            or report.get("observed_session_seconds", 0) < args.minimum_seconds
            or plan.get("minimum_seconds") != args.minimum_seconds
            or report.get("configured_minimum_rounds") != args.rounds):
        raise ValueError("Requested minimum duration/configuration was not met")
    if args.brushes == "mixed" and (not plan.get("mixed_brushes") or report["rounds"] < 3
            or [brush["name"] for brush in report.get("workload_brushes", [])]
                != ["G-pen", "Pencil", "QA textured stamp"]
            or report.get("generated_brush_samples_per_round") != 240):
        raise ValueError("Requested mixed-brush workload was not verified")
    return callback_continuity(report) if args.minimum_seconds else None


def validate_mode(args):
    if args.probe_only:
        if args.require_job_peaks:
            raise ValueError("Probe-only mode cannot evaluate resource acceptance")
    elif args.rounds < 30 or args.layers < 20:
        raise ValueError("Acceptance workloads need at least 30 rounds and 20 layers; use --probe-only for a small diagnostic")


def run(args):
    validate_mode(args)
    owned = {}
    try:
        with execution_request(True) as power:
            _run(args, power, owned)
    except BaseException as error:
        process = owned.get("process")
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if owned and owned["state"]["status"] in ("starting", "running"):
            owned["state"].update(status="controller-failed", pid=None, error=str(error))
        raise
    finally:
        if owned:
            write_state(owned["folder"], owned["state"])


def _run(args, power, owned):
    folder = args.output.resolve()
    if not folder.is_relative_to(REPOSITORY / "target" / "qa"):
        raise ValueError("Soak outputs must be inside this repository's target/qa")
    build = json.loads(args.build.read_text(encoding="utf-8-sig"))
    if sha(args.executable) != build["executable_sha256"]:
        raise ValueError("Executable does not match build identity")
    folder.mkdir(parents=True, exist_ok=False)
    executable = folder / "dudel.exe"
    shutil.copy2(args.executable, executable)
    (folder / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n", encoding="utf-8")
    state = {"status": "starting", "runs": args.runs, "rounds": args.rounds,
             "layers": args.layers, "density": args.density, "completed": [],
             "require_job_peaks": args.require_job_peaks,
             "minimum_seconds": args.minimum_seconds, "brushes": args.brushes,
             "probe_only": args.probe_only,
             "power_request": power,
             "frozen_executable_sha256": build["executable_sha256"]}
    owned.update(folder=folder, state=state)
    # Dense undo/redo verification needs a longer observation window; product
    # peak-memory and completed save/open acceptance budgets remain unchanged.
    per_round = 120 if args.density == "sparse" else 300
    native_window = max(args.rounds * per_round + 600, args.minimum_seconds + per_round + 600)
    timeout_seconds = max(args.rounds * (180 if args.density == "sparse" else 300) + 900,
                          native_window + 300)
    state["controller_timeout_seconds"] = timeout_seconds
    environment = dict(os.environ, DUDEL_RENDERER="dx12", DUDEL_SOAK_ROUNDS=str(args.rounds),
                       DUDEL_SOAK_LAYERS=str(args.layers), DUDEL_SOAK_DENSITY=args.density,
                       DUDEL_SOAK_MIN_SECONDS=str(args.minimum_seconds), DUDEL_SOAK_BRUSHES=args.brushes)
    for number in range(1, args.runs + 1):
        report = folder / f"run-{number}-sustained-100.json"
        stdout, stderr = folder / f"run-{number}.stdout.log", folder / f"run-{number}.stderr.log"
        print(f"Starting native run {number}/{args.runs}: {args.layers} layers, {args.rounds} rounds, {args.density}", flush=True)
        clock_started = system_clock()
        with stdout.open("wb") as out, stderr.open("wb") as err:
            process = subprocess.Popen([str(executable), "--soak-native-drawing-session", str(report), "1280", "800", "1"],
                cwd=REPOSITORY, env=environment, stdout=out, stderr=err,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            owned["process"] = process
            state.update(status="running", run=number, pid=process.pid,
                         launched_utc=datetime.now(timezone.utc).isoformat(), report=str(report), stderr=str(stderr))
            write_state(folder, state)
            try:
                code = process.wait(timeout=timeout_seconds)
            except BaseException as error:
                if process.poll() is None:
                    process.kill()
                process.wait()
                state.update(status="controller-interrupted", pid=None,
                             exit_code=process.returncode, error=str(error))
                write_state(folder, state)
                raise
        state.update(pid=None, exit_code=code)
        if code != 0 or not report.is_file():
            state["status"] = "failed"
            write_state(folder, state)
            raise RuntimeError(f"Native run {number} failed; inspect {stderr}. It was not restarted.")
        try:
            clock_finished = system_clock()
            summary = summarize(report)
            raw = json.loads(report.read_text(encoding="utf-8"))
            clocks = clock_delta(clock_started, clock_finished)
            continuity = validate_requested_plan(raw, args)
            if args.probe_only:
                result = {"per_run_memory_targets_pass": None, "release_gate_accepted": False,
                          "scope": "Probe only; resource acceptance was not evaluated."}
                jobs = {"per_run_job_peak_targets_pass": None, "release_gate_accepted": False,
                        "scope": "Probe only; resource acceptance was not evaluated."}
            else:
                result = assess(raw, summary)
                jobs = assess_jobs(raw)
        except Exception as error:
            state.update(status="assessment-failed", error=str(error))
            write_state(folder, state)
            raise
        (folder / f"run-{number}.summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        (folder / f"run-{number}.memory.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        (folder / f"run-{number}.jobs-peaks.json").write_text(json.dumps(jobs, indent=2) + "\n", encoding="utf-8")
        (folder / f"run-{number}.continuity.json").write_text(json.dumps({"clocks": clocks, "callbacks": continuity}, indent=2) + "\n", encoding="utf-8")
        state["completed"].append({"run": number, "elapsed_seconds": summary["elapsed_seconds"],
                                   "actual_rounds": summary["rounds"], "requested_plan_verified": True,
                                   "per_run_memory_targets_pass": result["per_run_memory_targets_pass"],
                                   "per_run_job_peak_targets_pass": jobs["per_run_job_peak_targets_pass"]})
        if not args.probe_only and not result["per_run_memory_targets_pass"]:
            state["status"] = "memory-targets-not-met"
            write_state(folder, state)
            raise RuntimeError("Memory targets not met; remaining repeats deferred for investigation")
        if args.require_job_peaks and not jobs["per_run_job_peak_targets_pass"]:
            state["status"] = "job-peak-targets-not-met"
            write_state(folder, state)
            raise RuntimeError("Completed-job/peak targets not met; inspect jobs-peaks.json; remaining repeats deferred")
        write_state(folder, state)
    state["status"] = "probe-completed" if args.probe_only else "completed"
    write_state(folder, state)
    print("Native probe completed; resource acceptance was not evaluated." if args.probe_only else
          "Sequential native checks completed. This does not certify all release gates.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rounds", type=int, choices=range(2, 241), default=30)
    parser.add_argument("--layers", type=int, choices=range(4, 33), default=20)
    parser.add_argument("--density", choices=("sparse", "pattern", "noise"), default="sparse")
    parser.add_argument("--minimum-seconds", type=duration_seconds, default=0,
                        help="Continue complete rounds until count and elapsed minimum are met; at most 240 rounds")
    parser.add_argument("--brushes", choices=("controlled", "mixed"), default="controlled")
    parser.add_argument("--runs", type=int, choices=range(1, 4), default=3)
    parser.add_argument("--probe-only", action="store_true",
                        help="Small semantic/instrumentation probe; never evaluates resource acceptance")
    parser.add_argument("--require-job-peaks", action="store_true",
                        help="Require completed warm save/open and lifetime-private-peak budgets as well as idle growth")
    args = parser.parse_args()
    try:
        validate_mode(args)
    except ValueError as error:
        parser.error(str(error))
    run(args)


if __name__ == "__main__":
    main()
